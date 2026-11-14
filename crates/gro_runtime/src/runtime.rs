//! Building a runtime: its mode, the `deno_core` runtime over a loader, and the
//! bootstrap's per-run steps.

use std::rc::Rc;

use deno_core::{JsRuntime, ModuleCodeString, ModuleName, v8};
use gro_v8::ArrayBufferRecord;

use crate::bootstrap::{self, Determinism, Ops};
use crate::host::HostData;
use crate::loader::Loader;
use crate::phases::{self, Phase};
use crate::prices::{self, EvalDefault};
use crate::report::{report, report_js};
use crate::snapshot::StartupSnapshot;
use crate::stack;
use crate::{Authority, ConsoleOptions, eval};

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

/// What `fgro run`'s host ops read, put in the runtime's `OpState` before
/// anything runs: the run's authority, which every host op checks first,
/// and the run's data they return. The default grants nothing.
#[derive(Debug, Clone, Default)]
pub struct HostState {
    /// The run's authority.
    pub authority: Authority,
    /// The run's data.
    pub data: HostData,
}

/// Starts `fgro run`'s runtime over `loader` from `snapshot` when there is
/// one, its host ops holding `host`, and runs the bootstrap's per-run steps
/// with the console as `console` says, returning the report of what failed
/// when either fails. Must run inside a tokio runtime, which `deno_core`
/// drives its event loop with.
pub fn start_runtime(
    snapshot: Option<StartupSnapshot>,
    console: ConsoleOptions,
    host: HostState,
    loader: Rc<Loader>,
) -> Result<JsRuntime, String> {
    let runtime = new_runtime(snapshot, loader, Mode::Run, None)?;
    let HostState { authority, data } = host;
    let state = runtime.op_state();
    state.borrow_mut().put(authority);
    state.borrow_mut().put(data);
    drop(state);
    start(runtime, console, Mode::Run)
}

/// Starts eval's runtime as [`start_runtime`] starts `fgro run`'s, built as
/// [`Mode::Eval`] says, its array buffers allocated by an allocator that
/// records whether its latest allocation failed, returned with the
/// runtime. It takes no authority: eval's host ops find none, so every one
/// refuses.
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
        // the builtins' TypeScript is stripped when the snapshot is made, so
        // only a start from source strips it
        extension_transpiler: if snapshot.is_none() {
            Some(Rc::new(|name: ModuleName, code: ModuleCodeString| {
                bootstrap::transpile(&name, code)
            }))
        } else {
            None
        },
        startup_snapshot: snapshot.map(|snapshot| snapshot.bytes),
        residual_lazy_js_sources: snapshot.map_or(&[], |snapshot| snapshot.residual_lazy_js),
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
    use std::path::Path;

    use super::*;
    use crate::STACK_SIZE;
    use crate::confine::Confinement;
    use crate::engine::{EngineOptions, init_engine};
    use crate::host::{HOST_OPS, HostOp};
    use crate::loader::graph::Recording;
    use crate::resolver::{BUILTINS, Capability, find_builtin};
    use crate::test_support::TestDir;
    use crate::{ArgsGrant, Grants};

    /// Runs `test` on a runtime built as `mode` says, from source, before
    /// the bootstrap's per-run steps, so `Deno.core.ops` is still there, on
    /// a thread reserving [`STACK_SIZE`].
    fn on_new_runtime<T: Send + 'static>(
        name: &str,
        mode: Mode,
        test: impl FnOnce(&mut JsRuntime) -> T + Send + 'static,
    ) -> T {
        use crate::resolver::DiskTree;

        init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
        let dir = TestDir::on_disk(name);
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
                let mut runtime = new_runtime(None, loader, mode, create_params).unwrap();
                test(&mut runtime)
            })
            .unwrap()
            .join()
            .unwrap()
    }

    /// Every op a runtime registers, `deno_core`'s own included, pinned in
    /// `tests/op_names.txt` in registration order, so a `deno_core` or
    /// `deno_web` bump that adds, removes, or renames one fails here and
    /// gets a look: an op that compiles code, imports, or reads the clock or
    /// randomness joins `bootstrap::REFUSED_OPS`. An eval runtime registers the same
    /// names, since the middleware keeps each refused op's name, and every
    /// name eval refuses is registered, so none is refused in name only. To
    /// accept a change deliberately, write the list the assertion prints
    /// over the file, one name a line.
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
            .chain(HOST_OPS.iter().map(|op| op.name))
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

    /// The bootstrap's factories (`gro_builtins` on `internals`, keyed by
    /// specifier in `js/bootstrap.js`) are exactly the builtin table's, in
    /// its order, each a function of one parameter, its grant's scope: the
    /// table is the one source of the builtins, and a key it lacks or
    /// misspells would leave its backing script calling nothing.
    #[test]
    fn the_bootstrap_s_factories_are_the_builtin_table_s() {
        let keys = on_new_runtime("builtin_factories", Mode::Run, |runtime| -> Vec<String> {
            let value = runtime
                .execute_script(
                    "<test>",
                    "const builtins = globalThis.__bootstrap.internals.gro_builtins;
JSON.stringify(Object.isFrozen(builtins) && Object.getPrototypeOf(builtins) === null
\t? Object.keys(builtins).filter((key) => typeof builtins[key] === 'function' && builtins[key].length === 1)
\t: null)",
                )
                .unwrap();
            deno_core::scope!(scope, runtime);
            let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
            serde_json::from_str(&json).unwrap()
        });
        let specifiers: Vec<&str> = BUILTINS.iter().map(|builtin| builtin.specifier).collect();
        assert_eq!(keys, specifiers);
    }

    /// The host ops' grants are the table's: each host op names the row of
    /// the module it serves and requires exactly that row's grant, none for
    /// a grant-free row's, so an op of a module that needs a grant can't be
    /// declared grant-free; and each grant a host op requires is a row's,
    /// and a table with a grant-free row has a grant-free op, so neither
    /// list grows a kind the other lacks.
    #[test]
    fn the_host_ops_grants_are_the_builtin_table_s() {
        for op in HOST_OPS {
            let row = find_builtin(op.module)
                .unwrap_or_else(|| panic!("{}: {} is no row of the table", op.name, op.module));
            assert_eq!(op.grant, row.grant, "{}", op.name);
        }
        let mut ops: Vec<Option<Capability>> = HOST_OPS.iter().map(|op| op.grant).collect();
        let mut rows: Vec<Option<Capability>> =
            BUILTINS.iter().map(|builtin| builtin.grant).collect();
        for grants in [&mut ops, &mut rows] {
            grants.sort_unstable_by_key(|grant| grant.map(|grant| grant.to_string()));
            grants.dedup();
        }
        assert_eq!(ops, rows);
        assert_eq!(
            HOST_OPS,
            [
                HostOp {
                    name: "op_gro_args",
                    module: "@fuzdev/gro/args.ts",
                    grant: Some(Capability::Args),
                },
                HostOp {
                    name: "op_gro_cwd",
                    module: "@fuzdev/gro/cwd.ts",
                    grant: None,
                },
                HostOp {
                    name: "op_gro_pid",
                    module: "@fuzdev/gro/pid.ts",
                    grant: None,
                },
                HostOp {
                    name: "op_gro_runtime_version",
                    module: "@fuzdev/gro/runtime_version.ts",
                    grant: None,
                },
            ]
        );
    }

    /// The run's data the host ops are called with below.
    fn host_data() -> HostData {
        HostData {
            args: vec!["a".to_owned()],
            cwd: Some("/work".to_owned()),
            runtime_version: "test/1.2.3".to_owned(),
        }
    }

    /// Calls every host op through `Deno.core.ops`, before the start
    /// removes it, on a runtime built as `mode` says whose `OpState` holds
    /// `authority` (none at all when unset, as eval's) and [`host_data`],
    /// returning each op's result as JSON or what it threw. Each op is
    /// called with no argument: a host op that takes one needs a sample
    /// here, or its argument's conversion throws before its check and the
    /// refusal test fails for the wrong reason.
    fn call_host_ops(name: &str, mode: Mode, authority: Option<Authority>) -> serde_json::Value {
        let names: Vec<&str> = HOST_OPS.iter().map(|op| op.name).collect();
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
        on_new_runtime(name, mode, move |runtime| {
            let state = runtime.op_state();
            if let Some(authority) = authority {
                state.borrow_mut().put(authority);
            }
            state.borrow_mut().put(host_data());
            drop(state);
            let value = runtime.execute_script("<test>", source).unwrap();
            deno_core::scope!(scope, runtime);
            let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
            serde_json::from_str(&json).unwrap()
        })
    }

    /// What the host op `op` throws in a runtime that holds no authority.
    fn refusal_without_authority(op: &HostOp) -> String {
        let name = op.name;
        op.grant.map_or_else(
            || {
                format!(
                    "Error: not_granted: {name} is a host op, and this runtime holds no \
                     authority, so it runs none"
                )
            },
            |grant| {
                format!(
                    "Error: not_granted: {name} needs the grant '{grant}', which this runtime \
                     doesn't hold"
                )
            },
        )
    }

    /// The choke point by behavior: every host op, a grant-free module's
    /// included, called by a runtime that holds no authority (eval's), in
    /// either mode, throws `not_granted` naming itself before its body
    /// runs; and each that requires a grant throws it, naming the grant,
    /// in a run that grants nothing. A body that skips its check returns
    /// its data here instead.
    #[test]
    fn every_host_op_refuses_without_its_grant() {
        for (case, mode) in [("host_none_eval", Mode::Eval), ("host_none_run", Mode::Run)] {
            let results = call_host_ops(case, mode, None);
            for op in HOST_OPS {
                assert_eq!(
                    results[op.name].as_str().unwrap_or_default(),
                    refusal_without_authority(op),
                    "{case}"
                );
            }
        }
        let results = call_host_ops("host_empty_run", Mode::Run, Some(Authority::default()));
        let granted: Vec<&HostOp> = HOST_OPS.iter().filter(|op| op.grant.is_some()).collect();
        assert!(!granted.is_empty());
        for op in granted {
            assert_eq!(
                results[op.name].as_str().unwrap_or_default(),
                refusal_without_authority(op)
            );
        }
    }

    /// Granted, the args op returns the script's arguments, so the
    /// refusals above are the grant's doing; and a grant-free module's op
    /// runs in a run whatever it grants, nothing included, returning the
    /// run's data, so its refusals above are the missing authority's.
    #[test]
    fn a_granted_host_op_runs_and_a_grant_free_one_in_every_run() {
        let granted = Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
        });
        let results = call_host_ops("host_granted", Mode::Run, Some(granted.clone()));
        assert_eq!(results["op_gro_args"], r#"["a"]"#);
        for (case, authority) in [
            ("host_free_granted", granted),
            ("host_free_empty", Authority::default()),
        ] {
            let results = call_host_ops(case, Mode::Run, Some(authority));
            assert_eq!(results["op_gro_cwd"], r#""/work""#, "{case}");
            assert_eq!(
                results["op_gro_pid"],
                std::process::id().to_string(),
                "{case}"
            );
            assert_eq!(
                results["op_gro_runtime_version"], r#""test/1.2.3""#,
                "{case}"
            );
            let free = HOST_OPS.iter().filter(|op| op.grant.is_none());
            for op in free {
                let result = results[op.name].as_str().unwrap_or_default();
                assert!(
                    !result.starts_with("Error"),
                    "{case}: {}: {result}",
                    op.name
                );
            }
        }
    }

    /// In an eval runtime, each refused op throws the refusal, whatever it's
    /// passed, through the one path a test still has to it, `Deno.core.ops`
    /// before the start removes it; an async op's stub returns the throw as
    /// a rejected promise, read once the microtasks have run. In `fgro
    /// run`'s runtime the same ops run.
    #[test]
    fn eval_refuses_the_refused_ops() {
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
}}",
            serde_json::to_string(&bootstrap::REFUSED_OPS).unwrap()
        );
        let call = |source: String| {
            move |runtime: &mut JsRuntime| -> serde_json::Value {
                runtime.execute_script("<test>", source).unwrap();
                runtime.v8_isolate().perform_microtask_checkpoint();
                let value = runtime
                    .execute_script("<test>", "JSON.stringify(globalThis.results)")
                    .unwrap();
                deno_core::scope!(scope, runtime);
                let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
                serde_json::from_str(&json).unwrap()
            }
        };
        let eval = on_new_runtime("refused_eval", Mode::Eval, call(source.clone()));
        let refusal = format!("Error: {}", bootstrap::REFUSED);
        for name in bootstrap::REFUSED_OPS {
            assert_eq!(eval[name], refusal.as_str(), "{name}");
        }
        // the clock reads run in `fgro run`'s runtime, unrefused
        let run = on_new_runtime("refused_run", Mode::Run, call(source));
        assert_eq!(run["op_timer_now"], "ran");
        assert_eq!(run["op_memory_usage"], "ran");
    }

    /// An op that raises a `DOMException` class reaches JS as a
    /// `DOMException` of that name, built by the bootstrap's builders:
    /// `atob`'s op and the geometry ops (which no global reaches) among
    /// them, and every class the bootstrap registers builds one.
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
\tread(() => ops.op_geometry_matrix_to_string(new ops.DOMMatrix([NaN, 0, 0, 1, 0, 0]))),
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
                "InvalidStateError 11: Cannot be serialized with NaN or Infinity values",
                "IndexSizeError 1: m",
                "InvalidCharacterError 5: m",
                "InvalidStateError 11: m",
                "OperationError 0: m",
                "SyntaxError 12: m",
            ]
        );
    }
}
