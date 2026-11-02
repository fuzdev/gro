//! `fgro eval`: one module evaluated to JSON data, written into one
//! [`Envelope`] that reports the value or the kind of failure.
//!
//! An eval runs in two steps, so that its process can be confined between
//! them ([`crate::confine`]): [`prepass()`] resolves the entry and computes the
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
mod failure;
mod limits;
mod prepass;
mod run;
mod serialize;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use url::Url;

pub use self::dump::{
    DumpConsulted, DumpError, DumpFormat, DumpImport, DumpLocal, DumpModule, DumpOrigin, DumpReach,
    DumpRole, GRAPH_VERSION, GraphDump, dump_graph,
};
pub use self::envelope::{
    Access, DENO_CORE_VERSION, DynamicImport, EVAL_PROTOCOL, Entry, Envelope, ErrorKind, EvalError,
    GraphField, RuntimeInfo, TSV_REV,
};
use self::envelope::{MAX_TEXT_KEPT, cap_envelope};
use self::failure::{access, breach_error, stop_error, unread_error};
pub use self::failure::{failed, no_snapshot_error};
use self::limits::Stopped;
pub use self::limits::{
    DEFAULT_THREAD_STACK, DEFAULT_TIMEOUT, HEAP_LIMIT, MAX_ENVELOPE_SIZE, MEMORY_LIMIT,
};
use self::prepass::entry_id;
pub use self::prepass::{Prepass, confine_error, prepass};
use self::run::evaluate;
use crate::engine::{ConfinedEngine, EngineOptions};
use crate::loader::{Loader, Rules};
use crate::phases;
use crate::prices::{self, EvalDefault};
use crate::snapshot::StartupSnapshot;

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
    let breach = loader.breach().map(Path::to_owned);
    // the runtime, and its share of the loader, is gone once the eval ends
    let graph = loader.take_graph();
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

/// The options of an eval of `entry` from `cwd`, with no driver.
#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
