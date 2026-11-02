//! Running an entry module to completion: [`run_main`] and the errors it
//! returns.

use std::path::Path;
use std::rc::Rc;

use deno_core::PollEventLoopOptions;
use url::Url;

use crate::engine::Engine;
use crate::loader::Loader;
use crate::loader::graph::{Graph, Recording};
use crate::phases::{self, Phase};
use crate::report::uncaught;
use crate::resolver::{self, DiskTree, ResolveError};
use crate::runtime::{Mode, start_runtime};
use crate::snapshot::StartupSnapshot;
use crate::{ConsoleOptions, Grants, RuntimeOptions};

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
    #[error("cannot build the event loop's runtime: {0}")]
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
/// [`STACK_SIZE`](crate::STACK_SIZE), as `fgro` does (`std::thread::Builder::stack_size`), not on
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
fn run_recorded(
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
    phases::mark(Phase::Teardown);
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
        .map_err(StartError::EventLoop)?;
    tokio_runtime.block_on(async {
        let mut runtime =
            start_runtime(snapshot, console, loader, Mode::Run).map_err(StartError::Runtime)?;
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
