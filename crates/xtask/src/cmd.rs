//! Running the programs a step names: the result a step returns, and the
//! helpers every step prints and spawns with.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::XtaskError;

/// A step's result: a failure is what `cargo xtask` prints and exits with.
pub type StepResult = Result<(), XtaskError>;

/// Cargo's two output directories, as `cargo metadata` reports them, so
/// `CARGO_TARGET_DIR` or `build.target-dir`, and `CARGO_BUILD_BUILD_DIR` or
/// `build.build-dir`, are honored. The V8 archive's path depends on neither:
/// `.cargo/config.toml` names it relative to the checkout.
#[derive(Debug, Clone)]
pub struct CargoDirs {
    /// The target directory, where final artifacts land
    /// (`<target>/<profile>/fgro`).
    pub target: PathBuf,
    /// The build directory, where intermediate outputs land, build scripts'
    /// among them (the `v8` library copy). The target directory unless
    /// `build.build-dir` moves it.
    pub build: PathBuf,
}

/// Reads [`CargoDirs`] from `cargo metadata` in `root`.
pub fn cargo_dirs(root: &Path) -> Result<CargoDirs, XtaskError> {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .map_err(|source| XtaskError::Spawn {
            display: "cargo metadata".to_owned(),
            source,
        })?;
    let metadata = output
        .status
        .success()
        .then(|| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
        .flatten();
    let dir = |key: &str| {
        metadata
            .as_ref()
            .and_then(|metadata| metadata[key].as_str().map(PathBuf::from))
            .ok_or_else(|| {
                XtaskError::Failed(format!(
                    "cargo metadata reported no {key} ({}): {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr).trim()
                ))
            })
    };
    Ok(CargoDirs {
        target: dir("target_directory")?,
        build: dir("build_directory")?,
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

/// `path` relative to `root`, for printing and for commands run from `root`.
pub fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_reports_both_output_directories() {
        let dirs = cargo_dirs(&crate::root()).unwrap();
        assert!(dirs.target.is_absolute(), "{dirs:?}");
        assert!(dirs.build.is_absolute(), "{dirs:?}");
        // the build directory is where the build is configured to put it
        let configured = Command::new("cargo")
            .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
            .env("CARGO_BUILD_BUILD_DIR", "/nonexistent/build-dir")
            .current_dir(crate::root())
            .output()
            .unwrap();
        assert!(configured.status.success());
        let metadata: serde_json::Value = serde_json::from_slice(&configured.stdout).unwrap();
        assert_eq!(metadata["build_directory"], "/nonexistent/build-dir");
    }
}
