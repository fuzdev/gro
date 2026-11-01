//! `fgro eval`: one module evaluated to JSON data, written into one
//! [`Envelope`] that reports the value or the kind of failure.
//!
//! An eval runs in two steps, so that its process can be confined between
//! them ([`crate::confine`]): [`prepass`] resolves the entry and computes the
//! box's roots, before any runtime exists, and [`eval`] runs the rest on an
//! engine started after the box.
//!
//! The protocol, in order:
//!
//! 1. The entry resolves, as `fgro run`'s does; a failure is a `resolve`. It
//!    must be a project's module: one inside `node_modules`, or one with no
//!    project root above it (no `package.json` outside `node_modules` that
//!    isn't a type marker, up to the filesystem's root or a world-writable
//!    directory), is refused as a `resolve` the caller must fix. The project
//!    root is the box's first root, and the link closure below it the rest
//!    (`confine`); every module and `package.json` the run reads must lie
//!    under one of them, or it isn't read and the eval fails as `internal`.
//!    A graph can reach past them: the resolver follows a link the closure
//!    doesn't (one in a nested `node_modules`, or in a subdirectory's own)
//!    or drops (one to a directory holding the project or another package's
//!    directory), and reads a `package.json` symlinked out of the box at its
//!    real path.
//! 2. With a driver (its source from stdin), the driver loads and evaluates
//!    first, so it holds the realm's primordials before any of the config's
//!    code can patch them. It's a module with a synthetic URL
//!    ([`DRIVER_SPECIFIER`]) that the loader resolves to nothing: every
//!    import and `import.meta.resolve` in it is refused ("a stdin driver is
//!    import-free").
//! 3. The entry's graph loads and evaluates. Every `import()` in the run is
//!    refused before anything is resolved, whatever it names, and each in
//!    the entry's graph is listed in the envelope's `access.dynamic`.
//! 4. With a driver, its `default` export is called with the entry's module
//!    namespace and awaited; without one, the namespace's export
//!    ([`DEFAULT_EXPORT`] unless the caller names another) is the value.
//! 5. The value is serialized by the strict host serializer
//!    (the rules in `serialize.rs`), which runs no JS.
//! 6. The event loop drains: whatever work is still pending runs, and an
//!    uncaught error or unhandled rejection in it fails the eval as a
//!    `late_rejection`, with no value.
//! 7. With a driver that exports [`AFTER_DRAIN`], the host calls it, and a
//!    throw is a `late_rejection` too: it lets a driver fail a run whose
//!    code made a call into it too late, after the value was taken, even when
//!    that code caught the driver's error.
//!
//! With [`EvalOptions::graph`], the envelope also carries the run's module
//! graph ([`GraphDump`], [`dump_graph`]), built after the run from the
//! loader's record, whatever the outcome: only an envelope with a value
//! describes a run a caller may cache on. With no graph to dump (the entry
//! didn't resolve, or the box didn't go up) it is `null`. Without the
//! option, the envelope has no `graph` field and nothing is kept for one.
//!
//! In a confined process, eval starts only from a startup snapshot: from
//! source, the runtime would read its extensions' JS from the build tree,
//! outside the box, so a build with none is refused as `internal` before the
//! runtime starts.
//!
//! The engine eval runs on is [`engine_options`]'s: jitless unless the
//! caller turns the JIT on (and with it `WebAssembly`), code generation from
//! strings off in both modes, and the locale [`LOCALE`]. [`eval`] takes a
//! [`ConfinedEngine`], which only a process held in its box has, and checks
//! it against them and against the eval's box. The console writes every level to stderr,
//! uncolored, leaving stdout to the envelope. Eval reads no environment
//! variable.
//!
//! Each eval's runtime starts from the same snapshot as `fgro run`'s and is
//! then made eval's, per run:
//!
//! - **`determinism: deny`**, installed by the bootstrap's start before the
//!   driver or any module runs (`js/bootstrap.js`'s `deny`): `Math.random`,
//!   `performance`'s `now`, `mark`, `measure`, `toJSON`, and `timeOrigin`,
//!   every `Temporal.Now` function, `Date()`, `new Date()`, and `Date.now`,
//!   `Intl.DateTimeFormat`'s `format` and `formatToParts` given no date,
//!   `AbortSignal.timeout`, and `Atomics.waitAsync` with a finite timeout
//!   throw an `Error` naming the operation and the reason; `WeakRef` and
//!   `FinalizationRegistry` are inert stand-ins, and `console.time`,
//!   `timeLog`, and `timeEnd` print nothing. Each is replaced where it
//!   lives, keeping its property's attributes, so no global or member user
//!   code can name leads to the original.
//! - **No timers and no blobs**: `setTimeout`, `setInterval`,
//!   `clearTimeout`, and `clearInterval` are deleted, and so are `Blob`,
//!   `File`, `URL.createObjectURL`, and `URL.revokeObjectURL` (a `File`
//!   keeps the clock in its own state, and a blob part a random id);
//!   beneath them the op middleware refuses the timer ops, the clock and
//!   memory ops, the blob ops, and the ops that compile code or import past
//!   the loader (`bootstrap::REFUSED_OPS`); `Atomics.wait` is refused on
//!   the isolate. Late work is promise jobs.
//! - **The time zone is checked**: ICU takes it from the host's `TZ`, the
//!   one variable `fgro eval` keeps, so a zone other than UTC fails the eval
//!   as `environment` before the driver or module runs.
//! - **The limits** (`limits.rs`): the heap capped at [`HEAP_LIMIT`], with
//!   an out-of-memory backstop, and a timeout ([`DEFAULT_TIMEOUT`] unless the
//!   caller sets one), each failing the eval as `heap` or `timeout`; the
//!   process's data segment, which `fgro eval` caps at [`MEMORY_LIMIT`]
//!   before it starts a thread, the backstop failing V8 running out of
//!   memory below the heap's cap as `memory`; the
//!   envelope capped at [`MAX_ENVELOPE_SIZE`], whatever takes it past (a
//!   value's replaced by a small `serialize` failure, a failure keeping its
//!   kind with its `access` and graph dropped and its long text cut); and
//!   each module's source at
//!   [`MAX_MODULE_SIZE`](crate::resolver::MAX_MODULE_SIZE), failing it as
//!   `link`.
//!
//! A failure can carry a hint, apart from its message: `WebAssembly` not
//! defined in a jitless eval, or a timeout in one, names `--jit`; an
//! uncaught `EvalError`, which V8 throws only when code generation from
//! strings is refused, says it's off.
//!
//! An error's kind comes from the phase that failed and, for a load, from
//! the failure the loader recorded for the graph's static load, never from a
//! message: a resolution's failure is `resolve`, a load's or none at all is
//! `link` (a syntax error, a missing export binding, a strip refusal), and
//! `internal` is a failure of the host: a host bug, a file changed on disk,
//! a read outside the box refused, or a build with no snapshot that a
//! confined eval refuses.

mod dump;
mod envelope;
mod limits;
mod serialize;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use deno_core::error::{CoreError, CoreErrorKind, JsError};
use deno_core::{FastStaticString, JsRuntime, PollEventLoopOptions, ascii_str, v8};
use url::Url;

pub use self::dump::{
    DumpConsulted, DumpError, DumpFormat, DumpImport, DumpLocal, DumpModule, DumpOrigin, DumpReach,
    DumpRole, GRAPH_VERSION, GraphDump, dump_graph,
};
pub use self::envelope::{
    Access, DENO_CORE_VERSION, DynamicImport, EVAL_PROTOCOL, Entry, Envelope, ErrorKind, EvalError,
    GraphField, RuntimeInfo, TSV_REV,
};
pub use self::limits::{
    DEFAULT_THREAD_STACK, DEFAULT_TIMEOUT, HEAP_LIMIT, MAX_ENVELOPE_SIZE, MEMORY_LIMIT,
};
use self::limits::{Stop, Stopped, Watchdog};
use crate::confine;
use crate::engine::{ConfinedEngine, EngineOptions};
use crate::loader::graph::{
    Failure, FailureError, FailureSite, Graph, LoadFailure, Reach, Reached,
};
use crate::loader::reads::{Reads, RecordingTree};
use crate::loader::{Loader, Rules};
use crate::prices::{self, EvalDefault};
use crate::resolver::{self, DiskTree, EntryBound, Resolution, Roots};
use crate::snapshot::StartupSnapshot;
use crate::{ConsoleOptions, ConsoleRouting, Mode, phases, report, report_js, start_runtime};

/// The driver's URL: a scheme the resolver refuses, so no module can import
/// it, and no path on disk.
pub const DRIVER_SPECIFIER: &str = "gro:driver";

/// The export serialized with no driver, unless the caller names another.
pub const DEFAULT_EXPORT: &str = "default";

/// The driver's optional export the host calls after the drain.
pub const AFTER_DRAIN: &str = "after_drain";

/// The ICU locale eval's `Intl` uses, whatever the host's: `en-US`, whose
/// output V8 gives byte for byte as ICU's `en_US_POSIX`.
pub const LOCALE: &str = "en-US";

/// The only time zone eval runs in, by its IANA id; [`UTC_EPOCH`] decides
/// it.
pub const TIME_ZONE: &str = "UTC";

/// The epoch as `Date#toString` writes it in [`TIME_ZONE`] under [`LOCALE`].
///
/// It names the zone as `Date` displays it, and eval accepts a zone exactly
/// when its epoch reads this: `UTC` and the aliases that display as it
/// (`Etc/UTC`, `Universal`, `Zulu`, `UCT`). An alias that displays another
/// name (`Etc/GMT`, `GMT0`, and `Greenwich` print `Greenwich Mean Time`), a
/// zone ICU doesn't know (printed `GMT`), and every other zone write
/// something else.
pub const UTC_EPOCH: &str = "Thu Jan 01 1970 00:00:00 GMT+0000 (Coordinated Universal Time)";

/// The engine eval runs on: jitless unless `jit`, code generation from
/// strings off either way, and [`LOCALE`]. A priced build can turn either
/// of the last two off ([`prices`]).
pub fn engine_options(jit: bool) -> EngineOptions {
    EngineOptions {
        locale: (!prices::off(EvalDefault::Locale)).then_some(LOCALE),
        jitless: !jit,
        disallow_code_generation_from_strings: !prices::off(EvalDefault::Codegen),
    }
}

/// What to evaluate, and how.
#[derive(Debug, Clone)]
pub struct EvalOptions {
    /// The entry module: relative to `cwd`, or absolute.
    pub entry: PathBuf,
    /// The working directory, an absolute path.
    pub cwd: PathBuf,
    /// The export to serialize with no driver; [`DEFAULT_EXPORT`] when
    /// unset. Ignored with a driver.
    pub export: Option<String>,
    /// The driver's source, as read from stdin; invalid UTF-8 reads as
    /// U+FFFD, as V8 decodes a module's bytes.
    pub driver: Option<Vec<u8>>,
    /// Whether the JIT is on, which [`engine_options`] the engine must have.
    pub jit: bool,
    /// The snapshot to start from, as for `fgro run`.
    pub snapshot: Option<StartupSnapshot>,
    /// The runtime the envelope reports.
    pub runtime: RuntimeInfo,
    /// How long the run may take, from the runtime's start to the drain's
    /// end, before it's stopped as a `timeout`.
    pub timeout: Duration,
    /// Where the out-of-memory backstop writes its `heap` envelope: a
    /// handle to the stream the envelope goes to, opened before the run.
    /// Set by the first eval in a process that passes one; a later eval
    /// that passes another fails as `internal`. Without one, the process
    /// still exits 1 out of memory, with no envelope.
    pub backstop: Option<Arc<File>>,
    /// Whether the envelope carries the module graph ([`GraphField`]).
    pub graph: bool,
}

/// An eval resolved, its box computed, before any runtime: what [`prepass`]
/// returns and [`eval`] runs.
#[derive(Debug)]
pub struct Prepass {
    /// What to evaluate, and how.
    options: EvalOptions,
    /// The entry, resolved.
    entry: Resolution,
    /// The roots the entry's graph starts with.
    roots: Roots,
    /// What the entry's resolution read and found absent, its graph's
    /// first reads.
    reads: Reads,
    /// The box: the project's real directory and the link closure below it.
    confinement: confine::Prepared,
}

impl Prepass {
    /// The box the eval's process is to be confined to, before the engine
    /// starts.
    pub const fn confinement(&self) -> &confine::Prepared {
        &self.confinement
    }

    /// Confines the process to the eval's box ([`confine::Prepared::confine`]),
    /// installed for the JIT the eval's engine is to have, read from
    /// [`engine_options`], so the box and the engine take the choice from one
    /// place: jitless, the box refuses executable mappings.
    ///
    /// # Errors
    ///
    /// As [`confine::Prepared::confine`].
    pub fn confine(&self) -> Result<confine::Confined, confine::ConfineError> {
        let jit = confine::Jit::from_jitless(engine_options(self.options.jit).jitless);
        self.confinement.confine(jit)
    }

    /// Whether the eval has a startup snapshot to start from, which a
    /// confined eval needs ([`no_snapshot_error`]).
    pub const fn has_snapshot(&self) -> bool {
        self.options.snapshot.is_some()
    }

    /// The envelope of this eval failed with `error` before it ran: the
    /// entry by its canonical path.
    pub fn failed(&self, error: EvalError) -> Envelope {
        let mut envelope = failed(&self.options, error);
        envelope.entry.id = entry_id(&self.entry);
        envelope
    }
}

/// The entry's canonical path, as the envelope names it.
fn entry_id(entry: &Resolution) -> String {
    entry.url.to_file_path().map_or_else(
        |()| entry.url.to_string(),
        |path| path.to_string_lossy().into_owned(),
    )
}

/// Resolves the entry of `options` and computes the box its eval runs in,
/// reading the disk and starting no runtime: the first step of the module
/// doc's protocol, the box's roots included.
///
/// # Errors
///
/// Returns the envelope of a failed eval, a `resolve`, when the entry
/// doesn't resolve, is inside `node_modules`, or has no project root, and an
/// `internal` when the project's directory can't be opened as the one
/// resolved ([`confine::Prepared::for_project`]).
pub fn prepass(options: EvalOptions) -> Result<Prepass, Box<Envelope>> {
    let refused = |options: &EvalOptions, message: String, specifier_error: bool| {
        let mut failure = EvalError::new(ErrorKind::Resolve, message);
        failure.usage = specifier_error;
        failure.specifier = Some(options.entry.display().to_string());
        failure.importer = Some(options.cwd.display().to_string());
        failure
    };
    let mut reads = Reads::default();
    let resolved = resolver::resolve_entry_with_bound(
        &RecordingTree::new(&DiskTree, &mut reads),
        &options.entry,
        &options.cwd,
    );
    let (entry, roots, bound) = match resolved {
        Ok(resolved) => resolved,
        Err(error) => {
            let failure = refused(&options, error.to_string(), error.kind.is_specifier_error());
            return Err(Box::new(failed(&options, failure)));
        }
    };
    let project = roots.project().dir.clone();
    let refusal = match bound {
        EntryBound::Found => None,
        EntryBound::InNodeModules => Some(format!(
            "cannot evaluate {}: it is inside node_modules, and eval evaluates a project's own \
             module, whose project root bounds what it reads. Fix: evaluate a module of the \
             project, outside node_modules",
            entry_id(&entry)
        )),
        EntryBound::Unbounded { stopped_at } => Some(format!(
            "cannot evaluate {}: it has no project root, which bounds what eval reads: no \
             package.json outside node_modules that holds more than \"type\" and \
             \"sideEffects\" is at or above {}, and the search for one {}. Fix: add a \
             package.json to the project's directory",
            entry_id(&entry),
            project.display(),
            unbounded_end(stopped_at.as_deref())
        )),
    };
    if let Some(message) = refusal {
        let failure = refused(&options, message, true);
        let mut envelope = failed(&options, failure);
        envelope.entry.id = entry_id(&entry);
        return Err(Box::new(envelope));
    }
    let confinement = match confine::Prepared::for_project(project) {
        Ok(confinement) => confinement,
        Err(error) => {
            let failure = EvalError::new(
                ErrorKind::Internal,
                format!("failed to open the project's directory for the box: {error}"),
            );
            let mut envelope = failed(&options, failure);
            envelope.entry.id = entry_id(&entry);
            return Err(Box::new(envelope));
        }
    };
    Ok(Prepass {
        options,
        entry,
        roots,
        reads,
        confinement,
    })
}

/// The error of an eval whose process `error` says couldn't be confined: an
/// `environment` when the host can't hold the box, an `internal` when the
/// process broke the order the box needs.
pub fn confine_error(error: &confine::ConfineError) -> EvalError {
    let kind = if error.is_host_unsupported() {
        ErrorKind::Environment
    } else {
        ErrorKind::Internal
    };
    EvalError::new(kind, format!("cannot confine the eval: {error}"))
}

/// How the walk for a project root ended without one.
fn unbounded_end(stopped_at: Option<&Path>) -> String {
    stopped_at.map_or_else(
        || "reached the filesystem's root".to_owned(),
        |dir| {
            format!(
                "stopped at {}, a directory any user can write, reading nothing in it",
                dir.display()
            )
        },
    )
}

/// Evaluates `prepass` as the module doc describes, on the engine `engine`
/// names, and returns the envelope, whatever happened.
///
/// The engine is a confined one, so there is no eval outside the box: one
/// confined to another box than the prepass's, or started with no snapshot,
/// fails as `internal`.
///
/// Runs on the calling thread, which must reserve
/// [`STACK_SIZE`](crate::STACK_SIZE), as [`crate::run_main`] requires.
pub fn eval(engine: ConfinedEngine, prepass: Prepass) -> Envelope {
    let Prepass {
        options,
        entry,
        roots,
        reads,
        confinement,
    } = prepass;
    let options = &options;
    let driver_url = Url::parse(DRIVER_SPECIFIER).ok();
    let mut envelope = failed(
        options,
        EvalError::new(ErrorKind::Internal, "not evaluated"),
    );
    envelope.entry.id = entry_id(&entry);
    if engine.confined().roots() != confinement.roots() {
        envelope.result = Err(EvalError::new(
            ErrorKind::Internal,
            "the process is confined to another box than this eval's",
        ));
        return envelope;
    }
    if options.snapshot.is_none() {
        envelope.result = Err(no_snapshot_error());
        return envelope;
    }
    let engine = engine.engine();
    let expected = engine_options(options.jit);
    if engine.options() != &expected {
        envelope.result = Err(EvalError::new(
            ErrorKind::Internal,
            format!(
                "the engine is set up with {:?}, not eval's {expected:?}",
                engine.options()
            ),
        ));
        return envelope;
    }
    let Some(driver_url) = driver_url else {
        envelope.result = Err(EvalError::new(
            ErrorKind::Internal,
            format!("the driver's URL {DRIVER_SPECIFIER} doesn't parse"),
        ));
        return envelope;
    };
    let stopped = Stopped::default();
    if let Some(out) = &options.backstop
        && !prices::off(EvalDefault::HeapWatch)
        && let Some(envelopes) = backstop_envelopes(&envelope)
        && !limits::set_backstop(Arc::clone(out), envelopes, stopped.clone())
    {
        envelope.result = Err(EvalError::new(
            ErrorKind::Internal,
            "the out-of-memory backstop is set once per process, and an earlier eval set it",
        ));
        return envelope;
    }
    let rules = Rules {
        refuse_dynamic_import: true,
        driver: options.driver.is_some().then(|| driver_url.clone()),
        box_roots: (!prices::off(EvalDefault::BoxRule)).then(|| confinement.roots().clone()),
    };
    let box_roots = options
        .graph
        .then(|| confinement.roots().as_slice().to_vec());
    // the roots' handles are done with once the box is up
    drop(confinement);
    let loader = Rc::new(
        Loader::new(&entry, roots, recording())
            .with_reads(reads)
            .with_rules(rules),
    );
    let result = evaluate(
        options,
        &entry.url,
        &driver_url,
        Rc::clone(&loader),
        &stopped,
    );
    phases::mark(phases::Phase::Teardown);
    let breach = loader.breach();
    // the runtime, and its share of the loader, is gone once the eval ends
    let graph = Rc::try_unwrap(loader).map_or_else(|loader| loader.graph(), Loader::into_graph);
    envelope.access = Some(access(&graph));
    let mut unread = None;
    if let Some(box_roots) = box_roots {
        envelope.graph = match dump_graph(&graph, &entry.url, &box_roots) {
            Ok(dump) => GraphField::Dumped(Box::new(dump)),
            Err(DumpError::Unread(path)) => {
                unread = Some(path);
                GraphField::Unavailable
            }
            Err(DumpError::NotUtf8(_) | DumpError::NotAPath(_)) => GraphField::Unavailable,
        };
    }
    // a read refused outside the box fails the eval, whatever the run did
    // with the error, and then a graph the host failed to record; a stop
    // decides the kind otherwise, whatever the run reported after it
    envelope.result = match (result, stopped.get(), breach, unread) {
        (_, _, Some(path), _) => Err(breach_error(&path)),
        (_, _, None, Some(path)) => Err(unread_error(&path)),
        (_, Some(stop), None, None) => Err(stop_error(stop, options)),
        (result, None, None, None) => {
            result.map_err(|failed| failed.into_error(&graph, options.jit))
        }
    };
    if !prices::off(EvalDefault::EnvelopeCap) {
        cap_envelope(&mut envelope, MAX_ENVELOPE_SIZE, MAX_TEXT_KEPT);
    }
    phases::mark(phases::Phase::Envelope);
    envelope
}

/// How much the loader records of the graph: all of it, which the
/// envelope's `access` and a failure's kind read, unless a priced build
/// turns it off.
fn recording() -> crate::loader::graph::Recording {
    if prices::off(EvalDefault::Recording) {
        crate::loader::graph::Recording::Off
    } else {
        crate::loader::graph::Recording::Full
    }
}

/// The bytes of each text field of a failure past the envelope's cap that
/// are kept (`message`, `at`, `specifier`, `importer`, and `hint`): 64 KiB.
///
/// Cut to this, a failure's five fields write at most six bytes of JSON
/// for each byte they keep (a control character's `\u` escape), with the
/// note naming each one's length, under 2 MiB in all, so a failure cut fits
/// the envelope's cap with what's left of it: what the run can't make, the
/// `runtime` and `entry` the caller passed, which `fgro` bounds by its
/// command line (each argument at the kernel's 128 KiB).
const MAX_TEXT_KEPT: usize = 64 << 10;

const _: () = assert!(5 * 6 * (MAX_TEXT_KEPT + 32) < MAX_ENVELOPE_SIZE / 32);

/// Brings `envelope`, when its text would pass `max` bytes, back under
/// it as far as dropping and cutting can, which at `fgro`'s sizes always
/// does ([`MAX_TEXT_KEPT`]). A value is replaced with a small `serialize` failure naming the
/// size. A failure keeps its kind: its `access` and graph are dropped
/// (`null`, as in the out-of-memory backstop's envelope), and if it's still
/// past `max`, each of its text fields longer than `kept` bytes is cut to
/// them, at a character boundary, and named by its length (`aaa… (70000
/// bytes)`). The value is capped as it's written, but what's around it
/// isn't (an error's message or specifier, the specifiers
/// `access.dynamic` lists, the graph), and the code that ran can make any
/// of them as long as its heap holds.
fn cap_envelope(envelope: &mut Envelope, max: usize, kept: usize) {
    // counted, not written: the text past the cap is never held
    let fits = |envelope: &Envelope| envelope.json_len().is_ok_and(|size| size <= max);
    if fits(envelope) {
        return;
    }
    if envelope.result.is_ok() {
        let size = envelope.json_len().unwrap_or_default();
        envelope.result = Err(EvalError::new(
            ErrorKind::Serialize,
            format!(
                "the envelope would be {size} bytes, past {}, the envelope's cap, so the \
                 value isn't written; write less data, and keep error messages and import \
                 specifiers short",
                serialize::format_size(max)
            ),
        ));
    }
    envelope.access = None;
    envelope.graph = match envelope.graph {
        GraphField::Omitted => GraphField::Omitted,
        GraphField::Unavailable | GraphField::Dumped(_) => GraphField::Unavailable,
    };
    if fits(envelope) {
        return;
    }
    if let Err(error) = &mut envelope.result {
        cut_text(&mut error.message, kept);
        let fields = [
            &mut error.at,
            &mut error.specifier,
            &mut error.importer,
            &mut error.hint,
        ];
        for text in fields.into_iter().flatten() {
            cut_text(text, kept);
        }
    }
}

/// Cuts `text`, when it's longer than `kept` bytes, to at most `kept`, at a
/// character boundary, ending in `…` and its length in bytes.
fn cut_text(text: &mut String, kept: usize) {
    use std::fmt::Write as _;
    let length = text.len();
    if length <= kept {
        return;
    }
    let mut end = kept;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    let _ = write!(text, "… ({length} bytes)");
    text.shrink_to_fit();
}

/// The error of an eval whose graph names `path` as consulted with no read
/// of it recorded, which only a bug in the host makes: the graph would name
/// a file with no hash, so the eval fails rather than write one partial or
/// none under a value.
fn unread_error(path: &Path) -> EvalError {
    EvalError::new(
        ErrorKind::Internal,
        format!(
            "the eval's graph names {} as consulted with no read of it recorded, a bug in \
             fgro, so no graph is written",
            path.display()
        ),
    )
}

/// The error of an eval whose loader was asked to read `path`, outside the
/// box: reached through a link the closure doesn't follow, or a
/// `package.json` symlinked out of the box.
fn breach_error(path: &Path) -> EvalError {
    let mut error = EvalError::new(
        ErrorKind::Internal,
        format!(
            "the eval was to read {}, which lies outside its box, so it wasn't read: the box \
             is the project root and each package linked directly into its node_modules (or \
             an @scope directory there) from outside it, followed through each one's own \
             node_modules, except a link to a directory with no package.json file of its \
             own, to a directory holding the project or another package's directory, or to \
             one a world-writable directory holds; a link elsewhere (in a nested \
             node_modules, or a subdirectory's own) or a package.json symlinked out of the \
             box reaches past it",
            path.display()
        ),
    );
    error.hint = Some(BREACH_HINT.to_owned());
    error
}

/// The hint for a read outside the box.
const BREACH_HINT: &str = "link the package's own directory directly into the project's \
                           node_modules, not a directory holding the project or another \
                           package, and keep each package.json a file rather than a symlink";

/// The error of a run the host stopped.
fn stop_error(stop: Stop, options: &EvalOptions) -> EvalError {
    match stop {
        Stop::Heap => EvalError::new(
            ErrorKind::Heap,
            format!(
                "the eval's heap reached its cap of {} MiB, and the run was stopped",
                HEAP_LIMIT >> 20
            ),
        ),
        Stop::Memory => EvalError::new(
            ErrorKind::Memory,
            format!(
                "the eval ran out of memory outside the heap's cap: an allocation failed with \
                 the process at its memory cap of {} MiB, or a lower one its caller set, and \
                 the run was stopped",
                MEMORY_LIMIT >> 20
            ),
        ),
        Stop::Timeout => {
            let mut error = EvalError::new(
                ErrorKind::Timeout,
                format!(
                    "the eval ran past its timeout of {} ms, and the run was stopped",
                    options.timeout.as_millis()
                ),
            );
            if !options.jit {
                error.hint = Some(TIMEOUT_HINT.to_owned());
            }
            error
        }
    }
}

/// The `heap` and `memory` envelopes the out-of-memory backstop chooses
/// between for an eval whose envelope so far is `envelope`, each one line:
/// written without the run, they carry no `access`, and no graph, `null`
/// when one was asked for.
fn backstop_envelopes(envelope: &Envelope) -> Option<limits::BackstopEnvelopes> {
    let prepared = |error: EvalError| {
        let backstop = Envelope {
            runtime: envelope.runtime.clone(),
            entry: envelope.entry.clone(),
            result: Err(error),
            access: None,
            // prepared before the run, so never a graph, only its absence
            graph: match envelope.graph {
                GraphField::Omitted => GraphField::Omitted,
                GraphField::Unavailable | GraphField::Dumped(_) => GraphField::Unavailable,
            },
        };
        let mut json = backstop.to_json().ok()?;
        json.push('\n');
        Some(json)
    };
    Some(limits::BackstopEnvelopes {
        heap: prepared(EvalError::new(
            ErrorKind::Heap,
            format!(
                "the eval's heap reached its cap of {} MiB with nothing left to give: one \
                 operation V8 can't stop ran out of memory, so the process exited",
                HEAP_LIMIT >> 20
            ),
        ))?,
        memory: prepared(EvalError::new(
            ErrorKind::Memory,
            format!(
                "the eval ran out of memory outside the heap's cap: the process reached its \
                 memory cap of {} MiB, or a lower one its caller set, so it exited",
                MEMORY_LIMIT >> 20
            ),
        ))?,
    })
}

/// The error of a confined eval with no startup snapshot, an `internal`.
///
/// Started from source, its runtime would read its extensions' JS from the
/// build tree, outside the box. [`eval`] refuses one, and a caller that
/// confines refuses one with this before it confines, whatever the host.
pub fn no_snapshot_error() -> EvalError {
    let mut error = EvalError::new(
        ErrorKind::Internal,
        "a confined eval starts only from a startup snapshot embedded in the binary, and this \
         build has none: started from source, the runtime reads its extensions' JS from the \
         build tree, outside the box",
    );
    error.hint = Some(NO_SNAPSHOT_HINT.to_owned());
    error
}

/// The hint for a confined eval with no startup snapshot.
const NO_SNAPSHOT_HINT: &str = "build fgro with its snapshot: cargo xtask build";

/// The hint for a `WebAssembly` that isn't defined in a jitless eval.
const WASM_HINT: &str = "eval runs jitless unless asked, and WebAssembly comes only with V8's \
                         JIT: pass --jit to turn it on";

/// The hint for an uncaught `EvalError`.
const CODE_GENERATION_HINT: &str = "eval turns code generation from strings off, with or \
                                    without --jit: eval(), new Function(), and the other \
                                    function constructors throw an EvalError; write the code \
                                    in the module instead";

/// The hint for a jitless eval that timed out.
const TIMEOUT_HINT: &str = "eval runs jitless unless asked, in V8's interpreter alone: \
                            --jit runs compute-heavy code faster, or pass a longer --timeout";

/// The envelope of an eval of `options` that failed with `error` before it
/// started, such as when the driver couldn't be read: the entry as given.
pub fn failed(options: &EvalOptions, error: EvalError) -> Envelope {
    Envelope {
        runtime: options.runtime.clone(),
        entry: Entry {
            id: options.entry.to_string_lossy().into_owned(),
            export: options.driver.is_none().then(|| {
                options
                    .export
                    .clone()
                    .unwrap_or_else(|| DEFAULT_EXPORT.to_owned())
            }),
            driver_blake3: options
                .driver
                .as_ref()
                .map(|driver| blake3::hash(driver).to_hex().to_string()),
        },
        result: Err(error),
        access: Some(Access::default()),
        graph: if options.graph {
            GraphField::Unavailable
        } else {
            GraphField::Omitted
        },
    }
}

/// The phase of an eval, which decides a failure's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Starting the runtime.
    Start,
    /// Loading and linking the driver.
    DriverLoad,
    /// Evaluating the driver.
    DriverEvaluate,
    /// Reading the driver's exports.
    DriverExports,
    /// Loading and linking the entry's graph.
    Load,
    /// Evaluating the entry's graph.
    Evaluate,
    /// Reading the export, with no driver.
    Export,
    /// Calling the driver's `default`.
    Call,
    /// Serializing the value.
    Serialize,
    /// Draining the event loop, and calling the driver's `after_drain`.
    Drain,
    /// Checking the process's environment, the time zone, before any code
    /// runs.
    Environment,
    /// The host broke an invariant of its own.
    Internal,
}

/// A failed phase: which, the message, and where when the serializer says.
#[derive(Debug)]
struct Failed {
    /// The phase that failed.
    phase: Phase,
    /// What failed.
    message: String,
    /// The JS path the serializer refused.
    at: Option<String>,
    /// The uncaught JS error, when one failed the phase.
    thrown: Option<Thrown>,
}

/// An uncaught JS error, as V8 named it: what the hints read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Thrown {
    /// Its `name`, the class for a native error (`EvalError`).
    name: Option<String>,
    /// Its `message`.
    message: Option<String>,
}

impl Failed {
    fn new(phase: Phase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
            at: None,
            thrown: None,
        }
    }

    /// The failure in `phase` that `error` reports.
    fn core(phase: Phase, error: &CoreError) -> Self {
        match &*error.0 {
            CoreErrorKind::Js(error) => Self::js(phase, error),
            _ => Self::new(phase, report(error)),
        }
    }

    /// The failure in `phase` of the uncaught JS `error`.
    fn js(phase: Phase, error: &JsError) -> Self {
        let mut failed = Self::new(phase, report_js(error));
        failed.thrown = Some(Thrown {
            name: error.name.clone(),
            message: error.message.clone(),
        });
        failed
    }

    /// The envelope's error: the kind from the phase and, for a load, the
    /// first failure of the static load in `graph`, with a hint when the
    /// uncaught error has one in an eval with the JIT as `jit` says.
    fn into_error(self, graph: &Graph, jit: bool) -> EvalError {
        let kind = match self.phase {
            Phase::Start | Phase::Internal => ErrorKind::Internal,
            Phase::DriverLoad | Phase::Load => {
                return load_error(self.message, first_static_failure(graph));
            }
            Phase::DriverEvaluate | Phase::Evaluate | Phase::Call => ErrorKind::ModuleThrew,
            Phase::DriverExports | Phase::Export => ErrorKind::ExportMissing,
            Phase::Serialize => ErrorKind::Serialize,
            Phase::Drain => ErrorKind::LateRejection,
            Phase::Environment => ErrorKind::Environment,
        };
        let mut error = EvalError::new(kind, self.message);
        error.at = self.at;
        error.hint = self.thrown.as_ref().and_then(|thrown| hint(thrown, jit));
        error
    }
}

/// The hint for an uncaught `thrown` in an eval with the JIT as `jit`
/// says, keyed on V8's class and, for `WebAssembly`, on the one message V8
/// gives an undefined name, the only signal there is.
fn hint(thrown: &Thrown, jit: bool) -> Option<String> {
    match (thrown.name.as_deref(), thrown.message.as_deref()) {
        // V8 throws an EvalError natively only for code generation refused
        (Some("EvalError"), _) => Some(CODE_GENERATION_HINT.to_owned()),
        (Some("ReferenceError"), Some("WebAssembly is not defined")) if !jit => {
            Some(WASM_HINT.to_owned())
        }
        _ => None,
    }
}

/// The first failure of a static load: a load records at most one, since
/// `deno_core` stops it at the first error and linking resolves nothing
/// again, and a failed driver load ends the eval before the entry's.
fn first_static_failure(graph: &Graph) -> Option<(&FailureSite, &Failure)> {
    graph
        .failures
        .iter()
        .filter(|(site, _)| site.reached == Reached::Import(Reach::Static))
        .find_map(|(site, failures)| failures.first().map(|failure| (site, failure)))
}

/// The error of a failed load, its kind from the first static failure
/// recorded: a resolution's is `resolve`; a load's is `link`, unless only a
/// host bug or a file changed on disk causes it; none is `link`.
fn load_error(message: String, failure: Option<(&FailureSite, &Failure)>) -> EvalError {
    let Some((site, failure)) = failure else {
        return EvalError::new(ErrorKind::Link, message);
    };
    let kind = match &failure.error {
        FailureError::Resolve(_) | FailureError::DriverImport => ErrorKind::Resolve,
        FailureError::Load(
            LoadFailure::Strip { .. } | LoadFailure::ImportAttributes | LoadFailure::TooLarge,
        ) => ErrorKind::Link,
        FailureError::Load(
            LoadFailure::NeverResolved
            | LoadFailure::IdentityMismatch
            | LoadFailure::Read(_)
            | LoadFailure::NotAFile
            | LoadFailure::Format(_),
        )
        | FailureError::UnknownImporter
        // never static: an `import()` is refused before it loads anything
        | FailureError::DynamicImport => ErrorKind::Internal,
    };
    let at = match &failure.error {
        // a position in the module stripped
        FailureError::Load(LoadFailure::Strip { at: Some(at), .. }) => {
            Some(format!("{}:{}:{}", site.specifier, at.line, at.column))
        }
        // the import's position in its importer
        _ => failure.at.and_then(|at| {
            site.importer
                .as_ref()
                .map(|importer| format!("{importer}:{}:{}", at.line, at.column))
        }),
    };
    let mut error = EvalError::new(kind, message);
    error.at = at;
    error.specifier = Some(site.specifier.clone());
    error.importer.clone_from(&site.importer);
    error
}

/// Every `import()` the graph refused, in specifier order, then importer
/// order, one entry per specifier and importer; the driver's are recorded as
/// driver imports, not listed.
fn access(graph: &Graph) -> Access {
    let dynamic = graph
        .failures
        .iter()
        .filter(|(_, failures)| {
            failures
                .iter()
                .any(|failure| failure.error == FailureError::DynamicImport)
        })
        .filter_map(|(site, _)| {
            Some(DynamicImport {
                specifier: site.specifier.clone(),
                importer: site.importer.clone()?,
            })
        })
        .collect();
    Access { dynamic }
}

/// The console eval writes with: every level to stderr, uncolored.
const CONSOLE: ConsoleOptions = ConsoleOptions {
    routing: ConsoleRouting::Stderr,
    color_stdout: false,
    color_stderr: false,
};

/// The driver's exports the host calls, read once it has evaluated.
struct Driver {
    /// Its `default`, called with the entry's namespace.
    default: v8::Global<v8::Function>,
    /// Its `after_drain`, when it exports one.
    after_drain: Option<v8::Global<v8::Function>>,
}

/// Runs the eval's phases on a runtime over `loader`, returning the value
/// as JSON text, or the phase that failed.
fn evaluate(
    options: &EvalOptions,
    entry: &Url,
    driver_url: &Url,
    loader: Rc<Loader>,
    stopped: &Stopped,
) -> Result<String, Failed> {
    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            Failed::new(
                Phase::Start,
                format!("failed to start the runtime: {error}"),
            )
        })?;
    tokio_runtime.block_on(async {
        let mut runtime = start_runtime(options.snapshot, CONSOLE, loader, Mode::Eval)
            .map_err(|message| Failed::new(Phase::Start, message))?;
        let result = run_phases(&mut runtime, options, entry, driver_url, stopped).await;
        // the near-heap-limit callback decides from the heap it reads; a
        // `memory` stop is read again for one large object the raised cap
        // let in after it
        if stopped.get() == Some(Stop::Memory) {
            let used = runtime.v8_isolate().get_heap_statistics().used_heap_size();
            stopped.check_heap(used);
        }
        result
    })
}

/// Runs the eval's phases on `runtime`, from the heap's watch and the
/// timeout's watchdog to the drain, returning the value as JSON text, or the
/// phase that failed.
#[expect(
    clippy::future_not_send,
    reason = "a runtime stays on its thread, driven by a current-thread tokio runtime"
)]
async fn run_phases(
    runtime: &mut JsRuntime,
    options: &EvalOptions,
    entry: &Url,
    driver_url: &Url,
    stopped: &Stopped,
) -> Result<String, Failed> {
    if !prices::off(EvalDefault::HeapWatch) {
        limits::watch_heap(runtime, stopped.clone());
    }
    // stopped, and joined, when the eval returns
    let _watchdog = if prices::off(EvalDefault::Watchdog) {
        None
    } else {
        Some(
            Watchdog::start(runtime, options.timeout, stopped.clone()).map_err(|error| {
                Failed::new(
                    Phase::Start,
                    format!("failed to start the timeout's watchdog: {error}"),
                )
            })?,
        )
    };
    if !prices::off(EvalDefault::UtcCheck) {
        check_time_zone(runtime)?;
    }
    phases::mark(phases::Phase::Guards);
    let driver = match &options.driver {
        Some(source) => {
            let driver = load_driver(runtime, driver_url, source).await?;
            phases::mark(phases::Phase::Driver);
            Some(driver)
        }
        None => None,
    };
    let id = runtime
        .load_main_es_module(entry)
        .await
        .map_err(|error| Failed::core(Phase::Load, &error))?;
    phases::mark(phases::Phase::Load);
    let evaluation = runtime.mod_evaluate(id);
    settle(runtime, evaluation)
        .await
        .map_err(|unsettled| unsettled.failed(Phase::Evaluate, "the module's evaluation"))?;
    let namespace = runtime
        .get_module_namespace(id)
        .map_err(|error| Failed::core(Phase::Internal, &error))?;
    let value = if let Some(driver) = &driver {
        let namespace = to_value(runtime, &namespace);
        let call = runtime.call_with_args(&driver.default, &[namespace]);
        settle(runtime, call)
            .await
            .map_err(|unsettled| unsettled.failed(Phase::Call, "the driver's call"))?
    } else {
        let name = options.export.as_deref().unwrap_or(DEFAULT_EXPORT);
        read_export(runtime, &namespace, name, Phase::Evaluate)?.ok_or_else(|| {
            Failed::new(
                Phase::Export,
                format!("the module has no export named `{name}` to evaluate"),
            )
        })?
    };
    phases::mark(phases::Phase::Evaluate);
    let json = serialize_value(runtime, &value, stopped)?;
    phases::mark(phases::Phase::Serialize);
    runtime
        .run_event_loop(PollEventLoopOptions::default())
        .await
        .map_err(|error| Failed::core(Phase::Drain, &error))?;
    if let Some(after_drain) = driver
        .as_ref()
        .and_then(|driver| driver.after_drain.as_ref())
    {
        let call = runtime.call(after_drain);
        settle(runtime, call)
            .await
            .map_err(|unsettled| unsettled.failed(Phase::Drain, "the driver's after_drain"))?;
    }
    phases::mark(phases::Phase::Drain);
    Ok(json)
}

/// Refuses a time zone other than [`TIME_ZONE`] on the started runtime,
/// before any user code runs: anything whose epoch isn't [`UTC_EPOCH`]
/// under [`LOCALE`], which only `UTC` and its aliases that display as it
/// write. ICU takes the zone from the host's `TZ`, which eval itself doesn't
/// read.
///
/// Acceptance reads the `Date` string alone, since the first
/// `Intl.DateTimeFormat` builds V8's set of every ICU date-format locale,
/// which costs more than the rest of eval's setup; `Intl` is consulted only
/// to name a refused zone.
fn check_time_zone(runtime: &mut JsRuntime) -> Result<(), Failed> {
    // `new Date(value)` is kept under `determinism: deny`
    let epoch = read_script(runtime, ascii_str!("`${new Date(0)}`"))?;
    if epoch == UTC_EPOCH {
        return Ok(());
    }
    // the id, empty when ICU knows no zone by that name
    let zone = read_script(
        runtime,
        ascii_str!(
            "((zone) => typeof zone === 'string' && zone !== 'Etc/Unknown' ? zone : \
             '')(new Intl.DateTimeFormat().resolvedOptions().timeZone)"
        ),
    )?;
    let found = if zone.is_empty() {
        "this process's time zone isn't one ICU knows".to_owned()
    } else if zone == TIME_ZONE {
        // the name `Date#toString` gives in parentheses
        let name = epoch
            .rsplit_once(" (")
            .map_or(epoch.as_str(), |(_, name)| name.trim_end_matches(')'));
        format!("this process's is an alias of {TIME_ZONE} that dates name {name}")
    } else {
        format!("this process's is {zone}")
    };
    Err(Failed::new(
        Phase::Environment,
        format!(
            "eval runs only in the time zone {TIME_ZONE}, so dates read and format the same \
             everywhere, and {found}: run fgro with TZ=UTC in its environment"
        ),
    ))
}

/// Runs `source`, the host's own script, on the started runtime, and reads
/// its value as a string.
fn read_script(runtime: &mut JsRuntime, source: FastStaticString) -> Result<String, Failed> {
    let read = runtime
        .execute_script("ext:gro/time_zone.js", source)
        .map_err(|error| Failed::new(Phase::Internal, report_js(&error)))?;
    deno_core::scope!(scope, runtime);
    Ok(v8::Local::new(scope, read).to_rust_string_lossy(scope))
}

/// Loads and evaluates the driver from `source` at `url`, and reads the
/// exports the host calls.
#[expect(
    clippy::future_not_send,
    reason = "a runtime stays on its thread, driven by a current-thread tokio runtime"
)]
async fn load_driver(runtime: &mut JsRuntime, url: &Url, source: &[u8]) -> Result<Driver, Failed> {
    let source = String::from_utf8_lossy(source).into_owned();
    let id = runtime
        .load_side_es_module_from_code(url, source)
        .await
        .map_err(|error| Failed::core(Phase::DriverLoad, &error))?;
    let evaluation = runtime.mod_evaluate(id);
    settle(runtime, evaluation)
        .await
        .map_err(|unsettled| unsettled.failed(Phase::DriverEvaluate, "the driver's evaluation"))?;
    let namespace = runtime
        .get_module_namespace(id)
        .map_err(|error| Failed::core(Phase::Internal, &error))?;
    let function = |runtime: &mut JsRuntime, name: &str| {
        let Some(value) = read_export(runtime, &namespace, name, Phase::DriverEvaluate)? else {
            return Ok(None);
        };
        deno_core::scope!(scope, runtime);
        let value = v8::Local::new(scope, value);
        if value.is_undefined() {
            return Ok(None);
        }
        let function = value.try_cast::<v8::Function>().map_err(|_| {
            Failed::new(
                Phase::DriverExports,
                format!("the driver's `{name}` export is not a function"),
            )
        })?;
        Ok(Some(v8::Global::new(scope, function)))
    };
    let default = function(runtime, DEFAULT_EXPORT)?.ok_or_else(|| {
        Failed::new(
            Phase::DriverExports,
            "the driver has no `default` export function to call with the module's namespace",
        )
    })?;
    let after_drain = function(runtime, AFTER_DRAIN)?;
    Ok(Driver {
        default,
        after_drain,
    })
}

/// `namespace` as a value to pass to a call.
fn to_value(runtime: &mut JsRuntime, namespace: &v8::Global<v8::Object>) -> v8::Global<v8::Value> {
    deno_core::scope!(scope, runtime);
    let namespace = v8::Local::new(scope, namespace);
    v8::Global::new(scope, v8::Local::<v8::Value>::from(namespace))
}

/// The export `name` of the module `namespace`, or `None` when it has no
/// such export. Reading a binding still uninitialized throws, a failure in
/// `phase`.
fn read_export(
    runtime: &mut JsRuntime,
    namespace: &v8::Global<v8::Object>,
    name: &str,
    phase: Phase,
) -> Result<Option<v8::Global<v8::Value>>, Failed> {
    deno_core::scope!(scope, runtime);
    v8::tc_scope!(let scope, scope);
    let namespace = v8::Local::new(scope, namespace);
    let internal = || Failed::new(Phase::Internal, format!("can't read the export `{name}`"));
    let key = v8::String::new(scope, name).ok_or_else(internal)?;
    // a namespace's own properties are its exports, read without running JS
    let has = namespace.has_own_property(scope, key.into());
    let value = match has {
        Some(false) => return Ok(None),
        Some(true) => namespace.get(scope, key.into()),
        None => None,
    };
    if let Some(value) = value {
        return Ok(Some(v8::Global::new(scope, value)));
    }
    Err(scope.exception().map_or_else(internal, |exception| {
        let error = JsError::from_v8_exception(scope, exception);
        Failed::js(phase, &error)
    }))
}

/// `value` serialized by the strict host serializer, which ends early once
/// `stopped` records a stop.
fn serialize_value(
    runtime: &mut JsRuntime,
    value: &v8::Global<v8::Value>,
    stopped: &Stopped,
) -> Result<String, Failed> {
    deno_core::scope!(scope, runtime);
    let value = v8::Local::new(scope, value);
    serialize::to_json(scope, value, &|| stopped.get().is_some(), MAX_ENVELOPE_SIZE).map_err(
        |error| Failed {
            phase: if error.is_internal() {
                Phase::Internal
            } else {
                Phase::Serialize
            },
            message: error.to_string(),
            at: Some(error.at),
            thrown: None,
        },
    )
}

/// Why a future the event loop drives didn't settle.
enum Unsettled {
    /// It failed, or the event loop did while it was pending: an uncaught
    /// error, an unhandled rejection, or a stalled top-level `await`.
    Failed(CoreError),
    /// The event loop ran out of work with it still pending.
    Pending,
}

impl Unsettled {
    /// The failure in `phase`, `what` naming what never settled.
    fn failed(self, phase: Phase, what: &str) -> Failed {
        match self {
            Self::Failed(error) => Failed::core(phase, &error),
            Self::Pending => Failed::new(
                phase,
                format!("{what} never settled: it awaits a promise nothing is left to settle"),
            ),
        }
    }
}

/// Drives `future` with `runtime`'s event loop until it settles, polling
/// the event loop first each time, so an error the loop reports (an
/// unhandled rejection: `deno_core` settles a module's evaluation even when
/// it throws, and reports the throw through the loop) wins over the future's
/// settling in the same turn.
#[expect(
    clippy::future_not_send,
    reason = "a runtime stays on its thread, driven by a current-thread tokio runtime"
)]
async fn settle<T, E>(
    runtime: &mut JsRuntime,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, Unsettled>
where
    CoreError: From<E>,
{
    let mut future = Box::pin(future);
    std::future::poll_fn(|cx| {
        let looped = runtime.poll_event_loop(cx, PollEventLoopOptions::default());
        if let Poll::Ready(Err(error)) = looped {
            return Poll::Ready(Err(Unsettled::Failed(error)));
        }
        if let Poll::Ready(result) = Pin::as_mut(&mut future).poll(cx) {
            return Poll::Ready(result.map_err(|error| Unsettled::Failed(error.into())));
        }
        if looped.is_ready() {
            return Poll::Ready(Err(Unsettled::Pending));
        }
        Poll::Pending
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::loader::graph::{Position, Recording};
    use crate::loader::strip::StripFailure;
    use crate::resolver::{FileId, Origin, Resolution, ResolveErrorKind, Roots};

    /// A graph of `/p/main.js` with `failures` recorded, each at its own
    /// site and reached as given.
    fn graph(failures: &[(&str, Reached, FailureError)]) -> Graph {
        let entry = Resolution {
            url: Url::parse("file:///p/main.js").unwrap(),
            file_id: FileId {
                dev: 1,
                ino: 1,
                born: None,
            },
            root: PathBuf::from("/p"),
            origin: Origin::Project,
            files_consulted: Vec::new(),
        };
        let mut graph = Graph::new(&entry, Roots::new(PathBuf::from("/p")), Recording::Full);
        for (specifier, reached, error) in failures {
            let failure = Failure {
                at: Some(Position { line: 2, column: 8 }),
                error: error.clone(),
            };
            graph.fail(specifier, Some("file:///p/main.js"), *reached, failure);
        }
        graph
    }

    const STATIC: Reached = Reached::Import(Reach::Static);
    const DYNAMIC: Reached = Reached::Import(Reach::Dynamic);

    fn load_kind(error: FailureError) -> ErrorKind {
        Failed::new(Phase::Load, "failed")
            .into_error(&graph(&[("./x.js", STATIC, error)]), false)
            .kind
    }

    /// A load's kind comes from its static failure's variant, never its
    /// message: a resolution's is `resolve`, a strip refusal, import
    /// attributes, or a module past its byte cap `link`, and what only a
    /// host bug or a file changed on disk causes `internal`.
    #[test]
    fn a_load_failure_takes_its_kind_from_the_recorded_failure() {
        let not_found = ResolveErrorKind::NotFound { ts_sibling: None };
        for (error, kind) in [
            (FailureError::Resolve(not_found.clone()), ErrorKind::Resolve),
            (FailureError::DriverImport, ErrorKind::Resolve),
            (
                FailureError::Load(LoadFailure::Strip {
                    failure: StripFailure::Parse,
                    at: None,
                }),
                ErrorKind::Link,
            ),
            (
                FailureError::Load(LoadFailure::ImportAttributes),
                ErrorKind::Link,
            ),
            (FailureError::Load(LoadFailure::TooLarge), ErrorKind::Link),
            (
                FailureError::Load(LoadFailure::IdentityMismatch),
                ErrorKind::Internal,
            ),
            (
                FailureError::Load(LoadFailure::Read(io::ErrorKind::PermissionDenied)),
                ErrorKind::Internal,
            ),
            (
                FailureError::Load(LoadFailure::NeverResolved),
                ErrorKind::Internal,
            ),
            (
                FailureError::Load(LoadFailure::NotAFile),
                ErrorKind::Internal,
            ),
            (
                FailureError::Load(LoadFailure::Format(not_found)),
                ErrorKind::Internal,
            ),
            (FailureError::UnknownImporter, ErrorKind::Internal),
        ] {
            assert_eq!(load_kind(error.clone()), kind, "{error:?}");
        }
    }

    /// With no static failure, a failed load is `link` (a syntax error, a
    /// missing export binding), whatever failed dynamically; a failure's
    /// site and position are reported.
    #[test]
    fn a_load_with_no_static_failure_is_link() {
        let dynamic = graph(&[("./y.js", DYNAMIC, FailureError::DynamicImport)]);
        let error = Failed::new(Phase::Load, "SyntaxError").into_error(&dynamic, false);
        assert_eq!(
            (error.kind, error.specifier, error.importer, error.at),
            (ErrorKind::Link, None, None, None)
        );
        let error = Failed::new(Phase::DriverLoad, "refused").into_error(
            &graph(&[(
                "./x.js",
                STATIC,
                FailureError::Resolve(ResolveErrorKind::NotFound { ts_sibling: None }),
            )]),
            false,
        );
        assert_eq!(error.kind, ErrorKind::Resolve);
        assert_eq!(error.specifier.as_deref(), Some("./x.js"));
        assert_eq!(error.importer.as_deref(), Some("file:///p/main.js"));
        assert_eq!(error.at.as_deref(), Some("file:///p/main.js:2:8"));
        assert!(!error.usage);
    }

    #[test]
    fn every_other_phase_has_its_kind() {
        let graph = graph(&[]);
        for (phase, kind) in [
            (Phase::Start, ErrorKind::Internal),
            (Phase::Internal, ErrorKind::Internal),
            (Phase::DriverEvaluate, ErrorKind::ModuleThrew),
            (Phase::Evaluate, ErrorKind::ModuleThrew),
            (Phase::Call, ErrorKind::ModuleThrew),
            (Phase::DriverExports, ErrorKind::ExportMissing),
            (Phase::Export, ErrorKind::ExportMissing),
            (Phase::Serialize, ErrorKind::Serialize),
            (Phase::Drain, ErrorKind::LateRejection),
            (Phase::Environment, ErrorKind::Environment),
        ] {
            assert_eq!(
                Failed::new(phase, "x").into_error(&graph, false).kind,
                kind,
                "{phase:?}"
            );
        }
    }

    /// `access.dynamic` lists the refused `import()` calls, not other
    /// failures.
    #[test]
    fn access_lists_the_refused_dynamic_imports() {
        let graph = graph(&[
            ("./b.js", DYNAMIC, FailureError::DynamicImport),
            ("./a.js", DYNAMIC, FailureError::DynamicImport),
            ("./c.js", STATIC, FailureError::DriverImport),
        ]);
        let importer = "file:///p/main.js".to_owned();
        assert_eq!(
            access(&graph).dynamic,
            [
                DynamicImport {
                    specifier: "./a.js".to_owned(),
                    importer: importer.clone(),
                },
                DynamicImport {
                    specifier: "./b.js".to_owned(),
                    importer,
                },
            ]
        );
    }

    /// An envelope with `result` and `access`, and the graph `graph`.
    fn envelope(
        result: Result<String, EvalError>,
        dynamic: &[&str],
        graph: GraphField,
    ) -> Envelope {
        Envelope {
            runtime: RuntimeInfo::new("0.0.0", None),
            entry: Entry {
                id: "/p/main.js".to_owned(),
                export: Some(DEFAULT_EXPORT.to_owned()),
                driver_blake3: None,
            },
            result,
            access: Some(Access {
                dynamic: dynamic
                    .iter()
                    .map(|specifier| DynamicImport {
                        specifier: (*specifier).to_owned(),
                        importer: "file:///p/main.js".to_owned(),
                    })
                    .collect(),
            }),
            graph,
        }
    }

    /// A value past the cap, or one whose envelope passes it with the
    /// specifiers `access.dynamic` lists (two caught `import()` calls of
    /// long specifiers), is replaced with a small `serialize` failure naming
    /// the size it would have been, with no `access` and no graph (`null`
    /// when it was asked for). One at the cap is kept.
    #[test]
    fn a_value_past_the_cap_fails_as_serialize() {
        let long = "a".repeat(4096);
        let dynamic = format!("./{long}");
        let cases = [
            envelope(Ok(format!("\"{long}\"")), &[], GraphField::Omitted),
            envelope(
                Ok("1".to_owned()),
                &[
                    format!("{dynamic}0").as_str(),
                    format!("{dynamic}1").as_str(),
                ],
                GraphField::Unavailable,
            ),
        ];
        for original in cases {
            let size = original.json_len().unwrap();
            let mut kept = original.clone();
            cap_envelope(&mut kept, size, 16);
            assert_eq!(kept.to_json().unwrap(), original.to_json().unwrap());
            let mut capped = original.clone();
            cap_envelope(&mut capped, size - 1, 16);
            let error = capped.result.as_ref().unwrap_err();
            assert_eq!(error.kind, ErrorKind::Serialize);
            assert_eq!(
                error.message,
                format!(
                    "the envelope would be {size} bytes, past {} bytes, the envelope's cap, so \
                     the value isn't written; write less data, and keep error messages and \
                     import specifiers short",
                    size - 1
                )
            );
            assert_eq!(
                (&error.at, &error.specifier, &error.importer, &error.hint),
                (&None, &None, &None, &None)
            );
            assert_eq!(capped.access, None);
            let graph = match original.graph {
                GraphField::Omitted => GraphField::Omitted,
                _ => GraphField::Unavailable,
            };
            assert_eq!(capped.graph, graph);
            assert!(capped.json_len().unwrap() < 1024);
        }
    }

    /// A failure past the cap keeps its kind and its error: a run stopped
    /// at the heap's cap after caught `import()` calls of long specifiers
    /// loses only its `access` and graph, when that's enough.
    #[test]
    fn a_failure_past_the_cap_drops_access_first() {
        let long = "a".repeat(4096);
        let dynamic = [format!("./{long}0"), format!("./{long}1")];
        let dynamic: Vec<&str> = dynamic.iter().map(String::as_str).collect();
        let stopped = || {
            stop_error(
                Stop::Heap,
                &options(Path::new("/p/main.js"), Path::new("/p")),
            )
        };
        let original = envelope(Err(stopped()), &dynamic, GraphField::Unavailable);
        let size = original.json_len().unwrap();
        let mut kept = original.clone();
        cap_envelope(&mut kept, size, 16);
        assert_eq!(kept.to_json().unwrap(), original.to_json().unwrap());
        let mut capped = original;
        cap_envelope(&mut capped, size - 1, 16);
        assert_eq!(capped.result, Err(stopped()));
        assert_eq!(capped.access, None);
        assert_eq!(capped.graph, GraphField::Unavailable);
        assert!(capped.json_len().unwrap() < 1024);
    }

    /// A failure still past the cap without its `access` keeps its kind,
    /// with each text field longer than the bytes kept cut to them at a
    /// character boundary and named by its length: a long late rejection,
    /// and an uncaught `import()` of a long specifier.
    #[test]
    fn a_failure_past_the_cap_cuts_its_text() {
        // the 16th byte is inside the é, so the cut keeps 15
        let long = format!("{}é{}", "b".repeat(15), "b".repeat(4096));
        let cut = format!("{}… ({} bytes)", "b".repeat(15), long.len());
        let late = EvalError::new(ErrorKind::LateRejection, format!("Uncaught {long}"));
        let mut uncaught = EvalError::new(ErrorKind::ModuleThrew, "TypeError: cannot import");
        uncaught.at = Some(long.clone());
        uncaught.specifier = Some(long.clone());
        uncaught.importer = Some(long.clone());
        uncaught.hint = Some(long);
        for (error, dynamic) in [(late, &[][..]), (uncaught, &["./x.js"][..])] {
            let mut capped = envelope(Err(error.clone()), dynamic, GraphField::Omitted);
            assert!(capped.json_len().unwrap() > 4096);
            cap_envelope(&mut capped, 1024, 16);
            let capped_error = capped.result.as_ref().unwrap_err();
            assert_eq!(capped_error.kind, error.kind);
            let expected = |text: &str| {
                if text.len() <= 16 {
                    text.to_owned()
                } else {
                    let mut end = 16;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}… ({} bytes)", &text[..end], text.len())
                }
            };
            assert_eq!(capped_error.message, expected(&error.message));
            for (field, original) in [
                (&capped_error.at, &error.at),
                (&capped_error.specifier, &error.specifier),
                (&capped_error.importer, &error.importer),
                (&capped_error.hint, &error.hint),
            ] {
                assert_eq!(
                    field.as_deref(),
                    original.as_deref().map(expected).as_deref()
                );
                if original.is_some() {
                    assert_eq!(field.as_deref(), Some(cut.as_str()));
                }
            }
            assert_eq!(capped.access, None);
            assert!(capped.json_len().unwrap() <= 1024);
        }
    }

    /// At the real sizes, a failure cut to [`MAX_TEXT_KEPT`] fits the cap
    /// however its text escapes: every field a control character, each
    /// written as six bytes.
    #[test]
    fn a_cut_failure_fits_the_cap() {
        let long = "\u{1}".repeat(MAX_TEXT_KEPT * 4);
        let mut error = EvalError::new(ErrorKind::LateRejection, long.clone());
        error.at = Some(long.clone());
        error.specifier = Some(long.clone());
        error.importer = Some(long.clone());
        error.hint = Some(long);
        let mut capped = envelope(Err(error), &[], GraphField::Omitted);
        // past a small cap, so every field is cut
        cap_envelope(&mut capped, 1 << 20, MAX_TEXT_KEPT);
        let size = capped.json_len().unwrap();
        assert!(size > 5 * 6 * MAX_TEXT_KEPT, "{size}");
        assert!(size <= 5 * 6 * (MAX_TEXT_KEPT + 32) + 1024, "{size}");
        assert_eq!(capped.result.unwrap_err().kind, ErrorKind::LateRejection);
    }

    /// The options of an eval of `entry` from `cwd`, with no driver.
    fn options(entry: &Path, cwd: &Path) -> EvalOptions {
        EvalOptions {
            entry: entry.to_owned(),
            cwd: cwd.to_owned(),
            export: None,
            driver: None,
            jit: false,
            snapshot: None,
            runtime: RuntimeInfo::new("0.0.0", None),
            timeout: DEFAULT_TIMEOUT,
            backstop: None,
            graph: false,
        }
    }

    /// The pre-pass refuses an entry inside `node_modules` and one with no
    /// project root as a `resolve` the command line must fix, naming the
    /// entry by its canonical path; a project's module gets the project's
    /// directory as its box. The temporary directory is world-writable, so
    /// the walk from a directory in it with no `package.json` stops there.
    #[test]
    fn the_prepass_refuses_an_entry_no_project_owns() {
        let dir = std::env::temp_dir().join(format!("gro_runtime_eval_{}_c1", std::process::id()));
        let files = [
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/main.mjs", ""),
            (
                "project/node_modules/pkg/package.json",
                r#"{"name": "pkg", "exports": "./i.mjs"}"#,
            ),
            ("project/node_modules/pkg/i.mjs", ""),
            ("bare/main.mjs", ""),
        ];
        for (file, content) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        let dir = dir.canonicalize().unwrap();
        let project = dir.join("project");

        let resolved = prepass(options(Path::new("main.mjs"), &project)).unwrap();
        assert_eq!(
            resolved.confinement().roots().as_slice(),
            std::slice::from_ref(&project)
        );
        assert_eq!(
            resolved
                .failed(EvalError::new(ErrorKind::Internal, "x"))
                .entry
                .id,
            project.join("main.mjs").to_string_lossy()
        );

        for (entry, says) in [
            ("node_modules/pkg/i.mjs", "it is inside node_modules"),
            ("../bare/main.mjs", "it has no project root"),
        ] {
            let envelope = prepass(options(Path::new(entry), &project)).unwrap_err();
            let error = envelope.result.as_ref().unwrap_err();
            assert_eq!(error.kind, ErrorKind::Resolve, "{entry}");
            assert!(error.usage, "{entry}");
            assert!(error.message.contains(says), "{}", error.message);
            assert_eq!(error.specifier.as_deref(), Some(entry));
            assert_eq!(
                Path::new(&envelope.entry.id),
                project.join(entry).canonicalize().unwrap()
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A graph with a consulted file the run didn't record reading is an
    /// `internal` naming the path.
    #[test]
    fn an_unread_consulted_file_is_internal() {
        let error = unread_error(Path::new("/p/package.json"));
        assert_eq!(error.kind, ErrorKind::Internal);
        assert_eq!(
            error.message,
            "the eval's graph names /p/package.json as consulted with no read of it recorded, a \
             bug in fgro, so no graph is written"
        );
    }

    /// A read outside the box names the path, says how the box is made, and
    /// hints at the fix; it is `internal`, not a resolve the code could fix.
    #[test]
    fn a_read_outside_the_box_names_the_path_and_the_box() {
        let error = breach_error(Path::new("/elsewhere/package.json"));
        assert_eq!(error.kind, ErrorKind::Internal);
        assert!(
            error.message.starts_with(
                "the eval was to read /elsewhere/package.json, which lies outside its box"
            ),
            "{}",
            error.message
        );
        assert!(
            error
                .message
                .contains("linked directly into its node_modules")
        );
        assert!(
            error.message.contains(
                "except a link to a directory with no package.json file of its own, to a \
                 directory holding the project or another package's directory"
            ),
            "{}",
            error.message
        );
        assert_eq!(error.hint.as_deref(), Some(BREACH_HINT));
    }

    #[test]
    fn eval_s_engine_is_jitless_unless_asked_and_never_generates_code() {
        assert_eq!(
            engine_options(false),
            EngineOptions {
                locale: Some("en-US"),
                jitless: true,
                disallow_code_generation_from_strings: true,
            }
        );
        assert!(!engine_options(true).jitless);
        assert!(engine_options(true).disallow_code_generation_from_strings);
    }
}
