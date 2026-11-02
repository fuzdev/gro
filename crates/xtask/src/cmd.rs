//! Running the programs a step names: the exit code a failed step returns,
//! and the helpers every step prints and spawns with.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// A failed step: the exit code `cargo xtask` returns.
pub type StepResult = Result<(), ExitCode>;

/// Cargo's target directory, as `cargo metadata` reports it, so a
/// `CARGO_TARGET_DIR` or `build.target-dir` setting is honored. The V8
/// archive's path doesn't depend on it: `.cargo/config.toml` names it relative
/// to the checkout.
pub fn target_dir(root: &Path) -> Result<PathBuf, ExitCode> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|error| {
            eprintln!("[xtask] failed to run cargo metadata: {error}");
            ExitCode::FAILURE
        })?;
    let target_dir = output
        .status
        .success()
        .then(|| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
        .flatten()
        .and_then(|metadata| metadata["target_directory"].as_str().map(PathBuf::from));
    target_dir.ok_or_else(|| {
        eprintln!(
            "[xtask] cargo metadata reported no target directory ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        ExitCode::FAILURE
    })
}

/// Runs `program` (found on `PATH`) from `root`.
pub fn run(root: &Path, env: &[(&str, &str)], program: &str, args: &[&str]) -> StepResult {
    run_program(root, env, Path::new(program), program, args)
}

/// Runs `program` from `root`, printing the command (with `display` as the
/// program's name) first, and maps a failure to its exit code.
pub fn run_program(
    root: &Path,
    env: &[(&str, &str)],
    program: &Path,
    display: &str,
    args: &[&str],
) -> StepResult {
    run_program_to(root, env, program, display, args, Stdio::inherit())
}

/// [`run_program`] with the program's stdout sent to `stdout`; its stderr,
/// where failures print, is always inherited.
pub fn run_program_to(
    root: &Path,
    env: &[(&str, &str)],
    program: &Path,
    display: &str,
    args: &[&str],
    stdout: Stdio,
) -> StepResult {
    print!("\n[xtask] ");
    for (key, value) in env {
        print!("{key}='{value}' ");
    }
    println!("{display} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .envs(env.iter().copied())
        .current_dir(root)
        .stdout(stdout)
        .status()
        .map_err(|error| {
            eprintln!("[xtask] failed to run {display}: {error}");
            ExitCode::FAILURE
        })?;
    if status.success() {
        return Ok(());
    }
    eprintln!("[xtask] {display} failed ({status})");
    // a signal-terminated step has no code
    let code = status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1);
    Err(ExitCode::from(code))
}

pub fn io_failure(path: &Path, error: &io::Error) -> ExitCode {
    eprintln!("[xtask] failed to read {}: {error}", path.display());
    ExitCode::FAILURE
}

/// `path` relative to `root`, for printing and for commands run from `root`.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
