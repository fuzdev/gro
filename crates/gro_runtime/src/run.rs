//! Running an entry module to completion: [`run_main`], the errors it
//! returns, and the `import()`s gro's rules refused during it.

use std::path::Path;
use std::rc::Rc;

use deno_core::PollEventLoopOptions;
use deno_core::error::{CoreError, CoreErrorKind};
use url::Url;

use crate::engine::Engine;
use crate::host::HostData;
use crate::loader::Loader;
use crate::loader::graph::{Graph, Reach, Reached, Recording};
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

/// How a run ended: its result, and each `import()` one of gro's rules
/// refused that the result's error isn't.
#[derive(Debug)]
#[must_use]
pub struct RunOutcome {
    /// Whether the run succeeded, or why it failed.
    pub result: Result<(), RunError>,
    /// The `import()`s refused by rule, and the imports of the modules they
    /// loaded, whether or not the program caught the refusal, so a
    /// `try`/`catch` can't hide a probe for authority; one refused because
    /// nothing is there to load or what it asks for isn't supported, as
    /// probing for an optional dependency or a feature is, isn't listed.
    /// Each refusal is listed once per specifier, importer, and reason, so
    /// a retry isn't listed again but a second reason at the same site is,
    /// sorted by specifier and importer, and none naming no host module that
    /// is the uncaught error the result's report shows: a probe is listed
    /// even then, since the program can bury or erase the report's text.
    /// Bounded as the loader's graph bounds what it keeps: the first refusal
    /// naming each host module by the project and by each of the first
    /// packages to probe, whatever else was refused, then the first
    /// few more naming a host module and the first few naming none, their
    /// text clipped past a bound.
    pub refused_imports: Vec<RefusedImport>,
    /// How many more times an `import()` was refused by rule than
    /// `refused_imports` lists, a retry of an unlisted one included.
    pub unlisted_refusals: u64,
    /// How many of `unlisted_refusals` named a host module, so a probe
    /// past the bound is told apart from the rest.
    pub unlisted_host_module_refusals: u64,
}

/// An `import()` one of gro's rules refused.
///
/// The rules guard authority and containment: a builtin the importer isn't
/// granted or imports with attributes, a package's import of the project, a
/// reserved specifier, a path leaving its root, a refused scheme, an
/// `import()` in code compiled from a string, and the like
/// (`FailureReason::guards_a_boundary` has the whole rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedImport {
    /// The specifier as written, or the URL whose load was refused.
    pub specifier: String,
    /// The importer's URL, when there is one: `deno_core` names none for the
    /// load of the module an `import()` call names, and code compiled from
    /// a string has none of its own.
    pub importer: Option<String>,
    /// The refusal's message, as the `TypeError` JS saw carries it.
    pub message: String,
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
/// Returns the result with the `import()`s refused by rule
/// ([`RunOutcome`]): its error is [`RunError::Entry`] when the entry doesn't
/// resolve, [`RunError::Start`] when the runtime can't start, and
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
/// [`STACK_SIZE`](crate::STACK_SIZE), as `fgro` does
/// (`std::thread::Builder::stack_size`), not on the main thread, whose stack
/// is the platform's (commonly 8 MiB on Linux, 1 MiB on Windows): below
/// tsv's contract, a module within the limits can still overflow the stack,
/// which aborts the process. V8's own limit for JS sits below wherever the
/// runtime starts on that thread, so the size doesn't change how deep JS can
/// go.
pub fn run_main(engine: Engine, options: &RuntimeOptions, entry: &Path, cwd: &Path) -> RunOutcome {
    let (entry, loader) = match resolve_main(options, entry, cwd, Recording::Off) {
        Ok(resolved) => resolved,
        Err(error) => {
            return RunOutcome {
                result: Err(error),
                refused_imports: Vec::new(),
                unlisted_refusals: 0,
                unlisted_host_module_refusals: 0,
            };
        }
    };
    run_resolved(engine, options, &entry, cwd, &loader)
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
) -> (RunOutcome, Option<Graph>) {
    let (entry, loader) = match resolve_main(options, entry, cwd, recording) {
        Ok(resolved) => resolved,
        Err(error) => {
            let outcome = RunOutcome {
                result: Err(error),
                refused_imports: Vec::new(),
                unlisted_refusals: 0,
                unlisted_host_module_refusals: 0,
            };
            return (outcome, None);
        }
    };
    let outcome = run_resolved(engine, options, &entry, cwd, &loader);
    // the runtime, and its share of the loader, is gone once the run ends
    (outcome, Some(loader.take_graph()))
}

/// Runs the resolved `entry` with `loader`, started in `cwd`, then marks
/// the teardown, returning the result with the `import()`s refused by rule.
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
        },
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
    // every way the run ends, the program's own exit to come, reaches its
    // outcome through this one call
    outcome(result, &loader.graph())
}

/// Why a run failed, with the message of the `TypeError` it threw, when
/// it threw one, which is how an uncaught refusal is told from the
/// refusals the program caught.
struct Failed {
    /// The run's error.
    error: RunError,
    /// The message of the uncaught error, when it is a `TypeError`, as
    /// every refused import's rejection is.
    type_error: Option<String>,
}

impl From<StartError> for Box<Failed> {
    fn from(error: StartError) -> Self {
        Self::new(Failed {
            error: error.into(),
            type_error: None,
        })
    }
}

/// The failure of a run that ended in `error`: its report, and its
/// message when it's a `TypeError`.
fn uncaught_failure(error: &CoreError) -> Box<Failed> {
    let type_error = match &*error.0 {
        CoreErrorKind::Js(js) if js.name.as_deref() == Some("TypeError") => js.message.clone(),
        _ => None,
    };
    Box::new(Failed {
        error: uncaught(error),
        type_error,
    })
}

/// The outcome of a run that ended with `result`, its loader's graph
/// `graph`: the result, and the `import()`s the graph records as refused by
/// rule, but one naming no host module that is the run's uncaught error: a
/// `TypeError` carrying its message, which its report prints. That is the
/// refusal's own rejection, or an error the program made with the same
/// class and text, which prints the same; `deno_core` hands the host the
/// error as a copy, so its identity is out of reach.
fn outcome(result: Result<(), Box<Failed>>, graph: &Graph) -> RunOutcome {
    let (result, type_error) = match result {
        Ok(()) => (Ok(()), None),
        Err(failed) => {
            let Failed { error, type_error } = *failed;
            (Err(error), type_error)
        }
    };
    // a kept refusal's message may be clipped, matched then by the hash of
    // the whole
    let type_error = type_error.map(|message| {
        let hash = blake3::hash(message.as_bytes());
        (message, hash)
    });
    let refused_imports = graph
        .failures()
        .iter()
        .filter(|(site, _)| site.reached == Reached::Import(Reach::Dynamic))
        .flat_map(|(site, failures)| {
            failures
                .iter()
                .filter(|failure| failure.reason.guards_a_boundary())
                // a probe is listed even when it's the uncaught error, whose
                // report the program can bury or erase with text of its own
                .filter(|failure| {
                    failure.reason.host_module().is_some()
                        || !type_error
                            .as_ref()
                            .is_some_and(|(message, hash)| failure.is_message(message, hash))
                })
                .map(|failure| RefusedImport {
                    specifier: site.specifier.clone(),
                    importer: site.importer.clone(),
                    message: failure.message.clone(),
                })
        })
        .collect();
    RunOutcome {
        result,
        refused_imports,
        unlisted_refusals: graph.unkept_refusals(),
        unlisted_host_module_refusals: graph.unkept_host_module_refusals(),
    }
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
) -> Result<(), Box<Failed>> {
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
    })
}

#[cfg(test)]
mod tests;
