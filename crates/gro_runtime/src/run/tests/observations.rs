//! Tests for the grant-free builtins, the observations (`cwd`, `pid`,
//! `runtime_version`), from source in a debug build: their values and
//! shapes through every import form, that a run granting nothing links
//! them for the project and a package alike, their hardening against
//! patched intrinsics, a working directory whose path isn't UTF-8, and
//! what a call site of one shows.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

use super::super::RunError;
use super::builtins::{run_files, run_files_from};
use crate::loader::graph::{Reach, Record};
use crate::resolver::find_builtin;
use crate::{Authority, RuntimeOptions, bootstrap};

const CWD: &str = "@fuzdev/gro/cwd.ts";
const PID: &str = "@fuzdev/gro/pid.ts";
const RUNTIME_VERSION: &str = "@fuzdev/gro/runtime_version.ts";

/// The runtime's name and version the tests' runs are given, unlike any
/// real one.
const VERSION: &str = "test/9.8.7";

/// A run that grants nothing, named [`VERSION`].
fn ungranted() -> RuntimeOptions {
    RuntimeOptions {
        authority: Authority::default(),
        runtime_version: VERSION.to_owned(),
        ..RuntimeOptions::default()
    }
}

/// The JS that checks the three namespaces `cwd_ns`, `pid_ns`, and
/// `version_ns` hold the run's values, given `check(what, ok)` and the
/// directory `expected_cwd`: the test process's id, [`VERSION`], and a
/// `cwd` that is a frozen function of no parameters, named `cwd`, with no
/// `prototype`, that no `new` constructs and any receiver calls.
fn value_checks() -> String {
    format!(
        "const read_cwd = cwd_ns.cwd;
check('cwd value', cwd_ns.cwd() === expected_cwd && read_cwd.call({{}}) === expected_cwd && read_cwd() === expected_cwd);
check('cwd function', typeof read_cwd === 'function' && Object.getPrototypeOf(read_cwd) === Function.prototype);
check('cwd frozen', Object.isFrozen(read_cwd));
check('cwd keys', JSON.stringify(Reflect.ownKeys(read_cwd)) === '[\"length\",\"name\"]' && read_cwd.name === 'cwd' && read_cwd.length === 0);
let constructed = 'constructed';
try {{ new read_cwd(); }} catch (error) {{ constructed = error.constructor.name; }}
check(`new cwd: ${{constructed}}`, constructed === 'TypeError');
check(`pid ${{pid_ns.pid}}`, pid_ns.pid === {pid});
check(`runtime_version ${{version_ns.runtime_version}}`, version_ns.runtime_version === '{VERSION}');
for (const [ns, name] of [[cwd_ns, 'cwd'], [pid_ns, 'pid'], [version_ns, 'runtime_version']]) {{
\tcheck(`${{name}} exports`, JSON.stringify(Reflect.ownKeys(ns).filter((key) => typeof key === 'string')) === JSON.stringify(['default', name].sort()));
\tcheck(`${{name}} default`, ns.default === undefined);
\tlet threw = 0;
\ttry {{ ns[name] = 1; }} catch {{ threw++; }}
\ttry {{ delete ns[name]; }} catch {{ threw++; }}
\tcheck(`${{name}} namespace writes`, threw === 2);
}}
",
        pid = std::process::id()
    )
}

/// Each observation is its run's value through a named, a default, a
/// namespace, and a dynamic import alike, one binding however it's
/// imported, in a run that grants nothing: `cwd()` the directory the run
/// was started in, `pid` the process's id, `runtime_version` the string
/// the embedder gave.
#[test]
fn the_observations_are_the_run_s_through_every_import_form() {
    let main = format!(
        "import cwd_default, {{ cwd }} from '{CWD}';
import pid_default, {{ pid }} from '{PID}';
import version_default, {{ runtime_version }} from '{RUNTIME_VERSION}';
import * as cwd_ns from '{CWD}';
import * as pid_ns from '{PID}';
import * as version_ns from '{RUNTIME_VERSION}';
const check = (what, ok) => {{ if (!ok) throw new Error(what); }};
const expected_cwd = import.meta.dirname;
{checks}
check('defaults', cwd_default === undefined && pid_default === undefined && version_default === undefined);
check('named', cwd === cwd_ns.cwd && pid === pid_ns.pid && runtime_version === version_ns.runtime_version);
const dynamic = [await import('{CWD}'), await import('{PID}'), await import('{RUNTIME_VERSION}')];
check('dynamic', dynamic[0] === cwd_ns && dynamic[1] === pid_ns && dynamic[2] === version_ns);
",
        checks = value_checks()
    );
    let (result, graph) = run_files("observations_forms", &[("main.mjs", &main)], ungranted());
    assert!(result.is_ok(), "{result:?}");
    for specifier in [CWD, PID, RUNTIME_VERSION] {
        let builtin = find_builtin(specifier).unwrap();
        assert_eq!(builtin.grant, None, "{specifier}");
        let url = url::Url::parse(builtin.url).unwrap();
        assert_eq!(graph.records()[&url], Record::Builtin(builtin));
    }
}

/// A package's module imports each observation as the project does,
/// statically and through `import()`, in a run that grants nothing, and
/// reads the same values; the graph records the package's edges to them.
#[test]
fn a_package_imports_the_observations_as_the_project_does() {
    let index = format!(
        "import * as cwd_ns from '{CWD}';
import * as pid_ns from '{PID}';
import * as version_ns from '{RUNTIME_VERSION}';
export const check_static = (check, expected_cwd) => {{
{checks}}};
export const load = () => Promise.all([import('{CWD}'), import('{PID}'), import('{RUNTIME_VERSION}')]);
",
        checks = value_checks()
    );
    let main = format!(
        "import {{ check_static, load }} from 'dep';
import {{ cwd }} from '{CWD}';
const check = (what, ok) => {{ if (!ok) throw new Error(what); }};
check_static(check, import.meta.dirname);
const [cwd_ns, pid_ns, version_ns] = await load();
const expected_cwd = import.meta.dirname;
{checks}
check('the project reads the same directory', cwd() === cwd_ns.cwd());
",
        checks = value_checks()
    );
    let files = [
        (
            "node_modules/dep/package.json",
            r#"{"name": "dep", "version": "1.0.0", "type": "module", "exports": "./index.js"}"#,
        ),
        ("node_modules/dep/index.js", index.as_str()),
        ("main.mjs", main.as_str()),
    ];
    let (result, graph) = run_files("observations_package", &files, ungranted());
    assert!(result.is_ok(), "{result:?}");
    let importer = url::Url::from_file_path(
        graph
            .roots()
            .project()
            .dir
            .join("node_modules/dep/index.js"),
    )
    .unwrap();
    for specifier in [CWD, PID, RUNTIME_VERSION] {
        let url = url::Url::parse(find_builtin(specifier).unwrap().url).unwrap();
        let reaches: Vec<Reach> = graph
            .edges()
            .iter()
            .filter(|(edge, to)| edge.importer == importer && **to == url)
            .map(|(edge, _)| edge.reach)
            .collect();
        assert_eq!(reaches, [Reach::Static, Reach::Dynamic], "{specifier}");
    }
    assert!(graph.failures().is_empty(), "{:?}", graph.failures());
}

/// User code that runs before the first import of an observation and
/// patches the intrinsics (the global `Object`'s and `Reflect`'s
/// functions, `Function.prototype`'s methods, the global `Error`, setters
/// and a `then` on `Object.prototype` and `Function.prototype`) changes
/// nothing the builtins make, and calls none of the patches.
#[test]
fn patched_intrinsics_change_nothing_an_observation_makes() {
    let main = format!(
        "let log = '';
const note = (what) => {{ log += `${{what}};`; }};
const check = (what, ok) => {{ if (!ok) throw new Error(what); }};
const expected_cwd = import.meta.dirname;
const define = Object.defineProperty;
const remove = Reflect.deleteProperty;
const RealError = Error;
const keys = ['cwd', 'pid', 'runtime_version', 'default', 'name', 'length'];
const patched = [
\t[Object, ['assign', 'create', 'freeze', 'defineProperty', 'defineProperties', 'hasOwn', 'keys', 'isFrozen']],
\t[Reflect, ['apply', 'defineProperty', 'ownKeys']],
\t[Function.prototype, ['call', 'apply', 'bind']],
];
const originals = patched.flatMap(([owner, names]) => names.map((name) => [owner, name, owner[name]]));
for (const proto of [Object.prototype, Function.prototype]) {{
\tfor (const key of keys) {{
\t\tif (proto === Function.prototype && (key === 'name' || key === 'length')) continue;
\t\tdefine(proto, key, {{
\t\t\tget() {{ note(`get ${{key}}`); return 'evil'; }},
\t\t\tset(value) {{ note(`set ${{key}}`); }},
\t\t\tconfigurable: true
\t\t}});
\t}}
}}
Object.prototype.then = function () {{ note('then'); }};
globalThis.Error = function () {{ note('Error'); }};
for (const [owner, name, original] of originals) {{
\towner[name] = function () {{ note(String(name)); throw new RealError('patched'); }};
}}
log = '';
const [cwd_ns, pid_ns, version_ns] = [await import('{CWD}'), await import('{PID}'), await import('{RUNTIME_VERSION}')];
const called = cwd_ns.cwd();
const recorded = log;
for (const [owner, name, original] of originals) owner[name] = original;
globalThis.Error = RealError;
remove(Object.prototype, 'then');
for (const proto of [Object.prototype, Function.prototype]) for (const key of keys) if (!(proto === Function.prototype && (key === 'name' || key === 'length'))) remove(proto, key);
check(`patches called: ${{recorded}}`, recorded === '');
check('called', called === expected_cwd);
{checks}",
        checks = value_checks()
    );
    let (result, _) = run_files(
        "observations_pollution",
        &[("main.mjs", &main)],
        ungranted(),
    );
    assert!(result.is_ok(), "{result:?}");
}

/// A builtin's frames show nothing of its functions: with
/// `Error.prepareStackTrace` set, the call sites of an error a builtin's
/// function threw, `cwd` in a directory whose path isn't UTF-8, name the
/// builtin's script, at the line its source throws on, and each of those
/// gives `undefined` for `getFunction()` and `getThis()`, so no call site
/// hands out a function that closes over the ops or the object its script
/// was called with.
#[test]
fn a_builtin_s_call_sites_give_no_function_and_no_receiver() {
    let cwd = Path::new(OsStr::from_bytes(b"/not_utf8_\xff"));
    let script = format!("ext:gro/builtin/{CWD}");
    let implementation = bootstrap::IMPLEMENTATIONS
        .iter()
        .find(|implementation| implementation.script == script)
        .unwrap();
    let throws = implementation
        .source
        .lines()
        .position(|line| line.contains("throw new Error("));
    let line = throws.unwrap() + 1;
    let main = format!(
        "import {{ cwd }} from '{CWD}';
let sites;
Error.prepareStackTrace = (error, call_sites) => {{
\tsites = call_sites;
\treturn 'prepared';
}};
let stack;
try {{
\tcwd();
}} catch (error) {{
\tstack = error.stack;
}}
delete Error.prepareStackTrace;
if (stack !== 'prepared') throw new Error(`the stack wasn't prepared: ${{stack}}`);
const builtin = sites.filter((site) => String(site.getFileName()).startsWith('ext:gro/builtin/'));
if (builtin.length === 0) throw new Error(`no frame of the builtin: ${{sites.map((site) => site.getFileName())}}`);
for (const site of builtin) {{
\tif (site.getFileName() !== '{script}') throw new Error(`named ${{site.getFileName()}}`);
\tif (site.getLineNumber() !== {line}) throw new Error(`at line ${{site.getLineNumber()}}, not {line}`);
\tif (site.getFunction() !== undefined || site.getThis() !== undefined) throw new Error('a call site gave a function or a receiver');
}}
"
    );
    let (result, _) = run_files_from(
        "observations_call_sites",
        &[("main.mjs", &main)],
        ungranted(),
        cwd,
    );
    assert!(result.is_ok(), "{result:?}");
}

/// In a working directory whose path isn't UTF-8, `cwd` imports, by a
/// static import and `import()`, and throws an `Error` saying so each time
/// it's called, where the Node module throws when it loads; it's still a
/// frozen function named `cwd`, and the other observations are unaffected.
#[test]
fn a_working_directory_that_is_not_utf8_fails_the_call_of_cwd() {
    let cwd = Path::new(OsStr::from_bytes(b"/not_utf8_\xff"));
    let message = "the working directory's path isn't UTF-8, so Gro can't name it";
    let main = format!(
        "import {{ cwd }} from '{CWD}';
const failures = [];
const dynamic = await import('{CWD}');
if (dynamic.cwd !== cwd || !Object.isFrozen(cwd) || cwd.name !== 'cwd' || cwd.length !== 0) failures.push('shape');
for (let i = 0; i < 2; i++) {{
\ttry {{
\t\tfailures.push(`returned ${{cwd()}}`);
\t}} catch (error) {{
\t\tif (Object.getPrototypeOf(error) !== Error.prototype || error.message !== \"{message}\") failures.push(String(error));
\t}}
}}
const {{ pid }} = await import('{PID}');
if (pid !== {pid}) failures.push(`pid ${{pid}}`);
if (failures.length > 0) throw new Error(failures.join('; '));
",
        pid = std::process::id()
    );
    let (result, _) = run_files_from(
        "observations_cwd_caught",
        &[("main.mjs", &main)],
        ungranted(),
        cwd,
    );
    assert!(result.is_ok(), "{result:?}");
    let main = format!("import {{ cwd }} from '{CWD}';\ncwd();\n");
    let (result, _) = run_files_from(
        "observations_cwd_uncaught",
        &[("main.mjs", &main)],
        ungranted(),
        cwd,
    );
    let Err(RunError::Uncaught(report)) = result else {
        panic!("{result:?}");
    };
    assert!(
        report.starts_with(&format!("Uncaught (in promise) Error: {message}\n")),
        "{report}"
    );
}
