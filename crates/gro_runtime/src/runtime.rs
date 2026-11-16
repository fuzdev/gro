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
use crate::host::native::NativeNamespaces;
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
    let runtime = new_runtime(snapshot, loader, Mode::Run, None).map_err(StartError::Runtime)?;
    let state = runtime.op_state();
    state.borrow_mut().put(authority);
    state.borrow_mut().put(data);
    state.borrow_mut().put(refusals);
    state.borrow_mut().put(ErrorClasses::default());
    state.borrow_mut().put(NativeNamespaces::default());
    state.borrow_mut().put(Blake3State::default());
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
        // process makes, stripping the builtins' TypeScript here
        residual_lazy_js_sources: match snapshot {
            Some(snapshot) => snapshot.residual_lazy_js,
            None => bootstrap::source_backing_scripts()?,
        },
        // a snapshot holds every op's function already, and each calls this
        // runtime's implementation of its op and reads this runtime's
        // `OpState`: V8 resolves an op's implementation and its context
        // through the external-reference table, which `deno_core` builds
        // from this runtime's ops, a refused op's refusal included. Unset,
        // `deno_core` would make every function again at each start and
        // write it to `Deno.core.ops`, which no code reads: the snapshot's
        // modules hold the functions they took when it was made, so does the
        // bootstrap object a lazy script is called with, a builtin's
        // implementation included, and the start removes `Deno`
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
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use serde_json::{Value, json};

    use super::*;
    use crate::STACK_SIZE;
    use crate::bootstrap::web_scripts;
    use crate::confine::{Confinement, Jit};
    use crate::engine::{EngineOptions, init_engine};
    use crate::host::error_class::{ErrorClasses, hands_over_error};
    use crate::host::native::{self, Member, NativeMember, NativeNamespaces, Place, Prototypes};
    use crate::host::{HOST_OPS, NATIVE_MEMBERS};
    use crate::loader::graph::Recording;
    use crate::process_tests::{backing_scripts_file, make_startup_snapshot, run_test_in_child};
    use crate::resolver::BUILTINS;
    use crate::test_support::TestDir;
    use crate::{ArgsGrant, EnvGrant, Grants};

    /// What a test's runtime starts from.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Start {
        /// The extensions' sources, as a plain build's does.
        Source,
        /// The startup snapshot, as the `fgro` that ships does. Only in
        /// [`snapshot_child`]'s process: V8 shares one read-only heap between
        /// a process's isolates, so every isolate of a process starts from
        /// the same snapshot or from none.
        Snapshot,
    }

    /// Both starts, for a test of what must hold from either.
    const STARTS: [Start; 2] = [Start::Source, Start::Snapshot];

    /// The variable naming the snapshot file [`snapshot_child`]'s runtimes
    /// start from.
    const SNAPSHOT_START: &str = "GRO_RUNTIME_TEST_SNAPSHOT_START";

    /// What the line [`snapshot_child`] prints its results on starts with.
    const RESULTS_MARK: &str = "snapshot_child results: ";

    /// The variable naming the engine a [`snapshot_child`]'s process sets
    /// up, one of [`EVAL_ENGINES`]' names: unset, the one `fgro run` does,
    /// as in a test process's own run.
    const ENGINE: &str = "GRO_RUNTIME_TEST_ENGINE";

    /// Eval's engines, as [`ENGINE`] names them: jitless, as `fgro eval`
    /// runs, and with the JIT, as `fgro eval --jit` does. An engine is its
    /// process's, so each has a child of its own.
    const EVAL_ENGINES: [(&str, Jit); 2] = [("eval_jitless", Jit::Off), ("eval_jit", Jit::On)];

    /// The options of the engine this process's runtimes run on, as
    /// [`ENGINE`] says.
    fn engine_options() -> EngineOptions {
        let Some(name) = std::env::var_os(ENGINE) else {
            return EngineOptions::default();
        };
        let jit = EVAL_ENGINES
            .iter()
            .find_map(|(engine, jit)| (name == *engine).then_some(*jit))
            .unwrap_or_else(|| panic!("{ENGINE} names no engine: {}", name.display()));
        eval::engine_options(jit)
    }

    /// The variable naming the directory of the snapshot the build made,
    /// `snapshot.bin` in it, which `fgro` embeds from there.
    const BUILT_SNAPSHOT_DIR: &str = "GRO_SNAPSHOT_DIR";

    /// The snapshot [`snapshot_child`]'s runtimes start from, read once from
    /// the file [`SNAPSHOT_START`] names, with the scripts in the file
    /// beside it as the residual table: the builtins' backing scripts, which
    /// the process that made the snapshot wrote there, so this process
    /// strips none, or none at all beside the build's snapshot. It comes
    /// with no other residual script, so a runtime started from it loads no
    /// web script the snapshot lacks.
    fn child_snapshot() -> StartupSnapshot {
        static SNAPSHOT: OnceLock<StartupSnapshot> = OnceLock::new();
        *SNAPSHOT.get_or_init(|| {
            let file = std::env::var_os(SNAPSHOT_START)
                .expect("a runtime starts from the snapshot only in `snapshot_child`'s process");
            let scripts = std::fs::read_to_string(backing_scripts_file(Path::new(&file))).unwrap();
            let scripts: Vec<(String, String)> = serde_json::from_str(&scripts).unwrap();
            let table: Vec<(&'static str, &'static str)> = scripts
                .into_iter()
                .map(|(specifier, script)| (&*specifier.leak(), &*script.leak()))
                .collect();
            StartupSnapshot {
                bytes: Box::leak(std::fs::read(file).unwrap().into_boxed_slice()),
                residual_lazy_js: Box::leak(table.into_boxed_slice()),
            }
        })
    }

    /// Runs `test` on a runtime built as `mode` says, from source, as
    /// [`on_runtime`] does.
    fn on_new_runtime<T: Send + 'static>(
        name: &str,
        mode: Mode,
        test: impl FnOnce(&mut JsRuntime) -> T + Send + 'static,
    ) -> T {
        on_runtime(name, Start::Source, mode, test)
    }

    /// Runs `test` on a runtime started as `start` says and built as `mode`
    /// says, before the bootstrap's per-run steps, so `Deno.core.ops` is
    /// still there, on a thread reserving [`STACK_SIZE`].
    fn on_runtime<T: Send + 'static>(
        name: &str,
        start: Start,
        mode: Mode,
        test: impl FnOnce(&mut JsRuntime) -> T + Send + 'static,
    ) -> T {
        use crate::resolver::DiskTree;

        init_engine(Confinement::Unconfined, &engine_options()).unwrap();
        let snapshot = (start == Start::Snapshot).then(child_snapshot);
        let dir = TestDir::on_disk(&format!("{name}_{start:?}"));
        std::fs::write(dir.path().join("main.mjs"), "").unwrap();
        let cwd = dir.path().to_owned();
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                let (entry, roots) =
                    crate::resolver::resolve_entry(&DiskTree, Path::new("main.mjs"), &cwd).unwrap();
                let loader = Rc::new(Loader::new(&entry, roots, Recording::Off));
                let tokio = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let _entered = tokio.enter();
                let create_params = (mode == Mode::Eval).then(eval_create_params);
                let mut runtime = new_runtime(snapshot, loader, mode, create_params).unwrap();
                test(&mut runtime)
            })
            .unwrap()
            .join()
            .unwrap()
    }

    /// A script's completion value, which must be a string of JSON, parsed.
    fn script_json(runtime: &mut JsRuntime, source: impl Into<String>) -> Value {
        let value = runtime.execute_script("<test>", source.into()).unwrap();
        deno_core::scope!(scope, runtime);
        let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
        serde_json::from_str(&json).unwrap()
    }

    /// A probe: it makes its own runtimes, started as it's told, and
    /// returns what it saw as JSON.
    type Probe = fn(Start) -> Value;

    /// What the tests below ask of a runtime from either start, by name.
    const PROBES: [(&str, Probe); 7] = [
        ("op_functions", op_functions),
        ("unregistered_reads", unregistered_reads),
        ("host_ops", host_ops_results),
        ("refused_ops", refused_ops_results),
        ("builtin_imports", builtin_imports),
        ("error_classes", error_classes),
        ("native_members", native_members),
    ];

    /// What the run's error classes are across calls of the namespace op by
    /// the error row's script on a runtime started as `start` says, in a run
    /// that grants nothing: how many classes the start made, before any
    /// call, and after the calls, what each value handed over as the realm's
    /// `Error` before any was held threw, whether each call got the one
    /// namespace, of the class the host made once, frozen, extending the
    /// `Error` first held, which a later value doesn't replace, and of the
    /// guard.
    fn error_classes(start: Start) -> Value {
        let source = "const op = (error) => Deno.core.ops.op_gro_namespace(undefined, error);
const not_errors = [
	undefined,
	{},
	function NotError() {},
	TypeError,
	new Proxy(Error, {}),
	Object.setPrototypeOf(function Fake() {}, Error),
];
const refused = not_errors.map((value) => {
	try {
		op(value);
		return 'returned';
	} catch (error) {
		return `${error.constructor.name}: ${error.message}`;
	}
});
const a = op(Error);
const b = op(Error);
const c = op(TypeError);
JSON.stringify({
	refused,
	held: c.EnvError === a.EnvError && Object.getPrototypeOf(a.EnvError) === Error,
	namespaces: a === b && b === c,
	frozen: Object.isFrozen(a.EnvError) && Object.isFrozen(a.EnvError.prototype) && Object.isFrozen(a.is_env_error),
	named: a.EnvError.name === 'EnvError' && a.is_env_error.name === 'is_env_error',
	keys: Reflect.ownKeys(a),
	prototype: Object.getPrototypeOf(a),
	error: new a.EnvError('not_granted', 'HOME') instanceof Error,
})";
        on_runtime("error_classes", start, Mode::Run, move |runtime| {
            let state = runtime.op_state();
            state.borrow_mut().put(Authority::default());
            state.borrow_mut().put(ErrorClasses::default());
            state.borrow_mut().put(NativeNamespaces::default());
            let before = state.borrow().borrow::<ErrorClasses>().len();
            drop(state);
            let row = backing_script("@fuzdev/gro/env_error.ts");
            let calls = named_script_json(runtime, row, source.to_owned());
            let after = runtime.op_state().borrow().borrow::<ErrorClasses>().len();
            json!({"before": before, "calls": calls, "after": after})
        })
    }

    /// A run's error class is made the first time it needs it, by the
    /// host, once: each call of the namespace op by the row's script gets
    /// the one namespace of the frozen class and the guard, from either
    /// start; a start makes none.
    #[test]
    fn an_error_class_is_made_once_a_run_at_first_need() {
        for start in STARTS {
            assert_eq!(
                probed("error_classes", start),
                json!({
                    "before": 0,
                    "calls": {
                        "refused": vec!["TypeError: an error class extends the realm's Error"; 6],
                        "held": true,
                        "namespaces": true,
                        "frozen": true,
                        "named": true,
                        "keys": ["EnvError", "is_env_error", "default"],
                        "prototype": null,
                        "error": true,
                    },
                    "after": 1,
                }),
                "{start:?}"
            );
        }
    }

    /// A family missing from the host's table is never made a class of:
    /// asked for its class, the host throws an `Error` naming it and holds
    /// no class, where its class's constructor would otherwise return a
    /// plain object.
    #[test]
    fn a_family_the_host_s_table_lacks_throws_when_its_class_is_made() {
        use crate::host::error_class::ErrorFamily;

        static MISSING: ErrorFamily = ErrorFamily {
            name: "MissingError",
            target: "target",
            rows: &[],
        };
        let (message, made) = on_new_runtime("missing_family", Mode::Run, |runtime| {
            let classes = ErrorClasses::default();
            deno_core::scope!(scope, runtime);
            let global = scope.get_current_context().global(scope);
            let key = v8::String::new(scope, "Error").unwrap();
            let error = global.get(scope, key.into()).unwrap();
            v8::tc_scope!(let tc, scope);
            classes.hand_over(tc, error);
            assert!(classes.class(tc, &MISSING).is_none());
            let thrown = tc.exception().unwrap();
            let thrown = v8::Local::<v8::Object>::try_from(thrown).unwrap();
            let message = v8::String::new(tc, "message").unwrap();
            let message = thrown.get(tc, message.into()).unwrap();
            (message.to_rust_string_lossy(tc), classes.len())
        });
        assert_eq!(
            message,
            "the host has no error family MissingError: a family's class is one of the host's \
             table"
        );
        assert_eq!(made, 0);
    }

    /// The realm's `Error` a row hands over is held unchecked until a class
    /// is first made of it: `env`'s first import, handed the wrong
    /// intrinsic, makes the namespace and no class; making the class then
    /// refuses it, as a `TypeError`, and drops it, so the next import's
    /// `Error` is the one checked and the class is made of it.
    #[test]
    fn the_realm_s_error_is_checked_when_a_class_is_first_made() {
        use crate::host::error_class::ENV_ERROR;

        let results = on_new_runtime("error_checked_at_class", Mode::Run, |runtime| {
            let state = runtime.op_state();
            state.borrow_mut().put(crate::test_support::run_authority());
            state.borrow_mut().put(host_data());
            state.borrow_mut().put(ErrorClasses::default());
            state.borrow_mut().put(NativeNamespaces::default());
            drop(state);
            let env = backing_script("@fuzdev/gro/env.ts");
            let import = |runtime: &mut JsRuntime, error: &str| {
                named_script_json(
                    runtime,
                    env,
                    format!(
                        "JSON.stringify(Object.keys(Deno.core.ops.op_gro_namespace(undefined, \
                         {error})))"
                    ),
                )
            };
            let make_class = |runtime: &mut JsRuntime| {
                let state = runtime.op_state();
                let state = state.borrow();
                let classes = state.borrow::<ErrorClasses>();
                deno_core::scope!(scope, runtime);
                v8::tc_scope!(let tc, scope);
                let made = classes.class(tc, &ENV_ERROR).is_some();
                let thrown = tc.exception().map(|thrown| {
                    let thrown = v8::Local::<v8::Object>::try_from(thrown).unwrap();
                    let message = v8::String::new(tc, "message").unwrap();
                    thrown
                        .get(tc, message.into())
                        .unwrap()
                        .to_rust_string_lossy(tc)
                });
                (made, thrown, classes.len())
            };
            let wrong = import(runtime, "TypeError");
            let refused = make_class(runtime);
            let right = import(runtime, "Error");
            let made = make_class(runtime);
            // the first held is the one checked: a later value is ignored
            runtime.op_state().borrow_mut().put(ErrorClasses::default());
            import(runtime, "Error");
            import(runtime, "TypeError");
            let first = make_class(runtime);
            json!({"wrong": wrong, "refused": refused, "right": right, "made": made, "first": first})
        });
        assert_eq!(
            results,
            json!({
                "wrong": ["env", "default"],
                "refused": [false, "an error class extends the realm's Error", 0],
                "right": ["env", "default"],
                "made": [true, null, 1],
                "first": [true, null, 1],
            })
        );
    }

    /// A refused env name in a runtime with no record of refusals, which no
    /// run's start leaves out, throws an `Error` saying the refusal can't
    /// fail the run, never an `EnvError` that would pass unrecorded; a
    /// granted name still reads.
    #[test]
    fn a_refusal_with_no_record_to_land_in_fails_loudly() {
        let results = on_new_runtime("no_refusal_log", Mode::Run, |runtime| {
            let state = runtime.op_state();
            state.borrow_mut().put(Authority::Confined(Grants {
                env: Some(EnvGrant {
                    names: ["A".to_owned()].into(),
                }),
                ..Grants::default()
            }));
            state.borrow_mut().put(host_data());
            state.borrow_mut().put(ErrorClasses::default());
            state.borrow_mut().put(NativeNamespaces::default());
            drop(state);
            named_script_json(
                runtime,
                backing_script("@fuzdev/gro/env.ts"),
                "const { env } = Deno.core.ops.op_gro_namespace(undefined, Error);
let thrown;
try {
	env.get('B');
} catch (error) {
	thrown = `${error.constructor.name}: ${error.message}`;
}
JSON.stringify({ thrown, read: env.get('A') })"
                    .to_owned(),
            )
        });
        assert_eq!(
            results,
            json!({
                "thrown": "Error: env refused a name, but this runtime was started without the \
                           run's record of refusals, so the refusal can't fail the run",
                "read": "1",
            })
        );
    }

    /// The probe eval's engines are asked: what eval refuses.
    const EVAL_ENGINE_PROBE: &str = "refused_ops";

    /// What the probe `name` saw of runtimes started as `start` says: from
    /// source, run here; from the snapshot, as [`snapshot_child`] ran it.
    /// Either way once a test process, since two tests ask for one probe
    /// and its runtimes' directories are named for it.
    fn probed(name: &str, start: Start) -> Value {
        static FROM_SOURCE: [OnceLock<Value>; PROBES.len()] =
            [const { OnceLock::new() }; PROBES.len()];
        match start {
            Start::Source => {
                let at = PROBES.iter().position(|(probe, _)| *probe == name).unwrap();
                FROM_SOURCE[at].get_or_init(|| PROBES[at].1(start)).clone()
            }
            Start::Snapshot => snapshot_results().run[name].clone(),
        }
    }

    /// What [`snapshot_child`] saw of runtimes started from the snapshot
    /// this build's extensions make.
    struct SnapshotResults {
        /// Every probe's results, by name, on the engine `fgro run` sets up.
        run: Value,
        /// [`eval_engine_results`], on each of [`EVAL_ENGINES`] in their
        /// order.
        eval: [Value; EVAL_ENGINES.len()],
    }

    /// The results from the snapshot this build's extensions make: a child
    /// makes it, and [`snapshot_child`] runs the probes in others, one an
    /// engine, once a test process.
    fn snapshot_results() -> &'static SnapshotResults {
        static RESULTS: OnceLock<SnapshotResults> = OnceLock::new();
        RESULTS.get_or_init(|| {
            let dir = TestDir::in_system_temp("snapshot_start");
            let file = make_startup_snapshot(dir.path());
            SnapshotResults {
                run: snapshot_child_results(&file, None),
                eval: EVAL_ENGINES.map(|(engine, _)| eval_engine_results(&file, engine)),
            }
        })
    }

    /// [`EVAL_ENGINE_PROBE`]'s results on runtimes started from the snapshot
    /// at `file`, on the engine `engine` names.
    fn eval_engine_results(file: &Path, engine: &str) -> Value {
        snapshot_child_results(file, Some(engine))[EVAL_ENGINE_PROBE].clone()
    }

    /// What a [`snapshot_child`] printed, run on the snapshot at `file` and
    /// the engine `engine` names, the one `fgro run` sets up with none: its
    /// probes' results, by name.
    fn snapshot_child_results(file: &Path, engine: Option<&str>) -> Value {
        let mut env = vec![(SNAPSHOT_START, file.as_os_str())];
        env.extend(engine.map(|engine| (ENGINE, OsStr::new(engine))));
        let output = run_test_in_child("runtime::tests::snapshot_child", &env);
        let stdout = String::from_utf8(output.stdout).unwrap();
        let results = stdout
            .lines()
            .find_map(|line| line.strip_prefix(RESULTS_MARK))
            .unwrap();
        serde_json::from_str(results).unwrap()
    }

    /// The child [`snapshot_child_results`] runs, and nothing in a test
    /// process's own run: the probes on runtimes started from the snapshot
    /// [`SNAPSHOT_START`] names, the only runtimes of its process, on the
    /// engine [`ENGINE`] names, their results printed as one line of JSON.
    /// On the engine `fgro run` sets up it runs every probe, and on one of
    /// eval's only [`EVAL_ENGINE_PROBE`].
    #[test]
    fn snapshot_child() {
        if std::env::var_os(SNAPSHOT_START).is_none() {
            return;
        }
        let eval_s = std::env::var_os(ENGINE).is_some();
        let results: serde_json::Map<String, Value> = PROBES
            .iter()
            .filter(|(name, _)| !eval_s || *name == EVAL_ENGINE_PROBE)
            .map(|(name, probe)| ((*name).to_owned(), probe(Start::Snapshot)))
            .collect();
        println!("\n{RESULTS_MARK}{}", Value::Object(results));
    }

    /// The names `ext:core/ops` exports, then those whose export isn't the
    /// function `Deno.core.ops` holds under the name.
    fn op_functions(start: Start) -> Value {
        on_runtime("op_functions", start, Mode::Run, |runtime| {
            runtime
                .lazy_load_es_module_with_code(
                    "ext:gro/test_ops.js",
                    "import * as ops from 'ext:core/ops';\nglobalThis.module_ops = ops;\n",
                )
                .unwrap();
            script_json(
                runtime,
                "const names = Object.keys(globalThis.module_ops);
JSON.stringify([names, names.filter((name) => Deno.core.ops[name] !== globalThis.module_ops[name])])",
            )
        })
    }

    /// A runtime's code holds the runtime's op functions, from either start:
    /// each export of `ext:core/ops`, which the bootstrap and the builtins
    /// import their ops from, is the function `Deno.core.ops` holds under
    /// its name, the object `deno_web`'s scripts read theirs from. A start
    /// from a snapshot that bound the ops again would hold a second function
    /// under every name there, made at each start and called by nothing.
    #[test]
    fn a_runtime_s_code_holds_its_op_functions() {
        for start in STARTS {
            let (names, rebound): (Vec<String>, Vec<String>) =
                serde_json::from_value(probed("op_functions", start)).unwrap();
            assert!(rebound.is_empty(), "{start:?}: {rebound:?}");
            // the comparison covers the ops eval refuses and gro's own
            let compared = bootstrap::REFUSED_OPS
                .iter()
                .chain(&bootstrap::UNGATED_OPS)
                .copied()
                .chain(HOST_OPS.iter().copied());
            for name in compared {
                assert!(
                    names.iter().any(|export| export == name),
                    "{start:?}: {name}"
                );
            }
        }
    }

    /// The names a script the runtime can load reads from `core.ops` that
    /// aren't a function there, in a runtime of each mode.
    fn unregistered_reads(start: Start) -> Value {
        let reads: Vec<String> = web_scripts::loadable_reads().into_keys().collect();
        let source = format!(
            "JSON.stringify({}.filter((name) => typeof Deno.core.ops[name] !== 'function'))",
            serde_json::to_string(&reads).unwrap()
        );
        let unregistered =
            |source: String| move |runtime: &mut JsRuntime| script_json(runtime, source);
        json!({
            "read": reads.len(),
            "eval": on_runtime("reads_eval", start, Mode::Eval, unregistered(source.clone())),
            "run": on_runtime("reads_run", start, Mode::Run, unregistered(source)),
        })
    }

    /// Every name a script the runtime can load reads from `core.ops` is a
    /// function there, from either start and in either mode: no script
    /// holds `undefined` for an op the runtime dropped, which would fail
    /// only where the script calls it.
    #[test]
    fn every_op_a_loadable_script_reads_is_registered() {
        for start in STARTS {
            let results = probed("unregistered_reads", start);
            assert!(results["read"].as_u64().unwrap() > 0, "{start:?}");
            for mode in ["eval", "run"] {
                assert_eq!(results[mode], json!([]), "{mode} {start:?}");
            }
        }
    }

    /// Every op a runtime registers, `deno_core`'s own included, pinned in
    /// `tests/op_names.txt` in registration order, so a `deno_core` bump
    /// that adds, removes, or renames one fails here and gets a look: an op
    /// that compiles code, imports, or reads the clock or randomness joins
    /// `bootstrap::REFUSED_OPS`. Of the web extensions' ops these are the
    /// kept ones (`bootstrap::WEB_OPS`, `bootstrap::WEB_CLASSES`); the ones
    /// the runtime drops are pinned beside them, in
    /// `tests/dropped_op_names.txt`, where a `deno_web` bump's new op lands.
    /// An eval runtime registers the same names, since the middleware keeps
    /// each refused op's name, and every name eval refuses is registered, so
    /// none is refused in name only. To accept a change deliberately, write
    /// the list the assertion prints over the file, one name a line.
    #[test]
    fn the_registered_ops_are_pinned() {
        let names = |runtime: &mut JsRuntime| -> Vec<String> {
            runtime
                .op_names()
                .into_iter()
                .map(ToOwned::to_owned)
                .collect()
        };
        let run = on_new_runtime("op_names_run", Mode::Run, names);
        let eval = on_new_runtime("op_names_eval", Mode::Eval, names);
        let pinned =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/op_names.txt"))
                .unwrap();
        let mut listed = run.join("\n");
        listed.push('\n');
        assert_eq!(
            listed, pinned,
            "the registered ops differ from tests/op_names.txt; update it only if the change is \
             deliberate (see this test's doc)"
        );
        assert_eq!(eval, run);
        for refused in bootstrap::REFUSED_OPS {
            assert!(run.iter().any(|name| name == refused), "{refused}");
        }
        // every blob op, so one dropped from the list, or added by a
        // `deno_web` bump, is caught
        for name in run.iter().filter(|name| name.starts_with("op_blob_")) {
            assert!(bootstrap::REFUSED_OPS.contains(&name.as_str()), "{name}");
        }
    }

    /// The ops eval refuses, pinned in `tests/refused_op_names.txt` in the
    /// list's order. The tests of the refusals, and the check a snapshot's
    /// making runs, read the names from `bootstrap::REFUSED_OPS`, so a name
    /// that left the list would be asked of none of them. To accept a
    /// change deliberately, write the list the assertion prints over the
    /// file, one name a line.
    #[test]
    fn the_refused_ops_are_pinned() {
        let pinned = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/refused_op_names.txt"
        ))
        .unwrap();
        let mut listed = bootstrap::REFUSED_OPS.join("\n");
        listed.push('\n');
        assert_eq!(
            listed, pinned,
            "the ops eval refuses differ from tests/refused_op_names.txt; update it only if the \
             change is deliberate (see this test's doc)"
        );
    }

    /// The op functions of the runtime the product starts for `mode`, by
    /// [`start_eval_runtime`] or [`start_runtime`], from source.
    fn started_op_functions(mode: Mode) -> Vec<crate::snapshot::OpFunctions> {
        use crate::resolver::DiskTree;

        init_engine(Confinement::Unconfined, &engine_options()).unwrap();
        let dir = TestDir::on_disk(&format!("started_{mode:?}"));
        std::fs::write(dir.path().join("main.mjs"), "").unwrap();
        let cwd = dir.path().to_owned();
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                let (entry, roots) =
                    crate::resolver::resolve_entry(&DiskTree, Path::new("main.mjs"), &cwd).unwrap();
                let loader = Rc::new(Loader::new(&entry, roots, Recording::Off));
                let tokio = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let _entered = tokio.enter();
                let console = ConsoleOptions::default();
                let runtime = match mode {
                    Mode::Eval => start_eval_runtime(None, console, loader).unwrap().0,
                    Mode::Run => {
                        start_runtime(None, console, HostState::default(), loader).unwrap()
                    }
                };
                crate::snapshot::op_functions(&runtime)
            })
            .unwrap()
            .join()
            .unwrap()
    }

    /// The runtime eval starts (`start_eval_runtime`) has the refusal as
    /// each refused op's function, slow only, and every other op's as
    /// `fgro run`'s (`start_runtime`) has them, where each refused op has
    /// its own. Read from the started runtimes' ops, which no script
    /// reaches once a runtime has started: the tests that call the ops
    /// build their runtimes a step short of that.
    #[test]
    fn the_runtime_eval_starts_has_the_refusal_at_each_refused_op() {
        let eval = started_op_functions(Mode::Eval);
        let run = started_op_functions(Mode::Run);
        assert_eq!(eval.len(), run.len());
        let refusal = *run.iter().find(|op| op.name == "op_gro_refused").unwrap();
        assert_eq!(refusal.fast, None);
        for (eval, run) in eval.iter().zip(&run) {
            assert_eq!(eval.name, run.name);
            if bootstrap::REFUSED_OPS.contains(&run.name) {
                assert_eq!((eval.slow, eval.fast), (refusal.slow, None), "{}", run.name);
                assert_ne!(run.slow, refusal.slow, "{}", run.name);
            } else {
                assert_eq!(eval, run);
            }
        }
    }

    /// This build's ops pass the check a snapshot's making runs on them
    /// (`snapshot::check_refused_functions`), on the addresses of a runtime
    /// built as the snapshot's is: no function of an op eval refuses is at
    /// an address the snapshot couldn't tell from another op's. The
    /// addresses are every registered op's, in order, each function's under
    /// its own kind: an op with a fast function lists it as its fast one,
    /// apart from its slow one, and the refusal, which is slow only, lists
    /// none.
    #[test]
    fn no_refused_op_shares_a_function_s_address() {
        let ops = on_new_runtime("op_functions_addresses", Mode::Run, |runtime| {
            let names: Vec<&str> = runtime.op_names();
            let ops = crate::snapshot::op_functions(runtime);
            assert_eq!(ops.iter().map(|op| op.name).collect::<Vec<_>>(), names);
            ops
        });
        let listed = |name: &str| *ops.iter().find(|op| op.name == name).unwrap();
        let declared = bootstrap::gro::init().ops;
        let color = declared
            .iter()
            .find(|op| op.name == "op_gro_color")
            .unwrap();
        let color_fast = color.fast_fn().address() as usize;
        assert_eq!(listed("op_gro_color").fast, Some(color_fast));
        assert_ne!(listed("op_gro_color").slow, color_fast);
        assert_eq!(listed("op_gro_refused").fast, None);
        crate::snapshot::check_refused_functions(&ops, &bootstrap::REFUSED_OPS).unwrap();
    }

    /// The choke point by registration: the ops gro's extension registers,
    /// whatever their names, are exactly the bootstrap's ungated list and
    /// the host ops, declared through `host_ops!`, a grant-free module's
    /// included, each once and on one list only, and the extension
    /// registers no op on an object. A hand-written op added to the
    /// extension fails here until it's declared as a host op or listed as
    /// ungated. Each is registered in the runtime as well.
    #[test]
    fn every_op_of_gro_s_extension_is_ungated_or_a_host_op() {
        let extension = bootstrap::gro::init();
        let mut registered: Vec<&str> = extension.ops.iter().map(|op| op.name).collect();
        registered.sort_unstable();
        let mut declared: Vec<&str> = bootstrap::UNGATED_OPS
            .iter()
            .copied()
            .chain(HOST_OPS.iter().copied())
            .collect();
        declared.sort_unstable();
        assert_eq!(registered, declared);
        let mut unique = declared.clone();
        unique.dedup();
        assert_eq!(unique, declared, "an op is listed twice");
        assert!(extension.objects.is_empty(), "an op on an object");
        let names = on_new_runtime("gro_ops", Mode::Run, |runtime| -> Vec<String> {
            runtime
                .op_names()
                .into_iter()
                .map(ToOwned::to_owned)
                .collect()
        });
        for name in declared {
            assert!(names.iter().any(|registered| registered == name), "{name}");
        }
    }

    /// Puts what the blake3 row's namespace and hashers need in `runtime`'s
    /// `OpState`, in a run that grants nothing, as a run's start does.
    fn put_blake3_state(runtime: &JsRuntime) {
        let state = runtime.op_state();
        state.borrow_mut().put(Authority::default());
        state.borrow_mut().put(ErrorClasses::default());
        state.borrow_mut().put(NativeNamespaces::default());
        state.borrow_mut().put(Blake3State::default());
    }

    /// The name `deno_core` compiles `row`'s backing script under, which
    /// the namespace op knows a row's script by.
    fn backing_script(row: &str) -> &'static str {
        let at = BUILTINS
            .iter()
            .position(|builtin| builtin.specifier == row)
            .unwrap();
        bootstrap::IMPLEMENTATIONS[at].script
    }

    /// `source`'s completion value, a string of JSON, parsed, the script run
    /// under `name`.
    fn named_script_json(runtime: &mut JsRuntime, name: &'static str, source: String) -> Value {
        let value = runtime.execute_script(name, source).unwrap();
        deno_core::scope!(scope, runtime);
        let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
        serde_json::from_str(&json).unwrap()
    }

    /// The JS the native members' probe defines first: `thrown`, what a
    /// call threw, as `Class: message`, or `null`; `shape`, a value's type,
    /// whether it's frozen, its prototype, and its own keys; `function`,
    /// a function's shape, name, `length`, whether its source text is native
    /// code's, and the class of what `new` of it threw; `functions`, the
    /// function shape of each of a record's functions the table names, each
    /// kept in `members` under its member's label for the refusals; and
    /// `namespaces`, where the probe keeps each row's.
    const NATIVE_HELPERS: &str = "globalThis.thrown = (call) => {
	try {
		call();
		return null;
	} catch (error) {
		return `${error.constructor.name}: ${error.message}`;
	}
};
globalThis.shape = (value) => {
	const prototype = Object.getPrototypeOf(value);
	return {
		type: typeof value,
		frozen: Object.isFrozen(value),
		prototype: prototype === null ? null : prototype === Function.prototype ? 'Function.prototype' : 'other',
		keys: Reflect.ownKeys(value).map(String),
	};
};
globalThis.function_shape = (fn) => ({
	...shape(fn),
	name: fn.name,
	length: fn.length,
	native_source: Function.prototype.toString.call(fn) === `function ${fn.name}() { [native code] }`,
	constructed: (() => {
		try {
			new fn();
			return null;
		} catch (error) {
			return error.constructor.name;
		}
	})(),
});
globalThis.members = [];
globalThis.functions = (record, module, names) => {
	const found = {};
	for (const key of names) {
		members.push([`${key} of ${module}`, record[key]]);
		found[key] = function_shape(record[key]);
	}
	return found;
};
globalThis.namespaces = {};";

    /// The names of the native functions `module` puts at `place`, in the
    /// table's order, as JSON.
    fn function_names(module: &str, place: Place) -> String {
        let names: Vec<&str> = NATIVE_MEMBERS
            .iter()
            .filter(|member| member.module == module && member.place == place)
            .filter(|member| matches!(member.member, Member::Function { .. }))
            .map(|member| member.name)
            .collect();
        serde_json::to_string(&names).unwrap()
    }

    /// The rows with native members, each once, in the table's order.
    fn native_rows() -> Vec<&'static str> {
        let mut rows: Vec<&str> = Vec::new();
        for member in NATIVE_MEMBERS {
            if !rows.contains(&member.module) {
                rows.push(member.module);
            }
        }
        rows
    }

    /// The kinds of handle native methods go on, each once, with the row
    /// each is of.
    fn native_kinds() -> Vec<(&'static str, &'static crate::host::handle::HandleKind)> {
        let mut kinds: Vec<(&str, &crate::host::handle::HandleKind)> = Vec::new();
        for member in NATIVE_MEMBERS {
            if let Place::Method(kind) = member.place
                && !kinds.iter().any(|(_, held)| *held == kind)
            {
                kinds.push((member.module, kind));
            }
        }
        kinds
    }

    /// What the probe reads of the values the rows' namespaces hold, made
    /// in a run that grants each row, with [`host_data`]: each as JSON,
    /// with whether the arguments' array is frozen, and what `cwd` returns.
    const NATIVE_VALUES: &str = "{
	const ns = (row) => namespaces[`@fuzdev/gro/${row}.ts`];
	JSON.stringify({
		args: ns('args').args,
		args_shape: shape(ns('args').args),
		args_array: Array.isArray(ns('args').args),
		cwd: ns('cwd').cwd(),
		pid: ns('pid').pid,
		runtime_version: ns('runtime_version').runtime_version,
		env: shape(ns('env').env),
		env_reads: [ns('env').env.get('GRO_TEST_ENV'), Reflect.ownKeys(ns('env').env.get_all())],
	});
}";

    /// What the native rows' namespaces are on a runtime started as
    /// `start` says: what the namespace op threw in a run that grants
    /// nothing, first, called by a script that is no row's (one that names a
    /// row's script in a `sourceURL` among them), by the script of a row
    /// whose grant the run lacks, and by a row's script with a scope or an
    /// `Error` it refuses, and whether a trap ran; in a run that grants
    /// every row, what each row's namespace op answered its own script,
    /// twice, and the shapes of its record and its functions; the shapes of
    /// each kind's prototype the host made with it and its methods; how many
    /// namespaces the run held before, after the refusals, and after; what a member
    /// of the probe table, which needs `args`, did in a run that grants
    /// nothing, one that grants `args`, and a runtime with no authority; and
    /// what every member threw called in that runtime.
    fn native_members(start: Start) -> Value {
        on_runtime("native_members", start, Mode::Run, move |runtime| {
            put_blake3_state(runtime);
            let state = runtime.op_state();
            state.borrow_mut().put(host_data());
            state.borrow_mut().put(ErrorClasses::default());
            drop(state);
            let held = |runtime: &mut JsRuntime| {
                runtime
                    .op_state()
                    .borrow()
                    .borrow::<NativeNamespaces>()
                    .len()
            };
            let before = held(runtime);
            runtime.execute_script("<test>", NATIVE_HELPERS).unwrap();
            // what each caller and each argument the op refuses gets, before
            // anything is held
            let call = |arguments: &str| {
                format!("JSON.stringify(thrown(() => Deno.core.ops.op_gro_namespace({arguments})))")
            };
            let trap = "new Proxy({}, { get() { globalThis.trapped = true; } })";
            let blake3 = backing_script("@fuzdev/gro/blake3.ts");
            // code of a caller's a row's script calls, which calls the op
            runtime
                .execute_script(
                    "<test>",
                    "globalThis.calls_op = () => Deno.core.ops.op_gro_namespace(undefined);",
                )
                .unwrap();
            let mut rows_refused = Vec::new();
            for (name, source) in [
                ("<test>", call("undefined")),
                (
                    blake3,
                    "JSON.stringify(thrown(() => calls_op()))".to_owned(),
                ),
                (
                    "file:///project/node_modules/x/@fuzdev/gro/blake3.ts",
                    call("undefined"),
                ),
                (
                    "<test>",
                    format!("{}\n//# sourceURL={blake3}", call("undefined")),
                ),
                ("ext:gro/builtin/nope.ts", call("undefined")),
                (backing_script("@fuzdev/gro/args.ts"), call("undefined")),
                (blake3, call("null")),
                (blake3, call(trap)),
                (blake3, call("undefined, Error")),
                (blake3, call("undefined, null")),
                (blake3, call("undefined, new Proxy(Error, {})")),
                (blake3, call(&format!("undefined, {trap}"))),
                (
                    backing_script("@fuzdev/gro/env.ts"),
                    call("undefined, Error"),
                ),
                (
                    backing_script("@fuzdev/gro/env_error.ts"),
                    call("null, Error"),
                ),
            ] {
                rows_refused.push(named_script_json(runtime, name, source));
            }
            rows_refused.push(script_json(
                runtime,
                "JSON.stringify(globalThis.trapped ?? false)",
            ));
            let errors_held = runtime
                .op_state()
                .borrow()
                .borrow::<NativeNamespaces>()
                .len();
            // a refused row's script hands its `Error` over to nothing
            rows_refused.push(json!(
                runtime
                    .op_state()
                    .borrow()
                    .borrow::<ErrorClasses>()
                    .holds_error()
            ));
            let mut namespaces = serde_json::Map::new();
            // every row's namespace is made in a run that grants each row
            runtime
                .op_state()
                .borrow_mut()
                .put(crate::test_support::run_authority());
            for row in native_rows() {
                let names = function_names(row, Place::Export);
                // what the row's script hands over, the realm's `Error` from
                // an error family's row alone
                let arguments = if hands_over_error(row) {
                    "undefined, Error"
                } else {
                    "undefined"
                };
                let source = format!(
                    "{{
	const ops = Deno.core.ops;
	const namespace = ops.op_gro_namespace({arguments});
	namespaces['{row}'] = namespace;
	JSON.stringify({{
		same: namespace === ops.op_gro_namespace({arguments}),
		record: shape(namespace),
		default: namespace.default === undefined,
		functions: functions(namespace, '{row}', {names}),
	}});
}}"
                );
                let made = named_script_json(runtime, backing_script(row), source);
                namespaces.insert(row.to_owned(), made);
            }
            let namespaces = Value::Object(namespaces);
            let after = held(runtime);
            let values = script_json(runtime, NATIVE_VALUES);
            runtime.op_state().borrow_mut().put(Authority::default());
            {
                let state = runtime.op_state();
                let state = state.borrow();
                deno_core::scope!(scope, runtime);
                let global = scope.get_current_context().global(scope);
                let prototypes = v8::Object::new(scope);
                for (_, kind) in native_kinds() {
                    // none when the run holds no namespace of its row
                    if let Some(prototype) = native::prototype(&state, scope, kind) {
                        let key = v8::String::new(scope, kind.brand).unwrap();
                        prototypes.set(scope, key.into(), prototype.into()).unwrap();
                    }
                }
                let key = v8::String::new(scope, "prototypes").unwrap();
                global.set(scope, key.into(), prototypes.into()).unwrap();
                let probe = native::probe::function(scope).unwrap();
                let key = v8::String::new(scope, "probe").unwrap();
                global.set(scope, key.into(), probe.into()).unwrap();
            }
            let prototypes = script_json(
                runtime,
                "{
	const result = {};
	for (const [brand, prototype] of Object.entries(prototypes)) {
		result[brand] = { record: shape(prototype), functions: functions(prototype, brand.split('#')[0], Object.keys(prototype)) };
	}
	JSON.stringify(result);
}",
            );
            let root_of_held = script_json(
                runtime,
                "JSON.stringify(Object.getPrototypeOf(namespaces['@fuzdev/gro/env.ts'].env) === prototypes['@fuzdev/gro/env.ts#handle'])",
            );
            let probe = "JSON.stringify(thrown(() => { globalThis.probed = probe(); }) ?? probed)";
            let probe_empty = script_json(runtime, probe);
            runtime
                .op_state()
                .borrow_mut()
                .put(Authority::Confined(Grants {
                    args: Some(ArgsGrant {}),
                    ..Grants::default()
                }));
            let probe_granted = script_json(runtime, probe);
            runtime.op_state().borrow_mut().take::<Authority>();
            let probe_none = script_json(runtime, probe);
            let hashers = |runtime: &mut JsRuntime| {
                runtime
                    .op_state()
                    .borrow()
                    .borrow::<Blake3State>()
                    .hashers_len()
            };
            let hashers_before = hashers(runtime);
            let refused = script_json(
                runtime,
                "JSON.stringify(Object.fromEntries(members.map(([label, fn]) => [label, thrown(() => fn())])))",
            );
            let hashers_after = hashers(runtime);
            // each value made with no authority: whether it was, and what
            // its maker threw
            let mut value_refusals = serde_json::Map::new();
            {
                let state = runtime.op_state();
                let state = state.borrow();
                deno_core::scope!(scope, runtime);
                v8::tc_scope!(let tc, scope);
                for member in NATIVE_MEMBERS {
                    let Member::Value(make) = member.member else {
                        continue;
                    };
                    let made = make(&state, tc, &Prototypes::none()).is_some();
                    let thrown = tc.exception().map(|thrown| thrown.to_rust_string_lossy(tc));
                    tc.reset();
                    let label = format!("{} of {}", member.name, member.module);
                    value_refusals.insert(label, json!([made, thrown]));
                }
            }
            let probe_ran = native::probe::RAN.load(std::sync::atomic::Ordering::Relaxed);
            // the probe table's value made in a run that grants nothing, one
            // that grants `args`, and a runtime with no authority: what it
            // gave or threw; then its namespace, made in a run that grants
            // `args`, whole, and with its value failing
            let granted_args = Authority::Confined(Grants {
                args: Some(ArgsGrant {}),
                ..Grants::default()
            });
            let mut value_probe = Vec::new();
            for authority in [Some(Authority::default()), Some(granted_args.clone()), None] {
                match authority {
                    Some(authority) => runtime.op_state().borrow_mut().put(authority),
                    None => drop(runtime.op_state().borrow_mut().try_take::<Authority>()),
                }
                let state = runtime.op_state();
                let state = state.borrow();
                deno_core::scope!(scope, runtime);
                v8::tc_scope!(let tc, scope);
                let made =
                    native::probe::value(&state, tc).map(|made| made.to_rust_string_lossy(tc));
                let thrown = tc.exception().map(|thrown| thrown.to_rust_string_lossy(tc));
                tc.reset();
                value_probe.push(json!([made, thrown]));
            }
            let value_made = native::probe::MADE.load(std::sync::atomic::Ordering::Relaxed);
            runtime.op_state().borrow_mut().put(granted_args);
            let mut namespace_probe = Vec::new();
            for fail in [false, true] {
                native::probe::FAIL.store(fail, std::sync::atomic::Ordering::Relaxed);
                let state = runtime.op_state();
                let state = state.borrow();
                deno_core::scope!(scope, runtime);
                v8::tc_scope!(let tc, scope);
                let made = native::probe::namespace_made(&state, tc);
                let thrown = tc.exception().map(|thrown| thrown.to_rust_string_lossy(tc));
                tc.reset();
                namespace_probe.push(json!([made, thrown]));
            }
            native::probe::FAIL.store(false, std::sync::atomic::Ordering::Relaxed);
            json!({
                "namespaces": namespaces,
                "values": values,
                "root_of_held": root_of_held,
                "prototypes": prototypes,
                "rows_refused": rows_refused,
                "held": [before, errors_held, after],
                "probe": [probe_empty, probe_granted, probe_none],
                "refused": refused,
                "refused_effects": [hashers_before, hashers_after, probe_ran],
                "value_refusals": value_refusals,
                "value_probe": [value_probe, value_made, namespace_probe],
                "pid": std::process::id(),
            })
        })
    }

    /// The function shape the probe reads of `member`'s function, by its
    /// name, `None` for a value: frozen, of the realm's
    /// `Function.prototype`, with its own `length` and `name` only, so no
    /// `prototype`, its name and `length` the table's, its source text
    /// native code's, and not a constructor.
    fn native_function_shape(member: &NativeMember) -> Option<(String, Value)> {
        let Member::Function { length, .. } = member.member else {
            return None;
        };
        let shape = json!({
            "type": "function",
            "frozen": true,
            "prototype": "Function.prototype",
            "keys": ["length", "name"],
            "name": member.name,
            "length": length,
            "native_source": true,
            "constructed": "TypeError",
        });
        Some((member.name.to_owned(), shape))
    }

    /// The native members from either start: each row's namespace is made
    /// once and held, the same object every ask, a frozen null-prototype
    /// record of exactly the table's exports and the `default` every
    /// builtin has, `undefined`; each kind's prototype a frozen
    /// null-prototype record of exactly the table's methods; each function
    /// frozen, named and of the `length` the table says, with no
    /// `prototype`, its source text native code's, and no constructor.
    #[test]
    fn a_native_row_s_namespace_is_the_table_s_made_once() {
        for start in STARTS {
            let results = probed("native_members", start);
            let rows = native_rows();
            assert_eq!(results["held"], json!([0, 0, rows.len()]), "{start:?}");
            for row in rows {
                let exports: Vec<&NativeMember> = NATIVE_MEMBERS
                    .iter()
                    .filter(|member| member.module == row && member.place == Place::Export)
                    .collect();
                let mut keys: Vec<&str> = exports.iter().map(|member| member.name).collect();
                keys.push("default");
                let functions: serde_json::Map<String, Value> = exports
                    .iter()
                    .filter_map(|member| native_function_shape(member))
                    .collect();
                assert_eq!(
                    results["namespaces"][row],
                    json!({
                        "same": true,
                        "record": {"type": "object", "frozen": true, "prototype": null, "keys": keys},
                        "default": true,
                        "functions": functions,
                    }),
                    "{start:?} {row}"
                );
            }
            let kinds = native_kinds();
            assert!(!kinds.is_empty());
            for (_, kind) in &kinds {
                let methods: Vec<&NativeMember> = NATIVE_MEMBERS
                    .iter()
                    .filter(|member| member.place == Place::Method(kind))
                    .collect();
                let keys: Vec<&str> = methods.iter().map(|member| member.name).collect();
                let functions: serde_json::Map<String, Value> = methods
                    .iter()
                    .filter_map(|member| native_function_shape(member))
                    .collect();
                assert_eq!(
                    results["prototypes"][kind.brand],
                    json!({
                        "record": {"type": "object", "frozen": true, "prototype": null, "keys": keys},
                        "functions": functions,
                    }),
                    "{start:?} {}",
                    kind.brand
                );
            }
            assert_eq!(
                results["prototypes"].as_object().unwrap().len(),
                kinds.len(),
                "{start:?}"
            );
        }
    }

    /// What `member`, a `what` (a host function or a host value), throws in
    /// a runtime that holds no authority: `not_granted` naming it, and its
    /// grant when it needs one.
    fn member_refusal(member: &NativeMember, what: &str) -> String {
        let label = format!("{} of {}", member.name, member.module);
        member.grant.map_or_else(
            || {
                format!(
                    "Error: not_granted: {label} is a {what}, and this runtime holds no \
                     authority, so it runs none"
                )
            },
            |grant| {
                format!(
                    "Error: not_granted: {label} needs the grant '{grant}', which this runtime \
                     doesn't hold"
                )
            },
        )
    }

    /// The form's check from either start: every native function, called
    /// in a runtime that holds no authority, throws `not_granted` naming
    /// itself, and its grant when it needs one, before its body runs,
    /// whatever it's passed; and a member that needs
    /// a grant (the probe table's) throws it, naming the grant, in a run that
    /// grants nothing and in a runtime with no authority, and runs in a run
    /// that grants it. A body that skips its check returns or throws what it
    /// does instead, and one that runs before its check leaves its effect:
    /// a refused factory made no hasher, and the probe's body ran only where
    /// it was granted.
    #[test]
    fn every_native_member_refuses_without_its_grant() {
        for start in STARTS {
            let results = probed("native_members", start);
            let refused = results["refused"].as_object().unwrap();
            let functions = NATIVE_MEMBERS
                .iter()
                .filter(|member| matches!(member.member, Member::Function { .. }));
            assert_eq!(refused.len(), functions.clone().count(), "{start:?}");
            for member in functions {
                let label = format!("{} of {}", member.name, member.module);
                assert_eq!(
                    refused[&label],
                    member_refusal(member, "host function"),
                    "{start:?}"
                );
            }
            assert_eq!(
                results["probe"],
                json!([
                    "Error: not_granted: probe of @fuzdev/gro/args.ts needs the grant 'args', \
                     which this runtime doesn't hold",
                    "ran",
                    "Error: not_granted: probe of @fuzdev/gro/args.ts needs the grant 'args', \
                     which this runtime doesn't hold",
                ]),
                "{start:?}"
            );
            // no refused call's body ran: no hasher was made, and the
            // probe's body ran once, in the run that grants it
            assert_eq!(results["refused_effects"], json!([0, 0, 1]), "{start:?}");
        }
    }

    /// A row's values, from either start: each made once with its
    /// namespace in a run that grants its row, of the run's data (the
    /// arguments a frozen array of strings, the runtime's name and version,
    /// the process's id), with `cwd` returning the run's directory; and each
    /// value's maker, run in a runtime that holds no authority, throws
    /// `not_granted` naming the value and makes nothing, a granted row's
    /// naming its grant.
    #[test]
    fn a_native_row_s_values_are_the_run_s() {
        for start in STARTS {
            let results = probed("native_members", start);
            assert_eq!(
                results["values"],
                json!({
                    "args": ["a"],
                    "args_shape": {
                        "type": "object",
                        "frozen": true,
                        "prototype": "other",
                        "keys": ["0", "length"],
                    },
                    "args_array": true,
                    "cwd": "/work",
                    "pid": results["pid"],
                    "runtime_version": "test/1.2.3",
                    "env": {"type": "object", "frozen": true, "prototype": "other", "keys": []},
                    "env_reads": [null, []],
                }),
                "{start:?}"
            );
            let values: Vec<&NativeMember> = NATIVE_MEMBERS
                .iter()
                .filter(|member| matches!(member.member, Member::Value(_)))
                .collect();
            assert!(!values.is_empty());
            // the root is a handle of the prototype the host made, and holds
            assert_eq!(results["root_of_held"], true, "{start:?}");
            let refused = results["value_refusals"].as_object().unwrap();
            assert_eq!(refused.len(), values.len(), "{start:?}");
            for member in values {
                let label = format!("{} of {}", member.name, member.module);
                let thrown = member_refusal(member, "host value");
                assert_eq!(refused[&label], json!([false, thrown]), "{start:?}");
            }
            // a value of a row that needs a grant is made where it's granted
            // alone, its maker's body running past the check only there, and
            // a namespace one of whose values failed is never made
            let refused = "Error: not_granted: probe_value of @fuzdev/gro/args.ts needs the \
                           grant 'args', which this runtime doesn't hold";
            assert_eq!(
                results["value_probe"],
                json!([
                    [[null, refused], ["made", null], [null, refused]],
                    1,
                    [[true, null], [false, "Error: the probe value failed"]],
                ]),
                "{start:?}"
            );
        }
    }

    /// The namespace op knows the row by the script that calls it, never by
    /// what the script says, after its own check of an authority: a caller
    /// that is no row's backing script is a `TypeError`, one that names a
    /// row's script in a `sourceURL` and one of a name no row has among
    /// them; a row whose grant the run lacks is refused naming the grant,
    /// before anything is made; and a scope that isn't `undefined`, or
    /// anything but `undefined` handed over as the realm's `Error` by a row
    /// of no error family's, is a `TypeError`, read with no
    /// conversion: no trap runs, and nothing is held.
    #[test]
    fn the_namespace_op_knows_the_row_by_its_script() {
        let not_a_row = "TypeError: a namespace is asked for by a builtin's own script";
        let not_a_scope = "TypeError: no row takes a scope yet";
        let not_handed = "TypeError: a row whose members throw no host error hands over no Error";
        for start in STARTS {
            assert_eq!(
                probed("native_members", start)["rows_refused"],
                json!([
                    not_a_row,
                    not_a_row,
                    not_a_row,
                    not_a_row,
                    not_a_row,
                    "Error: not_granted: op_gro_namespace needs the grant 'args', which this \
                     runtime doesn't hold",
                    not_a_scope,
                    not_a_scope,
                    not_handed,
                    not_handed,
                    not_handed,
                    not_handed,
                    "Error: not_granted: op_gro_namespace needs the grant 'env', which this \
                     runtime doesn't hold",
                    not_a_scope,
                    false,
                    false,
                ]),
                "{start:?}"
            );
        }
    }

    /// A hasher's native state lives as long as its handle can be reached:
    /// a collection drops the state of each hasher no longer reachable, and
    /// keeps a reachable one's, which still hashes; and V8 counts each live
    /// hasher's native state as external memory, and stops when it's
    /// dropped.
    #[test]
    fn a_hashers_native_state_is_dropped_once_its_handle_is_collected() {
        let (made, collected, digest, external) =
            on_runtime("blake3_collected", Start::Source, Mode::Run, |runtime| {
                put_blake3_state(runtime);
                // the external memory V8 counts, read as a change of nothing
                let external = |runtime: &mut JsRuntime| {
                    runtime
                        .v8_isolate()
                        .adjust_amount_of_external_allocated_memory(0)
                };
                // what was garbage before the hashers is collected first
                runtime.v8_isolate().low_memory_notification();
                let before = external(runtime);
                runtime
                    .execute_script(
                        backing_script("@fuzdev/gro/blake3.ts"),
                        "const { create_hasher } = Deno.core.ops.op_gro_namespace(undefined);
globalThis.kept = create_hasher();
for (let i = 0; i < 100; i++) create_hasher();",
                    )
                    .unwrap();
                let len = |runtime: &mut JsRuntime| {
                    runtime
                        .op_state()
                        .borrow()
                        .borrow::<Blake3State>()
                        .hashers_len()
                };
                let made = len(runtime);
                let made_external = external(runtime) - before;
                runtime.v8_isolate().low_memory_notification();
                let collected = len(runtime);
                let collected_external = external(runtime) - before;
                let digest = script_json(
                    runtime,
                    "globalThis.kept.update(new Uint8Array([1]));
JSON.stringify(globalThis.kept.finalize().length)",
                );
                (made, collected, digest, (made_external, collected_external))
            });
        assert_eq!(made, 101);
        assert_eq!(collected, 1);
        assert_eq!(digest, 32);
        let size = i64::try_from(size_of::<blake3::Hasher>()).unwrap();
        assert_eq!(external, (101 * size, size));
    }

    /// A hasher's number is never one another had: a live hasher keeps its
    /// own state across a collection of others and the making of more,
    /// which a number reused once its hasher was collected would hand to a
    /// new hasher, its entry overwriting the live one's.
    #[test]
    fn a_live_hasher_keeps_its_state_across_collections() {
        let kept = on_runtime("blake3_ids", Start::Source, Mode::Run, |runtime| {
            put_blake3_state(runtime);
            runtime
                .execute_script(
                    backing_script("@fuzdev/gro/blake3.ts"),
                    "globalThis.blake3 = Deno.core.ops.op_gro_namespace(undefined);
globalThis.live = [];
for (let i = 0; i < 4; i++) blake3.create_hasher();
for (let i = 0; i < 3; i++) {
\tconst hasher = blake3.create_hasher();
\thasher.update(new Uint8Array([i + 1]));
\tlive.push(hasher);
}
globalThis.before = live.map((hasher) => hasher.finalize().join());",
                )
                .unwrap();
            runtime.v8_isolate().low_memory_notification();
            script_json(
                runtime,
                "{
\tfor (let i = 0; i < 8; i++) {
\t\tconst hasher = blake3.create_hasher();
\t\thasher.update(new Uint8Array([200, i]));
\t\tlive.push(hasher);
\t}
\tJSON.stringify(live.slice(0, 3).map((hasher) => hasher.finalize().join()).join() === before.join());
}",
            )
        });
        assert_eq!(kept, true);
    }

    /// The run's data the native members are made with below.
    fn host_data() -> HostData {
        HostData {
            args: vec!["a".to_owned()],
            cwd: Some("/work".to_owned()),
            runtime_version: "test/1.2.3".to_owned(),
            env: [("A", "1"), ("B", "2")]
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .into_iter()
                .collect(),
        }
    }

    /// Calls every host op through `Deno.core.ops`, before the start
    /// removes it, on a runtime started as `start` says and built as `mode`
    /// says whose `OpState` holds `authority` (none at all when unset, as
    /// eval's) and [`host_data`], returning each op's result as JSON or
    /// what it threw. Each op is called with no argument: a host op that
    /// takes one takes it as the value passed, converting nothing
    /// (`host_ops!`), so its check comes first here too.
    fn call_host_ops(name: &str, start: Start, mode: Mode, authority: Option<Authority>) -> Value {
        let names: Vec<&str> = HOST_OPS.to_vec();
        let source = format!(
            "const results = {{}};
for (const name of {}) {{
\ttry {{
\t\tresults[name] = JSON.stringify(Deno.core.ops[name]());
\t}} catch (error) {{
\t\tresults[name] = `${{error.constructor.name}}: ${{error.message}}`;
\t}}
}}
JSON.stringify(results)",
            serde_json::to_string(&names).unwrap()
        );
        on_runtime(name, start, mode, move |runtime| {
            let state = runtime.op_state();
            if let Some(authority) = authority {
                state.borrow_mut().put(authority);
            }
            state.borrow_mut().put(host_data());
            state.borrow_mut().put(ErrorClasses::default());
            drop(state);
            script_json(runtime, source)
        })
    }

    /// Every host op's result in a runtime that holds no authority, in each
    /// mode, in a run that grants nothing, and in one that grants the
    /// script's arguments alone. No run here grants `exit`, whose function would
    /// end this process.
    fn host_ops_results(start: Start) -> Value {
        let granted = Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
            ..Grants::default()
        });
        json!({
            "none_eval": call_host_ops("host_none_eval", start, Mode::Eval, None),
            "none_run": call_host_ops("host_none_run", start, Mode::Run, None),
            "empty_run": call_host_ops("host_empty_run", start, Mode::Run, Some(Authority::default())),
            "granted_run": call_host_ops("host_granted_run", start, Mode::Run, Some(granted)),
        })
    }

    /// What a module that imports every builtin saw on a runtime started as
    /// `start` says, in a run that grants `args`: `ok`, or the report of
    /// what failed. The run's arguments are replaced between the runtime's
    /// start and the module's load, so the module tells a script run by its
    /// import from one the start ran. With it, how many implementations the
    /// process had stripped by then, and whether the import made an op's
    /// function again: `deno_core` remakes each op that has a fast call the
    /// first time code calls `core.loadExtScript` for a script the snapshot
    /// lacks, and a builtin's first import doesn't go through that call.
    fn builtin_imports(start: Start) -> Value {
        use std::sync::atomic::Ordering;

        use crate::resolver::DiskTree;

        init_engine(Confinement::Unconfined, &engine_options()).unwrap();
        let snapshot = (start == Start::Snapshot).then(child_snapshot);
        let dir = TestDir::on_disk(&format!("builtin_imports_{start:?}"));
        let rows: Vec<(&str, &[&str])> = BUILTINS
            .iter()
            .map(|builtin| (builtin.specifier, builtin.exports))
            .collect();
        let source = format!(
            "const check = (what, ok) => {{ if (!ok) throw new Error(what); }};
const seen = {{}};
for (const [specifier, exports] of {}) {{
\tconst ns = await import(specifier);
\tconst keys = Reflect.ownKeys(ns).filter((key) => typeof key === 'string');
\tcheck(`${{specifier}} exports ${{keys}}`, JSON.stringify(keys) === JSON.stringify([...exports, 'default'].sort()));
\tcheck(`${{specifier}} default`, ns.default === undefined);
\tObject.assign(seen, ns);
}}
check(`args ${{JSON.stringify(seen.args)}}`, JSON.stringify(seen.args) === '[\"a\"]');
check(`cwd ${{seen.cwd()}}`, seen.cwd() === '/work');
check(`pid ${{seen.pid}}`, seen.pid === {});
check(`runtime_version ${{seen.runtime_version}}`, seen.runtime_version === 'test/1.2.3');
const error = new seen.EnvError('not_granted', 'HOME');
check(`EnvError ${{error.message}}`, error.message === 'not_granted: HOME' && error instanceof Error);
check('is_env_error', seen.is_env_error(error, 'not_granted') && !seen.is_env_error(new Error('HOME')));
check('env', seen.env.get('GRO_TEST_ENV') === undefined && Reflect.ownKeys(seen.env.get_all()).length === 0);
try {{
	seen.env.get('A');
	throw new Error('read A');
}} catch (refused) {{
	check(`env refused ${{refused}}`, seen.is_env_error(refused, 'not_granted') && refused.message === 'not_granted: A');
}}
const hex = (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
const empty = hex(seen.hash(new Uint8Array()));
check(`hash ${{empty}}`, empty === 'af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262');
const hasher = seen.create_hasher();
hasher.update(new Uint8Array([1, 2]));
hasher.update(new Uint8Array([3]));
check('a hasher', hex(hasher.finalize()) === hex(seen.hash(new Uint8Array([1, 2, 3]))));
check('an op was made again', globalThis.core_ops.op_gro_color === globalThis.op_gro_color);
",
            serde_json::to_string(&rows).unwrap(),
            std::process::id()
        );
        std::fs::write(dir.path().join("main.mjs"), source).unwrap();
        let cwd = dir.path().to_owned();
        let imported = std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || -> Result<(), String> {
                let (entry, roots) =
                    crate::resolver::resolve_entry(&DiskTree, Path::new("main.mjs"), &cwd).unwrap();
                let authority = crate::test_support::run_authority();
                let loader = Loader::new(&entry, roots, Recording::Off);
                let loader = Rc::new(loader.with_authority(authority.clone()));
                let refusals = loader.refusal_log();
                let tokio = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                tokio.block_on(async {
                    let mut runtime = new_runtime(snapshot, loader, Mode::Run, None)?;
                    // a fast op's function, and the object the remaking writes to
                    runtime
                        .execute_script(
                            "<test>",
                            "globalThis.core_ops = Deno.core.ops;
globalThis.op_gro_color = Deno.core.ops.op_gro_color;",
                        )
                        .map_err(|error| report_js(&error))?;
                    let state = runtime.op_state();
                    state.borrow_mut().put(authority);
                    state.borrow_mut().put(HostData {
                        args: vec!["the start's".to_owned()],
                        ..host_data()
                    });
                    state.borrow_mut().put(ErrorClasses::default());
                    state.borrow_mut().put(NativeNamespaces::default());
                    state.borrow_mut().put(Blake3State::default());
                    state.borrow_mut().put(refusals);
                    let mut runtime = super::start(runtime, ConsoleOptions::default(), Mode::Run)?;
                    state.borrow_mut().put(host_data());
                    let id = runtime
                        .load_main_es_module(&entry.url)
                        .await
                        .map_err(|error| report(&error))?;
                    let evaluation = runtime.mod_evaluate(id);
                    runtime
                        .run_event_loop(deno_core::PollEventLoopOptions::default())
                        .await
                        .map_err(|error| report(&error))?;
                    evaluation.await.map_err(|error| report(&error))
                })
            })
            .unwrap()
            .join()
            .unwrap();
        json!({
            "imported": imported.err().unwrap_or_else(|| "ok".to_owned()),
            "stripped": bootstrap::STRIPPED.load(Ordering::Relaxed),
        })
    }

    /// Every builtin imports from either start with the run's values, its
    /// script run by its first import, never by the start, with the ops the
    /// runtime's code already holds; and a process whose runtimes start from
    /// a snapshot strips no implementation, where one that starts a runtime
    /// from source strips each.
    #[test]
    fn a_builtin_s_script_runs_at_its_first_import_from_either_start() {
        for start in STARTS {
            assert_eq!(
                probed("builtin_imports", start)["imported"],
                "ok",
                "{start:?}"
            );
        }
        assert_eq!(probed("builtin_imports", Start::Snapshot)["stripped"], 0);
        let stripped = probed("builtin_imports", Start::Source)["stripped"].as_u64();
        assert!(stripped.unwrap() >= BUILTINS.len() as u64, "{stripped:?}");
    }

    /// The choke point by behavior: every host op, called by a runtime that
    /// holds no authority (eval's), in either mode and from either start,
    /// throws `not_granted` naming itself before its body runs; and runs in
    /// a run whatever it grants, nothing included, so its refusals are the
    /// missing authority's: the namespace op, called by a script that is no
    /// row's, gets past its check to refuse the caller. From either start:
    /// an op reads the `OpState` of the runtime that calls it, and the
    /// snapshot was made by a process that held none of this.
    #[test]
    fn every_host_op_refuses_without_an_authority() {
        for start in STARTS {
            let results = probed("host_ops", start);
            for op in HOST_OPS {
                for case in ["none_eval", "none_run"] {
                    assert_eq!(
                        results[case][op].as_str().unwrap_or_default(),
                        format!(
                            "Error: not_granted: {op} is a host op, and this runtime holds no \
                             authority, so it runs none"
                        ),
                        "{case} {start:?}"
                    );
                }
                for case in ["granted_run", "empty_run"] {
                    assert_eq!(
                        results[case][op].as_str().unwrap_or_default(),
                        "TypeError: a namespace is asked for by a builtin's own script",
                        "{case} {start:?}"
                    );
                }
            }
        }
    }

    /// The calls of web globals whose own code calls an op eval refuses.
    const REFUSED_WEB_CALLS: [&str; 3] = ["performance.now()", "new Blob()", "setTimeout()"];

    /// What each refused op did, called through `Deno.core.ops` before the
    /// start removes it, whatever it's passed, and what each of
    /// [`REFUSED_WEB_CALLS`] did, in an eval runtime and in `fgro run`'s:
    /// `ran`, or what it threw. An async op's stub returns its throw as a
    /// rejected promise, read once the microtasks have run.
    fn refused_ops_results(start: Start) -> Value {
        let source = format!(
            "const results = (globalThis.results = {{}});
const failed = (error) => `${{error.constructor.name}}: ${{error.message}}`;
for (const name of {}) {{
\ttry {{
\t\tconst result = Deno.core.ops[name]('1', 1);
\t\tresults[name] = 'ran';
\t\tif (result instanceof Promise) {{
\t\t\tresult.catch((error) => {{
\t\t\t\tresults[name] = failed(error);
\t\t\t}});
\t\t}}
\t}} catch (error) {{
\t\tresults[name] = failed(error);
\t}}
}}
for (const [name, call] of Object.entries({{
\t'performance.now()': () => performance.now(),
\t'new Blob()': () => new Blob(['a']),
\t'setTimeout()': () => clearTimeout(setTimeout(() => {{}}, 1)),
}})) {{
\ttry {{
\t\tcall();
\t\tresults[name] = 'ran';
\t}} catch (error) {{
\t\tresults[name] = failed(error);
\t}}
}}",
            serde_json::to_string(&bootstrap::REFUSED_OPS).unwrap()
        );
        let call = |source: String| {
            move |runtime: &mut JsRuntime| -> Value {
                runtime.execute_script("<test>", source).unwrap();
                runtime.v8_isolate().perform_microtask_checkpoint();
                script_json(runtime, "JSON.stringify(globalThis.results)")
            }
        };
        json!({
            "eval": on_runtime("refused_eval", start, Mode::Eval, call(source.clone())),
            "run": on_runtime("refused_run", start, Mode::Run, call(source)),
        })
    }

    /// Fails unless `results`, [`refused_ops_results`]' of the runtimes
    /// `runtimes` describes, are that every refused op and each of
    /// [`REFUSED_WEB_CALLS`] threw the refusal in the eval runtime, and
    /// that the clock reads and the web calls ran in `fgro run`'s.
    fn assert_eval_refused(results: &Value, runtimes: &str) {
        let refusal = format!("Error: {}", bootstrap::REFUSED);
        for name in bootstrap::REFUSED_OPS.iter().chain(&REFUSED_WEB_CALLS) {
            assert_eq!(results["eval"][name], refusal.as_str(), "{name} {runtimes}");
        }
        // the clock reads run in `fgro run`'s runtime, unrefused
        let ran = ["op_timer_now", "op_memory_usage"];
        for name in ran.iter().chain(&REFUSED_WEB_CALLS) {
            assert_eq!(results["run"][name], "ran", "{name} {runtimes}");
        }
    }

    /// In an eval runtime, each refused op throws the refusal, and so does
    /// a web global whose own code calls one, before `determinism: deny`
    /// stands in front of it. In `fgro run`'s runtime the same ops run.
    /// From either start: the snapshot is made with every op as declared,
    /// and a runtime started from it calls its own implementation of each.
    #[test]
    fn eval_refuses_the_refused_ops() {
        for start in STARTS {
            assert_eval_refused(&probed("refused_ops", start), &format!("{start:?}"));
        }
    }

    /// On each of eval's engines, jitless and with the JIT, a runtime
    /// started from the snapshot refuses as it does on the engine
    /// `fgro run` sets up, which made the snapshot.
    #[test]
    fn eval_refuses_the_refused_ops_on_its_engines() {
        for ((engine, _), results) in EVAL_ENGINES.iter().zip(&snapshot_results().eval) {
            assert_eval_refused(results, engine);
        }
    }

    /// The snapshot the build made, the one `fgro` embeds, refuses the same
    /// on each of eval's engines, in runtimes this test binary starts from
    /// it. The build's `fgro` made it, encoding each op's function by its
    /// place in that process's table of external references, and a start
    /// reads the place in its own table, so an op the snapshot encodes by
    /// another op's place runs here as it would in `fgro`: a refused op as
    /// one eval keeps, or the other way. The runtimes start with no
    /// residual script, and the probe loads none.
    #[test]
    #[ignore = "needs the snapshot `cargo xtask build` made, in the directory GRO_SNAPSHOT_DIR \
                names: `cargo xtask build --verify` runs it"]
    fn the_built_snapshot_refuses_the_refused_ops() {
        let built = std::env::var_os(BUILT_SNAPSHOT_DIR)
            .unwrap_or_else(|| panic!("{BUILT_SNAPSHOT_DIR} names no directory"));
        let dir = TestDir::in_system_temp("built_snapshot");
        let file = dir.path().join("snapshot.bin");
        std::fs::copy(PathBuf::from(built).join("snapshot.bin"), &file).unwrap();
        std::fs::write(backing_scripts_file(&file), "[]").unwrap();
        for (engine, _) in EVAL_ENGINES {
            assert_eval_refused(
                &eval_engine_results(&file, engine),
                &format!("the built snapshot, {engine}"),
            );
        }
    }

    /// An op that raises a `DOMException` class reaches JS as a
    /// `DOMException` of that name, built by the bootstrap's builders:
    /// `atob`'s op among them, and every class the bootstrap registers
    /// builds one.
    #[test]
    fn op_errors_of_a_dom_exception_class_are_dom_exceptions() {
        let source = "const read = (f) => {
\ttry {
\t\tf();
\t\treturn 'ran';
\t} catch (error) {
\t\treturn error instanceof DOMException ? `${error.name} ${error.code}: ${error.message}` : `not a DOMException: ${error}`;
\t}
};
const {ops} = Deno.core;
JSON.stringify([
\tread(() => atob('!')),
\tread(() => ops.op_base64_atob('!')),
\t...['IndexSizeError', 'InvalidCharacterError', 'InvalidStateError', 'OperationError', 'SyntaxError'].map(
\t\t(name) => read(() => { throw Deno.core.buildCustomError(`DOMException${name}`, 'm'); })
\t)
])";
        let results = on_new_runtime("dom_exceptions", Mode::Run, move |runtime| {
            let value = runtime.execute_script("<test>", source).unwrap();
            deno_core::scope!(scope, runtime);
            let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
            serde_json::from_str::<Vec<String>>(&json).unwrap()
        });
        assert_eq!(
            results,
            [
                "InvalidCharacterError 5: Failed to decode base64",
                "InvalidCharacterError 5: Failed to decode base64",
                "IndexSizeError 1: m",
                "InvalidCharacterError 5: m",
                "InvalidStateError 11: m",
                "OperationError 0: m",
                "SyntaxError 12: m",
            ]
        );
    }
}
