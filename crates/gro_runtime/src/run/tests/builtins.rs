//! Tests for the builtins, from source in a debug build (whose runtime
//! checks every extension module was evaluated): `process.args`, the
//! object's shape and its hardening, the grant check, and what a run
//! records of a builtin import.

use super::super::{RunError, run_recorded};
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};
use crate::loader::graph::{FailureReason, Graph, LoadFailure, Reach, Reached, Record, Recording};
use crate::resolver::{BUILTINS, Origin, ResolveErrorKind};
use crate::test_support::TestDir;
use crate::{Authority, Grants, ProcessGrant, RuntimeOptions, STACK_SIZE};

const PROCESS: &str = "@fuzdev/gro/process.ts";

/// `fgro run`'s authority: `process` granted.
fn granted() -> Authority {
    Authority::Confined(Grants {
        process: Some(ProcessGrant {}),
    })
}

/// Writes `files` into a fresh directory named for `name` under a
/// `package.json` of a module project, runs `main.mjs` with `options`,
/// recording in full, and returns the run's result and graph.
fn run_files(
    name: &str,
    files: &[(&str, &str)],
    options: RuntimeOptions,
) -> (Result<(), RunError>, Graph) {
    run_linked(name, files, &[], options)
}

/// Runs as [`run_files`] does, `files` replacing the default `package.json`
/// when they hold one, with each of `links`, a path and its target, made a
/// symlink.
fn run_linked(
    name: &str,
    files: &[(&str, &str)],
    links: &[(&str, &str)],
    options: RuntimeOptions,
) -> (Result<(), RunError>, Graph) {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let test_dir = TestDir::on_disk(&format!("builtin_{name}"));
    let dir = test_dir.path().to_owned();
    let package_json = [("package.json", r#"{"name": "fixture", "type": "module"}"#)];
    for (file, content) in package_json.iter().chain(files) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    for (link, target) in links {
        let path = dir.join(link);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, path).unwrap();
    }
    let entry = dir.join("main.mjs");
    let (result, graph) = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || run_recorded(engine, &options, &entry, &dir, Recording::Full))
        .unwrap()
        .join()
        .unwrap();
    (result, graph.unwrap())
}

/// Runs `source` as `main.mjs` granted `process`, with `args`.
fn run_granted(name: &str, source: &str, args: &[&str]) -> (Result<(), RunError>, Graph) {
    let options = RuntimeOptions {
        authority: granted(),
        args: args.iter().map(ToString::to_string).collect(),
        ..RuntimeOptions::default()
    };
    run_files(name, &[("main.mjs", source)], options)
}

/// `process.args` is the script's arguments as given, whatever they look
/// like, through a named, a namespace, and a dynamic import alike.
#[test]
fn process_args_are_the_script_s_arguments() {
    let cases: [&[&str]; 5] = [
        &[],
        &["a", "b c", "d"],
        &["--help", "-A", "--", "--allow-env=A", "-"],
        &["", " ", ""],
        &["\u{e9}", "\u{1f600}", "\u{65e5}\u{672c}"],
    ];
    for (i, args) in cases.into_iter().enumerate() {
        let expected = serde_json::to_string(args).unwrap();
        let source = format!(
            "import {{ process }} from '{PROCESS}';
import * as ns from '{PROCESS}';
const dynamic = await import('{PROCESS}');
const expected = JSON.stringify({expected});
for (const [form, value] of [['named', process], ['namespace', ns.process], ['dynamic', dynamic.process]]) {{
\tif (JSON.stringify(value.args) !== expected) throw new Error(`${{form}}: ${{JSON.stringify(value.args)}}`);
}}
if (ns.process !== process || dynamic.process !== process) throw new Error('not one object');
"
        );
        let (result, _) = run_granted(&format!("args_{i}"), &source, args);
        assert!(result.is_ok(), "{args:?}: {result:?}");
    }
}

/// Each builtin's namespace has the table's named exports for it and a
/// `default` of `undefined`, every row of the table checked; `process` is
/// frozen and null-prototype, its only property `args`, an own data
/// property holding a frozen array of strings.
#[test]
fn the_namespace_and_the_object_have_their_shape() {
    let rows = serde_json::to_string(
        &BUILTINS
            .iter()
            .map(|builtin| {
                let mut exports = builtin.exports.to_vec();
                exports.push("default");
                exports.sort_unstable();
                (builtin.specifier, exports)
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let source = format!(
        "import default_export, {{ process }} from '{PROCESS}';
const check = (what, ok) => {{ if (!ok) throw new Error(what); }};
for (const [specifier, exports] of {rows}) {{
\tconst ns = await import(specifier);
\tcheck(`${{specifier}} exports`, JSON.stringify(Reflect.ownKeys(ns).filter((key) => typeof key === 'string')) === JSON.stringify(exports));
\tcheck(`${{specifier}} default`, ns.default === undefined && 'default' in ns);
}}
check('default', default_export === undefined);
check('process frozen', Object.isFrozen(process));
check('process null-prototype', Object.getPrototypeOf(process) === null);
check('process keys', JSON.stringify(Reflect.ownKeys(process)) === '[\"args\"]');
const descriptor = Object.getOwnPropertyDescriptor(process, 'args');
check('args data', 'value' in descriptor && !descriptor.writable && !descriptor.configurable && descriptor.enumerable);
check('args array', Array.isArray(process.args) && Object.getPrototypeOf(process.args) === Array.prototype);
check('args frozen', Object.isFrozen(process.args));
check('args strings', process.args.every((arg) => typeof arg === 'string'));
check('then', !('then' in process));
let threw = false;
try {{ process.args = []; }} catch {{ threw = true; }}
check('assign', threw && process.args.length === 2);
"
    );
    let (result, _) = run_granted("shape", &source, &["x", "y"]);
    assert!(result.is_ok(), "{result:?}");
}

/// User code that runs before the first import of `process` and patches
/// the intrinsics (setters on `Array.prototype` and `Object.prototype`, a
/// `then`, the global `Object`'s functions) changes nothing the builtin
/// makes, and calls none of the patches; the internals, the op among them,
/// stay unreachable before and after the import, by any own property or
/// prototype from the global object, the namespace, and the object.
#[test]
fn patched_intrinsics_change_nothing_and_the_internals_stay_unreachable() {
    let source = format!(
        "let log = '';
const note = (what) => {{ log += `${{what}};`; }};
const internals = () => [typeof Deno, typeof __bootstrap, typeof globalThis.Deno, typeof globalThis.__bootstrap].join();
const before = internals();
const keys = ['0', '1', 'args', 'process', 'default', 'length'];
const define = Object.defineProperty;
const remove = Reflect.deleteProperty;
const patched = [[Object, ['assign', 'create', 'freeze', 'defineProperty', 'hasOwn', 'keys']], [Array, ['from', 'of', 'isArray']], [Array.prototype, ['push', 'slice', 'map']]];
// read before the prototypes are patched, since a write to a new index would reach a setter
const originals = patched.flatMap(([owner, names]) => names.map((name) => [owner, name, owner[name]]));
for (const proto of [Array.prototype, Object.prototype]) {{
\tfor (const key of keys) {{
\t\tif (proto === Array.prototype && key === 'length') continue;
\t\tdefine(proto, key, {{
\t\t\tget() {{ note(`get ${{key}}`); return 'evil'; }},
\t\t\tset(value) {{ note(`set ${{key}}`); }},
\t\t\tconfigurable: true
\t\t}});
\t}}
}}
Object.prototype.then = function () {{ note('then'); }};
for (const [owner, name, original] of originals) {{
\towner[name] = function (...args) {{ note(name); return Reflect.apply(original, this, args); }};
}}
const {{ process }} = await import('{PROCESS}');
const recorded = log;
for (const [owner, name, original] of originals) owner[name] = original;
remove(Object.prototype, 'then');
for (const proto of [Array.prototype, Object.prototype]) for (const key of keys) if (!(proto === Array.prototype && key === 'length')) remove(proto, key);
if (recorded !== '') throw new Error(`patches called: ${{recorded}}`);
if (JSON.stringify(process.args) !== '[\"a\",\"b\"]' || !Object.isFrozen(process.args) || !Object.isFrozen(process))
\tthrow new Error(`args changed: ${{JSON.stringify(process.args)}}`);
if (Reflect.ownKeys(process.args).length !== 3) throw new Error(`args keys: ${{Reflect.ownKeys(process.args).length}}`);
const after = internals();
if (before !== 'undefined,undefined,undefined,undefined' || after !== before) throw new Error(`internals: ${{before}} ${{after}}`);
const ns = await import('{PROCESS}');
const seen = new Set();
const queue = [globalThis, ns, process];
while (queue.length > 0) {{
\tconst value = queue.shift();
\tif ((typeof value !== 'object' && typeof value !== 'function') || value === null || seen.has(value)) continue;
\tseen.add(value);
\tif (typeof value === 'function' && String(Reflect.get(value, 'name')).startsWith('op_')) throw new Error(`reached ${{value.name}}`);
\tqueue.push(Reflect.getPrototypeOf(value));
\tfor (const key of Reflect.ownKeys(value)) {{
\t\tif (typeof key === 'string' && /^op_|^__bootstrap$|^Deno$|^gro_builtins$/.test(key)) throw new Error(`reached key ${{key}}`);
\t\tconst descriptor = Reflect.getOwnPropertyDescriptor(value, key);
\t\tif (descriptor === undefined) continue;
\t\tqueue.push(descriptor.value, descriptor.get, descriptor.set);
\t}}
}}
if (seen.size < 100) throw new Error(`walked only ${{seen.size}}`);
"
    );
    let (result, _) = run_granted("pollution", &source, &["a", "b"]);
    assert!(result.is_ok(), "{result:?}");
}

/// A run that doesn't grant `process` refuses its import at link time,
/// statically and through `import()`, naming the grant, and records why.
#[test]
fn an_ungranted_import_is_refused_naming_the_grant() {
    let main = "import { process } from '@fuzdev/gro/process.ts';\n";
    let (result, graph) = run_files(
        "ungranted_static",
        &[("main.mjs", main)],
        RuntimeOptions::default(),
    );
    let Err(RunError::Uncaught(report)) = result else {
        panic!("{result:?}");
    };
    assert!(
        report.contains(&format!(
            "cannot import '{PROCESS}' from file://{}/main.mjs: it needs the grant 'process', \
             which this run doesn't grant",
            graph.roots().project().dir.display()
        )),
        "{report}"
    );
    assert_not_granted(&graph, "main.mjs", Reach::Static, &Origin::Project);
    let main = "try {
\tawait import('@fuzdev/gro/process.ts');
\tthrow new Error('imported');
} catch (error) {
\tif (!(error instanceof TypeError) || !error.message.includes(\"it needs the grant 'process'\")) throw error;
}
";
    let (result, graph) = run_files(
        "ungranted_dynamic",
        &[("main.mjs", main)],
        RuntimeOptions::default(),
    );
    assert!(result.is_ok(), "{result:?}");
    assert_not_granted(&graph, "main.mjs", Reach::Dynamic, &Origin::Project);
}

/// Asserts `graph` recorded the refusal of `process` imported by `importer`
/// as `reach`, of `origin`.
fn assert_not_granted(graph: &Graph, importer: &str, reach: Reach, origin: &Origin) {
    assert_failures(
        graph,
        importer,
        reach,
        &[FailureReason::NotGranted {
            builtin: &BUILTINS[0],
            origin: origin.clone(),
        }],
    );
}

/// Asserts the failures `graph` recorded of `process` imported by
/// `importer`, a path in the project, as `reach` are `expected`.
fn assert_failures(graph: &Graph, importer: &str, reach: Reach, expected: &[FailureReason]) {
    let importer = url::Url::from_file_path(graph.roots().project().dir.join(importer)).unwrap();
    let failures: Vec<_> = graph
        .failures()
        .iter()
        .filter(|(site, _)| {
            site.specifier == PROCESS
                && site.importer.as_deref() == Some(importer.as_str())
                && site.reached == Reached::Import(reach)
        })
        .flat_map(|(_, failures)| failures)
        .map(|failure| failure.reason.clone())
        .collect();
    assert_eq!(failures, expected, "{:?}", graph.failures());
}

/// The package's files for the tests below: `dep`, whose `static.js`
/// imports `process` statically and whose `index.js` exports a function
/// importing it dynamically and one resolving it.
const DEP: [(&str, &str); 3] = [
    (
        "node_modules/dep/package.json",
        r#"{"name": "dep", "version": "1.0.0", "type": "module", "exports": {".": "./index.js", "./static.js": "./static.js"}}"#,
    ),
    (
        "node_modules/dep/index.js",
        "export const load = () => import('@fuzdev/gro/process.ts');\n\
         export const resolve = () => import.meta.resolve('@fuzdev/gro/process.ts');\n",
    ),
    (
        "node_modules/dep/static.js",
        "import { process } from '@fuzdev/gro/process.ts';\nexport default process;\n",
    ),
];

/// A package's module is refused `process` whatever the run grants, the
/// message naming the package, while the project imports it in the same
/// run, before the package's attempt and after; `import.meta.resolve`
/// returns the `builtin:` URL to either, and that URL is refused as an
/// import.
#[test]
fn a_package_is_refused_while_the_project_imports_it() {
    let main = format!(
        "import {{ process }} from '{PROCESS}';
import {{ load, resolve }} from 'dep';
try {{
\tawait load();
\tthrow new Error('the package imported it');
}} catch (error) {{
\tif (!(error instanceof TypeError) || !error.message.includes(\"it needs the grant 'process', which package 'dep' isn't granted\")) throw error;
}}
const again = await import('{PROCESS}');
if (again.process !== process || process.args.length !== 1) throw new Error('the project lost it');
for (const url of [import.meta.resolve('{PROCESS}'), resolve()]) {{
\tif (url !== 'builtin:{PROCESS}') throw new Error(`resolved ${{url}}`);
\ttry {{
\t\tawait import(url);
\t\tthrow new Error('imported the URL');
\t}} catch (error) {{
\t\tif (!(error instanceof TypeError) || !error.message.includes(\"the scheme 'builtin:' is not supported\")) throw error;
\t}}
}}
"
    );
    let mut files = DEP.to_vec();
    files.push(("main.mjs", &main));
    let options = RuntimeOptions {
        authority: granted(),
        args: vec!["a".to_owned()],
        ..RuntimeOptions::default()
    };
    let (result, graph) = run_files("package_dynamic", &files, options.clone());
    assert!(result.is_ok(), "{result:?}");
    let package = Origin::Package {
        name: "dep".to_owned(),
        version: Some("1.0.0".to_owned()),
        linked: false,
    };
    assert_not_granted(
        &graph,
        "node_modules/dep/index.js",
        Reach::Dynamic,
        &package,
    );
    // the project's import, recorded with the builtin
    let url = url::Url::parse(BUILTINS[0].url).unwrap();
    assert_eq!(graph.records()[&url], Record::Builtin(&BUILTINS[0]));
    let main_url = url::Url::from_file_path(graph.roots().project().dir.join("main.mjs")).unwrap();
    assert!(
        graph
            .edges()
            .iter()
            .any(|(edge, to)| edge.importer == main_url
                && edge.specifier == PROCESS
                && edge.reach == Reach::Static
                && *to == url),
        "{:?}",
        graph.edges()
    );

    // a static import from the package fails the run
    let mut files = DEP.to_vec();
    files.push(("main.mjs", "import 'dep/static.js';\n"));
    let (result, graph) = run_files("package_static", &files, options);
    let Err(RunError::Uncaught(report)) = result else {
        panic!("{result:?}");
    };
    assert!(
        report.contains(&format!(
            "cannot import '{PROCESS}' from file://{}/node_modules/dep/static.js: it needs the \
             grant 'process', which package 'dep' isn't granted: only the project's own modules \
             are granted host modules",
            graph.roots().project().dir.display()
        )),
        "{report}"
    );
    assert_not_granted(
        &graph,
        "node_modules/dep/static.js",
        Reach::Static,
        &package,
    );
    assert!(!graph.records().contains_key(&url));
}

/// A source-phase import has no source to give of a builtin, a synthetic
/// module: `import.source` of one rejects with a `ReferenceError`, before
/// the builtin is imported and after, as one of a file does with
/// `deno_core`'s own refusal, and the builtin still imports; a static
/// `import source` of one fails the run with the same refusal. A deferred
/// import of one, static or `import.defer`, imports it.
#[test]
fn a_source_phase_import_of_a_builtin_is_refused() {
    let main = format!(
        "const refusal = async (specifier) => {{
\ttry {{
\t\tawait import.source(specifier);
\t\treturn 'imported';
\t}} catch (error) {{
\t\treturn `${{error.constructor.name}}: ${{error.message}}`;
\t}}
}};
const results = [await refusal('{PROCESS}'), await refusal('./plain.mjs')];
const {{ process }} = await import('{PROCESS}');
results.push(await refusal('{PROCESS}'));
const {{ ns }} = await import('./deferred.mjs');
const deferred = await import.defer('{PROCESS}');
if (ns.process !== process || deferred.process !== process || process.args[0] !== 'a') throw new Error('not imported');
const expected = JSON.stringify([
\t'ReferenceError: {SOURCE_REFUSED}',
\t'ReferenceError: Source phase imports are not supported for JavaScript modules',
\t'ReferenceError: {SOURCE_REFUSED}'
]);
if (JSON.stringify(results) !== expected) throw new Error(JSON.stringify(results));
"
    );
    let files = [
        ("main.mjs", main.as_str()),
        ("plain.mjs", "export const a = 1;\n"),
        (
            "deferred.mjs",
            "import defer * as ns from '@fuzdev/gro/process.ts';\nexport { ns };\n",
        ),
    ];
    let options = RuntimeOptions {
        authority: granted(),
        args: vec!["a".to_owned()],
        ..RuntimeOptions::default()
    };
    let (result, _) = run_files("source_phase", &files, options.clone());
    assert!(result.is_ok(), "{result:?}");
    let main = format!("import source s from '{PROCESS}';\n");
    let (result, _) = run_files("source_phase_static", &[("main.mjs", &main)], options);
    let Err(RunError::Uncaught(report)) = result else {
        panic!("{result:?}");
    };
    assert!(report.contains(SOURCE_REFUSED), "{report}");
}

/// The refusal of a source-phase import of the builtin.
const SOURCE_REFUSED: &str = "Source phase imports are not supported for synthetic modules: \
                              builtin:@fuzdev/gro/process.ts has no source";

/// A static import of a builtin with import attributes is refused,
/// whatever they say, before its grant is checked, recorded as a file's
/// are: a static import, a re-export, and a deferred import fail the run,
/// recorded as static, and one in a module an `import()` loads rejects it,
/// recorded as dynamic. An `import()`'s own attributes never reach the
/// loader, so `import()` of a builtin with attributes imports it.
#[test]
fn a_static_import_of_a_builtin_with_attributes_is_refused() {
    let refused = [FailureReason::Load(LoadFailure::ImportAttributes)];
    let options = RuntimeOptions {
        authority: granted(),
        args: vec!["a".to_owned()],
        ..RuntimeOptions::default()
    };
    let cases = [
        (
            "static",
            "import { process } from '@fuzdev/gro/process.ts' with { type: 'json' };\n",
            options.clone(),
        ),
        (
            "reexport",
            "export * from '@fuzdev/gro/process.ts' with { type: 'json' };\n",
            options.clone(),
        ),
        (
            "defer",
            "import defer * as ns from '@fuzdev/gro/process.ts' with { x: 'y' };\n",
            options.clone(),
        ),
        // ungranted, the attributes are refused before the grant is checked
        (
            "ungranted",
            "import { process } from '@fuzdev/gro/process.ts' with { type: 'json' };\n",
            RuntimeOptions::default(),
        ),
    ];
    for (case, main, options) in cases {
        let (result, graph) = run_files(
            &format!("attributes_{case}"),
            &[("main.mjs", main)],
            options,
        );
        let Err(RunError::Uncaught(report)) = result else {
            panic!("{case}: {result:?}");
        };
        assert!(
            report.contains(&format!(
                "cannot import '{PROCESS}' from file://{}/main.mjs: a builtin module takes no \
                 import attributes",
                graph.roots().project().dir.display()
            )),
            "{case}: {report}"
        );
        assert_failures(&graph, "main.mjs", Reach::Static, &refused);
    }
    let main = format!(
        "try {{
\tawait import('./attribute.mjs');
\tthrow new Error('imported');
}} catch (error) {{
\tif (!(error instanceof TypeError) || !error.message.endsWith('a builtin module takes no import attributes')) throw error;
}}
// TODO: refused once an `import()`'s attributes reach the loader
const {{ process }} = await import('{PROCESS}', {{ with: {{ type: 'json' }} }});
if (process.args[0] !== 'a') throw new Error('not imported');
"
    );
    let files = [
        ("main.mjs", main.as_str()),
        (
            "attribute.mjs",
            "import { process } from '@fuzdev/gro/process.ts' with { type: 'json' };\n",
        ),
    ];
    let (result, graph) = run_files("attributes_dynamic", &files, options);
    assert!(result.is_ok(), "{result:?}");
    assert_failures(&graph, "attribute.mjs", Reach::Dynamic, &refused);
}

/// The project, exporting `args.js`, which imports `process`, and linked
/// into its own `node_modules` as `self`; and `pkg`, whose modules import
/// `args.js` by that name: `hop.js` through `import()` and
/// `import.meta.resolve`, `hopstatic.js` statically.
const HOP: [(&str, &str); 5] = [
    (
        "package.json",
        r#"{"name": "fixture", "type": "module", "exports": {"./args.js": "./args.js"}}"#,
    ),
    (
        "args.js",
        "import { process } from '@fuzdev/gro/process.ts';\nexport const args = process.args;\n",
    ),
    (
        "node_modules/pkg/package.json",
        r#"{"name": "pkg", "type": "module", "exports": {"./hop.js": "./hop.js", "./hopstatic.js": "./hopstatic.js"}}"#,
    ),
    (
        "node_modules/pkg/hop.js",
        "export const hop = () => import('self/args.js');\n\
         export const where = () => import.meta.resolve('self/args.js');\n",
    ),
    (
        "node_modules/pkg/hopstatic.js",
        "import { args } from 'self/args.js';\nexport default args;\n",
    ),
];

/// [`HOP`]'s link of the project into its own `node_modules`.
const SELF_LINK: [(&str, &str); 1] = [("node_modules/self", "..")];

/// Asserts `graph` recorded the refusal of `pkg`'s import of the project's
/// `args.js` from `importer`, reached as `reached`.
fn assert_project_refused(graph: &Graph, importer: &str, reached: Reached) {
    let root = &graph.roots().project().dir;
    let importer = url::Url::from_file_path(root.join(importer)).unwrap();
    let failures: Vec<_> = graph
        .failures()
        .iter()
        .filter(|(site, _)| {
            site.specifier == "self/args.js"
                && site.importer.as_deref() == Some(importer.as_str())
                && site.reached == reached
        })
        .flat_map(|(_, failures)| failures)
        .map(|failure| failure.reason.clone())
        .collect();
    assert_eq!(
        failures,
        [FailureReason::Resolve(
            ResolveErrorKind::PackageImportsProject {
                package: Box::from("pkg"),
                path: root.join("args.js"),
                project: root.clone(),
            }
        )],
        "{:?}",
        graph.failures()
    );
}

/// A package's bare import of the project's own module, through the
/// project linked into `node_modules`, is refused whatever the run grants,
/// by `import()`, `import.meta.resolve`, and a static import, so the
/// package never reaches the project's grant of `process` through it; the
/// project imports the same module by that link and by self-reference.
#[test]
fn a_package_importing_the_project_by_name_is_refused() {
    let main = "import { args } from 'self/args.js';
import { args as own } from 'fixture/args.js';
import { hop, where } from 'pkg/hop.js';
if (args !== own || args[0] !== 'SECRET') throw new Error('the project lost it');
for (const reach of [hop, where]) {
\ttry {
\t\tawait reach();
\t\tthrow new Error('the package reached it');
\t} catch (error) {
\t\tif (!(error instanceof TypeError) || !error.message.includes(\"package 'pkg' imports it by name\")) throw error;
\t}
}
";
    let mut files = HOP.to_vec();
    files.push(("main.mjs", main));
    let options = RuntimeOptions {
        authority: granted(),
        args: vec!["SECRET".to_owned()],
        ..RuntimeOptions::default()
    };
    let (result, graph) = run_linked("project_by_name", &files, &SELF_LINK, options.clone());
    assert!(result.is_ok(), "{result:?}");
    let hop = "node_modules/pkg/hop.js";
    assert_project_refused(&graph, hop, Reached::Import(Reach::Dynamic));
    assert_project_refused(&graph, hop, Reached::MetaResolve);

    let mut files = HOP.to_vec();
    files.push(("main.mjs", "import 'pkg/hopstatic.js';\n"));
    let (result, graph) = run_linked("project_by_name_static", &files, &SELF_LINK, options);
    let Err(RunError::Uncaught(report)) = result else {
        panic!("{result:?}");
    };
    let root = graph.roots().project().dir.display().to_string();
    assert!(
        report.contains(&format!(
            "cannot resolve 'self/args.js' from file://{root}/node_modules/pkg/hopstatic.js: \
             package 'pkg' imports it by name, and it resolves to {root}/args.js, a module of the \
             project at {root}: a package can't import the project's own modules by name"
        )),
        "{report}"
    );
    assert_project_refused(
        &graph,
        "node_modules/pkg/hopstatic.js",
        Reached::Import(Reach::Static),
    );
    let url = url::Url::parse(BUILTINS[0].url).unwrap();
    assert!(!graph.records().contains_key(&url));
}
