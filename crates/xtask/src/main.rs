//! `cargo xtask`: dev automation for the gro repo.
//!
//! `cargo xtask check` is the one local command that checks both halves of
//! the repo. It runs these steps in order, printing each command first and
//! stopping at the first failure with that step's exit code:
//!
//! 1. the Rust gates: `cargo fmt`, `cargo clippy`, `cargo test`, and
//!    `cargo doc`, with warnings denied
//! 2. the JS/TS under `crates/`, which `gro check`'s typecheck and format
//!    don't reach: `tsc` over each `crates/*/js/tsconfig.json`, then
//!    `gro format --check` over every `.ts` and `.js` file under `crates/`
//!    (it says so when there are none)
//! 3. `gro check`
//!
//! Every step runs from the workspace root, so `cargo xtask` works the same
//! from any directory in the checkout.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use argh::FromArgs;

/// The workspace root, two levels above this crate's manifest.
fn root() -> PathBuf {
    // `cargo run` sets it at runtime; the compile-time path goes stale when the checkout moves
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    manifest
        .ancestors()
        .nth(2)
        .unwrap_or(&manifest)
        .to_path_buf()
}

/// The repo's own TypeScript compiler, relative to the root.
const TSC: &str = "node_modules/.bin/tsc";

/// dev automation for the gro repo
#[derive(FromArgs)]
struct Xtask {
    #[argh(subcommand)]
    command: Subcommand,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Subcommand {
    Check(Check),
}

/// run the Rust gates, check the JS/TS under crates/, then run `gro check`
#[derive(FromArgs)]
#[argh(subcommand, name = "check")]
struct Check {}

fn main() -> ExitCode {
    let xtask: Xtask = argh::from_env();
    let root = root();
    let result = match xtask.command {
        Subcommand::Check(Check {}) => check(&root),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// A failed step: the exit code `cargo xtask` returns.
type StepResult = Result<(), ExitCode>;

fn check(root: &Path) -> StepResult {
    run(root, &[], "cargo", &["fmt", "--all", "--check"])?;
    run(
        root,
        &[],
        "cargo",
        &[
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(
        root,
        &[],
        "cargo",
        &["test", "--locked", "--workspace", "--no-fail-fast"],
    )?;
    // private items too, so the links in their docs are checked
    run(
        root,
        &[("RUSTDOCFLAGS", "-D warnings")],
        "cargo",
        &[
            "doc",
            "--locked",
            "--workspace",
            "--no-deps",
            "--document-private-items",
        ],
    )?;
    check_crates_js(root)?;
    run(root, &[], "gro", &["check"])
}

/// Typechecks and format-checks the JS/TS under `crates/`.
fn check_crates_js(root: &Path) -> StepResult {
    let crates = root.join("crates");

    let tsconfigs = find_tsconfigs(&crates).map_err(|error| io_failure(&crates, &error))?;
    if tsconfigs.is_empty() {
        println!("[xtask] no crates/*/js/tsconfig.json found, nothing to typecheck");
    }
    let tsc = root.join(TSC);
    for tsconfig in &tsconfigs {
        let tsconfig = relative(root, tsconfig);
        run_program(root, &[], &tsc, TSC, &["--noEmit", "-p", &tsconfig])?;
    }

    let mut sources = Vec::new();
    find_js_sources(&crates, &mut sources).map_err(|error| io_failure(&crates, &error))?;
    if sources.is_empty() {
        println!("[xtask] no .ts or .js files under crates/, nothing to format-check");
        return Ok(());
    }
    sources.sort();
    let sources: Vec<String> = sources
        .iter()
        .map(|source| relative(root, source))
        .collect();
    // the paths go before `--check`, which would otherwise take the first as its value
    let mut args = vec!["format"];
    args.extend(sources.iter().map(String::as_str));
    args.push("--check");
    run(root, &[], "gro", &args)
}

/// Finds `crates/*/js/tsconfig.json`, sorted.
fn find_tsconfigs(crates: &Path) -> io::Result<Vec<PathBuf>> {
    let mut tsconfigs = Vec::new();
    for entry in fs::read_dir(crates)? {
        let tsconfig = entry?.path().join("js/tsconfig.json");
        if tsconfig.is_file() {
            tsconfigs.push(tsconfig);
        }
    }
    tsconfigs.sort();
    Ok(tsconfigs)
}

/// Collects every `.ts` and `.js` file under `dir`, skipping `node_modules`.
fn find_js_sources(dir: &Path, sources: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if entry.file_name() != "node_modules" {
                find_js_sources(&path, sources)?;
            }
        } else if path
            .extension()
            .is_some_and(|ext| ext == "ts" || ext == "js")
        {
            sources.push(path);
        }
    }
    Ok(())
}

/// Runs `program` (found on `PATH`) from `root`.
fn run(root: &Path, env: &[(&str, &str)], program: &str, args: &[&str]) -> StepResult {
    run_program(root, env, Path::new(program), program, args)
}

/// Runs `program` from `root`, printing the command (with `display` as the
/// program's name) first, and maps a failure to its exit code.
fn run_program(
    root: &Path,
    env: &[(&str, &str)],
    program: &Path,
    display: &str,
    args: &[&str],
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

fn io_failure(path: &Path, error: &io::Error) -> ExitCode {
    eprintln!("[xtask] failed to read {}: {error}", path.display());
    ExitCode::FAILURE
}

/// `path` relative to `root`, for printing and for commands run from `root`.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
