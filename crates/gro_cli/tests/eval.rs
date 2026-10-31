//! `fgro eval` end to end: the built binary on the modules in
//! `tests/fixtures/eval/`, run from that directory with a relative path, a
//! driver from `tests/fixtures/eval/drivers/` piped to stdin when a test
//! names one.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test's setup fails by panicking"
)]
#![expect(clippy::panic, reason = "a hung child fails its test by panicking")]

use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// How long a child may run before it's killed and its test fails, generous
/// for a debug build on a loaded machine.
const DEADLINE: Duration = Duration::from_secs(60);

/// The variables `fgro run` decides color from, all cleared, so a test sets
/// what it needs; eval reads none of them.
const COLOR_VARS: [&str; 5] = [
    "FORCE_COLOR",
    "NO_COLOR",
    "NODE_DISABLE_COLORS",
    "CLAUDECODE",
    "TERM",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/eval")
}

/// The fixtures directory as a `file:` URL prefix.
fn fixtures_url() -> String {
    url::Url::from_file_path(fixtures().canonicalize().unwrap())
        .unwrap()
        .into()
}

/// An eval's exit code, its envelope parsed from stdout, and its stderr.
struct Eval {
    code: i32,
    envelope: Value,
    stdout: String,
    stderr: String,
}

impl Eval {
    /// The envelope's `error.kind`.
    fn kind(&self) -> &str {
        self.envelope["error"]["kind"].as_str().unwrap_or_default()
    }

    /// The envelope's `error.message`.
    fn message(&self) -> &str {
        self.envelope["error"]["message"]
            .as_str()
            .unwrap_or_default()
    }

    /// Asserts the eval failed with `kind`, exiting `code`, with no value.
    fn assert_failed(&self, kind: &str, code: i32) {
        assert_eq!(
            (self.code, self.kind()),
            (code, kind),
            "{}\n{}",
            self.stdout,
            self.stderr
        );
        assert_eq!(self.envelope["ok"], false);
        assert!(self.envelope.get("value").is_none(), "{}", self.stdout);
    }

    /// Asserts the eval succeeded with `value`.
    fn assert_value(&self, value: &Value) {
        assert_eq!(self.code, 0, "{}\n{}", self.stdout, self.stderr);
        assert_eq!(self.envelope["ok"], true);
        assert_eq!(&self.envelope["value"], value);
        assert!(self.envelope.get("error").is_none());
    }
}

/// An eval's exit code and its output, unparsed.
struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs `fgro eval` with `args` from the fixtures directory, `driver`'s
/// file piped to stdin when given (stdin is empty otherwise), with `env`
/// set and every color variable cleared otherwise.
fn spawn_eval(args: &[&str], driver: Option<&str>, env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fgro"));
    for name in COLOR_VARS {
        command.env_remove(name);
    }
    command
        .arg("eval")
        .args(args)
        .envs(env.iter().copied())
        .current_dir(fixtures())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    {
        // a driver is far smaller than a pipe's buffer, so this can't block
        let mut stdin = child.stdin.take().unwrap();
        if let Some(driver) = driver {
            let source = std::fs::read(fixtures().join("drivers").join(driver)).unwrap();
            stdin.write_all(&source).unwrap();
        }
    }
    // read while the child runs, so an envelope larger than a pipe's buffer
    // can't block it
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > DEADLINE {
            child.kill().unwrap();
            child.wait().unwrap();
            let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
            panic!("killed after {DEADLINE:?}: {command:?}\n{stdout}\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stdout = stdout.join().unwrap();
    let stderr = stderr.join().unwrap();
    Output {
        code: status.code().expect("the eval exits, not killed"),
        stdout,
        stderr,
    }
}

/// Reads `pipe` to its end on a thread of its own, as UTF-8.
fn drain(mut pipe: impl Read + Send + 'static) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).unwrap();
        String::from_utf8(bytes).unwrap()
    })
}

/// [`spawn_eval`], its envelope parsed.
fn eval_with(args: &[&str], driver: Option<&str>, env: &[(&str, &str)]) -> Eval {
    let Output {
        code,
        stdout,
        stderr,
    } = spawn_eval(args, driver, env);
    // exactly one line of JSON, or nothing for a usage error
    let envelope = if stdout.is_empty() {
        Value::Null
    } else {
        assert!(
            stdout.ends_with('\n') && stdout.matches('\n').count() == 1,
            "{stdout}"
        );
        serde_json::from_str(&stdout).unwrap()
    };
    Eval {
        code,
        envelope,
        stdout,
        stderr,
    }
}

fn eval(args: &[&str]) -> Eval {
    eval_with(args, None, &[])
}

fn driven(module: &str, driver: &str) -> Eval {
    eval_with(&[module, "--driver", "-"], Some(driver), &[])
}

#[test]
fn the_default_export_is_the_value_with_no_driver() {
    let run = eval(&["value.js"]);
    run.assert_value(&json!({
        "2": "integer keys come first",
        "b": 1,
        "a": [1, "two", null, true, false],
        "zero": 0,
        "nested": {"x": {"y": []}},
        "text": "quote \" backslash \\ newline \n tab \t é 😀",
    }));
    // the value as written: key order kept, `-0` as `0`, compact
    assert!(
        run.stdout.contains(
            r#""value":{"2":"integer keys come first","b":1,"a":[1,"two",null,true,false],"zero":0,"#
        ),
        "{}",
        run.stdout
    );
    let envelope = &run.envelope;
    assert_eq!(envelope["eval_protocol"], 1);
    let id = fixtures().canonicalize().unwrap().join("value.js");
    assert_eq!(
        envelope["entry"],
        json!({"id": id.to_str().unwrap(), "export": "default", "driver_blake3": null})
    );
    assert_eq!(envelope["access"], json!({"dynamic": []}));
    let runtime = &envelope["runtime"];
    assert_eq!(runtime["fgro_version"], env!("CARGO_PKG_VERSION"));
    assert!(runtime["v8"].as_str().is_some_and(|v8| !v8.is_empty()));
    assert!(runtime["deno_core"].is_string());
    assert!(runtime["tsv_rev"].is_string());
    // the envelope's keys in their order
    let keys: Vec<&str> = envelope
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["eval_protocol", "ok", "runtime", "entry", "value", "access"]
    );
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

#[test]
fn export_names_another_export_and_typescript_evaluates() {
    let run = eval(&["value.js", "--export", "named"]);
    run.assert_value(&json!(["the", "named", "export"]));
    assert_eq!(run.envelope["entry"]["export"], "named");
    eval(&["typed.ts"]).assert_value(&json!({"x": 1, "y": 2}));
}

/// The driver is called with the module's namespace, and its result is the
/// value; the envelope names the driver by the hash of its bytes.
#[test]
fn a_driver_is_called_with_the_namespace() {
    let run = driven("driven.js", "call.js");
    run.assert_value(&json!({"exports": ["default", "other"], "value": 42}));
    let source = std::fs::read(fixtures().join("drivers/call.js")).unwrap();
    assert_eq!(
        run.envelope["entry"]["driver_blake3"],
        blake3::hash(&source).to_hex().as_str()
    );
    assert_eq!(run.envelope["entry"]["export"], Value::Null);
}

/// The driver evaluates before the config: what it captures at its top
/// level is the realm's original, which the config then patches.
#[test]
fn the_driver_evaluates_before_the_config() {
    driven("patch.js", "primordials.js")
        .assert_value(&json!({"patched_since": true, "captured": "[1]"}));
}

/// A builder call the config makes after its default export returned, and
/// catches, still fails the eval through the driver's `after_drain`.
#[test]
fn the_late_call_guard_fails_a_caught_late_call() {
    let run = driven("builder_config.js", "builder.js");
    run.assert_failed("late_rejection", 1);
    assert!(
        run.message().starts_with(
            "Uncaught Error: late builder calls: late\n    at after_drain (gro:driver:"
        ),
        "{}",
        run.message()
    );
}

#[test]
fn an_import_in_the_driver_is_refused() {
    let run = driven("value.js", "import.js");
    run.assert_failed("resolve", 1);
    assert_eq!(
        run.message(),
        "cannot import '../value.js' from the driver: a stdin driver is import-free, so it can \
         neither import nor resolve a module"
    );
    assert_eq!(run.envelope["error"]["specifier"], "../value.js");
    assert_eq!(run.envelope["error"]["importer"], "gro:driver");
    // `import.meta.resolve` too, a throw in the driver's call
    let run = driven("value.js", "meta_resolve.js");
    run.assert_failed("module_threw", 1);
    assert!(
        run.message().contains("a stdin driver is import-free"),
        "{}",
        run.message()
    );
}

/// Every `import()` is refused, whatever it names, catchably, and listed in
/// `access.dynamic`; uncaught, the refusal fails the evaluation.
#[test]
fn every_dynamic_import_is_refused() {
    let run = eval(&["dynamic.js"]);
    let url = format!("{}/dynamic.js", fixtures_url());
    let refusal = |specifier: &str| {
        json!({
            "type_error": true,
            "message": format!(
                "cannot import '{specifier}' from {url}: import() is refused in eval, whatever \
                 it names. Fix: import the module with a static import"
            ),
        })
    };
    run.assert_value(&json!([refusal("./value.js"), refusal("node:fs")]));
    assert_eq!(
        run.envelope["access"],
        json!({"dynamic": [
            {"specifier": "./value.js", "importer": url},
            {"specifier": "node:fs", "importer": url},
        ]})
    );
    let run = eval(&["dynamic_uncaught.js"]);
    run.assert_failed("module_threw", 1);
    assert!(
        run.message()
            .starts_with("Uncaught (in promise) TypeError: cannot import './value.js'"),
        "{}",
        run.message()
    );
    assert_eq!(
        run.envelope["access"]["dynamic"][0]["specifier"],
        "./value.js"
    );
}

#[test]
fn an_entry_that_does_not_resolve_is_a_resolve_exiting_2() {
    let run = eval(&["missing.js"]);
    run.assert_failed("resolve", 2);
    assert_eq!(run.envelope["entry"]["id"], "missing.js");
    assert_eq!(run.envelope["error"]["specifier"], "missing.js");
    assert!(run.message().contains("no such file"), "{}", run.message());
}

#[test]
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
fn a_value_that_is_not_data_is_serialize() {
    let run = eval(&["function.js"]);
    run.assert_failed("serialize", 1);
    assert_eq!(run.envelope["error"]["at"], "$.a[0].f");
    assert_eq!(
        run.message(),
        "$.a[0].f is a function; the value is data, so call it or write what it returns"
    );
}

/// A timer's error after the value was taken fails the eval with no value.
#[test]
fn an_error_in_the_drain_is_late_rejection() {
    let run = eval(&["late.js"]);
    run.assert_failed("late_rejection", 1);
    assert!(
        run.message().starts_with(&format!(
            "Uncaught Error: late\n    at {}/late.js:6:8",
            fixtures_url()
        )),
        "{}",
        run.message()
    );
}

/// Jitless by default, so `WebAssembly` is absent; `--jit` brings it.
#[test]
fn webassembly_comes_only_with_the_jit() {
    eval(&["wasm.js"]).assert_value(&json!("undefined"));
    eval(&["wasm.js", "--jit"]).assert_value(&json!("object"));
}

/// Code generation from strings throws an `EvalError` in both modes.
#[test]
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
fn the_console_writes_to_stderr_uncolored() {
    let run = eval_with(&["console.js"], None, &[("FORCE_COLOR", "1")]);
    run.assert_value(&json!("logged"));
    assert_eq!(run.stderr, "1001\n1002\n1003\n1004\n");
}

/// Every global name eval's code sees, jitless, pinned in
/// `global_names_jitless.txt`: `fgro run`'s list, `run/global_names.txt`,
/// without `WebAssembly`. With `--jit` the list is `fgro run`'s. To accept a
/// change deliberately, write the fixture's value over the file, one name a
/// line, from the fixtures directory:
/// `../../../../../target/debug/fgro eval global_names.js | python3 -c "import json, sys; print('\\n'.join(json.load(sys.stdin)['value']))" > global_names_jitless.txt`.
#[test]
fn the_global_names_are_pinned_jitless() {
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
    let jitless = std::fs::read_to_string(fixtures().join("global_names_jitless.txt")).unwrap();
    assert_eq!(
        names(&["global_names.js"]),
        jitless,
        "the global names differ from tests/fixtures/eval/global_names_jitless.txt; update it \
         only if the change is deliberate (see this test's doc)"
    );
    let run = std::fs::read_to_string(fixtures().join("../run/global_names.txt")).unwrap();
    assert_eq!(names(&["global_names.js", "--jit"]), run);
    let without_wasm: Vec<&str> = run.lines().filter(|name| *name != "WebAssembly").collect();
    let without_wasm = lines(&without_wasm);
    assert_eq!(jitless, without_wasm);
}

/// `names`, one a line, each line ended.
fn lines(names: &[&str]) -> String {
    let mut text = names.join("\n");
    text.push('\n');
    text
}

/// A usage error prints to stderr with no envelope and exits 2.
#[test]
fn usage_errors_print_no_envelope() {
    for args in [
        &["value.js", "--driver", "drivers/call.js"][..],
        &["value.js", "--driver", "-", "--export", "named"],
        &["value.js", "--graph"],
        &["value.js", "--timeout", "1"],
        &["value.js", "other.js"],
        &[],
    ] {
        let run = eval(args);
        assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty(), "{args:?}: {}", run.stdout);
        assert!(!run.stderr.is_empty(), "{args:?}");
    }
}

/// A value inside the depth cap serializes even when the walk allocates
/// enough near its deepest level to start a garbage collection.
#[test]
fn a_deep_value_serializes_through_a_garbage_collection() {
    // nested past what `serde_json` parses, so checked as text
    let run = spawn_eval(&["deep_gc.js"], None, &[]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(run.stdout.starts_with(r#"{"eval_protocol":1,"ok":true,"#));
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

/// The config can't import the driver: `gro:driver` resolves only as the
/// root of the driver's own load.
#[test]
fn the_config_cannot_import_the_driver() {
    let run = driven("import_driver.js", "call.js");
    run.assert_failed("resolve", 1);
    assert_eq!(run.envelope["error"]["specifier"], "gro:driver");
}
