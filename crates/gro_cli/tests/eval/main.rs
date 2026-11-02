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

#[path = "../support/mod.rs"]
mod support;

mod determinism;
mod driver;
mod environment;
mod globals;
mod graph;
mod kinds;
mod limits;
mod resolve_box;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;

use support::{fgro_command, wrapped_fgro_command};

/// The `--timeout` a test passes an eval it lets run long, the harness's
/// [`DEADLINE`](support::DEADLINE) less a margin, so a hung eval is stopped
/// by its own timeout, and the test sees eval's `timeout` failure rather
/// than the harness's kill.
const LONG_TIMEOUT: Duration = Duration::from_secs(support::DEADLINE.as_secs() - 5);

/// [`LONG_TIMEOUT`] as `--timeout` takes it, in milliseconds.
fn long_timeout() -> String {
    LONG_TIMEOUT.as_millis().to_string()
}

/// Held by every test whose child takes much of the machine's memory (a heap
/// filled to the cap, an array buffer of a gibibyte, the process's memory
/// cap reached), so no two such children run at once, across tests as
/// within one: the binary's tests run in parallel, beside whatever else
/// loads the machine.
static MEMORY_HEAVY: Mutex<()> = Mutex::new(());

/// Takes [`MEMORY_HEAVY`], whatever test last panicked holding it.
fn memory_heavy_turn() -> MutexGuard<'static, ()> {
    MEMORY_HEAVY.lock().unwrap_or_else(PoisonError::into_inner)
}

fn fixtures() -> PathBuf {
    support::fixtures("eval")
}

/// The fixtures directory as a `file:` URL prefix.
fn fixtures_url() -> String {
    support::fixtures_url("eval")
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
    spawn(command("eval", args, env), driver)
}

/// The command [`spawn_eval`] runs for `subcommand`, for a test to change
/// before [`spawn`] runs it.
fn command(subcommand: &str, args: &[&str], env: &[(&str, &str)]) -> Command {
    in_fixtures(fgro_command(&[&[subcommand], args].concat()), env)
}

/// [`command`] for `eval`, run by `wrapper` (see
/// [`support::wrapped_fgro_command`]).
fn wrapped_command(wrapper: Command, args: &[&str]) -> Command {
    in_fixtures(
        wrapped_fgro_command(wrapper, &[&["eval"], args].concat()),
        &[],
    )
}

/// `command` with `env` set, run from the fixtures directory with its
/// standard streams piped.
fn in_fixtures(mut command: Command, env: &[(&str, &str)]) -> Command {
    command
        .envs(env.iter().copied())
        .current_dir(fixtures())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Runs `command` (see [`support::spawn`]), `driver`'s file piped to stdin
/// when given.
fn spawn(command: Command, driver: Option<&str>) -> Output {
    let stdin = driver.map_or_else(Vec::new, |driver| {
        std::fs::read(fixtures().join("drivers").join(driver)).unwrap()
    });
    let output = support::spawn(command, &stdin);
    Output {
        code: output.status.code().expect("the eval exits, not killed"),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// [`spawn_eval`], its envelope parsed.
fn eval_with(args: &[&str], driver: Option<&str>, env: &[(&str, &str)]) -> Eval {
    parse(spawn_eval(args, driver, env))
}

/// An eval's output, its envelope parsed.
fn parse(output: Output) -> Eval {
    let Output {
        code,
        stdout,
        stderr,
    } = output;
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
