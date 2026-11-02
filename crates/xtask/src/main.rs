//! `cargo xtask`: dev automation for the gro repo.
//!
//! `cargo xtask fetch-v8` downloads the pinned V8 archive and verifies its
//! SHA-256 ([`v8_archive`]). Every cargo build links it through
//! `RUSTY_V8_ARCHIVE` in `.cargo/config.toml`, so a fresh checkout runs this
//! (or any xtask that builds) first.
//!
//! `cargo xtask build [--release]` builds `fgro` with an embedded startup
//! snapshot in two stages ([`snapshot`]), at
//! `<target>/gro_snapshot/<profile>/fgro`; `--verify` also runs the
//! integration tests against it and checks that it runs with its build tree
//! hidden.
//!
//! `cargo xtask check-rust` runs the Rust half of `check` (1 to 3 below), for
//! CI's rust job, and `cargo xtask check-js` only its JS/TS step (4 below),
//! for CI's Node job, which has no V8 build.
//!
//! `cargo xtask check` is the one local command that checks both halves of
//! the repo. It runs these steps in order, printing each command first and
//! stopping at the first failure with that step's exit code:
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
//!
//! Every step runs from the workspace root, so `cargo xtask` works the same
//! from any directory in the checkout.
//!
//! `cargo xtask bench <suite.json>` runs a measurement suite with `gro_bench`
//! and appends the run's record to the history,
//! `crates/gro_bench/history.jsonl` (`--history` writes elsewhere, relative to
//! the current directory). The copies of corpora collections its workloads run
//! inside go under the checkout's gitignored `target/bench/`. When a runtime
//! is `fgro` (named so, or with a program named so), it first builds the
//! release snapshot `fgro` (as `build --release`, cached when nothing
//! changed) and refuses the run unless that program is the binary just
//! built, so a stale, debug, or snapshot-less build is never timed. A
//! recorded run goes after `cargo xtask build --release`, so this build is a
//! cache hit and doesn't load the machine just before timing. After the
//! summary it prints each `fgro` row's median against its gate row (warm
//! rows against `node_compile_cache`, cold ones against `node`, `deno`
//! beside), or, for a suite with its own `gate`, each of the gate's runtimes
//! against its incumbent, every row it doesn't name marked reported only: a
//! report that fails nothing. `--samples N` replaces the suite's sample
//! count (even), which the record's config hash then carries.
//!
//! `--phases` builds `fgro` with its `phases` feature too
//! (`<target>/gro_snapshot/release-phases/fgro`), runs that build in place of
//! each `fgro` runtime, and records each sample's phase durations
//! (`gro_bench::phases`). `--counters` runs each sample under `perf stat`
//! and records its user-space instructions and cycles
//! (`gro_bench::counters`), refusing when perf can't count them in full.
//! Either is the record's `mode`; they don't combine.
//!
//! A suite with `prices` builds `fgro` with its `prices` feature
//! (`<target>/gro_snapshot/release-prices/fgro`), refuses it unless its
//! `--version` says `, prices`, runs it in place of every runtime (each must
//! be `fgro`), and after the summary prints the price table
//! (`gro_bench::prices`); it takes `--counters`, not `--phases` or `--base`.
//! The release build is refused when its `--version` names either
//! measurement build. The record carries the size of the release build's
//! snapshot (`snapshot.bin` beside it), which a suite's `budgets` can limit
//! with the executable's sizes: a run over one fails before its samples.
//!
//! `--counters --base <rev>` is the A/B for the perf rule: it builds the
//! release snapshot `fgro` at `rev` too, in a clone of its own outside the
//! checkout (`bench::base`, under `--base-dir`), runs each `fgro` runtime beside a
//! copy on that build (`<name>_base`), interleaved, with every other runtime
//! left out, and prints each row's instruction, cycle, and wall medians
//! against the base's, flagging an instruction move past 2%. It records
//! nothing.
//!
//! `cargo xtask bench report` reads the history (`--history` too) and
//! prints, per suite version and mode, each row's latest record against the
//! one before it and the last attested one (`gro_bench::report`).
//!
//! A failed run records nothing, and its exit code says what the caller must
//! do:
//!
//! - `0`: the run's record was appended
//! - `2`: change something before re-running, since unchanged it fails the
//!   same way: the suite is unreadable or invalid, a program, script, or file
//!   argument it names is missing, a program is a script, a workload or
//!   version probe exits unsuccessfully, prints no version, or runs past the
//!   suite's `timeout_ms` (two minutes by default), another run holds the
//!   work directory (`target/bench/`) or another A/B the `--base` build's
//!   directory, the suite's
//!   corpora checkout isn't at its pinned tree or has local changes, a
//!   collection a workload names isn't in the pinned tree or holds a
//!   symlink, a submodule, or a name that isn't UTF-8, a workload's entry
//!   would replace a file of its collection, a workload's project holds a
//!   symlink or a `node_modules` or links a missing checkout or a directory
//!   in no git checkout, a row's
//!   untimed correctness run fails (a nonzero exit, or stdout not what its
//!   runtime's `stdout` says) or breaks the suite's `parity`, `--history` is
//!   empty, names a directory, or names no file in an existing directory, or
//!   the `fgro` it names isn't the release snapshot build (or that build's
//!   compile, a measurement build's, or the `--base` revision's, failed), a
//!   phases run's program isn't a phases build, a size crosses its budget,
//!   perf can't count, the flags don't combine, or
//!   `--base` names no commit, or the history `report` reads is missing
//! - `1`: anything else, such as an artifact or a linked checkout changing
//!   mid-run, a failed read or write, or a failed fetch of the V8 archive or
//!   `cargo metadata` before a build
//!
//! A usage error (an unknown subcommand or flag, a missing argument) exits
//! `2` as well, printing to stderr; `--help` prints to stdout and exits `0`.
//! Every other step's failure ([`error::XtaskError`]) prints once, to
//! stderr, and exits with its code: a program a step ran exits with its own.

mod bench;
mod check;
mod cmd;
mod error;
#[cfg(test)]
mod lints;
mod snapshot;
mod v8_archive;

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
