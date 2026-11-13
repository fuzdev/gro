//! Running an entry module to completion: [`run_main`] and the errors it
//! returns.

use std::path::Path;
use std::rc::Rc;

use deno_core::PollEventLoopOptions;
use url::Url;

use crate::engine::Engine;
use crate::loader::Loader;
use crate::loader::graph::Recording;
use crate::phases::{self, Phase};
use crate::report::uncaught;
use crate::resolver::{self, DiskTree, ResolveError};
use crate::runtime::{HostState, start_runtime};
use crate::snapshot::StartupSnapshot;
use crate::{ConsoleOptions, RuntimeOptions};

/// Why a run failed.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The entry module didn't resolve.
    #[error(transparent)]
    Entry(#[from] ResolveError),
    /// The runtime couldn't start.
    #[error("failed to start the runtime: {0}")]
    Start(#[from] StartError),
    /// The program threw, rejected a promise no one handled, or failed to
    /// load a module. The report is printable as is.
    #[error("{0}")]
    Uncaught(String),
}

/// Why the runtime couldn't start, before any of the program ran.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// tokio's runtime, which drives the event loop, couldn't be built.
    #[error("failed to build the event loop's runtime: {0}")]
    EventLoop(#[source] std::io::Error),
    /// The JS runtime couldn't be made, or its bootstrap failed. The report
    /// is printable as is.
    #[error("{0}")]
    Runtime(String),
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
/// [`RunError::Start`] when the runtime can't start, and [`RunError::Uncaught`]
/// when the program fails, including when a module fails to load or strip.
///
/// # Stack
///
/// Everything runs on the calling thread, and the TypeScript strip is a
/// recursive descent. tsv's depth limits bound how deep it goes: a module
/// nested or chained past them fails to load, with a message naming the
/// limit, and every module within them strips in 2 MiB of stack in a release
/// build and 16 MiB in a debug one. Call this on a thread reserving
/// [`STACK_SIZE`](crate::STACK_SIZE), as `fgro` does
/// (`std::thread::Builder::stack_size`), not on the main thread, whose stack
/// is the platform's (commonly 8 MiB on Linux, 1 MiB on Windows): below
/// tsv's contract, a module within the limits can still overflow the stack,
/// which aborts the process. V8's own limit for JS sits below wherever the
/// runtime starts on that thread, so the size doesn't change how deep JS can
/// go.
pub fn run_main(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &Path,
    cwd: &Path,
) -> Result<(), RunError> {
    let (entry, loader) = resolve_main(options, entry, cwd, Recording::Off)?;
    run_resolved(engine, options, &entry, &loader)
}

/// Runs as [`run_main`] does, and returns the run's result with the graph its
/// loader recorded as `recording` says: in full, every module resolved and
/// loaded with its format and hash, the imports between them, the files
/// consulted, and the failures. The graph is unset only when the entry
/// didn't resolve.
#[cfg(test)]
fn run_recorded(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &Path,
    cwd: &Path,
    recording: Recording,
) -> (Result<(), RunError>, Option<crate::loader::graph::Graph>) {
    let (entry, loader) = match resolve_main(options, entry, cwd, recording) {
        Ok(resolved) => resolved,
        Err(error) => return (Err(error), None),
    };
    let result = run_resolved(engine, options, &entry, &loader);
    // the runtime, and its share of the loader, is gone once the run ends
    (result, Some(loader.take_graph()))
}

/// Runs the resolved `entry` with `loader`, then marks the teardown.
fn run_resolved(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &Url,
    loader: &Rc<Loader>,
) -> Result<(), RunError> {
    let host = HostState {
        authority: options.authority.clone(),
        args: options.args.clone(),
    };
    let result = run_loaded(
        engine,
        options.snapshot,
        options.console,
        host,
        entry,
        Rc::clone(loader),
    );
    phases::mark(Phase::Teardown);
    result
}

/// Resolves the entry at `entry` from `cwd`, returning its URL and the
/// loader of its graph, which checks each builtin import against the run's
/// authority, recording as `recording` says.
fn resolve_main(
    options: &RuntimeOptions,
    entry: &Path,
    cwd: &Path,
    recording: Recording,
) -> Result<(Url, Rc<Loader>), RunError> {
    let (entry, roots) = resolver::resolve_entry(&DiskTree, entry, cwd)?;
    let loader =
        Rc::new(Loader::new(&entry, roots, recording).with_authority(options.authority.clone()));
    Ok((entry.url, loader))
}

/// Runs the entry at `entry`, resolved into `loader`'s graph, on a runtime
/// started from `snapshot` when there is one, its console as `console` says,
/// its host ops holding `host`. `_engine` is the proof the engine was set up
/// before V8 initializes.
fn run_loaded(
    _engine: Engine,
    snapshot: Option<StartupSnapshot>,
    console: ConsoleOptions,
    host: HostState,
    entry: &Url,
    loader: Rc<Loader>,
) -> Result<(), RunError> {
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(StartError::EventLoop)?;
    tokio_runtime.block_on(async {
        let mut runtime =
            start_runtime(snapshot, console, host, loader).map_err(StartError::Runtime)?;
        let id = runtime
            .load_main_es_module(entry)
            .await
            .map_err(|error| uncaught(&error))?;
        phases::mark(Phase::Load);
        let evaluation = runtime.mod_evaluate(id);
        runtime
            .run_event_loop(PollEventLoopOptions::default())
            .await
            .map_err(|error| uncaught(&error))?;
        let evaluated = evaluation.await.map_err(|error| uncaught(&error));
        phases::mark(Phase::Evaluate);
        evaluated
    })
}

#[cfg(test)]
mod tests;
