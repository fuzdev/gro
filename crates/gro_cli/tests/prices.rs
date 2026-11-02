//! The `prices` feature from outside: a default build ignores
//! `GRO_PRICES_OFF`, and a priced build turns off each default it names
//! (each test turning off one it can see from outside) and refuses a name it
//! doesn't know. The priced build's test runs from
//! `cargo xtask build --verify`, which builds `fgro` with the feature for it.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

mod support;

use std::process::{Command, Output};

use support::{fgro_command, spawn};

/// The variable naming the defaults to turn off, `gro_runtime::prices`'s.
const PRICES_OFF_ENV: &str = "GRO_PRICES_OFF";

/// Runs `fgro` with `args` from the eval fixtures, with `off` as the
/// switches when given.
fn fgro(args: &[&str], off: Option<&str>) -> Output {
    fgro_in(args, off, &[])
}

/// Runs `fgro` as [`fgro`] does, with `env` set over its defaults.
fn fgro_in(args: &[&str], off: Option<&str>, env: &[(&str, &str)]) -> Output {
    let mut command = fgro_command(args);
    command
        .env_remove("LC_ALL")
        .envs(env.iter().copied())
        .current_dir(support::fixtures("eval"));
    spawn(with_switches(command, off), b"")
}

/// `command` with `off` as the switches when given, and none otherwise.
fn with_switches(mut command: Command, off: Option<&str>) -> Command {
    command.env_remove(PRICES_OFF_ENV);
    if let Some(off) = off {
        command.env(PRICES_OFF_ENV, off);
    }
    command
}

fn version(off: Option<&str>) -> String {
    let output = fgro(&["--version"], off);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[cfg(not(feature = "prices"))]
#[test]
fn a_default_build_ignores_the_switches() {
    let line = version(Some("not_a_default"));
    assert!(!line.contains(", prices"), "{line}");
}

/// The envelope's `value`, after checking the eval succeeded.
#[cfg(feature = "prices")]
fn value(output: &Output) -> serde_json::Value {
    assert!(output.status.success(), "{output:?}");
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], true, "{envelope}");
    envelope["value"].clone()
}

#[cfg(feature = "prices")]
#[test]
fn a_priced_build_says_so_and_refuses_an_unknown_switch() {
    assert!(version(None).contains(", prices)"), "{}", version(None));
    let output = fgro(&["--version"], Some("deny,jitless"));
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("`jitless`"),
        "{output:?}"
    );
}

#[cfg(feature = "prices")]
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_priced_build_turns_off_what_it_names() {
    use std::os::unix::fs::PermissionsExt as _;

    // every default on: the clock is refused, and code generation throws
    let output = fgro(&["eval", "clock.js"], None);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        value(&fgro(&["eval", "codegen.js"], Some(""))),
        serde_json::json!({
            "eval": "EvalError",
            "indirect_eval": "EvalError",
            "function": "EvalError",
            "async_function": "EvalError"
        })
    );
    // each turned off alone
    assert_eq!(value(&fgro(&["eval", "clock.js"], Some("deny"))), "number");
    assert_eq!(
        value(&fgro(&["eval", "codegen.js"], Some("codegen"))),
        serde_json::json!({
            "eval": "generated",
            "indirect_eval": "generated",
            "function": "generated",
            "async_function": "generated"
        })
    );
    // the time zone: refused unless the check is off
    let tokyo = [("TZ", "Asia/Tokyo")];
    let output = fgro_in(&["eval", "value.js"], None, &tokyo);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"environment\""),
        "{output:?}"
    );
    assert_eq!(
        value(&fgro_in(&["eval", "value.js"], Some("utc_check"), &tokyo))["b"],
        1
    );
    // the timeout: a millisecond stops the work, unless no watchdog keeps it
    let output = fgro(&["eval", "--timeout", "1", "busy.js"], None);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"timeout\""),
        "{output:?}"
    );
    assert!(
        value(&fgro(
            &["eval", "--timeout", "1", "busy.js"],
            Some("watchdog")
        ))
        .is_number()
    );
    // the memory cap: an array buffer its size stops the run, unless the
    // cap is off, when it's made (its pages untouched, so not resident)
    let output = fgro(&["eval", "memory_external.js"], None);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"memory\""),
        "{output:?}"
    );
    assert_eq!(
        value(&fgro(&["eval", "memory_external.js"], Some("data_limit"))),
        2_147_483_648_u64
    );
    // the locale: en-US whatever the host's, unless the pin is off and the
    // environment kept, so ICU reads the host's; then the time zone check,
    // which reads the epoch's date string, sees the zone named in German,
    // so it's off too
    let german = [("LC_ALL", "de_DE.UTF-8")];
    assert_eq!(
        value(&fgro_in(&["eval", "intl.js"], None, &german))["number_locale"],
        "en-US"
    );
    // the pin off alone, the cleared environment still keeps the host's
    // locale from ICU
    assert_eq!(
        value(&fgro_in(
            &["eval", "intl.js"],
            Some("locale,utc_check"),
            &german
        ))["number_locale"],
        "en-US"
    );
    assert_eq!(
        value(&fgro_in(
            &["eval", "intl.js"],
            Some("locale,utc_check,clear_env"),
            &german
        ))["number_locale"],
        "de-DE"
    );
    // the box: a package linked from a subdirectory's `node_modules`, which
    // the box's link closure doesn't follow, is refused by the loader's own
    // rule, then, with that off, by Landlock, and loads with both off
    let scratch = support::Scratch::in_system_temp(
        "box",
        &[
            ("p/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "p/sub/main.js",
                "import value from 'pkg';\nexport default value;\n",
            ),
            (
                "outside/pkg/package.json",
                r#"{"name": "pkg", "type": "module", "exports": "./index.js"}"#,
            ),
            ("outside/pkg/index.js", "export default 'outside';\n"),
        ],
    );
    let project = scratch.0.join("p");
    let outside = scratch.0.join("outside/pkg");
    // not world-writable, as the scratch's own aren't, which would end the
    // walks up and refuse the package
    std::fs::create_dir(project.join("sub/node_modules")).unwrap();
    std::fs::set_permissions(
        project.join("sub/node_modules"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, project.join("sub/node_modules/pkg")).unwrap();
    let outside_kind = |off: &str| {
        let mut command = fgro_command(&["eval", "sub/main.js"]);
        command.current_dir(&project);
        let output = spawn(with_switches(command, Some(off)), b"");
        let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if envelope["ok"] == true {
            envelope["value"].clone()
        } else {
            envelope["error"]["kind"].clone()
        }
    };
    let kinds = [
        outside_kind(""),
        outside_kind("box_rule"),
        outside_kind("box_rule,landlock"),
    ];
    // the loader's rule names the breach; Landlock's refused read fails the
    // resolution, which reads the package's `package.json`, as a failed read
    assert_eq!(kinds, ["internal", "internal", "outside"]);
    // the box gone, the eval runs the same
    assert_eq!(
        value(&fgro(&["eval", "clock.js"], Some("deny,landlock,seccomp"))),
        "number"
    );
}

/// Every thread eval starts sets its own stack, so a `RUST_MIN_STACK` too
/// large to reserve changes nothing even with the environment kept, the
/// clear off: the threads all start, the watchdog among them, and the value
/// is written.
#[cfg(feature = "prices")]
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn every_thread_sets_its_own_stack() {
    let output = fgro_in(
        &["eval", "value.js"],
        Some("clear_env"),
        &[("RUST_MIN_STACK", "1099511627776")],
    );
    assert_eq!(value(&output)["b"], 1);
}
