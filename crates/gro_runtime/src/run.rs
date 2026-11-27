//! Running an entry module to completion: [`run_main`] from a file, which it
//! resolves itself, [`run_file`] from a file resolved before the run
//! ([`resolve_file_entry`]), and [`run_generated`] from an entry the
//! embedder generated, the errors they return, and what gro's rules refused
//! during a run, which fails it.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use deno_core::PollEventLoopOptions;
use deno_core::error::{CoreError, CoreErrorKind};
use url::Url;

use crate::confine::program::ProgramRules;
use crate::engine::{Engine, RunEngine};
use crate::free_global::FreeGlobal;
use crate::host::HostData;
use crate::host::spawn::{RunPins, SpawnRun};
use crate::loader::Loader;
#[cfg(test)]
use crate::loader::graph::Graph;
use crate::loader::graph::Recording;
use crate::loader::refused::Refusals;
use crate::phases::{self, Phase};
use crate::report::{Report, uncaught};
use crate::resolver::{self, DiskTree, EntryBound, Resolution, ResolveError, Roots};
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

/// The URL of an entry the embedder generates ([`GeneratedEntry`]): a
/// `gro:` URL, whose scheme every module's import refuses, so no module
/// imports it, and the name stack traces give its frames.
pub const GENERATED_ENTRY_URL: &str = "gro:main";

/// An entry module the embedder generates, for [`run_generated`].
///
/// It's held in memory, never on disk: JavaScript, at
/// [`GENERATED_ENTRY_URL`], whose imports resolve as a module's at the root
/// of the project at `root` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedEntry {
    /// Its source, JavaScript, never stripped.
    pub source: String,
    /// The project's root, a real path: its own root, whose origin, the
    /// project's, it takes.
    pub root: PathBuf,
}

/// An entry module on disk, resolved with the roots its graph starts with
/// before its run ([`resolve_file_entry`]), for [`run_file`].
///
/// Only [`resolve_file_entry`] makes one, so its project root is the one
/// the entry's own resolution found, never taken from the working directory it was
/// resolved from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The entry's resolution, with the `package.json` files it read.
    resolution: Resolution,
    /// The roots its graph starts with, the project's among them.
    roots: Roots,
    /// How its project root was found.
    bound: EntryBound,
}

impl FileEntry {
    /// The project's root, a real path: the directory the entry's walk for a
    /// `package.json` found, or with none the entry's own directory, or for
    /// an entry inside `node_modules` the directory holding the first
    /// `node_modules` segment (`resolver::resolve_entry`).
    pub fn root(&self) -> &Path {
        &self.roots.project().dir
    }

    /// How the project's root was found.
    pub const fn bound(&self) -> &EntryBound {
        &self.bound
    }

    /// Whether no project's `package.json` is at or above the entry, so the
    /// project's root is the entry's own directory, which the run reads and
    /// writes only as a flag naming it grants (`HostData::project_unbounded`).
    pub const fn is_unbounded(&self) -> bool {
        matches!(self.bound, EntryBound::Unbounded { .. })
    }

    /// The type markers above the root its resolution read, each a
    /// `package.json` by its real path, for an entry whose root is its own
    /// directory ([`FileEntry::is_unbounded`]): a `.js` module's type comes
    /// from the nearest one above it as each module loads, so a box that
    /// goes up before then reads each, file-only. None for any other
    /// entry, whose root's own `package.json` ends every walk at or below
    /// it.
    pub fn markers(&self) -> Vec<PathBuf> {
        if !self.is_unbounded() {
            return Vec::new();
        }
        let root = self.root();
        self.resolution
            .files_consulted
            .iter()
            .filter(|file| !file.path.starts_with(root))
            .map(|file| file.path.clone())
            .collect()
    }
}

/// Resolves the entry module at `path` from the disk, before any run.
///
/// `path` is relative to `cwd`, an absolute path, unless it's absolute itself, which ignores `cwd`. The entry
/// comes with the roots its graph starts with and how its project root was
/// found, by the rules of `resolver::resolve_entry`.
///
/// # Errors
///
/// Returns a [`ResolveError`] when the path names no loadable module.
pub fn resolve_file_entry(path: &Path, cwd: &Path) -> Result<FileEntry, ResolveError> {
    let (resolution, roots, bound) = resolver::resolve_entry_with_bound(&DiskTree, path, cwd)?;
    Ok(FileEntry {
        resolution,
        roots,
        bound,
    })
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
    /// Whether the resolver refused a Node built-in during the run, by a
    /// `node:` specifier or a bare name Node resolves as one, caught or
    /// not, which no grant allows: what the program asked for is Node's, so
    /// an embedder can say how to run it there.
    ///
    /// Kind: a review signal the run's code controls. Any of its code can
    /// set it by asking for a Node built-in and catching the refusal,
    /// so it says only that one was asked for, never that the program
    /// needs Node.
    pub node_builtin_refused: bool,
    /// Whether a program the run started ended other than by exiting 0, by
    /// a signal or a code of its own, the run never having signaled it (a
    /// timeout, an abort, a stop, a program's own `kill`), so an embedder
    /// that boxed the run can say what walls its tools may have met.
    ///
    /// Kind: a review signal the run's code controls, which can start a
    /// program that fails on purpose.
    pub child_failed: bool,
    /// The global the run's uncaught error says was free, when it's one
    /// of the set a ported script reaches for ([`FreeGlobal`]), so an
    /// embedder can say what stands for it.
    ///
    /// Kind: a review signal the run's code controls, which can throw the
    /// `ReferenceError` itself.
    pub free_global: Option<FreeGlobal>,
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
            node_builtin_refused: false,
            child_failed: false,
            free_global: None,
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
    match resolve_file_entry(entry, cwd) {
        Ok(entry) => run_file_unboxed(engine, options, entry, cwd),
        Err(error) => RunOutcome::unstarted(error.into()),
    }
}

/// Runs `entry`, resolved before the run ([`resolve_file_entry`]), as
/// [`run_main`] runs one, on any engine.
///
/// Its granted programs are pinned once its entry has resolved, and start
/// in no box of their own: it's what an embedder runs where the run's box
/// doesn't go up, as [`run_main`] does from a path. [`run_main`]'s
/// `# Stack` holds for it too.
pub fn run_file_unboxed(
    engine: Engine,
    options: &RuntimeOptions,
    entry: FileEntry,
    cwd: &Path,
) -> RunOutcome {
    let pins = pins_of(options, &entry);
    run_file_on(engine, options, entry, pins, None, cwd)
}

/// The pins of the programs a run as `options` says grants, made once its
/// entry has resolved, so a name holding `/` has the project's root, and
/// before any module evaluates ([`RunPins::pin`]).
fn pins_of(options: &RuntimeOptions, entry: &FileEntry) -> RunPins {
    RunPins::pin(
        &options.authority,
        options.spawn_env.get("PATH"),
        entry.root(),
    )
}

/// Runs `entry`, an entry module on disk resolved before the run
/// ([`resolve_file_entry`]), as [`run_main`] runs one, its granted programs
/// held by `pins`.
///
/// Each program it starts starts in a box of its own built from
/// `programs`, the rules the run's box held for them, which a run that may
/// start a program has.
///
/// The embedder made `pins` for the run's authority beneath
/// [`FileEntry::root`] before the run ([`RunPins::pin`]), so a box it made
/// for the run's process grants the same files the run launches.
///
/// It takes a [`RunEngine`], so the run's process is held in the run's box
/// before any of the entry's code runs; [`run_main`]'s `# Stack` holds for
/// it too.
pub fn run_file(
    engine: RunEngine,
    options: &RuntimeOptions,
    entry: FileEntry,
    pins: RunPins,
    programs: Option<ProgramRules>,
    cwd: &Path,
) -> RunOutcome {
    run_file_on(engine.engine(), options, entry, pins, programs, cwd)
}

/// Runs `entry` as [`run_file`] does, on any engine: what that and
/// [`run_main`] run.
fn run_file_on(
    engine: Engine,
    options: &RuntimeOptions,
    entry: FileEntry,
    pins: RunPins,
    programs: Option<ProgramRules>,
    cwd: &Path,
) -> RunOutcome {
    let unbounded = entry.is_unbounded();
    let (url, loader) = file_main(options, entry, Recording::Off);
    run_resolved(
        engine, options, &url, cwd, &loader, pins, programs, unbounded,
    )
}

/// The URL of the resolved `entry` and the loader of its graph, which
/// checks each builtin import against the run's authority, recording as
/// `recording` says.
fn file_main(
    options: &RuntimeOptions,
    entry: FileEntry,
    recording: Recording,
) -> (Url, Rc<Loader>) {
    let FileEntry {
        resolution, roots, ..
    } = entry;
    let loader = Rc::new(
        Loader::new(&resolution, roots, recording)
            .with_authority(options.authority.clone())
            .with_embed(options.embed)
            .with_conditions(options.conditions),
    );
    (resolution.url, loader)
}

/// Runs `entry`, an entry module the embedder generated, as [`run_main`]
/// runs one from a file, its granted programs held by `pins`.
///
/// Each program it starts starts in a box of its own built from
/// `programs`, the rules the run's box held for them, which a run that may
/// start a program has.
///
/// The embedder made `pins` for the run's authority beneath `entry.root`
/// before the run ([`RunPins::pin`]), so a box it made for the run's
/// process grants the same files the run launches.
///
/// `entry.root` is the project's root, which a file's walk would find: the
/// entry's origin is the project's, its imports resolve from that root,
/// and the cwd host module's `cwd` returns `cwd`. The entry's own
/// resolution reads nothing, so it never fails.
///
/// It takes a [`RunEngine`], so the run's process is held in the run's box
/// before any of the entry's code runs.
pub fn run_generated(
    engine: RunEngine,
    options: &RuntimeOptions,
    entry: &GeneratedEntry,
    pins: RunPins,
    programs: Option<ProgramRules>,
    cwd: &Path,
) -> RunOutcome {
    run_generated_on(engine.engine(), options, entry, pins, programs, cwd)
}

/// Runs `entry` as [`run_generated`] does, on any engine: what that runs,
/// and what this crate's tests of a generated entry run unboxed.
fn run_generated_on(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &GeneratedEntry,
    pins: RunPins,
    programs: Option<ProgramRules>,
    cwd: &Path,
) -> RunOutcome {
    let (url, loader) = generated_main(options, entry, Recording::Off);
    run_resolved(engine, options, &url, cwd, &loader, pins, programs, false)
}

/// The URL of the generated `entry` and the loader of its graph, which
/// checks each builtin import against the run's authority, recording as
/// `recording` says.
fn generated_main(
    options: &RuntimeOptions,
    entry: &GeneratedEntry,
    recording: Recording,
) -> (Url, Rc<Loader>) {
    // a constant, which the module's tests parse
    #[expect(clippy::expect_used, reason = "a constant URL")]
    let url = Url::parse(GENERATED_ENTRY_URL).expect("the generated entry's URL parses");
    let roots = Roots::new(entry.root.clone());
    let loader = Rc::new(
        Loader::generated(&url, entry.source.clone(), roots, recording)
            .with_authority(options.authority.clone())
            .with_embed(options.embed)
            .with_conditions(options.conditions),
    );
    (url, loader)
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
    let entry = match resolve_file_entry(entry, cwd) {
        Ok(entry) => entry,
        Err(error) => return (RunOutcome::unstarted(error.into()).into(), None),
    };
    let pins = pins_of(options, &entry);
    let unbounded = entry.is_unbounded();
    let (entry, loader) = file_main(options, entry, recording);
    // the runtime is torn down here, so its share of the loader is gone
    // and the graph is taken whole
    let ended = Ended::from(run_resolved(
        engine, options, &entry, cwd, &loader, pins, None, unbounded,
    ));
    (ended, Some(loader.take_graph()))
}

/// Runs the resolved `entry` with `loader`, started in `cwd`, its granted
/// programs held by `pins`, each started in a box built from `programs`
/// where given, its project's root `unbounded` when it's the entry's own
/// directory with no project's `package.json` at or above it
/// ([`FileEntry::is_unbounded`]), returning the result with the
/// `import()`s refused by rule and the spent runtime.
#[expect(
    clippy::too_many_arguments,
    reason = "the run's parts, each its own input, which every runner passes on"
)]
fn run_resolved(
    engine: Engine,
    options: &RuntimeOptions,
    entry: &Url,
    cwd: &Path,
    loader: &Rc<Loader>,
    pins: RunPins,
    programs: Option<ProgramRules>,
    unbounded: bool,
) -> RunOutcome {
    let project = loader.graph().roots().project().dir.clone();
    let spawn = SpawnRun::start(
        &options.authority,
        &options.spawn_env,
        &project,
        pins,
        programs,
    )
    .map(Rc::new);
    let children = spawn.clone();
    let host = HostState {
        authority: options.authority.clone(),
        data: HostData {
            args: options.args.clone(),
            cwd: cwd.to_str().map(ToOwned::to_owned),
            runtime_version: options.runtime_version.clone(),
            env: options.env.clone(),
            project: Some(project),
            project_unbounded: unbounded,
            temp_dir: options.temp_dir.clone(),
            denied_write_cause: options.denied_write_cause.clone(),
            spawn,
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
    // the event loop has returned, however the run ended: no child it
    // launched outlives it
    let child_failed = children
        .as_ref()
        .is_some_and(|spawn| spawn.any_child_failed());
    if let Some(spawn) = children {
        spawn.kill_children();
    }
    phases::mark(Phase::Teardown);
    let exit_code = runtime.as_mut().and_then(SpentRuntime::exit_code);
    let (result, refused, free_global) = outcome(result, loader.take_refused());
    RunOutcome {
        result,
        refused,
        exit_code,
        runtime,
        node_builtin_refused: loader.node_builtin_refused(),
        child_failed,
        free_global,
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
/// stays listed ([`Refusals::unlist`]); and the global the uncaught error
/// says was free, if it's one of [`FreeGlobal`]'s, from the same class and
/// message.
fn outcome(
    result: Result<(), Box<Failed>>,
    mut refused: Refusals,
) -> (Result<(), RunError>, Refusals, Option<FreeGlobal>) {
    let mut free_global = None;
    let result = result.map_err(|failed| {
        let Failed { error, uncaught } = *failed;
        if let Some((class, message)) = uncaught {
            refused.unlist(&class, &message);
            refused.need_for_uncaught(&class, &message);
            free_global = FreeGlobal::of_uncaught(&class, &message);
        }
        error
    });
    (result, refused, free_global)
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
