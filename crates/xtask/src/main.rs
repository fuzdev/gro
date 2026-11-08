//! `cargo xtask`: dev automation for the gro repo. Every step runs from the
//! workspace root, so `cargo xtask` works the same from any directory in the
//! checkout.
//!
//! - `fetch-v8` downloads the pinned V8 archive and verifies its SHA-256
//!   ([`v8_archive`]); every cargo build links it through `RUSTY_V8_ARCHIVE`
//!   in `.cargo/config.toml`, so a fresh checkout runs this (or any xtask
//!   that builds) first
//! - `build [--release] [--verify]` builds `fgro` with an embedded startup
//!   snapshot at `<target>/gro_snapshot/<profile>/fgro` ([`snapshot`])
//! - `check`, `check-rust`, `check-js`: the repo's gates ([`check`])
//! - `vendor-patch <name>` regenerates `vendor/<name>.patch` from the
//!   vendored tree ([`vendor`])
//! - `bench <suite.json>` and `bench report`: the measurement harness
//!   ([`bench`](mod@bench))
//!
//! Exit codes are the xtask's contract: `0` success; `2` the caller must
//! change something before re-running, since unchanged it fails the same way
//! (a usage error, flags that don't combine, a wrong or failed `fgro` build,
//! a crossed budget, a bench's input, which
//! `gro_bench::error::BenchError::is_input_error` lists, and a vendored crate
//! that isn't its published crate plus its patch, or that `vendor/README.md`
//! doesn't describe); `1` anything else
//! (an artifact or linked checkout changing mid-run, a failed read or write,
//! a failed fetch). A program a step ran exits with its own code. Each
//! failure ([`error::XtaskError`], whose `exit_code` is the full mapping)
//! prints once, to stderr; `--help` prints to stdout and exits `0`. A failed
//! bench run records nothing.

mod bench;
mod check;
mod cmd;
mod error;
#[cfg(test)]
mod lints;
mod snapshot;
mod v8_archive;
mod vendor;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use argh::{EarlyExit, FromArgs};

use snapshot::Measurement;

/// The workspace root, two levels above this crate's manifest, canonicalized
/// like the history path, so a root reached through a symlink still contains
/// the history and the dirty check excludes it.
fn root() -> PathBuf {
    // `cargo run` sets it at runtime; the compile-time path goes stale when the checkout moves
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let root = manifest
        .ancestors()
        .nth(2)
        .unwrap_or(&manifest)
        .to_path_buf();
    root.canonicalize().unwrap_or(root)
}

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
    CheckRust(CheckRust),
    CheckJs(CheckJs),
    FetchV8(FetchV8),
    VendorPatch(VendorPatch),
    Build(Build),
    Bench(Bench),
}

/// fetch V8, run the Rust gates, and build and verify the snapshot fgro, the
/// steps of `check` that need no Node
#[derive(FromArgs)]
#[argh(subcommand, name = "check-rust")]
struct CheckRust {}

/// typecheck and format-check the JS/TS under crates/, the step of `check`
/// that needs Node and no Rust build
#[derive(FromArgs)]
#[argh(subcommand, name = "check-js")]
struct CheckJs {}

/// download the pinned V8 archive if it's missing, and verify its SHA-256
#[derive(FromArgs)]
#[argh(subcommand, name = "fetch-v8")]
struct FetchV8 {}

/// regenerate a vendored crate's patch, vendor/NAME.patch: its tree's diff
/// against its published crate
#[derive(FromArgs)]
#[argh(subcommand, name = "vendor-patch")]
struct VendorPatch {
    /// the vendored crate, a directory under vendor/
    #[argh(positional)]
    name: String,
}

/// build fgro with an embedded startup snapshot, in two stages
#[derive(FromArgs)]
#[argh(subcommand, name = "build")]
struct Build {
    /// build the release profile
    #[argh(switch)]
    release: bool,
    /// run the integration tests against the result and check that it runs
    /// with its build tree hidden
    #[argh(switch)]
    verify: bool,
}

/// run the Rust gates, check the JS/TS under crates/, then run `gro check`
#[derive(FromArgs)]
#[argh(subcommand, name = "check")]
struct Check {}

/// run a measurement suite and append its record to the history, or, given
/// `report`, report on the history
#[derive(FromArgs)]
#[argh(subcommand, name = "bench")]
struct Bench {
    /// the suite file (JSON), or `report` to report on the history (a suite
    /// file named `report` is `./report`)
    #[argh(positional)]
    suite: PathBuf,
    /// the history file to append to or report on (default:
    /// `crates/gro_bench/history.jsonl`)
    #[argh(option)]
    history: Option<PathBuf>,
    /// attest that the machine was quieted by hand, per the machine protocol
    #[argh(switch)]
    attest_quiet: bool,
    /// time fgro's phases: run a build with the `phases` feature in place of
    /// each fgro runtime and record each sample's phase durations
    #[argh(switch)]
    phases: bool,
    /// run each sample under `perf stat`, recording user-space instructions
    /// and cycles
    #[argh(switch)]
    counters: bool,
    /// with --counters, build fgro at this git revision too, run each fgro
    /// row against it interleaved, and print the instruction moves, recording
    /// nothing
    #[argh(option)]
    base: Option<String>,
    /// where --base clones and builds (default: `$XDG_CACHE_HOME/gro/bench_base`)
    #[argh(option)]
    base_dir: Option<PathBuf>,
    /// the timed samples per row, in place of the suite's (even)
    #[argh(option)]
    samples: Option<u32>,
}

/// The `bench` positional that reports on the history instead of running a
/// suite.
const REPORT: &str = "report";

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
        Subcommand::Check(Check {}) => check::check(&root),
        Subcommand::CheckRust(CheckRust {}) => check::check_rust(&root),
        Subcommand::CheckJs(CheckJs {}) => check::check_crates_js(&root),
        Subcommand::FetchV8(FetchV8 {}) => check::fetch_v8(&root).map(|_| ()),
        Subcommand::VendorPatch(VendorPatch { name }) => vendor::write_patch(&root, &name),
        Subcommand::Build(Build { release, verify }) => {
            check::fetch_v8(&root).and_then(|target_dir| {
                let build = snapshot::build(&root, &target_dir, release, Measurement::None)?;
                if verify {
                    snapshot::verify(&root, release, &build)?;
                }
                Ok(())
            })
        }
        Subcommand::Bench(bench_args) => bench::bench(&root, bench_args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error.print();
            ExitCode::from(error.exit_code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_canonical() {
        let root = root();
        assert_eq!(root, root.canonicalize().unwrap());
        assert!(root.join("Cargo.toml").is_file());
    }

    #[test]
    fn usage_errors_exit_2_and_help_exits_0() {
        let args = |args: &[&str]| -> Vec<String> {
            let mut all = vec!["xtask".to_owned()];
            all.extend(args.iter().map(|arg| (*arg).to_owned()));
            all
        };
        assert!(parse_args(&args(&["bench", "suite.json"])).is_ok());
        assert!(parse_args(&args(&["bench", "report"])).is_ok());
        assert!(
            parse_args(&args(&[
                "bench",
                "s.json",
                "--counters",
                "--base",
                "HEAD",
                "--samples",
                "20"
            ]))
            .is_ok()
        );
        assert!(parse_args(&args(&["bench", "s.json", "--phases"])).is_ok());
        assert!(parse_args(&args(&["build", "--release", "--verify"])).is_ok());
        assert!(parse_args(&args(&["fetch-v8"])).is_ok());
        assert!(parse_args(&args(&["check-js"])).is_ok());
        assert!(parse_args(&args(&["check-rust"])).is_ok());
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
}
