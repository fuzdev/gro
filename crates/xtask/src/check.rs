//! The checks of the repo: the Rust gates, the V8 archive they start from, and
//! the JS/TS under `crates/`.
//!
//! `cargo xtask check` runs these steps in order, printing each command first
//! and stopping at the first failure with that step's exit code;
//! `check-rust` runs 1 to 3 (CI's rust job), `check-js` only 4 (CI's Node
//! job, which has no V8 build):
//!
//! 1. the V8 archive, as `fetch-v8`
//! 2. the Rust gates: `cargo fmt`, `cargo clippy` (again with the `phases`
//!    and `prices` features on `gro_runtime` and `gro_cli`), `cargo test`
//!    (again for `gro_runtime`'s unit tests with the features), and
//!    `cargo doc`, with warnings denied; after clippy, the first build, the
//!    V8 library copy it left is checked before the tests run V8 code
//! 3. the snapshot build, as `build --verify`
//! 4. the JS/TS under `crates/`, which `gro check`'s typecheck and format
//!    don't reach: `tsc` over each `crates/*/js/tsconfig.json`, then
//!    `gro format --check` over every `.ts`, `.mts`, and `.js` file under
//!    `crates/` (it says so when there are none)
//! 5. `gro check`

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::cmd::{StepResult, io_failure, relative, run, run_program, target_dir};
use crate::error::XtaskError;
use crate::snapshot::{self, Measurement};
use crate::v8_archive;

/// The repo's own TypeScript compiler, relative to the root.
const TSC: &str = "node_modules/.bin/tsc";

/// Ensures the pinned V8 archive is in place and verified, and that the next
/// build rewrites any library copy missing from the target directory.
/// Returns cargo's target directory, which it reads to heal.
pub fn fetch_v8(root: &Path) -> Result<PathBuf, XtaskError> {
    let path = v8_archive::ensure(root)?;
    println!(
        "[xtask] the V8 archive is verified: {}",
        relative(root, &path)
    );
    let target_dir = target_dir(root)?;
    v8_archive::heal_missing_copies(root, &target_dir)?;
    Ok(target_dir)
}

/// Checks the V8 library copy the last build of `profile` left under
/// `target_dir` against the pin, before anything runs V8 code.
pub fn verify_linked_step(target_dir: &Path, profile: &str) -> StepResult {
    v8_archive::verify_linked(target_dir, profile)?;
    println!("[xtask] the V8 library copy matches the pinned archive");
    Ok(())
}

pub fn check(root: &Path) -> StepResult {
    check_rust(root)?;
    check_crates_js(root)?;
    run(root, &[], "gro", &["check"])
}

/// The Rust half of [`check`]: the V8 archive, the Rust gates, and the
/// snapshot build with its verification.
pub fn check_rust(root: &Path) -> StepResult {
    let target_dir = fetch_v8(root)?;
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
    // the workspace clippy builds without them, so the measurement features' code goes unlinted there
    run(
        root,
        &[],
        "cargo",
        &[
            "clippy",
            "--locked",
            "-p",
            "gro_runtime",
            "-p",
            "gro_cli",
            "--features",
            "gro_runtime/phases,gro_runtime/prices,gro_cli/phases,gro_cli/prices",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    // the first build step; check the V8 library copy before the tests run V8 code
    verify_linked_step(&target_dir, "debug")?;
    run(
        root,
        &[],
        "cargo",
        &["test", "--locked", "--workspace", "--no-fail-fast"],
    )?;
    // the measurement features' own unit tests, which the workspace's leave out
    run(
        root,
        &[],
        "cargo",
        &[
            "test",
            "--locked",
            "-p",
            "gro_runtime",
            "--features",
            "phases,prices",
            "--lib",
        ],
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
    let build = snapshot::build(root, &target_dir, false, Measurement::None)?;
    snapshot::verify(root, false, &build)
}

/// Typechecks and format-checks the JS/TS under `crates/`.
pub fn check_crates_js(root: &Path) -> StepResult {
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
        println!("[xtask] no .ts, .mts, or .js files under crates/, nothing to format-check");
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

/// Collects every `.ts`, `.mts`, and `.js` file under `dir`, skipping
/// `node_modules`. `.mjs` stays out: it's the extension for a fixture that
/// must not parse (a syntax-error case), which a format check would reject.
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
            .is_some_and(|ext| ext == "ts" || ext == "mts" || ext == "js")
        {
            sources.push(path);
        }
    }
    Ok(())
}
