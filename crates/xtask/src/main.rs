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
//!
//! `cargo xtask bench <suite.json>` runs a measurement suite with `gro_bench`
//! and appends the run's record to the history,
//! `crates/gro_bench/history.jsonl` (`--history` writes elsewhere, relative to
//! the current directory). A failed run records nothing, and its exit code
//! says what the caller must do:
//!
//! - `0`: the run's record was appended
//! - `2`: change something before re-running, since unchanged it fails the
//!   same way: the suite is unreadable or invalid, a program, script, or file
//!   argument it names is missing, a program is a script, a workload or
//!   version probe exits unsuccessfully or prints no version, or `--history`
//!   is empty or names no file in an existing directory
//! - `1`: anything else, such as an artifact changing mid-run or a failed
//!   write
//!
//! A usage error (an unknown subcommand or flag, a missing argument) exits
//! `2` as well, printing to stderr; `--help` prints to stdout and exits `0`.

use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use argh::{EarlyExit, FromArgs};
use gro_bench::error::BenchError;
use gro_bench::record::{GitState, Machine, MachineQuiet, RunRecord};
use gro_bench::run::{RunContext, run as run_suite};
use gro_bench::suite::Suite;

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

/// The measurement history, relative to the root: one JSON line per run,
/// append-only, beside the harness that writes it.
const HISTORY: &str = "crates/gro_bench/history.jsonl";

/// The caller must change the suite (or a program it names) before re-running.
const EXIT_INPUT: u8 = 2;

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
    Bench(Bench),
}

/// run the Rust gates, check the JS/TS under crates/, then run `gro check`
#[derive(FromArgs)]
#[argh(subcommand, name = "check")]
struct Check {}

/// run a measurement suite and append its record to the history
#[derive(FromArgs)]
#[argh(subcommand, name = "bench")]
struct Bench {
    /// the suite file (JSON)
    #[argh(positional)]
    suite: PathBuf,
    /// the history file to append to (default: `crates/gro_bench/history.jsonl`)
    #[argh(option)]
    history: Option<PathBuf>,
    /// attest that the machine was quieted by hand, per the machine protocol
    #[argh(switch)]
    attest_quiet: bool,
}

/// Parses the process's arguments. Unlike `argh::from_env`, a usage error
/// exits `2` (the caller must change the command line), and `--help` prints to
/// stdout and exits `0`.
fn parse_args(args: &[String]) -> Result<Xtask, ExitCode> {
    let Some((command, args)) = args.split_first() else {
        eprintln!("[xtask] no program name, argv is empty");
        return Err(ExitCode::from(EXIT_INPUT));
    };
    // the program's file name, as `argh::from_env` names it
    let command = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    Xtask::from_args(&[command], &args).map_err(|EarlyExit { output, status }| {
        if status.is_ok() {
            println!("{output}");
            ExitCode::SUCCESS
        } else {
            eprintln!("{output}\nRun {command} --help for more information.");
            ExitCode::from(EXIT_INPUT)
        }
    })
}

fn main() -> ExitCode {
    let args: Result<Vec<String>, _> = std::env::args_os().map(OsString::into_string).collect();
    let args = match args {
        Ok(args) => args,
        Err(arg) => {
            eprintln!(
                "[xtask] invalid utf8 in an argument: {}",
                arg.to_string_lossy()
            );
            return ExitCode::from(EXIT_INPUT);
        }
    };
    let xtask = match parse_args(&args) {
        Ok(xtask) => xtask,
        Err(code) => return code,
    };
    let root = root();
    let result = match xtask.command {
        Subcommand::Check(Check {}) => check(&root),
        Subcommand::Bench(bench_args) => bench(&root, bench_args),
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

/// Runs a suite, prints a summary, and appends the record to the history.
fn bench(root: &Path, args: Bench) -> StepResult {
    let history = history_path(root, args.history).map_err(|error| {
        eprintln!("[xtask] bench failed: invalid --history: {error}");
        ExitCode::from(EXIT_INPUT)
    })?;
    let suite_dir = args
        .suite
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let result = Suite::load(&args.suite).and_then(|suite| {
        let context = RunContext {
            git: GitState::read(root, &history)?,
            machine: Machine::detect(),
            machine_quiet: if args.attest_quiet {
                MachineQuiet::Attested
            } else {
                MachineQuiet::Unchecked
            },
            corpora: None,
            workspace: BTreeMap::new(),
        };
        println!(
            "[xtask] bench {} v{}: {} rows plus the noise row, {} samples each, interleaved",
            suite.name,
            suite.version,
            suite.runtimes.len() * suite.workloads.len(),
            suite.samples,
        );
        let record = run_suite(&suite, suite_dir, context)?;
        record.append_to(&history)?;
        Ok(record)
    });
    match result {
        Ok(record) => {
            print_summary(&record);
            println!("[xtask] appended the record to {}", history.display());
            Ok(())
        }
        Err(error) => {
            eprintln!("[xtask] bench failed: {error}");
            let mut source = error.source();
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            Err(ExitCode::from(bench_exit_code(&error)))
        }
    }
}

/// The history file, absolute: `--history` against the current directory, so
/// the dirty check recognizes it inside the checkout, or else the default.
/// The directory is canonicalized (it must exist), which drops any `..` that
/// git would reject in a pathspec; the file itself needn't exist yet.
fn history_path(root: &Path, history: Option<PathBuf>) -> io::Result<PathBuf> {
    let Some(history) = history else {
        return Ok(root.join(HISTORY));
    };
    let history = std::path::absolute(history)?;
    let (Some(dir), Some(name)) = (history.parent(), history.file_name()) else {
        return Err(io::Error::other(format!(
            "{} names no file",
            history.display()
        )));
    };
    Ok(dir.canonicalize()?.join(name))
}

const fn bench_exit_code(error: &BenchError) -> u8 {
    if error.is_input_error() {
        EXIT_INPUT
    } else {
        1
    }
}

fn print_summary(record: &RunRecord) {
    println!(
        "\n{:<10} {:<16} {:>12} {:>10} {:>7} {:>12} {:>9}",
        "runtime", "workload", "median ms", "mad ms", "cv", "min ms", "outliers"
    );
    for row in &record.rows {
        let stats = &row.wall_ns.stats;
        println!(
            "{:<10} {:<16} {:>12.3} {:>10.3} {:>6.1}% {:>12.3} {:>6}/{}",
            row.runtime,
            row.workload,
            stats.median / 1e6,
            stats.mad / 1e6,
            stats.cv * 100.0,
            stats.min / 1e6,
            stats.outliers,
            stats.n,
        );
    }
    let noise = &record.noise;
    println!(
        "\nnoise ({} {} against its copy): the medians sit {:.2}% apart",
        noise.runtime,
        noise.workload,
        noise.median_deviation * 100.0,
    );
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

#[cfg(test)]
mod tests {
    use std::process::ExitStatus;

    use super::*;

    #[test]
    fn a_relative_history_is_absolute_against_the_current_directory() {
        let root = root();
        let history = history_path(&root, Some(PathBuf::from("history.jsonl"))).unwrap();
        assert!(history.is_absolute());
        assert_eq!(
            history,
            std::env::current_dir().unwrap().join("history.jsonl")
        );
        // cargo runs tests from the crate's directory, inside the checkout
        assert!(history.starts_with(&root));
        assert_eq!(history_path(&root, None).unwrap(), root.join(HISTORY));
        assert!(history_path(&root, Some(PathBuf::new())).is_err());
    }

    #[test]
    fn a_history_with_dot_dot_is_normalized() {
        let root = root();
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        // outside the checkout's root when run from `crates/xtask`, inside it otherwise
        let history = history_path(&root, Some(PathBuf::from("../x.jsonl"))).unwrap();
        assert_eq!(history, cwd.parent().unwrap().join("x.jsonl"));
        assert!(history.components().all(|part| matches!(
            part,
            std::path::Component::RootDir | std::path::Component::Normal(_)
        )));
        // `..` and `/` name no file, and the directory must exist
        assert!(history_path(&root, Some(PathBuf::from(".."))).is_err());
        assert!(history_path(&root, Some(PathBuf::from("no_such_dir/x.jsonl"))).is_err());
    }

    #[test]
    fn usage_errors_exit_2_and_help_exits_0() {
        let args = |args: &[&str]| -> Vec<String> {
            let mut all = vec!["xtask".to_owned()];
            all.extend(args.iter().map(|arg| (*arg).to_owned()));
            all
        };
        assert!(parse_args(&args(&["bench", "suite.json"])).is_ok());
        for bad in [
            &["nope"][..],
            &["bench"],
            &["bench", "s.json", "--nope"],
            &[],
        ] {
            let code = parse_args(&args(bad)).err().expect("a usage error");
            assert_eq!(code, ExitCode::from(EXIT_INPUT), "{bad:?}");
        }
        let code = parse_args(&args(&["--help"]))
            .err()
            .expect("help exits early");
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn bench_errors_map_to_their_exit_codes() {
        let io_error = || io::Error::other("test");
        let path = || PathBuf::from("suite.json");
        let input = [
            BenchError::SuiteRead {
                path: path(),
                source: io_error(),
            },
            // a manifest isn't JSON
            Suite::load(Path::new("Cargo.toml")).unwrap_err(),
            BenchError::SuiteInvalid("test".to_owned()),
            BenchError::SuiteFileMissing {
                path: path(),
                source: io_error(),
            },
            BenchError::ProgramNotFound {
                runtime: "r".to_owned(),
                program: "p".to_owned(),
            },
            BenchError::ProgramIsScript {
                runtime: "r".to_owned(),
                path: path(),
            },
            BenchError::CommandFailed {
                command: "c".to_owned(),
                status: ExitStatus::default(),
            },
            BenchError::NoVersion {
                command: "c".to_owned(),
            },
        ];
        assert!(matches!(input[1], BenchError::SuiteParse { .. }));
        for error in &input {
            assert_eq!(bench_exit_code(error), EXIT_INPUT, "{error:?}");
        }
        let other = [
            BenchError::Read {
                path: path(),
                source: io_error(),
            },
            BenchError::Spawn {
                command: "c".to_owned(),
                source: io_error(),
            },
            BenchError::ArtifactsChanged {
                paths: vec![path()],
            },
            BenchError::HistoryWrite {
                path: path(),
                source: io_error(),
            },
        ];
        for error in &other {
            assert_eq!(bench_exit_code(error), 1, "{error:?}");
        }
    }
}
