//! What eval's code sees: the JIT, code generation from strings, the locale,
//! the console, and the pinned global names.

use serde_json::json;

use crate::{eval, eval_with, fixtures};

/// Jitless by default, so `WebAssembly` is absent; `--jit` brings it.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn webassembly_comes_only_with_the_jit() {
    eval(&["wasm.js"]).assert_value(&json!("undefined"));
    eval(&["wasm.js", "--jit"]).assert_value(&json!("object"));
}

/// Code generation from strings throws an `EvalError` in both modes.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn code_generation_from_strings_throws_in_both_modes() {
    let thrown = json!({
        "eval": "EvalError",
        "indirect_eval": "EvalError",
        "function": "EvalError",
        "async_function": "EvalError",
    });
    eval(&["codegen.js"]).assert_value(&thrown);
    eval(&["codegen.js", "--jit"]).assert_value(&thrown);
}

/// The locale is `en-US` whatever the host's, which `fgro run` would take.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn intl_is_en_us_whatever_the_host_locale() {
    let run = eval_with(&["intl.js"], None, &[("LC_ALL", "de_DE.UTF-8")]);
    run.assert_value(&json!({
        "number_locale": "en-US",
        "date_locale": "en-US",
        "number": "1,234.5",
    }));
}

/// Every console level goes to stderr, uncolored, even with `FORCE_COLOR`,
/// leaving stdout to the envelope.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_console_writes_to_stderr_uncolored() {
    let run = eval_with(&["console.js"], None, &[("FORCE_COLOR", "1")]);
    run.assert_value(&json!("logged"));
    assert_eq!(run.stderr, "1001\n1002\n1003\n1004\n");
}

/// Every global name eval's code sees, pinned: jitless in
/// `global_names_jitless.txt`, and with `--jit` in `global_names.txt`, which
/// adds only `WebAssembly`. Each is `fgro run`'s list, `run/global_names.txt`,
/// without the timers, `Blob`, and `File`, which eval deletes (and jitless,
/// without `WebAssembly`). To accept a change deliberately, rerun it with
/// `UPDATE_FIXTURES=1`, which writes both values over their files, one name
/// a line, instead of comparing them, then checks the two lists against
/// each other and `run`'s: `UPDATE_FIXTURES=1 cargo xtask build --verify`
/// (update `run`'s list first).
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_global_names_are_pinned() {
    let names = |args: &[&str]| -> String {
        let run = eval(args);
        assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
        let names: Vec<&str> = run.envelope["value"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect();
        lines(&names)
    };
    let (jitless_path, jit_path) = (
        fixtures().join("global_names_jitless.txt"),
        fixtures().join("global_names.txt"),
    );
    let (seen_jitless, seen_jit) = (
        names(&["global_names.js"]),
        names(&["global_names.js", "--jit"]),
    );
    if std::env::var_os("UPDATE_FIXTURES").is_some_and(|value| value == "1") {
        std::fs::write(&jitless_path, &seen_jitless).unwrap();
        std::fs::write(&jit_path, &seen_jit).unwrap();
    }
    let jitless = std::fs::read_to_string(&jitless_path).unwrap();
    assert_eq!(
        seen_jitless, jitless,
        "the global names differ from tests/fixtures/eval/global_names_jitless.txt; update it \
         only if the change is deliberate (see this test's doc)"
    );
    let jit = std::fs::read_to_string(&jit_path).unwrap();
    assert_eq!(
        seen_jit, jit,
        "the global names differ from tests/fixtures/eval/global_names.txt; update it only if \
         the change is deliberate (see this test's doc)"
    );
    let without_wasm: Vec<&str> = jit.lines().filter(|name| *name != "WebAssembly").collect();
    assert_eq!(jitless, lines(&without_wasm));
    let deleted = [
        "Blob",
        "File",
        "clearInterval",
        "clearTimeout",
        "setInterval",
        "setTimeout",
    ];
    let run = std::fs::read_to_string(fixtures().join("../run/global_names.txt")).unwrap();
    let run: Vec<&str> = run.lines().filter(|name| !deleted.contains(name)).collect();
    assert_eq!(jit, lines(&run));
}

/// `names`, one a line, each line ended.
fn lines(names: &[&str]) -> String {
    let mut text = names.join("\n");
    text.push('\n');
    text
}
