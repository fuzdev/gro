//! `fgro run` end to end: the built binary on the scripts in
//! `tests/fixtures/run/`, run from that directory with a relative path.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test's setup fails by panicking"
)]

#[path = "../support/mod.rs"]
mod support;

mod basics;
mod color;
mod env;
mod refusals;
mod reports;
mod resolution;
mod spawn;
mod stdio;
mod typescript;
mod usage;

use std::path::PathBuf;
use std::process::{Command, Output};

use gro_runtime::BINARY_NAME;
use support::{Scratch, fgro_command, spawn};

fn fixtures() -> PathBuf {
    support::fixtures("run")
}

fn fixtures_url() -> String {
    support::fixtures_url("run")
}

/// Runs `fgro` with `args` from the fixtures directory, with stdout and stderr
/// piped (never terminals) and no color variable set, so console output is
/// uncolored.
fn fgro(args: &[&str]) -> Output {
    fgro_with_env(args, &[])
}

/// [`fgro`] with `env` set.
fn fgro_with_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = fgro_command(args);
    command.envs(env.iter().copied()).current_dir(fixtures());
    spawn(command, b"")
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// The line `fgro run` prints after the refusals it lists, or counts, on
/// every path a run ends by.
const REFUSALS_FAIL: &str =
    "error: a refusal fails the run, whether or not the program caught it\n";

fn run(file: &str) -> Run {
    let output = fgro(&["run", file]);
    let run = Run {
        code: output
            .status
            .code()
            .expect("the run exits, not killed by a signal"),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    };
    // console output is uncolored when it isn't going to a terminal
    assert!(!run.stdout.contains('\x1b'), "{}", run.stdout);
    assert!(!run.stderr.contains('\x1b'), "{}", run.stderr);
    run
}

/// A directory of its own for `files` (see [`Scratch`]), so sources that
/// must not pass the format check (a parse error) or that it skips (`.tsx`,
/// `.cts`) live outside the fixtures, under the system's temporary
/// directory, which ends every walk up from them; the files under a
/// `package.json` with `"type": "module"` unless they have their own.
fn temp_fixtures(name: &str, files: &[(&str, &str)]) -> Scratch {
    let fixtures = Scratch::in_system_temp(name, files);
    let package_json = fixtures.0.join("package.json");
    if !package_json.exists() {
        std::fs::write(package_json, "{\"type\": \"module\"}\n").unwrap();
    }
    fixtures
}

/// Runs `file` in `fixtures`.
fn run_in(fixtures: &Scratch, file: &str) -> Run {
    let mut command = fgro_command(&["run", file]);
    command.current_dir(&fixtures.0);
    run_command(command)
}

/// Runs `file` in `fixtures` with `flags` before it, in an environment
/// that holds `env` and nothing else, so no test reads the one it was
/// started in.
fn run_with(fixtures: &Scratch, flags: &[&str], file: &str, env: &[(&str, &str)]) -> Run {
    let mut command = fgro_command(&[&["run"][..], flags, &[file]].concat());
    command
        .env_clear()
        .envs(env.iter().copied())
        .current_dir(&fixtures.0);
    run_command(command)
}

/// What `command`, a run, exited with and printed.
fn run_command(command: Command) -> Run {
    let output = spawn(command, b"");
    Run {
        code: output
            .status
            .code()
            .expect("the run exits, not killed by a signal"),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// The commit the binary under test was built from: none for the plain one
/// under `cargo test`, and for the snapshot one under
/// `cargo xtask build --verify`, the commit the xtask embedded (it builds
/// these tests with the same `GRO_COMMIT`).
fn commit() -> Option<&'static str> {
    option_env!("GRO_COMMIT").filter(|value| cfg!(feature = "snapshot") && !value.is_empty())
}

/// What a script run by the binary under test reads as `runtime_version`:
/// the binary's name, `/`, the version, then `+` and [`commit`] when the build
/// has one.
fn runtime_version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    commit().map_or_else(
        || format!("{BINARY_NAME}/{version}"),
        |commit| format!("{BINARY_NAME}/{version}+{commit}"),
    )
}
