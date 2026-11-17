//! Tests of the runtime's ops: which are registered, pinned, refused, or the host's,
//! and how an op's error reaches JS.

use super::*;

/// The names `ext:core/ops` exports, then those whose export isn't the
/// function `Deno.core.ops` holds under the name.
pub(super) fn op_functions(start: Start) -> Value {
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
pub(super) fn unregistered_reads(start: Start) -> Value {
    let reads: Vec<String> = web_scripts::loadable_reads().into_keys().collect();
    let source = format!(
        "JSON.stringify({}.filter((name) => typeof Deno.core.ops[name] !== 'function'))",
        serde_json::to_string(&reads).unwrap()
    );
    let unregistered = |source: String| move |runtime: &mut JsRuntime| script_json(runtime, source);
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
                Mode::Run => start_runtime(None, console, HostState::default(), loader).unwrap(),
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
