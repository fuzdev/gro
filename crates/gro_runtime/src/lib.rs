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
//!   `node_modules` through their `"exports"`; `.mjs`, `.ts`, and `.mts`
//!   load, and `.js` under `"type": "module"`)
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
/// recursive descent, so how deeply nested a `.ts` module can be is set by
/// that thread's stack: past it the process aborts on a stack overflow, which
/// nothing can catch, where JS nested as deeply fails as a catchable error.
/// Call this on a thread with a stated stack, as `fgro` does with 32 MiB
/// (`std::thread::Builder::stack_size`), not on the main thread, whose stack
/// is the platform's: commonly 8 MiB on Linux, 1 MiB on Windows. V8's own
/// limit for JS sits below wherever the runtime starts on that thread, so the
/// size changes only the strip's depth.
pub fn run_main(options: &RuntimeOptions, entry: &Path) -> Result<(), RunError> {
    // nothing to grant yet; destructured so a new field can't go unread
    let RuntimeOptions {
        grants: Grants {},
        snapshot,
    } = options;

    let cwd = std::env::current_dir().map_err(RunError::Start)?;
    let entry = resolver::resolve_entry(&DiskTree, entry, &cwd)?;

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(RunError::Start)?;
    tokio_runtime.block_on(async {
        let mut runtime = JsRuntime::try_new(deno_core::RuntimeOptions {
            module_loader: Some(Rc::new(loader::Loader::new(&entry))),
            // the same list the snapshot was made from, in the same order
            extensions: bootstrap::extensions(),
            startup_snapshot: snapshot.map(|snapshot| snapshot.bytes),
            residual_lazy_js_sources: snapshot.map_or(&[], |snapshot| snapshot.residual_lazy_js),
            residual_lazy_esm_sources: snapshot.map_or(&[], |snapshot| snapshot.residual_lazy_esm),
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
