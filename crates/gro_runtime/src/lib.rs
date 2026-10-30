//! gro's JS host: a minimal web-standards JavaScript runtime on V8.
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
//!   that strips TypeScript's types in process with `tsv_ts`
//! - `bootstrap`: the extension that installs the pure web globals from
//!   `deno_web` (console, URL, encoding, timers, streams, `structuredClone`,
//!   `AbortController`, `performance`, and the rest) and removes `Deno` and
//!   `__bootstrap` from the global object
//! - [`snapshot`]: the startup snapshot, built in a process of its own and
//!   embedded by the binary that starts from it
//!
//! `deno_core`'s types stay out of this crate's public API.
//!
//! Without a [`RuntimeOptions::snapshot`], the runtime starts from source:
//! `deno_web`'s JS is read from its crate's path on the build machine, so such
//! a binary fails to start off its build tree. Started from a snapshot, it
//! reads nothing from the build tree. Either way, the steps that must happen
//! per process (the time origin, removing the internals from the global
//! object) run once the runtime has started.

mod bootstrap;
mod loader;
pub mod resolver;
pub mod snapshot;

use std::path::Path;
use std::rc::Rc;

use deno_core::error::{CoreError, CoreErrorKind, JsError, JsStackFrame};
use deno_core::{JsRuntime, PollEventLoopOptions};

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

/// Runs the ES module at `entry` (relative to the working directory, or
/// absolute) and drives the event loop until no work is left.
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
pub fn run_main(options: &RuntimeOptions, entry: &Path) -> Result<(), RunError> {
    // nothing to grant yet; destructured so a new field can't go unread
    let RuntimeOptions {
        grants: Grants {},
        snapshot,
    } = options;

    let cwd = std::env::current_dir().map_err(RunError::Start)?;
    let (entry, roots) = resolver::resolve_entry(&DiskTree, entry, &cwd)?;

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(RunError::Start)?;
    tokio_runtime.block_on(async {
        let mut runtime = JsRuntime::try_new(deno_core::RuntimeOptions {
            module_loader: Some(Rc::new(loader::Loader::new(&entry, roots))),
            // the same list the snapshot was made from, in the same order
            extensions: bootstrap::extensions(),
            startup_snapshot: snapshot.map(|snapshot| snapshot.bytes),
            residual_lazy_js_sources: snapshot.map_or(&[], |snapshot| snapshot.residual_lazy_js),
            ..Default::default()
        })
        .map_err(|error| uncaught(&error))?;
        bootstrap::start(&mut runtime).map_err(|error| RunError::Uncaught(report_js(&error)))?;
        let id = runtime
            .load_main_es_module(&entry.url)
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

    use super::{RunError, RuntimeOptions, STACK_SIZE, bootstrap, run_main};

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
        let dir = std::env::temp_dir().join(format!("gro_runtime_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry: PathBuf = dir.join("main.mjs");
        std::fs::write(&entry, source).unwrap();
        let result = std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || run_main(&RuntimeOptions::default(), &entry))
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
    /// so this held only once the runtime stopped declaring them.
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
}
