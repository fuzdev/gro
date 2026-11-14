//! Tests for the `import()`s that fail a run, refused by rule: listed
//! whether or not the program caught them, each once, the first few, with
//! the graph recording off as `fgro run`'s is; a missing optional
//! dependency, an unsupported feature, a static import of the entry's
//! graph, and an uncaught refusal, which the run's error already shows, are
//! not.

use super::super::{RunError, RunOutcome, run_main};
use super::builtins::{HOP, SELF_LINK, granted, run_linked_outcome};
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};
use crate::test_support::TestDir;
use crate::{MAX_LISTED_MESSAGE, MAX_LISTED_REFUSALS, RuntimeOptions, STACK_SIZE};

/// Writes `files` and `links` (a path and its target, made a symlink) into
/// a fresh project named for `name`, runs its `main.mjs` granted `args`
/// as `fgro run` does, recording off, and returns the outcome with the
/// project's directory.
fn run_project(name: &str, files: &[(&str, &str)], links: &[(&str, &str)]) -> (RunOutcome, String) {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let test_dir = TestDir::on_disk(&format!("refused_{name}"));
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

/// Asserts `listed` holds one message per `expected`, in order, each
/// starting with its first text and containing its second.
fn assert_listed(listed: &[String], expected: &[(String, &str)]) {
    assert_eq!(listed.len(), expected.len(), "{listed:#?}");
    for (message, (start, reason)) in listed.iter().zip(expected) {
        assert!(message.starts_with(start.as_str()), "{start}: {message}");
        assert!(message.contains(reason), "{reason}: {message}");
    }
}

/// The package `pkg`, whose modules try `args` and the project's
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
\tawait attempt(() => import('@fuzdev/gro/args.ts'));
\tawait attempt(() => import('self/args.js'));
};
",
    ),
];

/// The project of [`a_caught_refusal_by_rule_is_listed_once`]: its
/// `main.mjs`, which tries each class three times, and its files.
fn classes() -> Vec<(&'static str, &'static str)> {
    let main = "import { probe } from 'pkg/probe.js';
const attempt = async (f) => { try { await f(); } catch {} };
for (let i = 0; i < 3; i++) {
\tawait probe();
\tawait attempt(() => import('@fuzdev/gro/args.js'));
\tawait attempt(() => import('ext:core/ops'));
\tawait attempt(() => import('./node_modules/pkg/probe.js'));
\tawait attempt(() => import('@fuzdev/gro/args.ts', { with: { type: 'json' } }));
\tawait attempt(() => (0, eval)(\"import('./who.mjs')\"));
\t// nothing there, or nothing that loads yet
\tawait attempt(() => import('./who.mjs', { with: { x: 'y' } }));
\tawait attempt(() => import('./who.mjs', { with: { type: 'json' } }));
\tawait attempt(() => import('optional-dependency'));
\tawait attempt(() => import('./missing.mjs'));
\tawait attempt(() => import('node:fs'));
\tawait attempt(() => import('npm:left-pad'));
\tawait attempt(() => import('./enum.ts'));
\ttry { import.meta.resolve('@fuzdev/gro/args.js'); } catch {}
}
";
    HOP.iter()
        .copied()
        .filter(|(path, _)| !path.starts_with("node_modules/pkg/"))
        .chain(PKG)
        .chain([
            ("main.mjs", main),
            ("who.mjs", "export default 'who';\n"),
            ("enum.ts", "enum E { A }\nexport default E;\n"),
        ])
        .collect()
}

/// Each class of refusal by rule, caught, is listed once, however often it
/// was retried, by its message, in the order each first happened: an
/// ungranted builtin and a package's import of the project (the package's),
/// a reserved specifier, a refused scheme, a path into `node_modules`, and
/// attributes on a builtin (the project's), and string-compiled code (no
/// importer's); an optional dependency, a missing file, a `node:` and a
/// registry specifier, an unsupported attribute, a file with a `type`, a
/// strip's refusal, and `import.meta.resolve` are not. The program ran to
/// completion.
#[test]
fn a_caught_refusal_by_rule_is_listed_once() {
    let (outcome, dir) = run_project("caught", &classes(), &SELF_LINK);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let main = format!("file://{dir}/main.mjs");
    let pkg = format!("file://{dir}/node_modules/pkg/probe.js");
    assert_listed(
        &outcome.refused.listed,
        &[
            (
                format!("cannot import '@fuzdev/gro/args.ts' from {pkg}: "),
                "which package 'pkg' isn't granted",
            ),
            (
                format!("cannot resolve 'self/args.js' from {pkg}: "),
                "can't import the project",
            ),
            (
                format!("cannot resolve '@fuzdev/gro/args.js' from {main}: "),
                "it is the .js spelling of the builtin module",
            ),
            (
                format!("cannot resolve 'ext:core/ops' from {main}: "),
                "the scheme 'ext:' is not supported",
            ),
            (
                format!("cannot resolve './node_modules/pkg/probe.js' from {main}: "),
                "node_modules",
            ),
            (
                format!("cannot import '@fuzdev/gro/args.ts' from {main}: "),
                "a builtin module takes no import attributes",
            ),
            (
                "cannot import './who.mjs' from code compiled from a string".to_owned(),
                "it has no importer of its own",
            ),
        ],
    );
    assert_eq!(outcome.refused.unlisted, 0);
}

/// The refusals listed don't depend on what the graph records: a run
/// recording in full lists the same.
#[test]
fn the_refusals_are_listed_whatever_the_graph_records() {
    let options = RuntimeOptions {
        authority: granted(),
        ..RuntimeOptions::default()
    };
    let (off, off_dir) = run_project("recording_off", &classes(), &SELF_LINK);
    let (full, graph) =
        run_linked_outcome("refused_recording_full", &classes(), &SELF_LINK, options);
    let full_dir = graph.roots().project().dir.display().to_string();
    assert!(full.result.is_ok(), "{:?}", full.result);
    assert!(!graph.failures().is_empty());
    let listed: Vec<String> = full
        .refused
        .listed
        .iter()
        .map(|message| message.replace(&full_dir, &off_dir))
        .collect();
    assert_eq!(listed, off.refused.listed);
    assert_eq!(full.refused.unlisted, off.refused.unlisted);
}

/// Every distinct refusal by rule of one specifier by one importer is
/// listed, once each however often it's retried, in the order each first
/// happened, so an earlier refusal can't hide a later one: a package's
/// builtin with attributes, then without, is two listings, while an
/// unsupported attribute before them is none, and its `import()` of the
/// same builtin in code compiled from a string is listed under no importer.
#[test]
fn every_distinct_refusal_of_one_import_is_listed() {
    let probe = "const attempt = async (f) => { try { await f(); } catch {} };
export const probe = async () => {
\tfor (let i = 0; i < 3; i++) {
\t\tawait attempt(() => new Function(\"return import('@fuzdev/gro/args.ts')\")());
\t\tawait attempt(() => import('@fuzdev/gro/args.ts', { with: { lazy: 'true' } }));
\t\tawait attempt(() => import('@fuzdev/gro/args.ts', { with: { type: 'json' } }));
\t\tawait attempt(() => import('@fuzdev/gro/args.ts'));
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
    let from_pkg = format!(
        "cannot import '@fuzdev/gro/args.ts' from file://{dir}/node_modules/pkg/probe.js: "
    );
    assert_listed(
        &outcome.refused.listed,
        &[
            (
                "cannot import '@fuzdev/gro/args.ts' from code compiled from a string".to_owned(),
                "it has no importer of its own",
            ),
            (
                from_pkg.clone(),
                "a builtin module takes no import attributes",
            ),
            (from_pkg, "which package 'pkg' isn't granted"),
        ],
    );
    assert_eq!(outcome.refused.unlisted, 0);
}

/// A refusal the program didn't catch is the run's error, which its report
/// shows, so it isn't listed, whatever it names, while one caught before
/// it is.
#[test]
fn an_uncaught_refusal_is_the_error_and_not_listed() {
    for (name, caught, uncaught, reason) in [
        (
            "uncaught",
            "ext:core/ops",
            "@fuzdev/gro/args.js",
            "it is the .js spelling",
        ),
        (
            "uncaught_other",
            "@fuzdev/gro/args.js",
            "ext:core/ops",
            "the scheme 'ext:'",
        ),
    ] {
        let main = format!(
            "try {{ await import('{caught}'); }} catch {{}}\nawait import('{uncaught}');\n"
        );
        let (outcome, dir) = run_project(name, &[("main.mjs", main.as_str())], &[]);
        let Err(RunError::Uncaught(report)) = &outcome.result else {
            panic!("{name}: {:?}", outcome.result);
        };
        assert!(report.contains(reason), "{name}: {report}");
        let listed = &outcome.refused.listed;
        assert_eq!(listed.len(), 1, "{name}: {listed:?}");
        assert!(
            listed[0].starts_with(&format!(
                "cannot resolve '{caught}' from file://{dir}/main.mjs: "
            )),
            "{name}: {listed:?}"
        );
        assert_eq!(outcome.refused.unlisted, 0, "{name}");
    }
}

/// An uncaught refusal is the run's error whether the program awaited the
/// `import()` or left its rejection unhandled, and so is a `TypeError` the
/// program made with the same message, which prints the same text; an
/// error that only quotes a caught refusal's message, or is of another
/// class, is another error, so the refusal is still listed.
#[test]
fn only_an_error_that_is_the_refusal_is_not_listed() {
    let unhandled = "import('ext:core/ops');\n";
    let (outcome, _) = run_project("unhandled", &[("main.mjs", unhandled)], &[]);
    assert!(
        matches!(outcome.result, Err(RunError::Uncaught(_))),
        "{:?}",
        outcome.result
    );
    assert!(outcome.refused.is_empty(), "{:?}", outcome.refused);
    let same = "let message = '';
try { await import('ext:core/ops'); } catch (error) { message = error.message; }
throw new TypeError(message);
";
    let (outcome, _) = run_project("same_text", &[("main.mjs", same)], &[]);
    let Err(RunError::Uncaught(report)) = &outcome.result else {
        panic!("{:?}", outcome.result);
    };
    assert!(report.contains("the scheme 'ext:'"), "{report}");
    assert!(outcome.refused.is_empty(), "{:?}", outcome.refused);
    for (name, thrown) in [
        ("quoted", "new Error('failed: ' + message)"),
        ("prefixed", "new TypeError('x' + message)"),
        ("another_class", "new Error(message)"),
    ] {
        let main = format!(
            "let message = '';
try {{ await import('ext:core/ops'); }} catch (error) {{ message = error.message; }}
throw {thrown};
"
        );
        let (outcome, dir) = run_project(name, &[("main.mjs", main.as_str())], &[]);
        let Err(RunError::Uncaught(report)) = &outcome.result else {
            panic!("{name}: {:?}", outcome.result);
        };
        assert!(report.contains("the scheme 'ext:'"), "{name}: {report}");
        assert_listed(
            &outcome.refused.listed,
            &[(
                format!("cannot resolve 'ext:core/ops' from file://{dir}/main.mjs: "),
                "the scheme 'ext:' is not supported",
            )],
        );
    }
}

/// Past the first [`MAX_LISTED_REFUSALS`] distinct refusals, in the order
/// they happened, the rest are counted, not listed, each time, a retry of
/// one included, while a retry of a listed one isn't counted.
#[test]
fn refusals_past_the_listed_are_counted() {
    let main = "const attempt = async (f) => { try { await f(); } catch {} };
for (let i = 0; i < 12; i++) await attempt(() => import(`ext:${i}`));
for (let i = 0; i < 3; i++) await attempt(() => import('ext:0'));
for (let i = 0; i < 2; i++) await attempt(() => import('ext:11'));
await attempt(() => import('@fuzdev/gro/args.js'));
";
    let (outcome, dir) = run_project("cap", &[("main.mjs", main)], &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let expected: Vec<_> = (0..MAX_LISTED_REFUSALS)
        .map(|i| {
            (
                format!("cannot resolve 'ext:{i}' from file://{dir}/main.mjs: "),
                "the scheme 'ext:' is not supported",
            )
        })
        .collect();
    assert_listed(&outcome.refused.listed, &expected);
    // the distinct ones past the listed, the two retries of one, and the last
    assert_eq!(
        outcome.refused.unlisted,
        (12 - MAX_LISTED_REFUSALS as u64) + 2 + 1
    );
}

/// A long refusal's message is listed cut to [`MAX_LISTED_MESSAGE`] bytes,
/// marked with its whole length; a retry is one listing, and an uncaught
/// one is the error, not listed.
#[test]
fn a_long_message_is_cut_in_the_listing() {
    let main = "const long = 'data:,' + 'a'.repeat(1 << 20);
for (let i = 0; i < 2; i++) { try { await import(long); } catch {} }
";
    let (outcome, dir) = run_project("cut", &[("main.mjs", main)], &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let whole = format!(
        "cannot resolve 'data:,{}' from file://{dir}/main.mjs: it is a remote or inline \
         specifier; only files on disk load. Fix: save the module into the project and import it \
         by a relative path",
        "a".repeat(1 << 20)
    );
    assert_eq!(
        outcome.refused.listed,
        [format!(
            "{}… ({} bytes)",
            &whole[..MAX_LISTED_MESSAGE],
            whole.len()
        )]
    );
    assert_eq!(outcome.refused.unlisted, 0);
    let uncaught = "await import('data:,' + 'a'.repeat(1 << 20));\n";
    let (outcome, _) = run_project("cut_uncaught", &[("main.mjs", uncaught)], &[]);
    assert!(
        matches!(outcome.result, Err(RunError::Uncaught(_))),
        "{:?}",
        outcome.result
    );
    assert!(outcome.refused.is_empty(), "{:?}", outcome.refused);
}

/// A refusal by rule of an import in a module an `import()` loads is
/// listed, under that module, while the same import in the entry's own
/// graph is the run's error and isn't.
#[test]
fn a_refusal_in_a_graph_an_import_loads_is_listed() {
    let dep = ("dep.mjs", "import '@fuzdev/gro/args.js';\n");
    let main = "try { await import('./dep.mjs'); } catch {}\n";
    let (outcome, dir) = run_project("edge", &[("main.mjs", main), dep], &[]);
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    assert_listed(
        &outcome.refused.listed,
        &[(
            format!("cannot resolve '@fuzdev/gro/args.js' from file://{dir}/dep.mjs: "),
            "it is the .js spelling of the builtin module",
        )],
    );
    let main = "import './dep.mjs';\n";
    let (outcome, _) = run_project("static_edge", &[("main.mjs", main), dep], &[]);
    let Err(RunError::Uncaught(report)) = &outcome.result else {
        panic!("{:?}", outcome.result);
    };
    assert!(report.contains("it is the .js spelling"), "{report}");
    assert!(outcome.refused.is_empty(), "{:?}", outcome.refused);
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
    assert!(outcome.refused.is_empty(), "{:?}", outcome.refused);
}
