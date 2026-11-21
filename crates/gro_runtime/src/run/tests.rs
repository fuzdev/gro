//! Tests for running an entry: the `ext:` and `builtin:` specifiers refused to
//! user code, then the graph a run records (`graph`), the builtins
//! (`builtins`, the exit module in `exit`, and the grant-free ones in `observations`), import attributes (`attributes`), `import()` in code
//! compiled from a string (`string_code`), and the refusals that fail a
//! run (`refused`).

use std::path::PathBuf;

use super::{RunError, resolve_main, run_main, run_resolved};
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};
use crate::loader::graph::Recording;
use crate::test_support::TestDir;
use crate::{RuntimeOptions, STACK_SIZE, bootstrap};

mod attributes;
mod builtins;
mod embedded;
mod exit;
mod graph;
mod observations;
mod refused;
mod string_code;

/// The modules `deno_core` provides itself, which it doesn't list publicly
/// (`CONTEXT_SETUP_SOURCES`, `BUILTIN_SOURCES`, `BUILTIN_ES_MODULES`, and
/// the virtual ops module, in its `runtime/jsruntime.rs`). Recheck this
/// list at every `deno_core` bump: a renamed module would leave the test
/// refusing a name that no longer exists while the new one goes
/// untested (they load eagerly, so the loader's refusal still covers
/// them).
const DENO_CORE_SPECIFIERS: [&str; 6] = [
    "ext:core/ops",
    "ext:core/mod.js",
    "ext:core/00_primordials.js",
    "ext:core/00_infra.js",
    "ext:core/01_core.js",
    "ext:core/02_timers.js",
];

/// Named by `deno_web`'s `webtransport.js` and in no linked extension.
const UNLINKED_SPECIFIERS: [&str; 1] = ["ext:deno_net/03_quic.js"];

/// Every `ext:` specifier this build knows: `deno_core`'s own, and each
/// module, script, lazy module, and synthetic module the extensions
/// declare, the ones [`bootstrap::extensions`] drops included, with the
/// synthetic modules' names, the builtins' `builtin:` URLs.
fn ext_specifiers() -> Vec<String> {
    let mut specifiers: Vec<String> = DENO_CORE_SPECIFIERS
        .iter()
        .chain(&UNLINKED_SPECIFIERS)
        .map(ToString::to_string)
        .collect();
    for extension in bootstrap::declared_extensions() {
        for files in [
            &extension.js_files,
            &extension.esm_files,
            &extension.lazy_loaded_js_files,
            &extension.lazy_loaded_esm_files,
        ] {
            specifiers.extend(files.iter().map(|file| file.specifier.to_owned()));
        }
        for (module, script) in extension.synthetic_esm_modules.iter() {
            specifiers.push((*module).to_owned());
            specifiers.push((*script).to_owned());
        }
    }
    specifiers.sort();
    specifiers.dedup();
    specifiers
}

/// Runs `source` as an `.mjs` entry in a fresh directory, on a thread
/// reserving [`STACK_SIZE`] as `run_main` requires.
fn run_source(name: &str, source: &str) -> Result<(), RunError> {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let dir = TestDir::in_system_temp(name);
    let entry: PathBuf = dir.path().join("main.mjs");
    std::fs::write(&entry, source).unwrap();
    let cwd = dir.path().to_owned();
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || run_main(engine, &RuntimeOptions::default(), &entry, &cwd).result)
        .unwrap()
        .join()
        .unwrap()
}

/// Runs an entry that ran to completion, returning how many shares of its
/// loader are alive, the test's own among them, while the outcome holds the
/// runtime, after `end` is done with the runtime, and whether a second run
/// on the same thread then ran to completion.
fn loader_shares(name: &str, end: fn(crate::SpentRuntime)) -> (usize, usize, bool) {
    let dir = TestDir::in_system_temp(name);
    let entry: PathBuf = dir.path().join("main.mjs");
    std::fs::write(&entry, "globalThis.ran = 1;").unwrap();
    let cwd = dir.path().to_owned();
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let options = RuntimeOptions::default();
            let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
            let (url, loader) = resolve_main(&options, &entry, &cwd, Recording::Off).unwrap();
            let outcome = run_resolved(engine, &options, &url, &cwd, &loader);
            outcome.result.unwrap();
            let held = std::rc::Rc::strong_count(&loader);
            end(outcome.runtime.unwrap());
            let left = std::rc::Rc::strong_count(&loader);
            let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
            let again = run_main(engine, &options, &entry, &cwd);
            (held, left, again.result.is_ok())
        })
        .unwrap()
        .join()
        .unwrap()
}

/// A run hands its runtime back whole: the outcome holds it, with its
/// share of the loader, and dropping it tears it down, so the share is
/// gone and the thread runs another module after. That's what an embedder
/// gets by doing nothing.
#[test]
fn dropping_a_spent_runtime_tears_it_down() {
    let (held, left, again) = loader_shares("spent_dropped", drop);
    assert!(held > 1, "{held}");
    assert_eq!(left, 1);
    assert!(again);
}

/// Forgetting it tears nothing down: the runtime's share of the loader
/// stays alive, as the runtime does, for a process about to exit, and a
/// later run on the thread is unaffected.
#[test]
fn forgetting_a_spent_runtime_leaves_it_whole() {
    let (held, left, again) = loader_shares("spent_forgotten", crate::SpentRuntime::forget);
    assert!(held > 1, "{held}");
    assert_eq!(left, held);
    assert!(again);
}

/// The refusal the resolver gives every `ext:` and `builtin:` specifier:
/// its scheme isn't supported.
fn refusal(specifier: &str) -> [String; 2] {
    let scheme = specifier.split_once(':').map_or("", |(scheme, _)| scheme);
    [
        format!("cannot resolve '{specifier}'"),
        format!("the scheme '{scheme}:' is not supported"),
    ]
}

#[test]
fn the_ext_specifiers_include_the_ones_that_reached_user_code() {
    let specifiers = ext_specifiers();
    for known in [
        "ext:core/ops",
        "ext:core/mod.js",
        "ext:deno_web/locks.js",
        "ext:deno_web/webtransport.js",
        "ext:deno_web/01_console.js",
        "ext:deno_webidl/00_webidl.js",
        "ext:gro/bootstrap.js",
        "builtin:@fuzdev/gro/args.ts",
        "ext:gro/builtin/@fuzdev/gro/args.ts",
        "builtin:@fuzdev/gro/cwd.ts",
        "ext:gro/builtin/@fuzdev/gro/cwd.ts",
        "builtin:@fuzdev/gro/pid.ts",
        "ext:gro/builtin/@fuzdev/gro/pid.ts",
        "builtin:@fuzdev/gro/runtime_version.ts",
        "ext:gro/builtin/@fuzdev/gro/runtime_version.ts",
    ] {
        assert!(
            specifiers.iter().any(|specifier| specifier == known),
            "{known} in {specifiers:?}"
        );
    }
}

/// A static import of any `ext:` module fails the run with the resolver's
/// refusal. `deno_core` falls back to a lazy ES module of the refused name,
/// so this holds only because the runtime declares none
/// (`bootstrap::extensions`).
#[test]
fn a_static_import_of_every_ext_specifier_is_refused() {
    let specifiers = ext_specifiers();
    for (i, specifier) in specifiers.iter().enumerate() {
        let source = format!("import {specifier:?};\n");
        let Err(RunError::Uncaught(report)) = run_source(&format!("static_{i}"), &source) else {
            panic!("a static import of {specifier} ran");
        };
        let report = report.to_string();
        for expected in refusal(specifier) {
            assert!(report.contains(&expected), "{specifier}: {report}");
        }
    }
}

/// `import()` and `import.meta.resolve` of any `ext:` module or `builtin:`
/// URL throw the resolver's refusal, a `TypeError`.
#[test]
fn a_dynamic_import_and_import_meta_resolve_of_every_ext_specifier_are_refused() {
    let specifiers = ext_specifiers();
    let source = format!(
        "const specifiers = {};
if (specifiers.length === 0) throw new Error('no specifiers');
const failures = [];
const check = (form, specifier, error) => {{
\tconst message = String(error?.message);
\tif (
\t\t!(error instanceof TypeError) ||
\t\t!message.includes(`cannot resolve '${{specifier}}'`) ||
\t\t!message.includes(`the scheme '${{specifier.split(':')[0]}}:' is not supported`)
\t) {{
\t\tfailures.push(`${{form}} ${{specifier}}: ${{error}}`);
\t}}
}};
for (const specifier of specifiers) {{
\ttry {{
\t\tawait import(specifier);
\t\tfailures.push(`import() loaded ${{specifier}}`);
\t}} catch (error) {{
\t\tcheck('import()', specifier, error);
\t}}
\ttry {{
\t\tconst url = import.meta.resolve(specifier);
\t\tfailures.push(`import.meta.resolve resolved ${{specifier}} to ${{url}}`);
\t}} catch (error) {{
\t\tcheck('import.meta.resolve', specifier, error);
\t}}
}}
if (failures.length > 0) throw new Error(failures.join('\\n'));
",
        serde_json::to_string(&specifiers).unwrap()
    );
    let result = run_source("dynamic", &source);
    assert!(result.is_ok(), "{result:?}");
}

/// An `import()` whose file resolves but can't be read rejects with a
/// `TypeError` too: `deno_core` rethrows every load failure as one.
#[test]
fn a_dynamic_import_failing_to_read_rejects_with_a_type_error() {
    use std::os::unix::fs::PermissionsExt as _;

    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let dir = TestDir::in_system_temp("unreadable");
    let unreadable = dir.path().join("unreadable.mjs");
    std::fs::write(&unreadable, "export default 1;\n").unwrap();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
    // a user the mode doesn't bind, root, reads it anyway
    if std::fs::read(&unreadable).is_ok() {
        eprintln!(
            "skipped: {} reads with mode 0o000 (running as root?), so no read can fail",
            unreadable.display()
        );
        return;
    }
    let entry = dir.path().join("main.mjs");
    std::fs::write(
        &entry,
        "try {
\tawait import('./unreadable.mjs');
\tthrow new Error('the unreadable module loaded');
} catch (error) {
\tif (!(error instanceof TypeError) || !String(error.message).includes('failed to load')) {
\t\tthrow new Error(`not a TypeError: ${error}`);
\t}
}
",
    )
    .unwrap();
    let cwd = dir.path().to_owned();
    let result = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || run_main(engine, &RuntimeOptions::default(), &entry, &cwd).result)
        .unwrap()
        .join()
        .unwrap();
    assert!(result.is_ok(), "{result:?}");
}
