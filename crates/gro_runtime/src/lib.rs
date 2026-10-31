//! gro's JS host: a minimal web-standards runtime on V8.
//!
//! [`run_main`] runs an ES module from disk to completion: it resolves the
//! entry, evaluates it on a `deno_core` `JsRuntime` driven by a current-thread
//! tokio runtime, and drains the event loop (timers, pending promises). An
//! uncaught error or an unhandled promise rejection ends the run with a
//! [`RunError::Uncaught`] carrying the report, stack and source positions
//! included.
//!
//! - [`resolver`]: the strict ESM resolver, a pure module (relative and
//!   absolute specifiers with mandatory extensions, and packages in
//!   `node_modules` through their `"exports"`, each module kept inside its
//!   project's or package's root; `.mjs`, `.ts`, and `.mts` load, and `.js`
//!   under `"type": "module"`)
//! - `loader`: the `deno_core` module loader, an adapter over the resolver
//!   that strips TypeScript's types in process with `tsv_ts`, and keeps the
//!   run's module graph: its roots and each module's root and file, and,
//!   recording in full, each load's format and hash, the imports between
//!   modules, the files consulted, and what failed
//! - `bootstrap`: the extension that installs the pure web globals from
//!   `deno_web` (console, URL, encoding, timers, streams, `structuredClone`,
//!   `AbortController`, `performance`, and the rest) and removes `Deno` and
//!   `__bootstrap` from the global object
//! - [`engine`]: V8's process-wide setup ([`engine::EngineOptions`]), applied
//!   once per process by [`engine::init_engine`] before the first runtime,
//!   whose token ([`engine::Engine`]) every runtime start takes, apart from
//!   each runtime's [`RuntimeOptions`]
//! - [`snapshot`]: the startup snapshot, built in a process of its own and
//!   embedded by the binary that starts from it
//!
//! `deno_core`'s types stay out of this crate's public API, and the crate
//! reads no environment variable: what a runtime decides per run, such as the
//! console's color and streams ([`ConsoleOptions`]), comes in through
//! [`RuntimeOptions`] from the embedder.
//!
//! Without a [`RuntimeOptions::snapshot`], the runtime starts from source:
//! `deno_web`'s JS is read from its crate's path on the build machine, so such
//! a binary fails to start off its build tree. Started from a snapshot, it
//! reads nothing from the build tree. Either way, the steps that must happen
//! in each run, never in a snapshot (the time origin, the console's
//! [`ConsoleOptions`], removing the internals from the global object), run
//! once the runtime has started.

mod bootstrap;
pub mod engine;
mod loader;
pub mod resolver;
pub mod snapshot;

use std::path::Path;
use std::rc::Rc;

use deno_core::error::{CoreError, CoreErrorKind, JsError, JsStackFrame};
use deno_core::{JsRuntime, PollEventLoopOptions};
use url::Url;

use crate::engine::Engine;
use crate::loader::Loader;
use crate::loader::graph::{Graph, Recording};
use crate::resolver::{DiskTree, ResolveError};
use crate::snapshot::StartupSnapshot;

/// The stack to reserve for the thread [`run_main`] runs on: tsv's
/// `STACK_SIZE`, 32 MiB, twice what its parsers need in a debug build (see
/// [`run_main`]'s `# Stack`).
///
/// A reservation, not an allocation: pages commit as they're touched, so a
/// run pays only for the stack it uses.
pub const STACK_SIZE: usize = tsv_ts::STACK_SIZE;

/// The version string of the V8 the runtime embeds, as V8 reports it.
pub fn v8_version() -> &'static str {
    deno_core::v8::V8::get_version()
}

/// How a runtime is constructed.
#[derive(Debug, Clone, Default)]
pub struct RuntimeOptions {
    /// The authority the runtime's code holds. Empty by default, the only
    /// value today.
    pub grants: Grants,
    /// The snapshot to start from, made by [`snapshot::make_snapshot`] from
    /// this same build. Without one, the runtime evaluates its extensions'
    /// sources at startup.
    pub snapshot: Option<StartupSnapshot>,
    /// Where the console writes and whether it colors.
    pub console: ConsoleOptions,
}

/// The console's output: which stream each level writes to, and whether each
/// stream is colored.
///
/// Decided per runtime, read when it starts, never carried in a snapshot. The
/// default writes by level, uncolored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsoleOptions {
    /// Which stream each level writes to.
    pub routing: ConsoleRouting,
    /// Whether output to stdout is colored.
    pub color_stdout: bool,
    /// Whether output to stderr is colored.
    pub color_stderr: bool,
}

/// Which stream each console level writes to. Output is colored by the
/// stream it goes to, whatever its level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConsoleRouting {
    /// `log`, `info`, and `debug` to stdout; `warn`, `error`, and `trace` to
    /// stderr, as `fgro run` writes.
    #[default]
    ByLevel,
    /// Every level to stderr, leaving stdout to the host.
    Stderr,
}

/// The grant set: what the runtime's code may reach beyond pure computation.
/// Nothing can be granted yet, so it has no fields.
#[derive(Debug, Clone, Default)]
pub struct Grants {}

/// Why a run failed.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The entry module didn't resolve.
    #[error(transparent)]
    Entry(#[from] ResolveError),
    /// The runtime couldn't start.
    #[error("failed to start the runtime: {0}")]
    Start(#[source] std::io::Error),
    /// The program threw, rejected a promise no one handled, or failed to
    /// load a module. The report is printable as is.
    #[error("{0}")]
    Uncaught(String),
}

/// Runs the ES module at `entry` (relative to `cwd`, or absolute) and drives
/// the event loop until no work is left, on the engine `engine` names.
///
/// `cwd` is the working directory, an absolute path. It is an input, not
/// read from the process, so an embedder decides it; `fgro` passes its own.
///
/// The event loop drains before the entry's evaluation is checked, so a
/// top-level `await` on a promise that never settles ends the run as an
/// error, not a hang.
///
/// # Errors
///
/// Returns [`RunError::Entry`] when the entry doesn't resolve,
/// [`RunError::Start`] when the runtime can't start, and
/// [`RunError::Uncaught`] when the program fails, including when a module
/// fails to load or strip.
///
/// # Stack
///
/// Everything runs on the calling thread, and the TypeScript strip is a
/// recursive descent. tsv's depth limits bound how deep it goes: a module
/// nested or chained past them fails to load, with a message naming the
/// limit, and every module within them strips in 2 MiB of stack in a release
/// build and 16 MiB in a debug one. Call this on a thread reserving
/// [`STACK_SIZE`], as `fgro` does (`std::thread::Builder::stack_size`), not on
/// the main thread, whose stack is the platform's (commonly 8 MiB on Linux,
/// 1 MiB on Windows): below tsv's contract, a module within the limits can
/// still overflow the stack, which aborts the process. V8's own limit for JS
/// sits below wherever the runtime starts on that thread, so the size doesn't
/// change how deep JS can go.
pub fn run_main(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &Path,
    cwd: &Path,
) -> Result<(), RunError> {
    run_recorded(engine, options, entry, cwd, Recording::Off).0
}

/// Runs as [`run_main`] does, and returns the run's result with the graph its
/// loader recorded as `recording` says: in full, every module resolved and
/// loaded with its format and hash, the imports between them, the files
/// consulted, and the failures. The graph is unset only when the entry
/// didn't resolve.
pub(crate) fn run_recorded(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &Path,
    cwd: &Path,
    recording: Recording,
) -> (Result<(), RunError>, Option<Graph>) {
    // nothing to grant yet; destructured so a new field can't go unread
    let RuntimeOptions {
        grants: Grants {},
        snapshot,
        console,
    } = options;

    let (entry, roots) = match resolver::resolve_entry(&DiskTree, entry, cwd) {
        Ok(resolved) => resolved,
        Err(error) => return (Err(error.into()), None),
    };
    let loader = Rc::new(Loader::new(&entry, roots, recording));
    let result = run_loaded(engine, *snapshot, *console, &entry.url, Rc::clone(&loader));
    // the runtime, and its share of the loader, is gone once the run ends
    let graph = Rc::try_unwrap(loader).map_or_else(|loader| loader.graph(), Loader::into_graph);
    (result, Some(graph))
}

/// Runs the entry at `entry`, resolved into `loader`'s graph, on a runtime
/// started from `snapshot` when there is one, its console as `console` says.
/// `_engine` is the proof the engine was set up before V8 initializes.
fn run_loaded(
    _engine: Engine,
    snapshot: Option<StartupSnapshot>,
    console: ConsoleOptions,
    entry: &Url,
    loader: Rc<Loader>,
) -> Result<(), RunError> {
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(RunError::Start)?;
    tokio_runtime.block_on(async {
        let mut runtime = JsRuntime::try_new(deno_core::RuntimeOptions {
            module_loader: Some(loader),
            // the same list the snapshot was made from, in the same order
            extensions: bootstrap::extensions(),
            startup_snapshot: snapshot.map(|snapshot| snapshot.bytes),
            residual_lazy_js_sources: snapshot.map_or(&[], |snapshot| snapshot.residual_lazy_js),
            ..Default::default()
        })
        .map_err(|error| uncaught(&error))?;
        // before the bootstrap's start reads it
        runtime.op_state().borrow_mut().put(console);
        bootstrap::start(&mut runtime).map_err(|error| RunError::Uncaught(report_js(&error)))?;
        let id = runtime
            .load_main_es_module(entry)
            .await
            .map_err(|error| uncaught(&error))?;
        let evaluation = runtime.mod_evaluate(id);
        runtime
            .run_event_loop(PollEventLoopOptions::default())
            .await
            .map_err(|error| uncaught(&error))?;
        evaluation.await.map_err(|error| uncaught(&error))
    })
}

/// Renders a failed run: a JS error as its message and stack frames, with
/// each `cause`; anything else by its message chain.
fn uncaught(error: &CoreError) -> RunError {
    RunError::Uncaught(match &*error.0 {
        CoreErrorKind::Js(error) => report_js(error),
        _ => error.print_with_cause(),
    })
}

/// `Uncaught Error: message`, then V8's frames, each `at
/// file:///path:line:column`, then each `cause` as `Caused by: Error: message`
/// with its frames. A module's top-level throw reads `Uncaught (in promise)`,
/// since evaluating a module settles a promise. A cyclic chain prints its
/// repeated error once more and ends there: `deno_core` converts the repeat
/// without its `cause`, matching objects by identity. The report doesn't mark
/// the repeat, because the converted errors are copies, and errors made at one
/// site (a retry loop's) are equal by value without being the same object.
fn report_js(error: &JsError) -> String {
    let mut report = error.exception_message.clone();
    push_frames(&mut report, error);
    let mut next = error.cause.as_deref();
    while let Some(cause) = next {
        report.push_str("\nCaused by: ");
        report.push_str(&describe(cause));
        push_frames(&mut report, cause);
        next = cause.cause.as_deref();
    }
    report
}

/// `Name: message` for an error object, else the thrown value as V8 printed it.
fn describe(error: &JsError) -> String {
    match (&error.name, &error.message) {
        (Some(name), Some(message)) if message.is_empty() => name.clone(),
        (Some(name), Some(message)) => format!("{name}: {message}"),
        _ => error
            .exception_message
            .strip_prefix("Uncaught (in promise) ")
            .or_else(|| error.exception_message.strip_prefix("Uncaught "))
            .unwrap_or(&error.exception_message)
            .to_owned(),
    }
}

/// Appends the error's frames: V8's `at` lines from its stack, or when it has
/// none (a syntax error's stack is its message alone), the location V8
/// reported for it.
fn push_frames(report: &mut String, error: &JsError) {
    let mut pushed = false;
    if let Some(stack) = &error.stack {
        // the stack's first lines repeat the message
        for frame in stack
            .lines()
            .filter(|line| line.trim_start().starts_with("at "))
        {
            report.push('\n');
            report.push_str(frame);
            pushed = true;
        }
    }
    if !pushed
        && let Some(location) = error
            .frames
            .first()
            .and_then(JsStackFrame::maybe_format_location)
    {
        report.push_str("\n    at ");
        report.push_str(&location);
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::engine::{EngineOptions, init_engine};

    use super::{
        Graph, Recording, RunError, RuntimeOptions, STACK_SIZE, bootstrap, run_main, run_recorded,
    };

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
    /// declare, the ones [`bootstrap::extensions`] drops included.
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
        let engine = init_engine(&EngineOptions::default()).unwrap();
        let dir = std::env::temp_dir().join(format!("gro_runtime_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry: PathBuf = dir.join("main.mjs");
        std::fs::write(&entry, source).unwrap();
        let cwd = dir.clone();
        let result = std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || run_main(engine, &RuntimeOptions::default(), &entry, &cwd))
            .unwrap()
            .join()
            .unwrap();
        std::fs::remove_dir_all(Path::new(&dir)).unwrap();
        result
    }

    /// The refusal the resolver gives every `ext:` specifier.
    fn refusal(specifier: &str) -> [String; 2] {
        [
            format!("cannot resolve '{specifier}'"),
            "the scheme 'ext:' is not supported".to_owned(),
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
            let Err(RunError::Uncaught(report)) = run_source(&format!("static_{i}"), &source)
            else {
                panic!("a static import of {specifier} ran");
            };
            for expected in refusal(specifier) {
                assert!(report.contains(&expected), "{specifier}: {report}");
            }
        }
    }

    /// `import()` and `import.meta.resolve` of any `ext:` module throw the
    /// resolver's refusal, a `TypeError`.
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
\t\t!message.includes(\"the scheme 'ext:' is not supported\")
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

    /// Writes `files` into a fresh directory named for `name`, runs its
    /// `entry` recording as `recording` says, removes the directory, and
    /// returns the run's result and graph, with the directory's real path.
    fn run_graph(
        name: &str,
        files: &[(&str, &str)],
        entry: &str,
        recording: Recording,
    ) -> (Result<(), RunError>, Option<Graph>, PathBuf) {
        use crate::resolver::test_tree::disk_test_dir;

        let engine = init_engine(&EngineOptions::default()).unwrap();
        let dir = disk_test_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        for (file, content) in files {
            std::fs::write(dir.join(file), content).unwrap();
        }
        let entry = dir.join(entry);
        let cwd = dir.clone();
        let (result, graph) = std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                run_recorded(engine, &RuntimeOptions::default(), &entry, &cwd, recording)
            })
            .unwrap()
            .join()
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        (result, graph, dir)
    }

    const PACKAGE_JSON: (&str, &str) = ("package.json", r#"{"name": "fixture", "type": "module"}"#);

    /// A run records its graph in full: the static import and the modules a
    /// dynamic one loads as edges, an `import()` that failed and was caught
    /// as a failure, `import.meta.resolve` as neither, the files consulted
    /// from the entry on, and the format and hash of each module's bytes
    /// before the strip. Recording off, the same run keeps only its roots
    /// and records.
    #[test]
    fn a_run_records_its_graph() {
        use std::collections::{BTreeMap, BTreeSet};

        use url::Url;

        use crate::loader::graph::{
            Edge, Failure, FailureError, FailureSite, Loaded, Reach, Reached,
        };
        use crate::resolver::{ConsultedFile, FileRole, ModuleFormat, ResolveErrorKind};

        const MAIN: &str = "import {a} from './a.ts';
let caught = false;
try {
\tawait import('./missing.js');
} catch (error) {
\tcaught = error instanceof TypeError;
}
if (!caught) throw new Error('the import() did not fail with a TypeError');
const url = import.meta.resolve('./b.js');
if (!url.endsWith('/b.js')) throw new Error(url);
const {d} = await import('./c.js');
if (a !== 1 || d !== 4) throw new Error('wrong values');
";
        const A: &str = "export const a: number = 1;\n";
        const C: &str = "export {d} from './d.js';\n";
        const D: &str = "export const d = 4;\n";
        let files = [
            PACKAGE_JSON,
            ("main.js", MAIN),
            ("a.ts", A),
            ("b.js", "export {};\n"),
            ("c.js", C),
            ("d.js", D),
        ];
        let name = "run_records_its_graph";
        let (result, graph, dir) = run_graph(name, &files, "main.js", Recording::Full);
        assert!(result.is_ok(), "{result:?}");
        let (off_result, off, _) = run_graph(name, &files, "main.js", Recording::Off);
        assert!(off_result.is_ok(), "{off_result:?}");
        let graph = graph.unwrap();
        let off = off.unwrap();
        let url = |file: &str| Url::from_file_path(dir.join(file)).unwrap();

        let main = url("main.js");
        let edge = |importer: &Url, specifier: &str, reach| Edge {
            importer: importer.clone(),
            specifier: specifier.to_owned(),
            reach,
        };
        assert_eq!(
            graph.edges,
            BTreeMap::from([
                (edge(&main, "./a.ts", Reach::Static), url("a.ts")),
                (edge(&main, "./c.js", Reach::Dynamic), url("c.js")),
                // a static import inside a dynamically loaded module
                (edge(&url("c.js"), "./d.js", Reach::Dynamic), url("d.js")),
            ])
        );
        // `import.meta.resolve` loaded nothing, and the failed import nothing
        let mut recorded: Vec<_> = graph.records.keys().cloned().collect();
        recorded.sort();
        let mut expected = vec![main.clone(), url("a.ts"), url("c.js"), url("d.js")];
        expected.sort();
        assert_eq!(recorded, expected);
        for (file, source, format) in [
            ("main.js", MAIN, ModuleFormat::JavaScript),
            ("a.ts", A, ModuleFormat::TypeScript),
            ("c.js", C, ModuleFormat::JavaScript),
            ("d.js", D, ModuleFormat::JavaScript),
        ] {
            let loaded = Loaded {
                format,
                hash: blake3::hash(source.as_bytes()),
            };
            assert_eq!(graph.records[&url(file)].loaded, Some(loaded), "{file}");
        }

        assert_eq!(graph.failures.len(), 1, "{:?}", graph.failures);
        let site = FailureSite {
            specifier: "./missing.js".to_owned(),
            importer: Some(main.to_string()),
            reached: Reached::Import(Reach::Dynamic),
        };
        assert!(
            matches!(
                graph.failures[&site].as_slice(),
                [Failure {
                    at: None,
                    error: FailureError::Resolve(ResolveErrorKind::NotFound { .. }),
                }]
            ),
            "{:?}",
            graph.failures
        );

        let package_json = dir.join("package.json");
        let consulted = |role| ConsultedFile {
            path: package_json.clone(),
            role,
        };
        assert_eq!(
            graph.consulted,
            BTreeSet::from([
                consulted(FileRole::SearchBound),
                consulted(FileRole::PackageScope),
            ])
        );

        // off, the same roots and records, with nothing else
        assert_eq!(off.roots, graph.roots);
        let mut off_recorded: Vec<_> = off.records.keys().cloned().collect();
        off_recorded.sort();
        assert_eq!(off_recorded, expected);
        assert!(off.records.values().all(|record| record.loaded.is_none()));
        assert!(off.edges.is_empty());
        assert!(off.consulted.is_empty());
        assert!(off.failures.is_empty());
    }

    /// The graph is recorded for a run that fails, and there is none when
    /// the entry doesn't resolve.
    #[test]
    fn a_failed_run_still_records_its_graph() {
        use crate::loader::graph::{FailureError, LoadFailure};

        let files = [("main.ts", "enum E { A }\n")];
        let name = "failed_run_records_its_graph";
        let (result, graph, _) = run_graph(name, &files, "main.ts", Recording::Full);
        let (missing, none, _) = run_graph(name, &files, "missing.ts", Recording::Full);
        assert!(matches!(result, Err(RunError::Uncaught(_))), "{result:?}");
        let failures = graph.unwrap().failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        let (site, failures) = failures.iter().next().unwrap();
        // the entry's own load names no importer
        assert_eq!(site.importer, None);
        assert!(matches!(
            failures.as_slice(),
            [crate::loader::graph::Failure {
                error: FailureError::Load(LoadFailure::Strip { .. }),
                ..
            }]
        ));
        assert!(matches!(missing, Err(RunError::Entry(_))), "{missing:?}");
        assert!(none.is_none());
    }

    /// A load that fails is recorded by how `deno_core` reached it: the
    /// module an `import()` call loads and every static import below it as
    /// dynamic, the root with no importer and the import below it with its
    /// importer and position, and a static import of the entry's graph as
    /// static.
    #[test]
    fn a_load_failure_is_recorded_by_its_reach() {
        use url::Url;

        use crate::loader::graph::{
            Failure, FailureError, FailureSite, LoadFailure, Position, Reach, Reached,
        };

        const MAIN: &str = "try {
\tawait import('./e.ts');
} catch {}
try {
\tawait import('./c.js');
} catch {}
await import('./s.js');
";
        let mut files = [
            PACKAGE_JSON,
            ("main.js", MAIN),
            ("e.ts", "enum E { A }\n"),
            ("c.js", "import './f.ts';\n"),
            ("f.ts", "enum F { A }\n"),
            ("s.js", "export {};\n"),
        ];
        let name = "load_failure_reach";
        let (result, graph, dir) = run_graph(name, &files, "main.js", Recording::Full);
        // the entry's own load, failing, is static with no importer
        files[1].1 = "import './e.ts';\n";
        let (failed, static_graph, _) = run_graph(name, &files, "main.js", Recording::Full);
        let url = |file: &str| Url::from_file_path(dir.join(file)).unwrap().to_string();
        assert!(result.is_ok(), "{result:?}");
        assert!(failed.is_err());
        let failures = graph.unwrap().failures;
        let dynamic = Reached::Import(Reach::Dynamic);
        let sites: Vec<_> = failures.keys().cloned().collect();
        assert_eq!(
            sites,
            [
                FailureSite {
                    specifier: url("e.ts"),
                    importer: None,
                    reached: dynamic,
                },
                FailureSite {
                    specifier: url("f.ts"),
                    importer: Some(url("c.js")),
                    reached: dynamic,
                },
            ]
        );
        assert!(matches!(
            failures[&sites[1]].as_slice(),
            [Failure {
                at: Some(Position { line: 1, column: 8 }),
                error: FailureError::Load(LoadFailure::Strip { .. }),
            }]
        ));
        let failures = static_graph.unwrap().failures;
        let sites: Vec<_> = failures.keys().cloned().collect();
        assert_eq!(
            sites,
            [FailureSite {
                specifier: url("e.ts"),
                importer: Some(url("main.js")),
                reached: Reached::Import(Reach::Static),
            }]
        );
    }

    /// A run records at most one failure reached statically: `deno_core`
    /// stops the entry's load at its first error, and linking reuses the
    /// specifiers it already resolved, so the other refused imports are
    /// never loaded or resolved again.
    #[test]
    fn a_run_records_at_most_one_static_failure() {
        use crate::loader::graph::{FailureError, LoadFailure, Reach, Reached};

        let files = [
            PACKAGE_JSON,
            (
                "main.js",
                "import './z.ts';\nimport './a.ts';\nimport './y.ts';\n",
            ),
            ("z.ts", "enum E { A }\n"),
            ("a.ts", "enum E { A }\n"),
            ("y.ts", "enum E { A }\n"),
        ];
        let (result, graph, _) =
            run_graph("one_static_failure", &files, "main.js", Recording::Full);
        assert!(matches!(result, Err(RunError::Uncaught(_))), "{result:?}");
        let failures = graph.unwrap().failures;
        let all: Vec<_> = failures
            .iter()
            .flat_map(|(site, failures)| failures.iter().map(move |failure| (site, failure)))
            .collect();
        let [(site, failure)] = all.as_slice() else {
            panic!("{failures:?}");
        };
        assert_eq!(site.reached, Reached::Import(Reach::Static));
        assert!(matches!(
            failure.error,
            FailureError::Load(LoadFailure::Strip { .. })
        ));
    }
}
