//! Running the programs a step names: the result a step returns, and the
//! helpers every step prints and spawns with.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::XtaskError;

/// A step's result: a failure is what `cargo xtask` prints and exits with.
pub type StepResult = Result<(), XtaskError>;

/// Cargo's target directory, as `cargo metadata` reports it, so a
/// `CARGO_TARGET_DIR` or `build.target-dir` setting is honored. The V8
/// archive's path doesn't depend on it: `.cargo/config.toml` names it relative
/// to the checkout.
pub fn target_dir(root: &Path) -> Result<PathBuf, XtaskError> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|source| XtaskError::Spawn {
            display: "cargo metadata".to_owned(),
            source,
        })?;
    let target_dir = output
        .status
        .success()
        .then(|| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
        .flatten()
        .and_then(|metadata| metadata["target_directory"].as_str().map(PathBuf::from));
    target_dir.ok_or_else(|| {
        XtaskError::Failed(format!(
            "cargo metadata reported no target directory ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    })
}

/// Runs `program` (found on `PATH`) from `root`.
pub fn run(root: &Path, env: &[(&str, &str)], program: &str, args: &[&str]) -> StepResult {
    run_program(root, env, Path::new(program), program, args)
}

/// Runs `program` from `root`, printing the command (with `display` as the
/// program's name) first, failing with [`XtaskError::StepFailed`] when it
/// exits unsuccessfully.
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
        .map_err(|source| XtaskError::Spawn {
            display: display.to_owned(),
            source,
        })?;
    if status.success() {
        return Ok(());
    }
    Err(XtaskError::StepFailed {
        display: display.to_owned(),
        status,
    })
}

/// A failed read of `path`.
pub fn io_failure(path: &Path, error: &io::Error) -> XtaskError {
    XtaskError::Failed(format!("failed to read {}: {error}", path.display()))
}

/// `path` relative to `root`, for printing and for commands run from `root`.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
