//! The driver protocol: the default export or a named one, the driver called with
//! the namespace, its import-free rule, refused `import()`, and usage errors.

use serde_json::{Value, json};

use crate::{driven, eval, fixtures, fixtures_url};

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
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
    assert_eq!(envelope["eval_protocol"], 2);
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
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn export_names_another_export_and_typescript_evaluates() {
    let run = eval(&["value.js", "--export", "named"]);
    run.assert_value(&json!(["the", "named", "export"]));
    assert_eq!(run.envelope["entry"]["export"], "named");
    eval(&["typed.ts"]).assert_value(&json!({"x": 1, "y": 2}));
}

/// The driver is called with the module's namespace, and its result is the
/// value; the envelope names the driver by the hash of its bytes.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
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
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_driver_evaluates_before_the_config() {
    driven("patch.js", "primordials.js")
        .assert_value(&json!({"patched_since": true, "captured": "[1]"}));
}

/// A builder call the config makes after its default export returned, and
/// catches, still fails the eval through the driver's `after_drain`.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
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
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
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
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
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

/// A usage error prints to stderr with no envelope and exits 2.
#[test]
fn usage_errors_print_no_envelope() {
    for args in [
        &["value.js", "--driver", "drivers/call.js"][..],
        &["value.js", "--driver", "-", "--export", "named"],
        &["value.js", "--unknown"],
        &["value.js", "--timeout", "0"],
        &["value.js", "--timeout", "-1"],
        &["value.js", "other.js"],
        &[],
    ] {
        let run = eval(args);
        assert_eq!(run.code, 2, "{args:?}: {}", run.stderr);
        assert!(run.stdout.is_empty(), "{args:?}: {}", run.stdout);
        assert!(!run.stderr.is_empty(), "{args:?}");
    }
}

/// The config can't import the driver: `gro:driver` resolves only as the
/// root of the driver's own load.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_config_cannot_import_the_driver() {
    let run = driven("import_driver.js", "call.js");
    run.assert_failed("resolve", 1);
    assert_eq!(run.envelope["error"]["specifier"], "gro:driver");
}
