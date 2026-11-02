//! The eval's run: the runtime started over the loader, the driver loaded, the entry's
//! graph evaluated, and the value serialized and drained.

use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;

use deno_core::error::{CoreError, JsError};
use deno_core::{FastStaticString, JsRuntime, PollEventLoopOptions, ascii_str, v8};
use url::Url;

use super::failure::{Failed, Phase};
use super::limits::{self, Stop, Stopped, Watchdog};
use super::{
    AFTER_DRAIN, DEFAULT_EXPORT, EvalOptions, MAX_ENVELOPE_SIZE, TIME_ZONE, UTC_EPOCH, serialize,
};
use crate::loader::Loader;
use crate::prices::{self, EvalDefault};
use crate::report::report_js;
use crate::runtime::{Mode, start_runtime};
use crate::{ConsoleOptions, ConsoleRouting, phases};

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
pub(super) fn evaluate(
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
