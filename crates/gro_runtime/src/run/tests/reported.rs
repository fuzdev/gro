//! Tests for the `import()`s a run reports as refused by rule: listed
//! whether or not the program caught them, once each, bounded, with the
//! graph recording off as `fgro run`'s is; a missing optional dependency,
//! an unsupported feature, and an uncaught refusal, which the run's error
//! already shows, are not.

use super::super::{RunError, RunOutcome, run_main};
use super::builtins::{HOP, PROCESS, SELF_LINK, granted};
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};
use crate::loader::graph::{MAX_KEPT_MESSAGE, MAX_KEPT_REFUSALS, MAX_KEPT_TEXT};
use crate::test_support::TestDir;
use crate::{RuntimeOptions, STACK_SIZE};

/// Writes `files` and `links` (a path and its target, made a symlink) into
/// a fresh project named for `name`, runs its `main.mjs` granted `process`
/// as `fgro run` does, recording off, and returns the outcome with the
/// project's directory.
fn run_project(name: &str, files: &[(&str, &str)], links: &[(&str, &str)]) -> (RunOutcome, String) {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let test_dir = TestDir::on_disk(&format!("reported_{name}"));
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
    let options = RuntimeOptions {
        authority: granted(),
        ..RuntimeOptions::default()
    };
    let entry = dir.join("main.mjs");
    let cwd = dir.clone();
    let outcome = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || run_main(engine, &options, &entry, &cwd))
        .unwrap()
        .join()
        .unwrap();
    (outcome, dir.display().to_string())
}

/// The package `pkg`, whose modules try `process` and the project's
/// `args.js` by name, and the project linked into its `node_modules`
/// ([`HOP`]).
const PKG: [(&str, &str); 2] = [
    (
        "node_modules/pkg/package.json",
        r#"{"name": "pkg", "type": "module", "exports": {"./probe.js": "./probe.js"}}"#,
    ),
    (
        "node_modules/pkg/probe.js",
        "const attempt = async (f) => { try { await f(); } catch {} };
export const probe = async () => {
\tawait attempt(() => import('@fuzdev/gro/process.ts'));
\tawait attempt(() => import('self/args.js'));
};
",
    ),
];

/// Each class of refusal by rule, caught, is listed once, however often it
/// was retried, by its specifier, its importer, and its message: an
/// ungranted builtin and a package's import of the project (the package's),
/// a reserved specifier, a refused scheme, a path into `node_modules`, and
/// attributes on a builtin (the project's), and string-compiled code (no
/// importer's); an optional dependency, a missing file, a `node:` and a
/// registry specifier, an unsupported attribute, a file with a `type`, a
/// strip's refusal, and `import.meta.resolve` are not. The run succeeds.
#[test]
fn a_caught_refusal_by_rule_is_listed_once() {
    let main = "import { probe } from 'pkg/probe.js';
const attempt = async (f) => { try { await f(); } catch {} };
for (let i = 0; i < 3; i++) {
\tawait probe();
\tawait attempt(() => import('@fuzdev/gro/process.js'));
\tawait attempt(() => import('ext:core/ops'));
\tawait attempt(() => import('./node_modules/pkg/probe.js'));
\tawait attempt(() => import('@fuzdev/gro/process.ts', { with: { type: 'json' } }));
\tawait attempt(() => (0, eval)(\"import('./who.mjs')\"));
\t// nothing there, or nothing that loads yet
\tawait attempt(() => import('./who.mjs', { with: { x: 'y' } }));
\tawait attempt(() => import('./who.mjs', { with: { type: 'json' } }));
\tawait attempt(() => import('optional-dependency'));
\tawait attempt(() => import('./missing.mjs'));
\tawait attempt(() => import('node:fs'));
\tawait attempt(() => import('npm:left-pad'));
\tawait attempt(() => import('./enum.ts'));
\ttry { import.meta.resolve('@fuzdev/gro/process.js'); } catch {}
}
";
    let files: Vec<_> = HOP
        .iter()
        .copied()
        .filter(|(path, _)| !path.starts_with("node_modules/pkg/"))
        .chain(PKG)
        .chain([
            ("main.mjs", main),
            ("who.mjs", "export default 'who';\n"),
            ("enum.ts", "enum E { A }\nexport default E;\n"),
        ])
        .collect();
    let (outcome, dir) = run_project("caught", &files, &SELF_LINK);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let main = format!("file://{dir}/main.mjs");
    let pkg = format!("file://{dir}/node_modules/pkg/probe.js");
    let listed: Vec<_> = outcome
        .refused_imports
        .iter()
        .map(|refused| (refused.specifier.as_str(), refused.importer.as_deref()))
        .collect();
    assert_eq!(
        listed,
        [
            ("./node_modules/pkg/probe.js", Some(main.as_str())),
            ("./who.mjs", None),
            ("@fuzdev/gro/process.js", Some(main.as_str())),
            (PROCESS, Some(main.as_str())),
            (PROCESS, Some(pkg.as_str())),
            ("ext:core/ops", Some(main.as_str())),
            ("self/args.js", Some(pkg.as_str())),
        ],
        "{:#?}",
        outcome.refused_imports
    );
    let messages: Vec<_> = outcome
        .refused_imports
        .iter()
        .map(|refused| refused.message.as_str())
        .collect();
    for (message, expected) in messages.iter().zip([
        "node_modules",
        "from code compiled from a string",
        "it is the .js spelling of the builtin module",
        "a builtin module takes no import attributes",
        "which package 'pkg' isn't granted",
        "the scheme 'ext:' is not supported",
        "can't import the project",
    ]) {
        assert!(message.contains(expected), "{expected}: {message}");
    }
}

/// Every distinct refusal by rule at one site is listed, once each however
/// often it's retried, in the order each first happened, so an earlier
/// refusal can't hide a later one: a package's builtin with attributes,
/// then without, is two listings, while an unsupported attribute before
/// them is none, and its `import()` of the same builtin in code compiled
/// from a string is listed under no importer.
#[test]
fn every_distinct_refusal_at_a_site_is_listed() {
    let probe = "const attempt = async (f) => { try { await f(); } catch {} };
export const probe = async () => {
\tfor (let i = 0; i < 3; i++) {
\t\tawait attempt(() => new Function(\"return import('@fuzdev/gro/process.ts')\")());
\t\tawait attempt(() => import('@fuzdev/gro/process.ts', { with: { lazy: 'true' } }));
\t\tawait attempt(() => import('@fuzdev/gro/process.ts', { with: { type: 'json' } }));
\t\tawait attempt(() => import('@fuzdev/gro/process.ts'));
\t}
};
";
    let files = [
        (
            "node_modules/pkg/package.json",
            r#"{"name": "pkg", "type": "module", "exports": {"./probe.js": "./probe.js"}}"#,
        ),
        ("node_modules/pkg/probe.js", probe),
        (
            "main.mjs",
            "import { probe } from 'pkg/probe.js';\nawait probe();\n",
        ),
    ];
    let (outcome, dir) = run_project("every_reason", &files, &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let pkg = format!("file://{dir}/node_modules/pkg/probe.js");
    let listed: Vec<_> = outcome
        .refused_imports
        .iter()
        .map(|refused| (refused.importer.as_deref(), refused.message.as_str()))
        .collect();
    assert!(
        outcome
            .refused_imports
            .iter()
            .all(|refused| refused.specifier == PROCESS),
        "{listed:?}"
    );
    let expected = [
        (None, "from code compiled from a string"),
        (
            Some(pkg.as_str()),
            "a builtin module takes no import attributes",
        ),
        (Some(pkg.as_str()), "which package 'pkg' isn't granted"),
    ];
    assert_eq!(listed.len(), expected.len(), "{listed:#?}");
    for ((importer, message), (expected_importer, expected)) in listed.iter().zip(expected) {
        assert_eq!(*importer, expected_importer, "{message}");
        assert!(message.contains(expected), "{expected}: {message}");
    }
    assert_eq!(outcome.unlisted_refusals, 0);
}

/// A refusal the program didn't catch is the run's error; it's listed
/// again only when it names a host module, as one it caught before is.
#[test]
fn an_uncaught_refusal_is_the_error_and_listed_when_a_probe() {
    let main = "try { await import('ext:core/ops'); } catch {}
await import('@fuzdev/gro/process.js');
";
    let (outcome, _) = run_project("uncaught", &[("main.mjs", main)], &[]);
    let Err(RunError::Uncaught(report)) = &outcome.result else {
        panic!("{:?}", outcome.result);
    };
    assert!(report.contains("it is the .js spelling"), "{report}");
    let listed: Vec<_> = outcome
        .refused_imports
        .iter()
        .map(|refused| refused.specifier.as_str())
        .collect();
    assert_eq!(
        listed,
        ["@fuzdev/gro/process.js", "ext:core/ops"],
        "{:?}",
        outcome.refused_imports
    );
    let main = "try { await import('@fuzdev/gro/process.js'); } catch {}
await import('ext:core/ops');
";
    let (outcome, _) = run_project("uncaught_other", &[("main.mjs", main)], &[]);
    let listed: Vec<_> = outcome
        .refused_imports
        .iter()
        .map(|refused| refused.specifier.as_str())
        .collect();
    assert_eq!(
        listed,
        ["@fuzdev/gro/process.js"],
        "{:?}",
        outcome.refused_imports
    );
}

/// An uncaught refusal is the run's error whether the program awaited the
/// `import()` or left its rejection unhandled, and so is a `TypeError` the
/// program made with the same message, which prints the same text; an
/// error that only quotes a caught refusal's message, the way a program
/// would hide it behind text a terminal doesn't show, is another error, so
/// the refusal is still listed. A refusal naming no host module shows
/// this, since a probe is listed whatever the error.
#[test]
fn only_an_error_that_is_the_refusal_is_not_listed() {
    let unhandled = "import('ext:core/ops');\n";
    let (outcome, _) = run_project("unhandled", &[("main.mjs", unhandled)], &[]);
    assert!(
        matches!(outcome.result, Err(RunError::Uncaught(_))),
        "{:?}",
        outcome.result
    );
    assert!(
        outcome.refused_imports.is_empty(),
        "{:?}",
        outcome.refused_imports
    );
    let same = "let message = '';
try { await import('ext:core/ops'); } catch (error) { message = error.message; }
throw new TypeError(message);
";
    let (outcome, _) = run_project("same_text", &[("main.mjs", same)], &[]);
    let Err(RunError::Uncaught(report)) = &outcome.result else {
        panic!("{:?}", outcome.result);
    };
    assert!(report.contains("the scheme 'ext:'"), "{report}");
    assert!(
        outcome.refused_imports.is_empty(),
        "{:?}",
        outcome.refused_imports
    );
    for (name, thrown) in [
        ("quoted", "new Error('failed\\x1b[8m' + message)"),
        ("prefixed", "new TypeError('x' + message)"),
        ("another_class", "new Error(message)"),
    ] {
        let main = format!(
            "let message = '';
try {{ await import('ext:core/ops'); }} catch (error) {{ message = error.message; }}
throw {thrown};
"
        );
        let (outcome, _) = run_project(name, &[("main.mjs", main.as_str())], &[]);
        let Err(RunError::Uncaught(report)) = &outcome.result else {
            panic!("{name}: {:?}", outcome.result);
        };
        assert!(report.contains("the scheme 'ext:'"), "{name}: {report}");
        let listed: Vec<_> = outcome
            .refused_imports
            .iter()
            .map(|refused| refused.specifier.as_str())
            .collect();
        assert_eq!(
            listed,
            ["ext:core/ops"],
            "{name}: {:?}",
            outcome.refused_imports
        );
    }
}

/// Past the first [`MAX_KEPT_REFUSALS`] refusals naming no host module,
/// the rest are counted, not listed, each time, while a retry of a listed
/// one isn't counted, and the project's probe of a builtin after the flood
/// is listed.
#[test]
fn refusals_past_the_cap_are_counted_not_listed() {
    let main = "const attempt = async (f) => { try { await f(); } catch {} };
for (let i = 0; i < 150; i++) await attempt(() => import(`ext:${i}`));
for (let i = 0; i < 3; i++) await attempt(() => import('ext:0'));
for (let i = 0; i < 2; i++) await attempt(() => import('@fuzdev/gro/process.js'));
";
    let (outcome, _) = run_project("cap", &[("main.mjs", main)], &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let mut listed: Vec<_> = outcome
        .refused_imports
        .iter()
        .map(|refused| refused.specifier.clone())
        .collect();
    listed.sort_by_key(|specifier| specifier[4..].parse::<usize>().ok());
    let expected: Vec<_> = std::iter::once("@fuzdev/gro/process.js".to_owned())
        .chain((0..MAX_KEPT_REFUSALS).map(|i| format!("ext:{i}")))
        .collect();
    assert_eq!(listed, expected);
    assert_eq!(outcome.unlisted_refusals, 150 - 100);
}

/// A package `name` whose `probe.js` runs `body` inside `probe()`, with
/// `attempt` catching each refusal.
fn package(name: &str, body: &str) -> [(String, String); 2] {
    [
        (
            format!("node_modules/{name}/package.json"),
            format!(
                r#"{{"name": "{name}", "type": "module", "exports": {{"./probe.js": "./probe.js"}}}}"#
            ),
        ),
        (
            format!("node_modules/{name}/probe.js"),
            format!(
                "const attempt = async (f) => {{ try {{ await f(); }} catch {{}} }};
export const probe = async () => {{
{body}
}};
"
            ),
        ),
    ]
}

/// Runs `main.mjs`, which calls each of `packages`' `probe()` in order,
/// returning the outcome with the project's directory.
fn run_packages(name: &str, packages: &[[(String, String); 2]]) -> (RunOutcome, String) {
    let main: String = (0..packages.len())
        .map(|i| format!("import {{ probe as probe{i} }} from 'p{i}/probe.js';\n"))
        .chain((0..packages.len()).map(|i| format!("await probe{i}();\n")))
        .collect();
    let files: Vec<(&str, &str)> = packages
        .iter()
        .flatten()
        .map(|(path, content)| (path.as_str(), content.as_str()))
        .chain([("main.mjs", main.as_str())])
        .collect();
    run_project(name, &files, &[])
}

/// The flood of refusals naming no host module `run_packages` packages use.
const FLOOD: &str = "\tfor (let i = 0; i < 150; i++) await attempt(() => import(`ext:${i}`));";

/// A package's probe of a builtin is listed whether a flood of other
/// refusals comes before it or after it, with the flood past the bound
/// counted.
#[test]
fn a_probe_is_listed_before_or_after_a_flood() {
    let probe = "\tawait attempt(() => import('@fuzdev/gro/process.ts'));";
    for (name, body) in [
        ("flood_first", format!("{FLOOD}\n{probe}")),
        ("probe_first", format!("{probe}\n{FLOOD}")),
    ] {
        let (outcome, dir) = run_packages(name, &[package("p0", &body)]);
        assert!(outcome.result.is_ok(), "{name}: {:?}", outcome.result);
        let probes: Vec<_> = outcome
            .refused_imports
            .iter()
            .filter(|refused| refused.specifier == PROCESS)
            .map(|refused| (refused.importer.clone(), refused.message.as_str()))
            .collect();
        let [(importer, message)] = probes.as_slice() else {
            panic!("{name}: {probes:?}");
        };
        assert_eq!(
            importer.as_deref(),
            Some(format!("file://{dir}/node_modules/p0/probe.js").as_str())
        );
        assert!(
            message.contains("which package 'p0' isn't granted"),
            "{name}: {message}"
        );
        assert_eq!(
            outcome.refused_imports.len(),
            1 + MAX_KEPT_REFUSALS,
            "{name}"
        );
        assert_eq!(outcome.unlisted_refusals, 50, "{name}");
    }
}

/// One package's flood of respellings of a builtin, past every bound,
/// pushes out neither another package's probe of it nor its own probe of
/// another host module: past each origin's first, the respellings are
/// listed up to [`MAX_KEPT_REFUSALS`] and counted after.
#[test]
fn a_flood_of_respellings_hides_no_other_probe() {
    let respellings = "\tfor (let i = 0; i < 150; i++) await attempt(() => import(`@fuzdev/gro/process.ts?${i}`));
\tawait attempt(() => import('@fuzdev/gro/fs.ts'));";
    let probe = "\tawait attempt(() => import('@fuzdev/gro/process.ts'));";
    let (outcome, dir) = run_packages(
        "respellings",
        &[
            package("p0", &format!("{FLOOD}\n{respellings}")),
            package("p1", probe),
        ],
    );
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let first = format!("file://{dir}/node_modules/p0/probe.js");
    let second = format!("file://{dir}/node_modules/p1/probe.js");
    let listed = |specifier: &str, importer: &str| {
        outcome.refused_imports.iter().any(|refused| {
            refused.specifier == specifier && refused.importer.as_deref() == Some(importer)
        })
    };
    assert!(listed(PROCESS, &second), "{:#?}", outcome.refused_imports);
    assert!(listed("@fuzdev/gro/fs.ts", &first));
    let respelled = outcome
        .refused_imports
        .iter()
        .filter(|refused| refused.specifier.starts_with("@fuzdev/gro/process.ts?"))
        .count();
    // the first is p0's probe of `process`, then the bound
    assert_eq!(respelled, 1 + MAX_KEPT_REFUSALS);
    assert_eq!(outcome.unlisted_refusals, 50 + 49);
    // the respellings past the bound, counted apart from the other flood
    assert_eq!(outcome.unlisted_host_module_refusals, 49);
}

/// A long specifier refused in a module deep in a package keeps the
/// package's name and the reason in its listed message: the specifier is
/// clipped where it stands, and the importer's path in its middle.
#[test]
fn a_long_refusal_keeps_who_refused() {
    let deep = ["d".repeat(200).as_str(); 12].join("/");
    let path = format!("node_modules/p0/{deep}/probe.js");
    let probe = "export const probe = async () => {
\ttry { await import('https://x/' + 'a'.repeat(5000)); } catch {}
};
";
    let package_json = format!(
        r#"{{"name": "p0", "type": "module", "exports": {{"./probe.js": "./{deep}/probe.js"}}}}"#
    );
    let files = [
        ("node_modules/p0/package.json", package_json.as_str()),
        (path.as_str(), probe),
        (
            "main.mjs",
            "import { probe } from 'p0/probe.js';\nawait probe();\n",
        ),
    ];
    let (outcome, dir) = run_project("deep", &files, &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let [refused] = outcome.refused_imports.as_slice() else {
        panic!("{:#?}", outcome.refused_imports);
    };
    assert_eq!(
        refused.importer.as_deref(),
        Some(format!("file://{dir}/{path}").as_str())
    );
    assert!(
        refused.message.starts_with(&format!(
            "cannot resolve '{}' from file://{dir}/node_modules/p0… (",
            refused.specifier
        )),
        "{}",
        refused.message
    );
    assert!(
        refused.message.ends_with(
            "/probe.js: it is a remote or inline specifier; only files on disk load. Fix: save \
             the module into the project and import it by a relative path"
        ),
        "{}",
        refused.message
    );
    assert!(
        refused.message.len() < MAX_KEPT_MESSAGE,
        "{}",
        refused.message.len()
    );
}

/// A long specifier is listed clipped to [`MAX_KEPT_TEXT`] bytes, marked
/// with its length, and its message to about [`MAX_KEPT_MESSAGE`], keeping
/// the importer and the reason at its end; a retry is one listing, and an
/// uncaught one is the error, not listed, though its record was clipped.
#[test]
fn a_long_specifier_is_clipped_in_the_listing() {
    let main = "const long = 'data:,' + 'a'.repeat(1 << 20);
for (let i = 0; i < 2; i++) { try { await import(long); } catch {} }
try { await import('x'.repeat(5000) + ':y'); } catch {}
";
    let (outcome, dir) = run_project("clip", &[("main.mjs", main)], &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let [data, scheme] = outcome.refused_imports.as_slice() else {
        panic!("{:?}", outcome.refused_imports.len());
    };
    let whole = format!("data:,{}", "a".repeat(1 << 20));
    assert_eq!(
        data.specifier,
        format!(
            "data:,{}… ({} bytes, clipped, blake3 {})",
            "a".repeat(MAX_KEPT_TEXT - "data:,".len()),
            whole.len(),
            &blake3::hash(whole.as_bytes()).to_hex()[..16]
        )
    );
    // the specifier is clipped where it stands, which leaves the rest whole
    assert_eq!(
        data.message,
        format!(
            "cannot resolve '{}' from file://{dir}/main.mjs: it is a remote or inline specifier; \
             only files on disk load. Fix: save the module into the project and import it by a \
             relative path",
            data.specifier
        )
    );
    assert!(
        scheme.specifier.contains("… (5002 bytes, clipped, blake3 "),
        "{}",
        scheme.specifier
    );
    // the scheme in the reason is what's left to cut, past the importer
    assert!(scheme.message.len() < MAX_KEPT_MESSAGE + 64);
    assert!(
        scheme.message.starts_with(&format!(
            "cannot resolve '{}' from file://{dir}/main.mjs: the scheme 'xxx",
            scheme.specifier
        )),
        "{}",
        scheme.message
    );
    assert!(
        scheme.message.contains(" bytes clipped) …"),
        "{}",
        scheme.message
    );
    let uncaught = "await import('data:,' + 'a'.repeat(1 << 20));\n";
    let (outcome, _) = run_project("clip_uncaught", &[("main.mjs", uncaught)], &[]);
    assert!(
        matches!(outcome.result, Err(RunError::Uncaught(_))),
        "{:?}",
        outcome.result
    );
    assert!(
        outcome.refused_imports.is_empty(),
        "{:?}",
        outcome.refused_imports
    );
    // an error with the clipped refusal's head and tail but another middle
    // is another error, so the refusal is listed
    let alike = "const long = 'data:,' + 'a'.repeat(1 << 20);
let message = '';
try { await import(long); } catch (error) { message = error.message; }
const middle = message.length >> 1;
throw new TypeError(message.slice(0, middle) + 'b' + message.slice(middle + 1));
";
    let (outcome, _) = run_project("clip_alike", &[("main.mjs", alike)], &[]);
    assert!(
        matches!(outcome.result, Err(RunError::Uncaught(_))),
        "{:?}",
        outcome.result
    );
    assert_eq!(
        outcome.refused_imports.len(),
        1,
        "{:?}",
        outcome.refused_imports
    );
}

/// Two long specifiers that share their clipped head and their length are
/// two listings, told apart by their hash in the clip's marker.
#[test]
fn long_specifiers_sharing_a_clipped_head_are_listed_apart() {
    let main = "const head = 'https://x/' + 'a'.repeat(1100);
for (const tail of ['ONE', 'TWO']) { try { await import(head + tail); } catch {} }
";
    let (outcome, _) = run_project("clip_apart", &[("main.mjs", main)], &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let [one, two] = outcome.refused_imports.as_slice() else {
        panic!("{:?}", outcome.refused_imports);
    };
    assert_ne!(one.specifier, two.specifier);
    assert_ne!(one.message, two.message);
    assert_eq!(outcome.unlisted_refusals, 0);
}

/// A refusal by rule of an import in a module an `import()` loads is
/// listed, under that module.
#[test]
fn a_refusal_in_a_graph_an_import_loads_is_listed() {
    let main = "try { await import('./dep.mjs'); } catch {}\n";
    let files = [
        ("main.mjs", main),
        ("dep.mjs", "import '@fuzdev/gro/process.js';\n"),
    ];
    let (outcome, dir) = run_project("edge", &files, &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let listed: Vec<_> = outcome
        .refused_imports
        .iter()
        .map(|refused| (refused.specifier.as_str(), refused.importer.clone()))
        .collect();
    assert_eq!(
        listed,
        [(
            "@fuzdev/gro/process.js",
            Some(format!("file://{dir}/dep.mjs"))
        )],
    );
}

/// Probing for what isn't there, or for a feature that isn't, lists
/// nothing, and a run with no refusal lists nothing.
#[test]
fn nothing_there_lists_nothing() {
    let main = "for (const specifier of ['optional-dependency', './missing.mjs', 'node:fs', 'jsr:@std/x', '#lib/x.js', './x.json', './x']) {
\ttry { await import(specifier); } catch {}
}
// features: an attribute that doesn't exist, and a module type
try { await import('./main.mjs', { with: { lazy: 'true' } }); } catch {}
try { await import('./main.mjs', { with: { type: 'json' } }); } catch {}
";
    let (outcome, _) = run_project("nothing", &[("main.mjs", main)], &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    assert!(
        outcome.refused_imports.is_empty(),
        "{:?}",
        outcome.refused_imports
    );
}
