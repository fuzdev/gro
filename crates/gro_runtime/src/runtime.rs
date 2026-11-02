//! Building a runtime: its mode, the `deno_core` runtime over a loader, and the
//! bootstrap's per-run steps.

use std::rc::Rc;

use deno_core::JsRuntime;

use crate::loader::Loader;
use crate::phases::{self, Phase};
use crate::prices::{self, EvalDefault};
use crate::report::{report, report_js};
use crate::snapshot::StartupSnapshot;
use crate::{ConsoleOptions, bootstrap, eval};

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

/// Starts a runtime over `loader` from `snapshot` when there is one, built
/// as `mode` says, and runs the bootstrap's per-run steps with the console
/// as `console` says, returning the report of what failed when either
/// fails. Must run inside a tokio runtime, which `deno_core` drives its
/// event loop with.
pub fn start_runtime(
    snapshot: Option<StartupSnapshot>,
    console: ConsoleOptions,
    loader: Rc<Loader>,
    mode: Mode,
) -> Result<JsRuntime, String> {
    let mut runtime = new_runtime(snapshot, loader, mode)?;
    phases::mark(Phase::Runtime);
    // before the bootstrap's start reads them
    let state = runtime.op_state();
    state.borrow_mut().put(console);
    state.borrow_mut().put(bootstrap::Deny(
        mode == Mode::Eval && !prices::off(EvalDefault::Deny),
    ));
    drop(state);
    bootstrap::start(&mut runtime).map_err(|error| report_js(&error))?;
    phases::mark(Phase::Bootstrap);
    Ok(runtime)
}

/// A runtime over `loader` from `snapshot` when there is one, built as
/// `mode` says, before the bootstrap's per-run steps.
fn new_runtime(
    snapshot: Option<StartupSnapshot>,
    loader: Rc<Loader>,
    mode: Mode,
) -> Result<JsRuntime, String> {
    let create_params = match mode {
        Mode::Run => None,
        Mode::Eval if prices::off(EvalDefault::HeapCap) => {
            Some(deno_core::v8::Isolate::create_params().allow_atomics_wait(false))
        }
        Mode::Eval => Some(
            deno_core::v8::Isolate::create_params()
                .heap_limits(0, eval::HEAP_LIMIT)
                .allow_atomics_wait(false),
        ),
    };
    JsRuntime::try_new(deno_core::RuntimeOptions {
        module_loader: Some(loader),
        // the same list the snapshot was made from, in the same order
        extensions: bootstrap::extensions(
            mode == Mode::Eval && !prices::off(EvalDefault::RefuseOps),
        ),
        startup_snapshot: snapshot.map(|snapshot| snapshot.bytes),
        residual_lazy_js_sources: snapshot.map_or(&[], |snapshot| snapshot.residual_lazy_js),
        create_params,
        ..Default::default()
    })
    .map_err(|error| report(&error))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::STACK_SIZE;
    use crate::confine::Confinement;
    use crate::engine::{EngineOptions, init_engine};
    use crate::loader::graph::Recording;
    use crate::resolver::test_tree::TestDir;

    /// Runs `test` on a runtime built as `mode` says, from source, before
    /// the bootstrap's per-run steps, so `Deno.core.ops` is still there, on
    /// a thread reserving [`STACK_SIZE`].
    fn on_new_runtime<T: Send + 'static>(
        name: &str,
        mode: Mode,
        test: impl FnOnce(&mut JsRuntime) -> T + Send + 'static,
    ) -> T {
        use std::rc::Rc;

        use crate::loader::Loader;
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
                let mut runtime = new_runtime(None, loader, mode).unwrap();
                test(&mut runtime)
            })
            .unwrap()
            .join()
            .unwrap()
    }

    const EVAL: Mode = Mode::Eval;

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
        let eval = on_new_runtime("op_names_eval", EVAL, names);
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
                let json = deno_core::v8::Local::new(scope, value).to_rust_string_lossy(scope);
                serde_json::from_str(&json).unwrap()
            }
        };
        let eval = on_new_runtime("refused_eval", EVAL, call(source.clone()));
        let refusal = format!("Error: {}", bootstrap::REFUSED);
        for name in bootstrap::REFUSED_OPS {
            assert_eq!(eval[name], refusal.as_str(), "{name}");
        }
        // the clock reads run in `fgro run`'s runtime, unrefused
        let run = on_new_runtime("refused_run", Mode::Run, call(source));
        assert_eq!(run["op_timer_now"], "ran");
        assert_eq!(run["op_memory_usage"], "ran");
    }
}
