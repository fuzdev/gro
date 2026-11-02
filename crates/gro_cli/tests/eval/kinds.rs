//! The failure kinds of an envelope: `link`, `module_threw`, `export_missing`,
//! `serialize`, late work, and the hints that name a fix.

use serde_json::json;

use crate::{driven, eval, fixtures_url, spawn_eval};

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_static_import_that_does_not_resolve_is_a_resolve_exiting_1() {
    let run = eval(&["import_missing.js"]);
    run.assert_failed("resolve", 1);
    assert_eq!(run.envelope["error"]["specifier"], "./missing.js");
    assert_eq!(
        run.envelope["error"]["importer"],
        format!("{}/import_missing.js", fixtures_url())
    );
}

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_graph_that_does_not_link_is_a_link() {
    let run = eval(&["syntax_error.mjs"]);
    run.assert_failed("link", 1);
    assert!(
        run.message().starts_with("Uncaught SyntaxError: "),
        "{}",
        run.message()
    );
    let run = eval(&["enum.ts"]);
    run.assert_failed("link", 1);
    assert!(
        run.message().starts_with("cannot strip an enum."),
        "{}",
        run.message()
    );
    let url = format!("{}/enum.ts", fixtures_url());
    assert_eq!(run.envelope["error"]["specifier"], url);
    assert_eq!(run.envelope["error"]["at"], format!("{url}:1:1"));
}

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_throw_is_module_threw() {
    let run = eval(&["throw.js"]);
    run.assert_failed("module_threw", 1);
    assert_eq!(
        run.message(),
        format!(
            "Uncaught (in promise) Error: boom\n    at {}/throw.js:3:7",
            fixtures_url()
        )
    );
    // the driver's call throwing
    let run = driven("value.js", "throws.js");
    run.assert_failed("module_threw", 1);
    assert!(
        run.message()
            .starts_with("Uncaught Error: driver boom\n    at default (gro:driver:2:8)"),
        "{}",
        run.message()
    );
}

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_missing_export_is_export_missing() {
    let run = eval(&["value.js", "--export", "nope"]);
    run.assert_failed("export_missing", 1);
    assert_eq!(
        run.message(),
        "the module has no export named `nope` to evaluate"
    );
    let run = driven("value.js", "no_default.js");
    run.assert_failed("export_missing", 1);
    // an export that holds `undefined` is present, and doesn't serialize
    let run = eval(&["value.js", "--export", "missing_value"]);
    run.assert_failed("serialize", 1);
    assert_eq!(run.envelope["error"]["at"], "$");
}

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_value_that_is_not_data_is_serialize() {
    let run = eval(&["function.js"]);
    run.assert_failed("serialize", 1);
    assert_eq!(run.envelope["error"]["at"], "$.a[0].f");
    assert_eq!(
        run.message(),
        "$.a[0].f is a function; the value is data, so call it or write what it returns"
    );
}

/// With no timers, work left after the evaluation is promise jobs, which all
/// run before the value is taken: an error in one, however long the chain
/// before it, fails the evaluation.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn late_work_runs_before_the_value_is_taken() {
    let run = eval(&["late.js"]);
    run.assert_failed("module_threw", 1);
    assert_eq!(
        run.message(),
        format!(
            "Uncaught (in promise) Error: late\n    at {}/late.js:7:8",
            fixtures_url()
        )
    );
}

/// A value inside the depth cap serializes even when the walk allocates
/// enough near its deepest level to start a garbage collection.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_deep_value_serializes_through_a_garbage_collection() {
    // nested past what `serde_json` parses, so checked as text
    let run = spawn_eval(&["deep_gc.js"], None, &[]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(run.stdout.starts_with(r#"{"eval_protocol":2,"ok":true,"#));
    let value = format!(
        "{}[{}]{}",
        "[".repeat(995),
        vec!["{}"; 100_000].join(","),
        "]".repeat(995)
    );
    assert!(
        run.stdout
            .contains(&format!(r#","value":{value},"access":"#))
    );
}

/// `WebAssembly` undefined in a jitless eval hints at `--jit`, and an
/// uncaught `EvalError` that code generation from strings is off.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_hints_name_the_fix() {
    let run = eval(&["wasm_global.js"]);
    run.assert_failed("module_threw", 1);
    assert_eq!(
        run.envelope["error"]["hint"],
        "eval runs jitless unless asked, and WebAssembly comes only with V8's JIT: pass --jit to \
         turn it on"
    );
    eval(&["wasm_global.js", "--jit"]).assert_value(&json!("function"));
    let run = eval(&["codegen_uncaught.js"]);
    run.assert_failed("module_threw", 1);
    assert!(
        run.message()
            .starts_with("Uncaught (in promise) EvalError: Code generation from strings"),
        "{}",
        run.message()
    );
    assert_eq!(
        run.envelope["error"]["hint"],
        "eval turns code generation from strings off, with or without --jit: eval(), new \
         Function(), and the other function constructors throw an EvalError; write the code in \
         the module instead"
    );
    // keyed on the class, never the message: an error that only reads like
    // the refusal gets none
    let run = eval(&["codegen_text.js"]);
    run.assert_failed("module_threw", 1);
    assert!(
        run.envelope["error"].get("hint").is_none(),
        "{}",
        run.stdout
    );
    // no hint where there's nothing to add
    let run = eval(&["throw.js"]);
    assert!(
        run.envelope["error"].get("hint").is_none(),
        "{}",
        run.stdout
    );
}
