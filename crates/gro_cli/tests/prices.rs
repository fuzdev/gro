//! The `prices` feature from outside: a default build ignores `GRO_PRICES_OFF`,
//! and a priced build turns off each default it names, eval's and the steps
//! of a command's box (each test turning off one it can see from outside),
//! and refuses a name it doesn't know. The priced build's test runs from
//! `cargo xtask build --verify`, which builds `fgro` with the feature for it.
//! The box's cases need what `verb_box.rs`'s do: Landlock ABI 6, `python3`,
//! and the checkout's `node_modules`.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

mod support;

use std::process::{Command, Output};

use support::{PRICES_OFF_ENV, fgro_command, spawn};

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

/// The op refusals stand behind `determinism: deny` in a build that starts
/// from its snapshot: with `deny` off, the clock, a blob, and a timer each
/// reach an op eval refuses and throw its refusal, and with the refusals
/// off too each runs.
#[cfg(feature = "prices")]
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_op_refusals_stand_behind_deny() {
    const REFUSED: &str = "a host operation eval refuses was called: eval has no timers, no \
                           clock or machine reads, no blobs, no code generation from strings, \
                           no write to stdout, and no import past its loader";
    assert_eq!(
        value(&fgro(&["eval", "refused_ops.js"], Some("deny"))),
        serde_json::json!({ "now": REFUSED, "blob": REFUSED, "timer": REFUSED })
    );
    assert_eq!(
        value(&fgro(&["eval", "refused_ops.js"], Some("deny,refuse_ops"))),
        serde_json::json!({ "now": "ran", "blob": "ran", "timer": "ran" })
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

/// The genfile a priced command's box runs: the probe, under `python3`.
#[cfg(feature = "prices")]
const BOX_GENFILE: &str = "import {spawn} from '@fuzdev/gro/spawn.ts';
export const gen = async () => {
\tawait spawn.program('python3').run(['-I', 'probe.py']);
\treturn '// probed\\n';
};
";

/// The probe, run in the box: a read of a file in the home directory, the
/// network namespace it's in, and the seccomp filters it holds, a line
/// each.
#[cfg(feature = "prices")]
const BOX_PROBE: &str = r#"
import errno, os
try:
    open(os.environ["HOME"] + "/secret.txt").read()
    print("probe: home_secret ok", flush=True)
except OSError as error:
    print("probe: home_secret", errno.errorcode.get(error.errno, error.errno), flush=True)
print("probe: net", os.readlink("/proc/self/ns/net"), flush=True)
for line in open("/proc/self/status").read().splitlines():
    if line.startswith("Seccomp_filters:"):
        print("probe: filters", line.split()[1], flush=True)
"#;

/// A private directory a run gone left in the temporary directory, which
/// the sweep removes: no process has its pid, above any `pid_max`.
#[cfg(feature = "prices")]
const GONE_PRIVATE_DIR: &str = "tmp/gro-4194305-1";

/// What the probe of a boxed `fgro gen`, with `off` as the switches,
/// printed, by name, and whether the gone run's private directory is still
/// there after it.
#[cfg(feature = "prices")]
fn probed_box(off: &str) -> (std::collections::BTreeMap<String, String>, bool) {
    let name = format!("box_{}", off.replace(',', "_"));
    let scratch = support::Scratch::new(
        &name,
        &[
            ("home/secret.txt", "secret"),
            ("tmp/gro-4194305-1/.keep", ""),
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            ("project/src/probe.gen.ts", BOX_GENFILE),
            ("project/probe.py", BOX_PROBE),
        ],
    );
    // the checkout's node_modules, linked whole, where Gro's embedded
    // library finds its peers
    let node_modules = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules")
        .canonicalize()
        .unwrap();
    std::os::unix::fs::symlink(node_modules, scratch.0.join("project/node_modules")).unwrap();
    let mut command = fgro_command(&["gen", "--allow-run=python3"]);
    for name in [
        "GRO_FALLBACK",
        "NODE_ENV",
        "TMP",
        "TEMP",
        "XDG_CONFIG_HOME",
        "GIT_CONFIG_GLOBAL",
    ] {
        command.env_remove(name);
    }
    command
        .current_dir(scratch.0.join("project"))
        .env("HOME", scratch.0.join("home"))
        .env("TMPDIR", scratch.0.join("tmp"));
    let output = spawn(with_switches(command, Some(off)), b"");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{off}: {text}");
    let probed = text
        .lines()
        .filter_map(|line| line.strip_prefix("probe: "))
        .filter_map(|line| line.split_once(' '))
        .map(|(name, what)| (name.to_owned(), what.to_owned()))
        .collect();
    (probed, scratch.0.join(GONE_PRIVATE_DIR).exists())
}

/// A priced build turns off each step of a command's box it names, each
/// seen from inside the box by a program the command runs, every other
/// step taken: `run_landlock` lets it read the home directory, `run_filter`
/// leaves it only the filters its launcher holds, `run_namespace` leaves it
/// in the host's network namespace, and `run_sweep` leaves a gone run's
/// private directory where it was.
#[cfg(feature = "prices")]
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs in its box only from a snapshot build"
)]
fn a_priced_build_turns_off_each_step_of_a_command_s_box() {
    let own_filters: u32 = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("Seccomp_filters:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let own_net = std::fs::read_link("/proc/self/ns/net")
        .unwrap()
        .display()
        .to_string();
    let boxed_filters = (own_filters + 1).to_string();
    let (all, kept) = probed_box("");
    assert_eq!(all["home_secret"], "EACCES", "{all:?}");
    assert_eq!(all["filters"], boxed_filters, "{all:?}");
    // where user namespaces are restricted the run takes the host's network
    // by default too
    let namespaced = all["net"] != own_net;
    assert!(!kept, "{all:?}");

    let (landlock, kept) = probed_box("run_landlock");
    assert_eq!(landlock["home_secret"], "ok", "{landlock:?}");
    assert_eq!(landlock["filters"], boxed_filters, "{landlock:?}");
    assert!(!kept, "{landlock:?}");

    let (filter, kept) = probed_box("run_filter");
    assert_eq!(filter["filters"], own_filters.to_string(), "{filter:?}");
    assert_eq!(filter["home_secret"], "EACCES", "{filter:?}");
    assert!(!kept, "{filter:?}");

    let (namespace, kept) = probed_box("run_namespace");
    assert_eq!(namespace["net"], own_net, "{namespace:?}");
    assert_eq!(namespace["home_secret"], "EACCES", "{namespace:?}");
    assert_eq!(namespace["filters"], boxed_filters, "{namespace:?}");
    assert!(!kept, "{namespace:?}");
    if namespaced {
        assert_ne!(landlock["net"], own_net, "{landlock:?}");
    }

    let (sweep, kept) = probed_box("run_sweep");
    assert!(kept, "{sweep:?}");
    assert_eq!(sweep["home_secret"], "EACCES", "{sweep:?}");
}
