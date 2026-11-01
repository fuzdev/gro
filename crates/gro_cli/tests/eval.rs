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

/// The most resident memory a child may reach, in KiB, before it's killed
/// and its test fails: 1.5 GiB, twice the most a heap test's child reaches,
/// so a cap that stops holding fails its test rather than taking the
/// machine's memory.
const MAX_RSS_KIB: u64 = 3 << 19;

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
/// set, `TZ=UTC` unless `env` sets it, and every color variable cleared
/// otherwise.
fn spawn_eval(args: &[&str], driver: Option<&str>, env: &[(&str, &str)]) -> Output {
    spawn_command("eval", args, driver, env)
}

/// [`spawn_eval`] with another subcommand of eval's arguments.
fn spawn_command(
    subcommand: &str,
    args: &[&str],
    driver: Option<&str>,
    env: &[(&str, &str)],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fgro"));
    for name in COLOR_VARS {
        command.env_remove(name);
    }
    command
        .arg(subcommand)
        .args(args)
        // eval refuses any other time zone, and ICU reads the host's
        .env("TZ", "UTC")
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
        let rss = peak_rss_kib(child.id());
        if start.elapsed() > DEADLINE || rss > MAX_RSS_KIB {
            child.kill().unwrap();
            child.wait().unwrap();
            let (stdout, stderr) = (stdout.join().unwrap(), stderr.join().unwrap());
            panic!(
                "killed after {:?} at {rss} KiB resident: {command:?}\n{stdout}\n{stderr}",
                start.elapsed()
            );
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

/// The most resident memory the process `pid` has reached, in KiB, as
/// Linux reports it (`VmHWM`), or 0 when it can't be read.
fn peak_rss_kib(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
            line.split_whitespace().nth(1)?.parse().ok()
        })
        .unwrap_or(0)
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

#[test]
fn an_entry_that_does_not_resolve_is_a_resolve_exiting_2() {
    let run = eval(&["missing.js"]);
    run.assert_failed("resolve", 2);
    assert_eq!(run.envelope["entry"]["id"], "missing.js");
    assert_eq!(run.envelope["error"]["specifier"], "missing.js");
    assert!(run.message().contains("no such file"), "{}", run.message());
}

/// Files written under a fresh directory in the target's temporary
/// directory, which no world-writable directory holds, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("eval_{}_{name}", std::process::id()));
        for (file, content) in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        Self(dir.canonicalize().unwrap())
    }

    fn path(&self, file: &str) -> String {
        self.0.join(file).display().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Eval evaluates a project's own module: one inside `node_modules`, or one
/// with no project root above it, is refused as a `resolve` exiting 2,
/// named by its canonical path. The system's temporary directory is
/// world-writable, so the walk from a directory in it stops there.
#[test]
fn an_entry_no_project_owns_is_a_resolve_exiting_2() {
    let scratch = Scratch::new(
        "unowned",
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/node_modules/pkg/package.json",
                r#"{"name": "pkg", "exports": "./i.mjs"}"#,
            ),
            ("project/node_modules/pkg/i.mjs", "export default 1;\n"),
        ],
    );
    let entry = scratch.path("project/node_modules/pkg/i.mjs");
    let run = eval(&[&entry]);
    run.assert_failed("resolve", 2);
    assert_eq!(run.envelope["entry"]["id"], entry.as_str());
    assert!(
        run.message().contains("it is inside node_modules"),
        "{}",
        run.message()
    );

    let loose = std::env::temp_dir().join(format!("gro_cli_eval_{}_loose", std::process::id()));
    std::fs::create_dir_all(&loose).unwrap();
    std::fs::write(loose.join("main.mjs"), "export default 1;\n").unwrap();
    let entry = loose.join("main.mjs").display().to_string();
    let run = eval(&[&entry]);
    std::fs::remove_dir_all(&loose).unwrap();
    run.assert_failed("resolve", 2);
    assert!(
        run.message().contains("it has no project root"),
        "{}",
        run.message()
    );
}

/// A build with no snapshot refuses to evaluate once the entry resolves,
/// before it confines its process, so the same on every host: its runtime
/// would read its extensions' JS from the build tree, outside the box.
#[cfg(not(feature = "snapshot"))]
#[test]
fn a_build_with_no_snapshot_refuses_to_evaluate() {
    let run = eval(&["value.js"]);
    run.assert_failed("internal", 1);
    assert!(
        run.message()
            .starts_with("a confined eval starts only from a startup snapshot"),
        "{}",
        run.message()
    );
    assert_eq!(
        run.envelope["error"]["hint"],
        "build fgro with its snapshot: cargo xtask build"
    );
}

/// Every task of a confined eval's process, its main thread, the thread
/// that evaluates, V8's workers, and the watchdog, has `no_new_privs` and
/// seccomp's filter mode, read from outside while the driver runs.
#[cfg(all(feature = "snapshot", target_os = "linux"))]
#[test]
fn every_task_of_a_confined_eval_is_boxed() {
    use std::io::BufRead as _;

    let mut command = Command::new(env!("CARGO_BIN_EXE_fgro"));
    command
        .args(["eval", "value.js", "--driver", "-", "--timeout", "60000"])
        .env("TZ", "UTC")
        .current_dir(fixtures())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(fixtures().join("drivers/spin.js")).unwrap())
        .unwrap();
    // the driver says it runs once every thread the eval starts with is up
    let mut stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let tasks: Vec<(String, String)> = std::fs::read_dir(format!("/proc/{}/task", child.id()))
        .unwrap()
        .map(|task| {
            let path = task.unwrap().path().join("status");
            (
                path.display().to_string(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect();
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(line, "ready\n", "the eval didn't start: {line}");
    // the main thread, the thread that evaluates, and V8's workers at least
    assert!(tasks.len() >= 3, "{tasks:?}");
    for (path, status) in &tasks {
        let field = |name: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .map(str::trim)
        };
        assert_eq!(field("NoNewPrivs:"), Some("1"), "{path}\n{status}");
        assert_eq!(field("Seccomp:"), Some("2"), "{path}\n{status}");
        assert_eq!(field("Seccomp_filters:"), Some("2"), "{path}\n{status}");
    }
}

/// A descriptor the eval inherits above stderr, a file outside the box
/// opened by the shell that starts it, is closed before the box goes up, so
/// the process no longer holds it while the driver runs; the eval still runs.
#[cfg(all(feature = "snapshot", target_os = "linux"))]
#[test]
fn an_inherited_descriptor_is_closed() {
    use std::io::BufRead as _;

    let scratch = Scratch::new("inherited", &[("outside/secret.txt", "outside\n")]);
    let outside = scratch.0.join("outside/secret.txt");
    // bash leaves the descriptor open across its exec of fgro, as a shell
    // that leaks one into every command does
    let mut command = Command::new("bash");
    command
        .args(["-c", r#"exec 7<"$1"; shift; exec "$@""#, "bash"])
        .arg(&outside)
        .arg(env!("CARGO_BIN_EXE_fgro"))
        .args(["eval", "value.js", "--driver", "-", "--timeout", "60000"])
        .env("TZ", "UTC")
        .current_dir(fixtures())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(fixtures().join("drivers/spin.js")).unwrap())
        .unwrap();
    let mut stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let held: Vec<PathBuf> = std::fs::read_dir(format!("/proc/{}/fd", child.id()))
        .unwrap()
        .filter_map(|fd| std::fs::read_link(fd.unwrap().path()).ok())
        .collect();
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(line, "ready\n", "the eval didn't start: {line}");
    assert!(!held.contains(&outside), "{held:?}");
}

/// A package linked into the project's `node_modules` from outside it is in
/// the link closure, so it loads.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_package_linked_from_outside_the_project_loads() {
    let scratch = Scratch::new(
        "linked",
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/config.js",
                "import {value} from '@x/lib/value.js';\nexport default {value};\n",
            ),
            (
                "lib/package.json",
                r#"{"name": "@x/lib", "type": "module", "exports": {"./value.js": "./value.js"}}"#,
            ),
            ("lib/value.js", "export const value = 'linked';\n"),
        ],
    );
    std::fs::create_dir_all(scratch.0.join("project/node_modules/@x")).unwrap();
    std::os::unix::fs::symlink(
        "../../../lib",
        scratch.0.join("project/node_modules/@x/lib"),
    )
    .unwrap();
    let run = eval(&[&scratch.path("project/config.js")]);
    run.assert_value(&json!({"value": "linked"}));
}

/// A project's `package.json` symlinked to a file outside the box is read
/// at its real path. The pre-pass reads it before the box exists; a bare
/// import's walk up passes it again, and that read is refused as outside
/// the box, though the run read the file once already, failing the eval as
/// `internal`, with or without the graph.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_package_json_linked_out_of_the_box_is_internal() {
    let scratch = Scratch::new(
        "linked_out",
        &[
            ("out/pkg.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/main.ts",
                "import x from 'foo';\nexport default x;\n",
            ),
            (
                "project/node_modules/foo/package.json",
                r#"{"name": "foo", "exports": "./i.mjs"}"#,
            ),
            ("project/node_modules/foo/i.mjs", "export default 7;\n"),
        ],
    );
    std::os::unix::fs::symlink("../out/pkg.json", scratch.0.join("project/package.json")).unwrap();
    let entry = scratch.path("project/main.ts");
    let outside = scratch.path("out/pkg.json");
    for args in [&[entry.as_str()][..], &[entry.as_str(), "--graph"]] {
        let run = eval(args);
        run.assert_failed("internal", 1);
        assert!(
            run.message().contains(&format!(
                "the eval was to read {outside}, which lies outside its box"
            )),
            "{}",
            run.message()
        );
    }
}

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
/// without `WebAssembly`). To
/// accept a change deliberately, write the fixture's value over the file,
/// one name a line, from the fixtures directory:
/// `TZ=UTC ../../../../../target/debug/fgro eval global_names.js | python3 -c "import json, sys; print('\\n'.join(json.load(sys.stdin)['value']))" > global_names_jitless.txt`,
/// and with `--jit` into `global_names.txt`.
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
    let jitless = std::fs::read_to_string(fixtures().join("global_names_jitless.txt")).unwrap();
    assert_eq!(
        names(&["global_names.js"]),
        jitless,
        "the global names differ from tests/fixtures/eval/global_names_jitless.txt; update it \
         only if the change is deliberate (see this test's doc)"
    );
    let jit = std::fs::read_to_string(fixtures().join("global_names.txt")).unwrap();
    assert_eq!(
        names(&["global_names.js", "--jit"]),
        jit,
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
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_config_cannot_import_the_driver() {
    let run = driven("import_driver.js", "call.js");
    run.assert_failed("resolve", 1);
    assert_eq!(run.envelope["error"]["specifier"], "gro:driver");
}

/// The reason each operation `determinism: deny` refuses gives, and the
/// text after it, one per reason.
const WHY: [(&str, &str); 4] = [
    (
        "clock",
        "it reads the clock, so the value would differ from run to run; write the date or time \
         as a literal with its offset, such as new Date('2026-01-01T00:00:00Z')",
    ),
    (
        "random",
        "it is random, so the value would differ from run to run; write the value as a literal, \
         or derive it from the module's own data",
    ),
    (
        "machine",
        "it is a fact of this process or machine, so the value would differ from host to host",
    ),
    (
        "timer",
        "it schedules work on the clock, and eval has no timers; chain promise jobs instead",
    ),
];

/// The refusal of `shown` for `why`.
fn refusal(shown: &str, why: &str) -> String {
    let (_, text) = WHY.iter().find(|(reason, _)| *reason == why).unwrap();
    format!("{shown} is refused in eval ({why}): {text}")
}

/// Each operation `determinism: deny` refuses throws its refusal, naming
/// it and the reason, and no way back to its original runs: not its
/// descriptor, its prototype or constructor, `Reflect`, a subclass,
/// deleting the replacement, or (for the formatters) passing `undefined`
/// for the date. `Blob`, `File`, and the object URLs are absent. What's
/// kept still works, with each replaced member's attributes, and the
/// stand-ins are inert.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn determinism_deny_refuses_each_entry_and_keeps_no_original() {
    let run = eval(&["deny.js"]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    let value = &run.envelope["value"];
    let mut entries = vec![
        ("Math.random", "Math.random()", "random"),
        ("performance.now", "performance.now()", "clock"),
        ("performance.mark", "performance.mark()", "clock"),
        ("performance.measure", "performance.measure()", "clock"),
        ("performance.toJSON", "performance.toJSON()", "clock"),
        ("performance.timeOrigin", "performance.timeOrigin", "clock"),
        ("Date()", "Date()", "clock"),
        ("new Date()", "new Date()", "clock"),
        ("Date.now", "Date.now()", "clock"),
        (
            "Intl.DateTimeFormat format",
            "Intl.DateTimeFormat format() with no date",
            "clock",
        ),
        (
            "Intl.DateTimeFormat formatToParts",
            "Intl.DateTimeFormat formatToParts() with no date",
            "clock",
        ),
        ("AbortSignal.timeout", "AbortSignal.timeout()", "timer"),
        (
            "Atomics.waitAsync",
            "Atomics.waitAsync() with a timeout",
            "timer",
        ),
    ];
    let temporal = [
        ("Temporal.Now.instant", "Temporal.Now.instant()", "clock"),
        (
            "Temporal.Now.timeZoneId",
            "Temporal.Now.timeZoneId()",
            "machine",
        ),
        (
            "Temporal.Now.plainDateTimeISO",
            "Temporal.Now.plainDateTimeISO()",
            "clock",
        ),
        (
            "Temporal.Now.zonedDateTimeISO",
            "Temporal.Now.zonedDateTimeISO()",
            "clock",
        ),
        (
            "Temporal.Now.plainDateISO",
            "Temporal.Now.plainDateISO()",
            "clock",
        ),
        (
            "Temporal.Now.plainTimeISO",
            "Temporal.Now.plainTimeISO()",
            "clock",
        ),
    ];
    entries.extend(temporal);
    for (entry, shown, why) in &entries {
        assert_eq!(
            value[entry],
            json!({"refused": refusal(shown, why), "recovered": []}),
            "{entry}"
        );
    }
    // the fixture tried exactly these, so a new entry needs a line here
    let mut tried: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .filter(|key| !["deleted", "absent", "kept", "inert"].contains(key))
        .collect();
    tried.sort_unstable();
    let mut expected: Vec<&str> = entries.iter().map(|(entry, ..)| *entry).collect();
    expected.sort_unstable();
    assert_eq!(tried, expected);
    assert_eq!(value["deleted"], json!([]));
    assert_eq!(
        value["absent"],
        json!([
            "undefined",
            "undefined",
            "undefined",
            "undefined",
            false,
            false
        ])
    );
    assert_eq!(
        value["kept"],
        json!({
            "date": "1970-01-01T00:00:00.000Z",
            "utc": 1_767_225_600_000_u64,
            "parse": 1_767_225_600_000_u64,
            "format": "1/1/1970",
            "instanceof": true,
            "date_name": "Date/7",
            "wait_async_now": "not-equal",
            "wait_async_zero": "timed-out",
            "attributes": ["true/false/true", "true/false/true", "undefined/false/true"],
        })
    );
    assert_eq!(
        value["inert"],
        json!({
            "deref": true,
            "weak_ref_tag": "[object WeakRef]",
            "registry_tag": "[object FinalizationRegistry]",
            "unregister": true,
            "weak_ref_refuses": "invalid value used as a weak reference target",
            "console_time": "time/timeLog/timeEnd",
        })
    );
    // the console's timers printed nothing
    assert!(run.stderr.is_empty(), "{}", run.stderr);
}

/// The driver, evaluated before any of the module's code, already sees the
/// refusals and no timers.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_driver_sees_determinism_deny() {
    driven("value.js", "deny.js").assert_value(&json!({
        "random": "Math.random() is refused in eval (random)",
        "now": "Date.now() is refused in eval (clock)",
        "date": "new Date() is refused in eval (clock)",
        "performance": "performance.now() is refused in eval (clock)",
        "timers": ["undefined", "undefined", "undefined", "undefined"],
    }));
}

/// The timers are absent, `AbortSignal.timeout` refuses, and `Atomics.wait`
/// can't block the main thread.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn no_timer_is_reachable() {
    eval(&["timers.js"]).assert_value(&json!({
        "globals": ["undefined", "undefined", "undefined", "undefined"],
        "abort_signal_timeout": format!(
            "Error: {}",
            refusal("AbortSignal.timeout()", "timer")
        ),
        "atomics_wait": "TypeError: Atomics.wait cannot be called in this context",
    }));
}

/// A time zone other than UTC fails the eval before any code runs, naming
/// the fix; `Etc/UTC`, which ICU resolves to `UTC`, is accepted.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_time_zone_other_than_utc_is_refused() {
    let run = eval_with(&["value.js"], None, &[("TZ", "America/New_York")]);
    run.assert_failed("environment", 1);
    assert_eq!(
        run.message(),
        "eval runs only in the time zone UTC, so dates read and format the same everywhere, and \
         this process's is America/New_York: run fgro with TZ=UTC in its environment"
    );
    assert_eq!(run.envelope["access"], json!({"dynamic": []}));
    // before the driver runs, too
    let run = eval_with(
        &["value.js", "--driver", "-"],
        Some("throws.js"),
        &[("TZ", "America/New_York")],
    );
    run.assert_failed("environment", 1);
    eval_with(&["typed.ts"], None, &[("TZ", "Etc/UTC")]).assert_value(&json!({"x": 1, "y": 2}));
}

/// The zone must display as UTC too: an alias `Intl` resolves to `UTC` but
/// dates name `Greenwich Mean Time` is refused, naming that, while the
/// aliases that print exactly as `UTC` are accepted. A zone ICU doesn't know
/// (which `Intl` reports as no zone, or `Etc/Unknown` for an empty `TZ`) is
/// refused as unknown.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_utc_alias_with_another_display_name_is_refused() {
    let refused = |found: &str| {
        format!(
            "eval runs only in the time zone UTC, so dates read and format the same everywhere, \
             and {found}: run fgro with TZ=UTC in its environment"
        )
    };
    for zone in [
        "Etc/GMT",
        "GMT0",
        "Etc/GMT0",
        "Etc/GMT+0",
        "Etc/GMT-0",
        "Greenwich",
    ] {
        let run = eval_with(&["value.js"], None, &[("TZ", zone)]);
        run.assert_failed("environment", 1);
        assert_eq!(
            run.message(),
            refused("this process's is an alias of UTC that dates name Greenwich Mean Time"),
            "{zone}"
        );
    }
    for zone in ["UTC0", "XYZ", "/nonexistent/zone", ""] {
        let run = eval_with(&["value.js"], None, &[("TZ", zone)]);
        run.assert_failed("environment", 1);
        assert_eq!(
            run.message(),
            refused("this process's time zone isn't one ICU knows"),
            "{zone:?}"
        );
    }
    for zone in [
        "UTC",
        "Etc/UTC",
        "Universal",
        "Zulu",
        "UCT",
        "Etc/UCT",
        ":UTC",
    ] {
        eval_with(&["typed.ts"], None, &[("TZ", zone)]).assert_value(&json!({"x": 1, "y": 2}));
    }
}

/// The message of a heap stop, and of the out-of-memory backstop's
/// envelope.
const HEAP_STOPPED: &str = "the eval's heap reached its cap of 512 MiB, and the run was stopped";
const HEAP_BACKSTOP: &str = "the eval's heap reached its cap of 512 MiB with nothing left to \
                             give: one operation V8 can't stop ran out of memory, so the process \
                             exited";

/// The heap's cap: growth the interpreter can stop is stopped as `heap`
/// with the run's envelope; under the JIT, which doesn't act on the stop,
/// and in one builtin call, which nothing stops, the out-of-memory backstop
/// writes its prepared `heap` envelope, with no `access`, and exits 1.
/// Run one after another, so no two of these children hold their heaps at
/// once.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_heap_cap_fails_the_eval_as_heap() {
    let run = eval(&["heap_grow.js"]);
    run.assert_failed("heap", 1);
    assert_eq!(run.message(), HEAP_STOPPED);
    assert_eq!(run.envelope["access"], json!({"dynamic": []}));
    for args in [
        &["heap_grow.js", "--jit"][..],
        &["heap_builtin.js"],
        &["heap_builtin.js", "--jit"],
    ] {
        let run = eval(args);
        run.assert_failed("heap", 1);
        assert_eq!(run.message(), HEAP_BACKSTOP, "{args:?}");
        assert_eq!(run.envelope["access"], Value::Null, "{args:?}");
        let id = fixtures().canonicalize().unwrap().join(args[0]);
        assert_eq!(
            run.envelope["entry"]["id"],
            id.to_str().unwrap(),
            "{args:?}"
        );
        assert_eq!(run.envelope["eval_protocol"], 1);
    }
}

/// The heap's cap is V8's heap only: an `ArrayBuffer` twice the cap is
/// made, since its bytes live outside the heap. This pins a known gap, not
/// a goal: a runaway allocation of buffers whose bytes are written grows the
/// process until the kernel ends it, with no envelope, which a caller
/// treats as a failure (see `HEAP_LIMIT`). When eval caps that memory too,
/// this becomes a `heap` failure.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn array_buffer_memory_sits_outside_the_heap_cap() {
    eval(&["heap_external.js"]).assert_value(&json!(1_073_741_824_u64));
}

/// Every thread eval starts sets its own stack, so a `RUST_MIN_STACK` too
/// large to reserve changes nothing: the watchdog still starts, and the value
/// is written.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_huge_rust_min_stack_changes_nothing() {
    let run = eval_with(&["value.js"], None, &[("RUST_MIN_STACK", "1099511627776")]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert_eq!(run.envelope["ok"], true, "{}", run.stdout);
}

/// The timeout stops a loop that never ends, jitless or with the JIT, as
/// `timeout`, with a hint naming `--jit` only jitless.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_run_past_its_timeout_fails_as_timeout() {
    let run = eval(&["loop.js", "--timeout", "200"]);
    run.assert_failed("timeout", 1);
    assert_eq!(
        run.message(),
        "the eval ran past its timeout of 200 ms, and the run was stopped"
    );
    assert_eq!(
        run.envelope["error"]["hint"],
        "eval runs jitless unless asked, in V8's interpreter alone: --jit runs compute-heavy \
         code faster, or pass a longer --timeout"
    );
    let run = eval(&["loop.js", "--timeout", "200", "--jit"]);
    run.assert_failed("timeout", 1);
    assert!(
        run.envelope["error"].get("hint").is_none(),
        "{}",
        run.stdout
    );
}

/// The timeout stops the serializer's walk, which runs no JS for a
/// terminate to stop: this value's walk would outlast the test.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_timeout_stops_the_serializer_walk() {
    let started = Instant::now();
    let run = eval(&["walk.js", "--timeout", "500"]);
    run.assert_failed("timeout", 1);
    assert!(started.elapsed() < DEADLINE);
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

/// The files of a project with a self-reference, a scoped package with
/// another nested in its own `node_modules`, and a package linked into its
/// `node_modules` from `lib/` beside it, which [`graph_project`] writes.
const GRAPH_FILES: [(&str, &str); 11] = [
    (
        "project/package.json",
        r#"{"name": "app", "type": "module", "exports": {"./self.js": "./src/self.js"}}"#,
    ),
    (
        "project/src/config.ts",
        "import {a} from './a.js';\nimport './a.js?x#y';\nimport {s} from 'app/self.js';\n\
         import {v} from '@sc/pkg';\nimport {l} from 'lnk/l.js';\n\
         export default {a, s, v, l} as const;\n",
    ),
    ("project/src/a.js", "export const a = 'a';\n"),
    ("project/src/self.js", "export const s = 'self';\n"),
    (
        "project/node_modules/@sc/pkg/package.json",
        r#"{"name": "@sc/pkg", "version": "1.0.0", "type": "module", "exports": "./i.js"}"#,
    ),
    (
        "project/node_modules/@sc/pkg/i.js",
        "import {n} from 'nested';\nexport const v = n + 1;\n",
    ),
    (
        "project/node_modules/@sc/pkg/node_modules/nested/package.json",
        r#"{"name": "nested", "version": "2.0.0", "exports": "./n.mjs"}"#,
    ),
    (
        "project/node_modules/@sc/pkg/node_modules/nested/n.mjs",
        "export const n = 1;\n",
    ),
    (
        "lib/package.json",
        r#"{"name": "lnk", "version": "0.1.0", "type": "module", "exports": {"./l.js": "./l.js"}}"#,
    ),
    ("lib/l.js", "export const l = 'linked';\n"),
    // a type marker, outside every walk: it changes nothing
    ("lib/sub/package.json", r#"{"type": "module"}"#),
];

/// [`GRAPH_FILES`] written under a scratch directory named `name`, with
/// `project/node_modules/lnk` linked to `../../lib`.
fn graph_project(name: &str) -> Scratch {
    let scratch = Scratch::new(name, &GRAPH_FILES);
    std::os::unix::fs::symlink("../../lib", scratch.0.join("project/node_modules/lnk")).unwrap();
    scratch
}

/// The BLAKE3 hash of `content`, in hex.
fn hex(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

/// The content [`GRAPH_FILES`] gives `path`.
fn graph_file(path: &str) -> &'static str {
    GRAPH_FILES
        .iter()
        .find(|(file, _)| *file == path)
        .unwrap()
        .1
}

/// The expected `graph` of [`graph_project`]'s config, but for `local`.
fn expected_graph() -> Value {
    let file = |path: &str| hex(graph_file(path));
    let project = json!({"kind": "project"});
    let import = |specifier: &str, resolved: &str| json!({"specifier": specifier, "resolved": resolved, "reach": "static"});
    let consulted =
        |id: &str, path: &str, role: &str| json!({"id": id, "blake3": file(path), "role": role});
    json!({
        "graph_version": 1,
        "roots": ["project:src/config.ts"],
        "modules": [
            {
                "id": "linked:../lib/l.js",
                "blake3": file("lib/l.js"),
                "origin": {"kind": "package", "name": "lnk", "version": "0.1.0", "linked": true},
                "format": "javascript",
                "imports": [],
            },
            {
                "id": "project:node_modules/@sc/pkg/i.js",
                "blake3": file("project/node_modules/@sc/pkg/i.js"),
                "origin": {"kind": "package", "name": "@sc/pkg", "version": "1.0.0", "linked": false},
                "format": "javascript",
                "imports": [import("nested", "project:node_modules/@sc/pkg/node_modules/nested/n.mjs")],
            },
            {
                "id": "project:node_modules/@sc/pkg/node_modules/nested/n.mjs",
                "blake3": file("project/node_modules/@sc/pkg/node_modules/nested/n.mjs"),
                "origin": {"kind": "package", "name": "nested", "version": "2.0.0", "linked": false},
                "format": "javascript",
                "imports": [],
            },
            {
                "id": "project:src/a.js",
                "blake3": file("project/src/a.js"),
                "origin": project,
                "format": "javascript",
                "imports": [],
            },
            {
                "id": "project:src/a.js?x#y",
                "blake3": file("project/src/a.js"),
                "origin": project,
                "format": "javascript",
                "imports": [],
            },
            {
                "id": "project:src/config.ts",
                "blake3": file("project/src/config.ts"),
                "origin": project,
                "format": "typescript",
                "imports": [
                    import("./a.js", "project:src/a.js"),
                    import("./a.js?x#y", "project:src/a.js?x#y"),
                    import("@sc/pkg", "project:node_modules/@sc/pkg/i.js"),
                    import("app/self.js", "project:src/self.js"),
                    import("lnk/l.js", "linked:../lib/l.js"),
                ],
            },
            {
                "id": "project:src/self.js",
                "blake3": file("project/src/self.js"),
                "origin": project,
                "format": "javascript",
                "imports": [],
            },
        ],
        "consulted": [
            consulted("linked:../lib/package.json", "lib/package.json", "exports"),
            consulted("linked:../lib/package.json", "lib/package.json", "package_scope"),
            consulted(
                "project:node_modules/@sc/pkg/node_modules/nested/package.json",
                "project/node_modules/@sc/pkg/node_modules/nested/package.json",
                "exports",
            ),
            consulted(
                "project:node_modules/@sc/pkg/package.json",
                "project/node_modules/@sc/pkg/package.json",
                "exports",
            ),
            consulted(
                "project:node_modules/@sc/pkg/package.json",
                "project/node_modules/@sc/pkg/package.json",
                "package_scope",
            ),
            consulted("project:package.json", "project/package.json", "exports"),
            consulted("project:package.json", "project/package.json", "package_scope"),
            consulted("project:package.json", "project/package.json", "search_bound"),
        ],
        // the source directory's own package.json, on the walks up from it,
        // and its node_modules entries, on the package searches from it
        "absent": [
            "project:src/node_modules/@sc/pkg",
            "project:src/node_modules/lnk",
            "project:src/package.json",
        ],
    })
}

/// `--graph` adds the graph to the envelope as its last field, every file by
/// its path relative to the project, `local` alone holding absolute paths;
/// without the flag the envelope has none.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_graph_dump_names_every_file_by_its_path() {
    let scratch = graph_project("graph");
    let config = scratch.path("project/src/config.ts");
    let run = eval(&[&config, "--graph"]);
    run.assert_value(&json!({"a": "a", "s": "self", "v": 2, "l": "linked"}));
    let keys: Vec<&str> = run
        .envelope
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "eval_protocol",
            "ok",
            "runtime",
            "entry",
            "value",
            "access",
            "graph"
        ]
    );
    let mut graph = run.envelope["graph"].clone();
    let local = graph.as_object_mut().unwrap().remove("local").unwrap();
    assert_eq!(graph, expected_graph(), "{:#}", run.envelope["graph"]);
    let project = scratch.path("project");
    let lib = scratch.path("lib");
    assert_eq!(local["project"], project.as_str());
    assert_eq!(local["box"], json!([project, lib]));
    // every id but a query's has its absolute path, and nothing else does
    let paths = local["paths"].as_object().unwrap();
    for (id, path) in paths {
        let path = path.as_str().unwrap();
        let expected = id.strip_prefix("project:").map_or_else(
            || {
                format!(
                    "{}/{}",
                    scratch.0.display(),
                    id.strip_prefix("linked:../").unwrap()
                )
            },
            |relative| format!("{project}/{relative}"),
        );
        assert_eq!(path, expected, "{id}");
    }
    let mut ids: Vec<&str> = paths.keys().map(String::as_str).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "linked:../lib/l.js",
            "linked:../lib/package.json",
            "project:node_modules/@sc/pkg/i.js",
            "project:node_modules/@sc/pkg/node_modules/nested/n.mjs",
            "project:node_modules/@sc/pkg/node_modules/nested/package.json",
            "project:node_modules/@sc/pkg/package.json",
            "project:package.json",
            "project:src/a.js",
            "project:src/config.ts",
            "project:src/node_modules/@sc/pkg",
            "project:src/node_modules/lnk",
            "project:src/package.json",
            "project:src/self.js",
        ]
    );
    let without = eval(&[&config]);
    assert!(
        without.envelope.get("graph").is_none(),
        "{}",
        without.stdout
    );
    // the same envelope but for the graph
    assert_eq!(
        without.stdout.trim_end(),
        run.stdout
            .trim_end()
            .rsplit_once(",\"graph\":")
            .map(|(before, _)| format!("{before}}}"))
            .unwrap()
    );
}

/// The dump is the same on every run, and the same for the project moved
/// elsewhere, but for `local`.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_graph_dump_is_deterministic_and_location_independent() {
    let here = graph_project("graph_here");
    let moved = graph_project("graph_moved_elsewhere");
    let graph = |scratch: &Scratch| {
        let run = eval(&[&scratch.path("project/src/config.ts"), "--graph"]);
        assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
        run.envelope["graph"].clone()
    };
    let first = graph(&here);
    assert_eq!(graph(&here), first);
    let mut elsewhere = graph(&moved);
    assert_ne!(elsewhere["local"], first["local"]);
    elsewhere["local"] = first["local"].clone();
    assert_eq!(elsewhere, first);
}

/// What a cache would key on moves with what the run depends on: an
/// edited `package.json` changes its hash and nothing else, and a path the
/// package search passed appearing (a package in the source directory's
/// own `node_modules`, which then shadows the linked one) leaves `absent`
/// and changes the value.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_graph_dump_moves_with_what_the_run_read() {
    let scratch = graph_project("graph_changes");
    let config = scratch.path("project/src/config.ts");
    let graph = || {
        let run = eval(&[&config, "--graph"]);
        assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
        let mut graph = run.envelope["graph"].clone();
        graph.as_object_mut().unwrap().remove("local");
        (run.envelope["value"].clone(), graph)
    };
    assert_eq!(graph().1, expected_graph());

    let edited = r#"{"name": "lnk", "version": "0.1.0", "type": "module", "description": "x", "exports": {"./l.js": "./l.js"}}"#;
    std::fs::write(scratch.0.join("lib/package.json"), edited).unwrap();
    let mut expected = expected_graph();
    for file in expected["consulted"].as_array_mut().unwrap() {
        if file["id"] == "linked:../lib/package.json" {
            file["blake3"] = json!(hex(edited));
        }
    }
    assert_eq!(graph().1, expected);

    let shadow =
        r#"{"name": "lnk", "version": "9.0.0", "type": "module", "exports": {"./l.js": "./l.js"}}"#;
    let dir = scratch.0.join("project/src/node_modules/lnk");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("package.json"), shadow).unwrap();
    std::fs::write(dir.join("l.js"), "export const l = 'shadow';\n").unwrap();
    let (value, after) = graph();
    assert_eq!(value["l"], "shadow");
    assert_eq!(
        after["absent"],
        json!([
            "project:src/node_modules/@sc/pkg",
            "project:src/package.json"
        ])
    );
    let ids = |key: &str| -> Vec<String> {
        after[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(ids("modules").contains(&"project:src/node_modules/lnk/l.js".to_owned()));
    assert!(!ids("modules").contains(&"linked:../lib/l.js".to_owned()));
    assert!(ids("consulted").contains(&"project:src/node_modules/lnk/package.json".to_owned()));
}

/// A run that refuses its `import()`s dumps the graph without them, since
/// nothing an `import()` names is resolved; a failed run dumps the graph
/// as far as it got, under `ok: false`; and with no graph to dump (the
/// entry didn't resolve, or the process ran out of memory) `graph` is
/// `null`.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_graph_dump_of_refused_and_failed_runs() {
    let fixtures = fixtures().canonicalize().unwrap();
    let id = |file: &str| format!("project:{file}");
    let hash = |file: &str| hex(&std::fs::read_to_string(fixtures.join(file)).unwrap());

    let run = eval(&["dynamic.js", "--graph"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(
        run.envelope["access"]["dynamic"].as_array().unwrap().len(),
        2
    );
    let graph = &run.envelope["graph"];
    assert_eq!(
        graph["modules"],
        json!([{
            "id": id("dynamic.js"),
            "blake3": hash("dynamic.js"),
            "origin": {"kind": "project"},
            "format": "javascript",
            "imports": [],
        }])
    );
    assert_eq!(graph["roots"], json!([id("dynamic.js")]));
    assert_eq!(
        graph["consulted"],
        json!([
            {"id": "project:package.json", "blake3": hash("package.json"), "role": "package_scope"},
            {"id": "project:package.json", "blake3": hash("package.json"), "role": "search_bound"},
        ])
    );
    assert_eq!(graph["absent"], json!([]));

    let run = eval(&["import_missing.js", "--graph"]);
    run.assert_failed("resolve", 1);
    assert_eq!(
        run.envelope["graph"]["modules"],
        json!([{
            "id": id("import_missing.js"),
            "blake3": hash("import_missing.js"),
            "origin": {"kind": "project"},
            "format": "javascript",
            "imports": [],
        }])
    );

    let run = eval(&["missing.js", "--graph"]);
    run.assert_failed("resolve", 2);
    assert_eq!(run.envelope["graph"], Value::Null);
    assert!(
        run.stdout.trim_end().ends_with(",\"graph\":null}"),
        "{}",
        run.stdout
    );

    let run = eval(&["heap_builtin.js", "--graph"]);
    run.assert_failed("heap", 1);
    assert_eq!(run.message(), HEAP_BACKSTOP);
    assert_eq!(run.envelope["graph"], Value::Null);
}

/// `fgro eval-box` runs eval's path through the box and stops: confined, a
/// driven module that evaluates exits 0 with no output.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn eval_box_confines_and_exits_without_output() {
    let run = spawn_command(
        "eval-box",
        &["driven.js", "--driver", "-"],
        Some("call.js"),
        &[],
    );
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "", "")
    );
    let run = spawn_command("eval-box", &["value.js"], None, &[]);
    assert_eq!((run.code, run.stdout.as_str()), (0, ""), "{}", run.stderr);
}

/// `fgro eval-box` fails as eval does, with eval's envelope and exit code:
/// before the box (a module that doesn't resolve) and, in a build with no
/// snapshot, at the snapshot check.
#[test]
fn eval_box_fails_as_eval_does() {
    for args in [&["missing.js"][..], &["value.js", "--timeout", "100"]] {
        let boxed = spawn_command("eval-box", args, None, &[]);
        let evaled = spawn_eval(args, None, &[]);
        if cfg!(feature = "snapshot") && evaled.code == 0 {
            // a snapshot build evaluates the value, which eval-box stops short of
            assert_eq!((boxed.code, boxed.stdout.as_str()), (0, ""), "{args:?}");
            continue;
        }
        assert_eq!(
            (boxed.code, &boxed.stdout, &boxed.stderr),
            (evaled.code, &evaled.stdout, &evaled.stderr),
            "{args:?}"
        );
        assert_ne!(boxed.code, 0, "{args:?}");
    }
}
