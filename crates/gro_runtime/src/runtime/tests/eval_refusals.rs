//! Tests of what eval refuses, from either start and on each of its engines.

use super::*;

/// The calls of web globals whose own code calls an op eval refuses.
const REFUSED_WEB_CALLS: [&str; 3] = ["performance.now()", "new Blob()", "setTimeout()"];

/// What each refused op did, called through `Deno.core.ops` before the
/// start removes it, whatever it's passed, and what each of
/// [`REFUSED_WEB_CALLS`] did, in an eval runtime and in `fgro run`'s:
/// `ran`, or what it threw. An async op's stub returns its throw as a
/// rejected promise, read once the microtasks have run.
pub(super) fn refused_ops_results(start: Start) -> Value {
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
