//! `fgro eval`: one module evaluated to JSON data, written into one
//! [`Envelope`] that reports the value or the kind of failure.
//!
//! The protocol, its determinism, limits, serializer, envelope, and error
//! kinds are `src/docs/eval.md`'s; this covers how the code holds them.
//!
//! An eval runs in two steps, so that its process can be confined between them
//! ([`crate::confine`]): [`prepass()`] resolves the entry and computes the
//! box's roots, before any runtime exists, and [`eval`] runs the rest on an
//! engine started after the box. [`eval`] takes a [`ConfinedEngine`], which
//! only a process held in its box has, checks it against [`engine_options`] and
//! against the eval's box, and refuses a build with no startup snapshot as
//! `internal` before the runtime starts. Eval reads no environment variable.
//!
//! The driver is a module at the synthetic URL [`DRIVER_SPECIFIER`], which
//! the loader resolves to nothing; with none, the namespace's
//! [`DEFAULT_EXPORT`] (unless the caller names another) is the value, and a
//! driver's [`AFTER_DRAIN`] runs after the drain. The engine is jitless
//! unless the caller turns the JIT on, with code generation from strings
//! off and the locale [`LOCALE`]; the time zone must be [`TIME_ZONE`]
//! ([`UTC_EPOCH`]). The limits are `limits.rs`'s: [`HEAP_LIMIT`],
//! [`DEFAULT_TIMEOUT`], [`MEMORY_LIMIT`], [`MAX_ENVELOPE_SIZE`], and each
//! module's [`MAX_MODULE_SIZE`](crate::resolver::MAX_MODULE_SIZE). With
//! [`EvalOptions::graph`], the envelope carries the run's graph
//! ([`GraphDump`], `dump.rs`); without it, nothing is kept for one.
//!
//! Each eval's runtime starts from the same snapshot as `fgro run`'s and is
//! made eval's per run, never in the snapshot: the bootstrap's `deny`
//! (`js/bootstrap.js`) before the driver or any module runs, and beneath it
//! the op middleware's refusals (`bootstrap::REFUSED_OPS`). An error's kind
//! comes from the step that failed and, for a load, from the failure the
//! loader recorded for the graph's static load, never from a message.

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

use self::dump::dump_graph;
pub use self::dump::{
    DumpConsulted, DumpError, DumpFormat, DumpImport, DumpLocal, DumpModule, DumpOrigin, DumpReach,
    DumpRole, GRAPH_VERSION, GraphDump,
};
pub use self::envelope::{
    Access, DENO_CORE_VERSION, DynamicImport, EVAL_PROTOCOL, Entry, Envelope, ErrorKind, EvalError,
    GraphField, RuntimeInfo, TSV_REV, TSV_VERSION,
};
use self::envelope::{MAX_TEXT_KEPT, access, cap_envelope};
pub use self::failure::no_snapshot_error;
use self::failure::{breach_error, stop_error, unread_error};
pub use self::limits::{
    DEFAULT_THREAD_STACK, DEFAULT_TIMEOUT, HEAP_LIMIT, MAX_ENVELOPE_SIZE, MEMORY_LIMIT,
};
use self::limits::{Stopped, format_size};
pub use self::prepass::{Prepass, PrepassError, prepass};
use self::run::evaluate;
use crate::confine::{BoxRoots, Jit};
use crate::engine::{ConfinedEngine, EngineOptions};
use crate::loader::graph::Recording;
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

/// The engine eval runs on: jitless unless `jit` is on, code generation
/// from strings off either way, and [`LOCALE`]. A priced build can turn
/// either of the last two off ([`prices`]).
pub fn engine_options(jit: Jit) -> EngineOptions {
    EngineOptions {
        locale: (!prices::off(EvalDefault::Locale)).then_some(LOCALE),
        jit,
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
    /// Whether the JIT is on, which [`engine_options`] the engine must have
    /// and the box is installed for ([`Prepass::confine`]).
    pub jit: Jit,
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

/// Evaluates `prepass`, its process confined to its box ([`Prepass::confine`]),
/// as the module doc describes, on the engine `engine` names, and returns the
/// envelope, whatever happened.
///
/// The engine is a confined one, so there is no eval outside the box: one
/// confined to another box than the prepass's, or started with no snapshot,
/// fails as `internal`.
///
/// Runs on the calling thread, which must reserve
/// [`STACK_SIZE`](crate::STACK_SIZE), as [`crate::run_main`] requires.
pub fn eval(engine: ConfinedEngine, prepass: Prepass<BoxRoots>) -> Envelope {
    let Prepass {
        options,
        entry,
        roots,
        reads,
        box_roots,
    } = prepass;
    let options = &options;
    let mut envelope = Envelope::failed_resolved(
        options,
        &entry,
        EvalError::new(ErrorKind::Internal, "not evaluated"),
    );
    let driver_url = match check_preconditions(engine, options, &box_roots) {
        Ok(driver_url) => driver_url,
        Err(error) => {
            envelope.result = Err(*error);
            return envelope;
        }
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
        box_roots: (!prices::off(EvalDefault::BoxRule)).then(|| box_roots.clone()),
    };
    let graph_box = options.graph.then(|| box_roots.as_slice().to_vec());
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
    if let Some(graph_box) = graph_box {
        envelope.graph = match dump_graph(&graph, &entry.url, &graph_box) {
            Ok(dump) => GraphField::Dumped(Box::new(dump)),
            Err(DumpError::Unread(path)) => {
                unread = Some(path);
                GraphField::Unavailable
            }
            // a path no id can name: the documented `null`
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

/// Checks what an eval needs before it starts, returning the driver's URL:
/// a process confined to the eval's own box, a startup snapshot, and an
/// engine set up with [`engine_options`].
fn check_preconditions(
    engine: ConfinedEngine,
    options: &EvalOptions,
    box_roots: &BoxRoots,
) -> Result<Url, Box<EvalError>> {
    if engine.confined().roots() != box_roots {
        return Err(Box::new(EvalError::new(
            ErrorKind::Internal,
            "the process is confined to another box than this eval's",
        )));
    }
    if options.snapshot.is_none() {
        return Err(Box::new(no_snapshot_error()));
    }
    let engine = engine.engine();
    let expected = engine_options(options.jit);
    if engine.options() != &expected {
        return Err(Box::new(EvalError::new(
            ErrorKind::Internal,
            format!(
                "the engine is set up with {:?}, not eval's {expected:?}",
                engine.options()
            ),
        )));
    }
    Url::parse(DRIVER_SPECIFIER).map_err(|_| {
        Box::new(EvalError::new(
            ErrorKind::Internal,
            format!("the driver's URL {DRIVER_SPECIFIER} doesn't parse"),
        ))
    })
}

/// How much the loader records of the graph: all of it, which the
/// envelope's `access` and a failure's kind read, unless a priced build
/// turns it off.
fn recording() -> Recording {
    if prices::off(EvalDefault::Recording) {
        Recording::Off
    } else {
        Recording::Full
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
            graph: envelope.graph.dropped(),
        };
        let mut json = backstop.to_json().ok()?;
        json.push('\n');
        Some(json)
    };
    Some(limits::BackstopEnvelopes {
        heap: prepared(EvalError::new(
            ErrorKind::Heap,
            format!(
                "the eval's heap reached its cap of {} with nothing left to give: one \
                 operation V8 can't stop ran out of memory, so the process exited",
                format_size(HEAP_LIMIT as u64)
            ),
        ))?,
        memory: prepared(EvalError::new(
            ErrorKind::Memory,
            format!(
                "the eval ran out of memory outside the heap's cap: the process reached its \
                 memory cap of {}, or a lower one its caller set, so it exited",
                format_size(MEMORY_LIMIT)
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
        jit: Jit::Off,
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
            engine_options(Jit::Off),
            EngineOptions {
                locale: Some("en-US"),
                jit: Jit::Off,
                disallow_code_generation_from_strings: true,
            }
        );
        assert_eq!(engine_options(Jit::On).jit, Jit::On);
        assert!(engine_options(Jit::On).disallow_code_generation_from_strings);
    }
}
