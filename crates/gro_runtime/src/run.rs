//! Running an entry module to completion: [`run_main`], the errors it
//! returns, and what gro's rules refused during it, which fails
//! the run.

use std::path::Path;
use std::rc::Rc;

use deno_core::PollEventLoopOptions;
use deno_core::error::{CoreError, CoreErrorKind};
use url::Url;

use crate::engine::Engine;
use crate::host::HostData;
use crate::loader::Loader;
#[cfg(test)]
use crate::loader::graph::Graph;
use crate::loader::graph::Recording;
use crate::loader::refused::Refusals;
use crate::phases::{self, Phase};
use crate::report::{Report, uncaught};
use crate::resolver::{self, DiskTree, ResolveError};
use crate::runtime::{HostState, SpentRuntime, start_runtime};
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
    /// load a module: its report, as lines that can hold text the program
    /// chose, line breaks among it ([`Report`]).
    #[error("{0}")]
    Uncaught(Report),
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
    /// The run grants `exit`, and the embedder gave no step to end the
    /// process by ([`RuntimeOptions::exit_step`]).
    #[error("the run grants `exit`, and the embedder gave no step to end the process by")]
    NoExitStep,
}

/// How a run that ended on its own ended. It failed when its result is an
/// error, or when one of gro's rules refused an `import()` or a host
/// function's call during it.
///
/// A program's `exit` ends the process by the embedder's step instead
/// ([`Exit`](crate::Exit)), and no outcome follows it.
///
/// It holds the run's runtime ([`SpentRuntime`]), so it isn't `Send`: it
/// stays on the thread that ran the program.
#[derive(Debug)]
#[must_use]
pub struct RunOutcome {
    /// Whether the program ran to completion, or why it didn't.
    pub result: Result<(), RunError>,
    /// What was refused by rule: the `import()`s, a refused import in a
    /// module an `import()` loaded among them, and the host functions'
    /// calls refused for a grant (a name outside an env handle's set). Any of
    /// them fails the run, whatever `result` says and whether or not the
    /// program caught the refusal, so a `try`/`catch` can't hide a probe
    /// for authority. The import rules guard authority and containment: a
    /// builtin the importer isn't granted or imports with attributes, a
    /// package's import of the project, a reserved specifier, a path
    /// leaving its root, a refused scheme, an `import()` in code compiled
    /// from a string, and the like (`FailureReason::guards_a_boundary` has
    /// the whole rule). An import refused because nothing is there to load
    /// or what it asks for isn't supported, as probing for an optional
    /// dependency or a feature is, isn't among them, nor is a host
    /// function's refusal of an argument that is no value it takes (a `TypeError`);
    /// one that is the uncaught error `result`'s report shows isn't listed,
    /// unless another listed one shares its class and message or any was
    /// counted (`Refusals::unlist`), though past the listed ones it's
    /// counted.
    pub refused: Refusals,
    /// The exit code the program set with `set_exit_code`, if it did: the
    /// code of a run that ended on its own and didn't fail. A run that
    /// failed has failed whatever it set.
    pub exit_code: Option<u8>,
    /// The run's runtime, which is torn down when this is dropped, or left
    /// as it is by [`SpentRuntime::forget`] in a process about to exit.
    /// Unset when the run ended before a runtime was made.
    pub runtime: Option<SpentRuntime>,
}

impl RunOutcome {
    /// The outcome of a run that failed with `error` before its runtime
    /// was made, and so before any import.
    fn unstarted(error: RunError) -> Self {
        Self {
            result: Err(error),
            refused: Refusals::default(),
            exit_code: None,
            runtime: None,
        }
    }
}

/// Runs the ES module at `entry` (relative to `cwd`, or absolute) and drives
/// the event loop until no work is left, on the engine `engine` names.
///
/// `cwd` is the working directory, an absolute path. It is an input, not
/// read from the process, so an embedder decides it; `fgro` passes its own.
/// The entry resolves from it, and the cwd host module's `cwd` returns it
/// as given, when its path is UTF-8.
///
/// The event loop drains before the entry's evaluation is checked, so a
/// top-level `await` on a promise that never settles ends the run as an
/// error, not a hang.
///
/// Returns the result with the `import()`s refused by rule, which fail the
/// run too ([`RunOutcome`]): its error is [`RunError::Entry`] when the
/// entry doesn't resolve, [`RunError::Start`] when the runtime can't start,
/// and [`RunError::Uncaught`] when the program fails, including when a
/// module fails to load or strip.
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
/// the kernel sizes by `RLIMIT_STACK` (commonly 8 MiB): below
/// tsv's contract, a module within the limits can still overflow the stack,
/// which aborts the process. V8's own limit for JS sits below wherever the
/// runtime starts on that thread, so the size doesn't change how deep JS can
/// go.
pub fn run_main(engine: Engine, options: &RuntimeOptions, entry: &Path, cwd: &Path) -> RunOutcome {
    let (entry, loader) = match resolve_main(options, entry, cwd, Recording::Off) {
        Ok(resolved) => resolved,
        Err(error) => return RunOutcome::unstarted(error),
    };
    run_resolved(engine, options, &entry, cwd, &loader)
}

/// How a run ended, its runtime torn down: what a test's thread returns of
/// a [`RunOutcome`], whose runtime stays on the thread that ran it.
#[cfg(test)]
#[derive(Debug)]
struct Ended {
    /// [`RunOutcome::result`].
    result: Result<(), RunError>,
    /// [`RunOutcome::refused`].
    refused: Refusals,
    /// [`RunOutcome::exit_code`].
    exit_code: Option<u8>,
}

#[cfg(test)]
impl From<RunOutcome> for Ended {
    fn from(outcome: RunOutcome) -> Self {
        Self {
            result: outcome.result,
            refused: outcome.refused,
            exit_code: outcome.exit_code,
        }
    }
}

/// Runs as [`run_main`] does, and returns how the run ended with the graph
/// its loader recorded as `recording` says: in full, every module resolved
/// and loaded with its format and hash, the imports between them, the files
/// consulted, and the failures. The graph is unset only when the entry
/// didn't resolve.
#[cfg(test)]
fn run_recorded(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &Path,
    cwd: &Path,
    recording: Recording,
) -> (Ended, Option<Graph>) {
    let (entry, loader) = match resolve_main(options, entry, cwd, recording) {
        Ok(resolved) => resolved,
        Err(error) => return (RunOutcome::unstarted(error).into(), None),
    };
    // the runtime is torn down here, so its share of the loader is gone
    // and the graph is taken whole
    let ended = Ended::from(run_resolved(engine, options, &entry, cwd, &loader));
    (ended, Some(loader.take_graph()))
}

/// Runs the resolved `entry` with `loader`, started in `cwd`, returning
/// the result with the `import()`s refused by rule and the spent runtime.
fn run_resolved(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &Url,
    cwd: &Path,
    loader: &Rc<Loader>,
) -> RunOutcome {
    let host = HostState {
        authority: options.authority.clone(),
        data: HostData {
            args: options.args.clone(),
            cwd: cwd.to_str().map(ToOwned::to_owned),
            runtime_version: options.runtime_version.clone(),
            env: options.env.clone(),
            project: Some(loader.graph().roots().project().dir.clone()),
        },
        exit_step: options.exit_step,
    };
    let (result, mut runtime) = run_loaded(
        engine,
        options.snapshot,
        options.console,
        host,
        entry,
        Rc::clone(loader),
    );
    phases::mark(Phase::Teardown);
    let exit_code = runtime.as_mut().and_then(SpentRuntime::exit_code);
    let (result, refused) = outcome(result, loader.take_refused());
    RunOutcome {
        result,
        refused,
        exit_code,
        runtime,
    }
}

/// Why a run failed, with the class name and message of the JS error it
/// threw, when it threw one, which is how an uncaught refusal is told from
/// the refusals the program caught.
struct Failed {
    /// The run's error.
    error: RunError,
    /// The uncaught error's class name and message, when it's a JS error
    /// with both, as every refusal's is (a refused import's `TypeError`,
    /// a refused call's `EnvError`).
    uncaught: Option<(String, String)>,
}

impl From<StartError> for Box<Failed> {
    fn from(error: StartError) -> Self {
        Self::new(Failed {
            error: error.into(),
            uncaught: None,
        })
    }
}

/// The failure of a run that ended in `error`: its report, and its class
/// name and message when it's a JS error.
fn uncaught_failure(error: &CoreError) -> Box<Failed> {
    let thrown = match &*error.0 {
        CoreErrorKind::Js(js) => js.name.clone().zip(js.message.clone()),
        _ => None,
    };
    Box::new(Failed {
        error: uncaught(error),
        uncaught: thrown,
    })
}

/// How a run that ended with `result` ended, its loader having recorded
/// `refused` by rule: the result, and the refusals but the one that is the
/// run's uncaught error, an error of the class and with the message the
/// refusal's has, which the result's report prints. That is the refusal's
/// own error, or one the program made with the same class and text, which
/// prints the same; `deno_core` hands the host the error as a copy, so its
/// identity is out of reach, so when more than one listed refusal has
/// that class and message, or any refusal was counted, not listed, each
/// stays listed ([`Refusals::unlist`]).
fn outcome(
    result: Result<(), Box<Failed>>,
    mut refused: Refusals,
) -> (Result<(), RunError>, Refusals) {
    let result = result.map_err(|failed| {
        let Failed { error, uncaught } = *failed;
        if let Some((class, message)) = uncaught {
            refused.unlist(&class, &message);
        }
        error
    });
    (result, refused)
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
/// its native members holding `host`, returning the result with the runtime, once
/// one was made. `_engine` is the proof the engine was set up before V8
/// initializes.
fn run_loaded(
    _engine: Engine,
    snapshot: Option<StartupSnapshot>,
    console: ConsoleOptions,
    host: HostState,
    entry: &Url,
    loader: Rc<Loader>,
) -> (Result<(), Box<Failed>>, Option<SpentRuntime>) {
    let event_loop = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(event_loop) => event_loop,
        Err(error) => return (Err(StartError::EventLoop(error).into()), None),
    };
    let mut started = None;
    let result = event_loop.block_on(async {
        let runtime = started.insert(start_runtime(snapshot, console, host, loader)?);
        let id = runtime
            .load_main_es_module(entry)
            .await
            .map_err(|error| uncaught_failure(&error))?;
        phases::mark(Phase::Load);
        let evaluation = runtime.mod_evaluate(id);
        runtime
            .run_event_loop(PollEventLoopOptions::default())
            .await
            .map_err(|error| uncaught_failure(&error))?;
        let evaluated = evaluation.await.map_err(|error| uncaught_failure(&error));
        phases::mark(Phase::Evaluate);
        evaluated
    });
    let runtime = started.map(|runtime| SpentRuntime::new(runtime, event_loop));
    (result, runtime)
}

#[cfg(test)]
mod tests;
