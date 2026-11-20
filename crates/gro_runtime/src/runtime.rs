//! Building a runtime: its mode, the `deno_core` runtime over a loader, and the
//! bootstrap's per-run steps.

use std::rc::Rc;

use deno_core::{JsRuntime, v8};
use gro_v8::ArrayBufferRecord;

use crate::bootstrap::{self, Determinism, Ops};
use crate::host::HostData;
use crate::host::blake3::Blake3State;
use crate::host::error_class::ErrorClasses;
use crate::host::exit::{ExitState, ExitStep};
use crate::host::fs::FsState;
use crate::host::later::LaterState;
use crate::host::native::NativeNamespaces;
use crate::host::tsv::TsvState;
use crate::loader::Loader;
use crate::phases::{self, Phase};
use crate::prices::{self, EvalDefault};
use crate::report::{report, report_js};
use crate::snapshot::StartupSnapshot;
use crate::stack;
use crate::{Authority, ConsoleOptions, StartError, eval};

/// What kind of runtime to build: `fgro run`'s, or eval's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `fgro run`'s: V8's own heap limits, the timers, the clock, and
    /// every op.
    Run,
    /// Eval's: the heap capped at `eval::HEAP_LIMIT`, `Atomics.wait`
    /// refused, the ops in `bootstrap::REFUSED_OPS` refused, and
    /// `determinism: deny` installed when the runtime starts.
    Eval,
}

/// The runtime of a run or an eval that is over, with the event loop's
/// runtime under it, handed back whole so the embedder decides what its
/// teardown is worth.
///
/// Dropping it tears both down: the isolate is disposed and its heap
/// unmapped, then the event loop's runtime is shut down. That is what an
/// embedder that goes on running wants, so it's what happens unless
/// [`SpentRuntime::forget`] is called, which skips the teardown for a
/// process about to exit: the OS reclaims what the teardown would free, for
/// less. Nothing of the run is kept in it for the embedder to read.
#[must_use = "dropping it tears the runtime down; a process about to exit can `forget` it"]
pub struct SpentRuntime {
    /// The JS runtime, unset only while it is being dropped.
    runtime: Option<JsRuntime>,
    /// The event loop's runtime, which the JS runtime is dropped inside.
    event_loop: tokio::runtime::Runtime,
}

impl SpentRuntime {
    /// The spent `runtime`, which `event_loop` drove.
    pub(crate) const fn new(runtime: JsRuntime, event_loop: tokio::runtime::Runtime) -> Self {
        Self {
            runtime: Some(runtime),
            event_loop,
        }
    }

    /// The exit code the run's program set with `set_exit_code`, if it
    /// did, and if the runtime is a run's.
    pub(crate) fn exit_code(&mut self) -> Option<u8> {
        let state = self.runtime.as_mut()?.op_state();
        let state = state.borrow();
        state.try_borrow::<ExitState>()?.code()
    }

    /// Leaves the runtime as it is and never tears it down: its memory
    /// and threads stay until the process exits. For a process about to
    /// exit, as `fgro` is after a run or an eval; one that goes on running
    /// drops it instead.
    pub const fn forget(self) {
        std::mem::forget(self);
    }
}

impl std::fmt::Debug for SpentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpentRuntime").finish_non_exhaustive()
    }
}

impl Drop for SpentRuntime {
    fn drop(&mut self) {
        // the JS runtime first, inside the event loop's context, as it was made
        let _context = self.event_loop.enter();
        self.runtime.take();
    }
}

/// What `fgro run`'s native members read, put in the runtime's `OpState`
/// before anything runs: the run's authority, which every native member
/// and the host op check first, the run's data the members make, and the
/// step `exit` ends the process by. The default grants nothing.
#[derive(Debug, Clone, Default)]
pub struct HostState {
    /// The run's authority.
    pub authority: Authority,
    /// The run's data.
    pub data: HostData,
    /// The step `exit` ends the process by, which a run that grants `exit`
    /// must have.
    pub exit_step: Option<ExitStep>,
}

/// Starts `fgro run`'s runtime over `loader` from `snapshot` when there is
/// one, its native members holding `host` and the loader's record of refusals,
/// and runs the bootstrap's per-run steps with the console as `console`
/// says, returning why either failed. Must run inside a tokio runtime,
/// which `deno_core` drives its event loop with.
///
/// # Errors
///
/// Returns [`StartError::NoExitStep`] for a run that grants `exit` without
/// a step to end the process by, and [`StartError::Runtime`] with the
/// report of what failed when the runtime or its bootstrap fails.
pub fn start_runtime(
    snapshot: Option<StartupSnapshot>,
    console: ConsoleOptions,
    host: HostState,
    loader: Rc<Loader>,
) -> Result<JsRuntime, StartError> {
    let HostState {
        authority,
        data,
        exit_step,
    } = host;
    let exit = match (&authority, exit_step) {
        (Authority::Confined(grants), _) if grants.exit.is_none() => None,
        (_, Some(step)) => Some(ExitState::new(step)),
        (_, None) => return Err(StartError::NoExitStep),
    };
    let refusals = loader.refusal_log();
    let mut runtime =
        new_runtime(snapshot, loader, Mode::Run, None).map_err(StartError::Runtime)?;
    // before any code of a program's runs
    let namespaces = NativeNamespaces::new(&mut runtime);
    let state = runtime.op_state();
    state.borrow_mut().put(authority);
    state.borrow_mut().put(data);
    state.borrow_mut().put(refusals);
    state.borrow_mut().put(ErrorClasses::default());
    state.borrow_mut().put(namespaces);
    state.borrow_mut().put(Blake3State::default());
    state.borrow_mut().put(FsState::default());
    state.borrow_mut().put(TsvState::default());
    state.borrow_mut().put(LaterState::default());
    if let Some(exit) = exit {
        state.borrow_mut().put(exit);
    }
    drop(state);
    start(runtime, console, Mode::Run).map_err(StartError::Runtime)
}

/// Starts eval's runtime as [`start_runtime`] starts `fgro run`'s, built as
/// [`Mode::Eval`] says, its array buffers allocated by an allocator that
/// records whether its latest allocation failed, returned with the
/// runtime. It takes no authority: eval's host op and any native member
/// find none, so each refuses.
pub fn start_eval_runtime(
    snapshot: Option<StartupSnapshot>,
    console: ConsoleOptions,
    loader: Rc<Loader>,
) -> Result<(JsRuntime, ArrayBufferRecord), String> {
    let (create_params, record) = gro_v8::record_array_buffers(eval_create_params());
    let runtime = new_runtime(snapshot, loader, Mode::Eval, Some(create_params))?;
    Ok((start(runtime, console, Mode::Eval)?, record))
}

/// Runs the bootstrap's per-run steps on `runtime`, built as `mode` says,
/// with the console as `console` says.
fn start(mut runtime: JsRuntime, console: ConsoleOptions, mode: Mode) -> Result<JsRuntime, String> {
    phases::mark(Phase::Runtime);
    // before the bootstrap's start reads them
    let state = runtime.op_state();
    state.borrow_mut().put(console);
    state
        .borrow_mut()
        .put(if mode == Mode::Eval && !prices::off(EvalDefault::Deny) {
            Determinism::Denied
        } else {
            Determinism::Allowed
        });
    drop(state);
    bootstrap::start(&mut runtime).map_err(|error| report_js(&error))?;
    phases::mark(Phase::Bootstrap);
    Ok(runtime)
}

/// The isolate's parameters for an eval: the heap capped at
/// `eval::HEAP_LIMIT`, and `Atomics.wait` refused.
fn eval_create_params() -> v8::CreateParams {
    let params = v8::Isolate::create_params().allow_atomics_wait(false);
    if prices::off(EvalDefault::HeapCap) {
        params
    } else {
        params.heap_limits(0, eval::HEAP_LIMIT)
    }
}

/// A runtime over `loader` from `snapshot` when there is one, its isolate
/// built from `create_params` (V8's defaults with none), its extensions as
/// `mode` says, before the bootstrap's per-run steps.
fn new_runtime(
    snapshot: Option<StartupSnapshot>,
    loader: Rc<Loader>,
    mode: Mode,
    create_params: Option<v8::CreateParams>,
) -> Result<JsRuntime, String> {
    let mut runtime = JsRuntime::try_new(deno_core::RuntimeOptions {
        module_loader: Some(loader),
        // the same list the snapshot was made from, in the same order
        extensions: bootstrap::extensions(
            if mode == Mode::Eval && !prices::off(EvalDefault::RefuseOps) {
                Ops::Refused
            } else {
                Ops::Kept
            },
        ),
        startup_snapshot: snapshot.map(|snapshot| snapshot.bytes),
        // the scripts no snapshot holds, the builtins' backing scripts among
        // them: a snapshot's own table, made with it, or the one this
        // process makes, once
        residual_lazy_js_sources: snapshot
            .map_or_else(bootstrap::source_backing_scripts, |snapshot| {
                snapshot.residual_lazy_js
            }),
        // a snapshot holds every op's function already, and each calls this
        // runtime's implementation of its op and reads this runtime's
        // `OpState`: V8 resolves an op's implementation and its context
        // through the external-reference table, which `deno_core` builds
        // from this runtime's ops, a refused op's refusal included. Unset,
        // `deno_core` would make every function again at each start and
        // write it to `Deno.core.ops`, which no code reads: the snapshot's
        // modules hold the functions they took when it was made, so does the
        // bootstrap object a lazy script is called with, a builtin's
        // backing script included, and the start removes `Deno`
        skip_op_registration: snapshot.is_some(),
        create_params,
        ..Default::default()
    })
    .map_err(|error| report(&error))?;
    // the isolate's own settings, never the snapshot's
    stack::install(runtime.v8_isolate());
    Ok(runtime)
}

#[cfg(test)]
mod tests;
