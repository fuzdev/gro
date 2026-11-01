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
//! checkout ([`base`], under `--base-dir`), runs each `fgro` runtime beside a
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
//!   version probe exits unsuccessfully or prints no version, the suite's
//!   corpora checkout isn't at its pinned tree or has local changes, a
//!   collection a workload names isn't in the pinned tree or holds a
//!   symlink, a submodule, or a name that isn't UTF-8, a workload's entry
//!   would replace a file of its collection, a workload's project holds a
//!   symlink or a `node_modules` or links a missing checkout or a directory
//!   in no git checkout, a row's
//!   untimed correctness run fails (a nonzero exit, or stdout not what its
//!   runtime's `stdout` says) or breaks the suite's `parity`, `--history` is
//!   empty, names a directory, or names no file in an existing directory, or
//!   the `fgro` it names isn't the release snapshot build (or that build, or
//!   a measurement build, failed), a size crosses its budget, perf can't
//!   count, the flags don't combine, or
//!   `--base` names no commit, or the history `report` reads is missing
//! - `1`: anything else, such as an artifact or a linked checkout changing
//!   mid-run or a failed write
//!
//! A usage error (an unknown subcommand or flag, a missing argument) exits
//! `2` as well, printing to stderr; `--help` prints to stdout and exits `0`.

use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use argh::{EarlyExit, FromArgs};
use gro_bench::error::BenchError;
use gro_bench::record::{GitState, Machine, MachineQuiet, Mode, RunRecord};
use gro_bench::report::INSTRUCTIONS_FLAG;
use gro_bench::run::{Measure, RunContext, run as run_suite};
use gro_bench::suite::{Gate, Runtime, Suite};

mod base;
mod snapshot;
mod v8_archive;

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

/// The repo's own TypeScript compiler, relative to the root.
const TSC: &str = "node_modules/.bin/tsc";

/// The measurement history, relative to the root: one JSON line per run,
/// append-only, beside the harness that writes it.
const HISTORY: &str = "crates/gro_bench/history.jsonl";

/// Where a bench writes, relative to the root: its copies of corpora
/// collections, under the gitignored `target/`.
const BENCH_DIR: &str = "target/bench";

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
        Subcommand::Check(Check {}) => check(&root),
        Subcommand::CheckRust(CheckRust {}) => check_rust(&root),
        Subcommand::CheckJs(CheckJs {}) => check_crates_js(&root),
        Subcommand::FetchV8(FetchV8 {}) => fetch_v8(&root),
        Subcommand::Build(Build { release, verify }) => fetch_v8(&root).and_then(|()| {
            let target_dir = target_dir(&root)?;
            let build = snapshot::build(&root, &target_dir, release, Measurement::None)?;
            if verify {
                snapshot::verify(&root, release, &build)?;
            }
            Ok(())
        }),
        Subcommand::Bench(bench_args) => bench(&root, bench_args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// A failed step: the exit code `cargo xtask` returns.
type StepResult = Result<(), ExitCode>;

/// Ensures the pinned V8 archive is in place and verified.
fn fetch_v8(root: &Path) -> StepResult {
    match v8_archive::ensure(root) {
        Ok(path) => {
            println!(
                "[xtask] the V8 archive is verified: {}",
                relative(root, &path)
            );
            Ok(())
        }
        Err(error) => {
            eprintln!("[xtask] {error}");
            Err(ExitCode::FAILURE)
        }
    }
}

/// Cargo's target directory, as `cargo metadata` reports it, so a
/// `CARGO_TARGET_DIR` or `build.target-dir` setting is honored. The V8
/// archive's path doesn't depend on it: `.cargo/config.toml` names it relative
/// to the checkout.
fn target_dir(root: &Path) -> Result<PathBuf, ExitCode> {
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

/// Checks the V8 library copy the last build of `profile` left under
/// `target_dir` against the pin, before anything runs V8 code.
fn verify_linked_step(target_dir: &Path, profile: &str) -> StepResult {
    v8_archive::verify_linked(target_dir, profile).map_err(|error| {
        eprintln!("[xtask] {error}");
        ExitCode::FAILURE
    })?;
    println!("[xtask] the V8 library copy matches the pinned archive");
    Ok(())
}

fn check(root: &Path) -> StepResult {
    check_rust(root)?;
    check_crates_js(root)?;
    run(root, &[], "gro", &["check"])
}

/// The Rust half of [`check`]: the V8 archive, the Rust gates, and the
/// snapshot build with its verification.
fn check_rust(root: &Path) -> StepResult {
    fetch_v8(root)?;
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
    let target_dir = target_dir(root)?;
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

/// Runs a suite, prints a summary, and appends the record to the history;
/// or, with `report`, reports on the history; or, with `--base`, runs the
/// A/B and prints it.
fn bench(root: &Path, args: Bench) -> StepResult {
    let usage = |message: &str| {
        eprintln!("[xtask] bench: {message}");
        Err(ExitCode::from(EXIT_INPUT))
    };
    if args.suite == Path::new(REPORT) {
        if args.attest_quiet
            || args.phases
            || args.counters
            || args.base.is_some()
            || args.base_dir.is_some()
            || args.samples.is_some()
        {
            return usage("`report` takes only --history");
        }
        return bench_report(root, args.history);
    }
    if args.phases && args.counters {
        return usage("--phases and --counters measure apart: a phases build counts more");
    }
    if args.base.is_some() && !args.counters {
        return usage("--base compares instruction counts, so it needs --counters");
    }
    if args.base_dir.is_some() && args.base.is_none() {
        return usage("--base-dir is where --base builds, so it needs --base");
    }
    if args.base.is_some() {
        let set = base::codegen_overrides(
            std::env::vars_os().map(|(name, _)| name.to_string_lossy().into_owned()),
        );
        if !set.is_empty() {
            return usage(&format!(
                "--base builds the base from a cleared environment, so the head's build would take \
                 {} and the base's wouldn't; unset them for the A/B",
                set.join(", ")
            ));
        }
    }
    let history = history_path(root, args.history).map_err(|error| {
        eprintln!("[xtask] bench failed: invalid --history: {error}");
        ExitCode::from(EXIT_INPUT)
    })?;
    let suite_dir = args
        .suite
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut suite = Suite::load(&args.suite).map_err(|error| bench_failed(&error))?;
    if let Some(samples) = args.samples {
        let Some(samples) = NonZeroU32::new(samples) else {
            return usage("--samples is at least 2");
        };
        suite.samples = samples;
        suite.validate().map_err(|error| bench_failed(&error))?;
    }
    if suite.prices.is_some() {
        if let Some(runtime) = suite.runtimes.iter().find(|runtime| !is_fgro(runtime)) {
            return usage(&format!(
                "a prices suite prices fgro's defaults, and its runtime `{}` isn't fgro",
                runtime.name
            ));
        }
        if args.phases || args.base.is_some() {
            return usage(
                "a prices suite runs the priced build, which --phases and --base would replace",
            );
        }
    }
    let release = build_fgro(root, &suite, suite_dir)?;
    // beside the release binary, as `cargo xtask build` writes it
    let snapshot = release.map(|binary| binary.with_file_name("snapshot.bin"));
    let fgro_names: BTreeSet<String> = suite
        .runtimes
        .iter()
        .filter(|runtime| is_fgro(runtime))
        .map(|runtime| runtime.name.clone())
        .collect();
    if suite.prices.is_some() {
        let target_dir = target_dir(root).map_err(|_| ExitCode::from(EXIT_INPUT))?;
        let built = snapshot::build(root, &target_dir, true, Measurement::Prices)
            .map_err(|_| ExitCode::from(EXIT_INPUT))?
            .binary;
        require_marker(&built, PRICES_MARKER)?;
        for runtime in &mut suite.runtimes {
            runtime.program = built.display().to_string();
        }
        println!();
    }
    let measure = if args.phases {
        if fgro_names.is_empty() {
            return usage("--phases times fgro's phases, and the suite runs no fgro");
        }
        let target_dir = target_dir(root).map_err(|_| ExitCode::from(EXIT_INPUT))?;
        let built = snapshot::build(root, &target_dir, true, Measurement::Phases)
            .map_err(|_| ExitCode::from(EXIT_INPUT))?
            .binary;
        for runtime in &mut suite.runtimes {
            if fgro_names.contains(&runtime.name) {
                runtime.program = built.display().to_string();
            }
        }
        println!();
        Measure::Phases {
            runtimes: fgro_names.clone(),
        }
    } else if args.counters {
        Measure::Counters
    } else {
        Measure::Wall
    };
    let base = match &args.base {
        Some(rev) => {
            if fgro_names.is_empty() {
                return usage("--base compares fgro builds, and the suite runs no fgro");
            }
            let sha = base::resolve(root, rev)?;
            let dir = base::base_dir(args.base_dir)?;
            let lock = base::lock(&dir)?;
            let built = base::build(root, &dir, &sha, &lock)?;
            suite = base_suite(&suite, &fgro_names, &built.binary);
            // the toolchain the head's build ran: this process's, which
            // `cargo xtask` started under the checkout's pin
            let mut rustc = Command::new("rustc");
            rustc.current_dir(root);
            let head_rustc = base::rustc_version(rustc)?;
            Some(BaseRun {
                rev: rev.clone(),
                sha,
                rustc: built.rustc,
                head_rustc,
                _lock: lock,
            })
        }
        None => None,
    };
    let result = GitState::read(root, &history).and_then(|git| {
        let context = RunContext {
            git,
            machine: Machine::detect(),
            machine_quiet: if args.attest_quiet {
                MachineQuiet::Attested
            } else {
                MachineQuiet::Unchecked
            },
            work_dir: root.join(BENCH_DIR),
            measure,
            snapshot,
        };
        println!(
            "[xtask] bench {} v{}: {} rows plus the noise row, {} samples each, interleaved",
            suite.name,
            suite.version,
            suite.rows().count(),
            suite.samples,
        );
        let record = run_suite(&suite, suite_dir, context)?;
        if base.is_none() {
            record.append_to(&history)?;
        }
        Ok(record)
    });
    let record = result.map_err(|error| bench_failed(&error))?;
    if let Some(tool) = &record.counters
        && !tool
            .events
            .iter()
            .any(|event| event == gro_bench::counters::KERNEL_EVENT)
    {
        println!(
            "[xtask] perf can't count the kernel's instructions for this user (`perf_event_paranoid` above 1), so this run counted user space alone"
        );
    }
    print_summary(&record);
    if let Some(base) = &base {
        print_base(&record, base);
        println!("[xtask] an A/B records nothing");
        return Ok(());
    }
    if let Some(prices) = &suite.prices {
        println!();
        print!("{}", gro_bench::prices::table(&record, prices));
    } else if record.mode == Mode::Wall {
        print_gate(&suite, &record);
    } else {
        println!("\n(no gate table: it reads wall times, which this mode's processes inflate)");
    }
    println!("[xtask] appended the record to {}", history.display());
    Ok(())
}

/// The suite an A/B runs: its fgro runtimes (`fgro_names`), each beside a
/// copy named `<name>_base` running `base_binary`, and nothing else, with no
/// `parity` or `gate`, since the copies are the comparison.
fn base_suite(suite: &Suite, fgro_names: &BTreeSet<String>, base_binary: &Path) -> Suite {
    let mut suite = suite.clone();
    suite.runtimes = suite
        .runtimes
        .iter()
        .filter(|runtime| fgro_names.contains(&runtime.name))
        .flat_map(|runtime| {
            let mut base = runtime.clone();
            base.name = format!("{}{BASE_SUFFIX}", runtime.name);
            base.program = base_binary.display().to_string();
            [runtime.clone(), base]
        })
        .collect();
    suite.parity.clear();
    suite.gate = None;
    suite
}

/// The suffix of an A/B's base copy of a runtime.
const BASE_SUFFIX: &str = "_base";

/// What an A/B compared against.
#[derive(Debug)]
struct BaseRun {
    /// `--base` as given.
    rev: String,
    sha: String,
    /// The `rustc --version` line of the base's toolchain.
    rustc: String,
    /// The `rustc --version` line of the head's.
    head_rustc: String,
    /// The base dir's lock, held from the base's build to the end of the
    /// run.
    _lock: base::BaseLock,
}

/// Prints an A/B's moves: each fgro row's median instructions, cycles, and
/// wall time against its base copy's, an instruction move past
/// [`INSTRUCTIONS_FLAG`] (the perf rule's 2%) flagged.
fn print_base(record: &RunRecord, base: &BaseRun) {
    println!(
        "\n[xtask] A/B: this checkout ({}{}) against --base {} ({}), same session, interleaved",
        record.git.sha.get(..12).unwrap_or(&record.git.sha),
        if record.git.dirty { "-dirty" } else { "" },
        base.rev,
        base.sha,
    );
    println!(
        "[xtask] toolchains: this checkout {}, the base {}",
        base.head_rustc, base.rustc
    );
    println!(
        "{:<10} {:<16} {:>14} {:>14} {:>9} {:>9} {:>9}",
        "runtime", "workload", "instructions", "base", "x instr", "x cycles", "x wall"
    );
    let mut flagged = 0;
    for row in &record.rows {
        let Some(base) = record.rows.iter().find(|base| {
            base.workload == row.workload && base.runtime == format!("{}{BASE_SUFFIX}", row.runtime)
        }) else {
            continue;
        };
        let median = |metric: Option<&gro_bench::record::Metric>| {
            metric.map_or(f64::NAN, |metric| metric.stats.median)
        };
        let instructions = median(row.instructions.as_ref());
        let base_instructions = median(base.instructions.as_ref());
        let ratio = instructions / base_instructions;
        let moved = (ratio - 1.0).abs() > INSTRUCTIONS_FLAG;
        if moved {
            flagged += 1;
        }
        println!(
            "{:<10} {:<16} {:>14.0} {:>14.0} {:>8.4}x{} {:>8.4}x {:>8.4}x",
            row.runtime,
            row.workload,
            instructions,
            base_instructions,
            ratio,
            if moved { "!" } else { " " },
            median(row.cycles.as_ref()) / median(base.cycles.as_ref()),
            row.wall_ns.stats.median / base.wall_ns.stats.median,
        );
    }
    println!(
        "(user-space medians, this checkout over the base; the noise row's instructions sat {} apart; `!` marks a move past {:.0}%, which needs a written reason)",
        record.noise.instructions_median_deviation.map_or_else(
            || "-".to_owned(),
            |deviation| format!("{:.3}%", deviation * 100.0)
        ),
        INSTRUCTIONS_FLAG * 100.0
    );
    if flagged > 0 {
        println!("[xtask] {flagged} row(s) moved past the perf rule");
    }
}

/// Prints the report on the history (`--history`, or the default).
fn bench_report(root: &Path, history: Option<PathBuf>) -> StepResult {
    let history = history_path(root, history).map_err(|error| {
        eprintln!("[xtask] bench report: invalid --history: {error}");
        ExitCode::from(EXIT_INPUT)
    })?;
    let text = fs::read_to_string(&history).map_err(|error| {
        eprintln!(
            "[xtask] bench report: can't read {}: {error}",
            history.display()
        );
        ExitCode::from(EXIT_INPUT)
    })?;
    println!("[xtask] the history {}", history.display());
    print!("{}", gro_bench::report::report(&text));
    Ok(())
}

/// Prints a failed bench's error with its causes, and returns its exit code.
fn bench_failed(error: &BenchError) -> ExitCode {
    eprintln!("[xtask] bench failed: {error}");
    let mut source = error.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
    ExitCode::from(bench_exit_code(error))
}

/// The runtime the bench builds before it runs: gro's own `fgro`.
const FGRO: &str = "fgro";

/// Whether a runtime is `fgro`, by its name or its program's file name. One
/// rule for both the build-and-refuse check and the gate report, so a runtime
/// can't pass as `fgro` in one and not the other.
fn is_fgro(runtime: &Runtime) -> bool {
    runtime.name == FGRO || Path::new(&runtime.program).file_name() == Some(OsStr::new(FGRO))
}

/// Builds the release snapshot `fgro` when the suite has an `fgro` runtime
/// ([`is_fgro`]), and refuses the run unless each one's program is the binary
/// just built: the only `fgro` worth timing is the release build with its
/// startup snapshot, the one path that verifies the release V8 library.
/// Building first (cached when nothing changed) means a stale or debug binary
/// is never timed. A failed build has printed its own error, and exits `2`,
/// since the run can't go ahead until it's fixed. Returns the binary built,
/// when the suite has an `fgro`.
fn build_fgro(root: &Path, suite: &Suite, suite_dir: &Path) -> Result<Option<PathBuf>, ExitCode> {
    if !suite.runtimes.iter().any(is_fgro) {
        return Ok(None);
    }
    let input = |_| ExitCode::from(EXIT_INPUT);
    fetch_v8(root).map_err(input)?;
    let target_dir = target_dir(root).map_err(input)?;
    let built = snapshot::build(root, &target_dir, true, Measurement::None)
        .map_err(input)?
        .binary;
    let built = built.canonicalize().map_err(|error| {
        eprintln!("[xtask] bench failed: {}: {error}", built.display());
        ExitCode::FAILURE
    })?;
    check_fgro_programs(suite, suite_dir, &built).map_err(|error| bench_failed(&error))?;
    refuse_measurement_build(&built)?;
    println!();
    Ok(Some(built))
}

/// The marker a phases build's `--version` line carries.
const PHASES_MARKER: &str = ", phases";

/// The marker a priced build's `--version` line carries.
const PRICES_MARKER: &str = ", prices";

/// `built --version`'s first line, failing when it doesn't run.
fn version_line(built: &Path) -> Result<String, ExitCode> {
    let output = Command::new(built)
        .arg("--version")
        .output()
        .map_err(|error| {
            eprintln!(
                "[xtask] bench failed: {} --version: {error}",
                built.display()
            );
            ExitCode::FAILURE
        })?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() {
        eprintln!(
            "[xtask] bench failed: {} doesn't run ({}): {line}",
            built.display(),
            output.status,
        );
        return Err(ExitCode::from(EXIT_INPUT));
    }
    Ok(line)
}

/// Refuses a release build that is a measurement build (phases or priced),
/// which a feature unified in from elsewhere in the build graph would make:
/// it would time what a release doesn't run.
fn refuse_measurement_build(built: &Path) -> StepResult {
    let line = version_line(built)?;
    if line.contains(PHASES_MARKER) || line.contains(PRICES_MARKER) {
        eprintln!(
            "[xtask] bench failed: the release build {} is a measurement build: {line}",
            built.display(),
        );
        return Err(ExitCode::from(EXIT_INPUT));
    }
    Ok(())
}

/// Refuses a measurement build whose `--version` lacks its `marker`, which
/// would time the release under its name: a priced build that reads no
/// switches prices every default at nothing.
fn require_marker(built: &Path, marker: &str) -> StepResult {
    let line = version_line(built)?;
    if !line.contains(marker) {
        eprintln!(
            "[xtask] bench failed: {} isn't the measurement build it was built as (no `{marker}`): {line}",
            built.display(),
        );
        return Err(ExitCode::from(EXIT_INPUT));
    }
    Ok(())
}

/// Refuses an `fgro` runtime whose program doesn't resolve to `built`.
fn check_fgro_programs(suite: &Suite, suite_dir: &Path, built: &Path) -> Result<(), BenchError> {
    for runtime in suite.runtimes.iter().filter(|runtime| is_fgro(runtime)) {
        let found = gro_bench::run::find_program(&runtime.program, suite_dir);
        if found.as_deref() != Some(built) {
            return Err(BenchError::SuiteInvalid(format!(
                "the runtime `{}` names {}, which isn't the release snapshot build at {}",
                runtime.name,
                found.map_or_else(
                    || runtime.program.clone(),
                    |path| path.display().to_string()
                ),
                built.display()
            )));
        }
    }
    Ok(())
}

/// The history file, absolute: `--history` against the current directory, so
/// the dirty check recognizes it inside the checkout, or else the default.
/// The directory is canonicalized (it must exist), which drops any `..` that
/// git would reject in a pathspec; the file itself needn't exist yet, but
/// mustn't be a directory, which the append would fail on only after the run.
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
    let history = dir.canonicalize()?.join(name);
    if history.is_dir() {
        return Err(io::Error::other(format!(
            "{} is a directory",
            history.display()
        )));
    }
    Ok(history)
}

const fn bench_exit_code(error: &BenchError) -> u8 {
    if error.is_input_error() {
        EXIT_INPUT
    } else {
        1
    }
}

fn print_summary(record: &RunRecord) {
    // the runtime column fits the longest name
    let width = record
        .runtimes
        .iter()
        .map(|runtime| runtime.name.len())
        .fold("runtime".len(), usize::max);
    println!(
        "\n{:<width$} {:<16} {:>12} {:>10} {:>7} {:>12} {:>9}",
        "runtime", "workload", "median ms", "mad ms", "cv", "min ms", "outliers"
    );
    for row in &record.rows {
        let stats = &row.wall_ns.stats;
        println!(
            "{:<width$} {:<16} {:>12.3} {:>10.3} {:>6.1}% {:>12.3} {:>6}/{}",
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
    if record.rows.iter().any(|row| row.instructions.is_some()) {
        println!(
            "\n{:<width$} {:<16} {:>14} {:>9} {:>14} {:>14} {:>14}",
            "runtime", "workload", "instructions", "cv", "min", "cycles", "kernel instr"
        );
        for row in &record.rows {
            let (Some(instructions), Some(cycles)) = (&row.instructions, &row.cycles) else {
                continue;
            };
            println!(
                "{:<width$} {:<16} {:>14.0} {:>8.3}% {:>14.0} {:>14.0} {:>14}",
                row.runtime,
                row.workload,
                instructions.stats.median,
                instructions.stats.cv * 100.0,
                instructions.stats.min,
                cycles.stats.median,
                row.kernel_instructions
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), |k| format!("{:.0}", k.stats.median)),
            );
        }
        println!(
            "(medians under perf stat, user space but the kernel's instructions; the wall times above include perf's startup)"
        );
    }
    if record.rows.iter().any(|row| !row.phases.is_empty()) {
        println!(
            "\nphase medians in ms (start: spawn to entry; reap: the last mark to the reaped exit)"
        );
        for row in record.rows.iter().filter(|row| !row.phases.is_empty()) {
            let phases: Vec<String> = row
                .phases
                .iter()
                .map(|phase| format!("{} {:.3}", phase.name, phase.ns.stats.median / 1e6))
                .collect();
            println!("{} {}: {}", row.runtime, row.workload, phases.join(", "));
        }
    }
    if record.rows.iter().any(|row| row.peak_rss_kib.is_some()) {
        let mb_of = |kib: f64| kib * 1024.0 / 1e6;
        println!(
            "\n{:<width$} {:<16} {:>10} {:>10}",
            "runtime", "workload", "peak MB", "max MB"
        );
        for row in &record.rows {
            let Some(peak) = &row.peak_rss_kib else {
                continue;
            };
            #[expect(
                clippy::cast_precision_loss,
                reason = "peaks in KiB are far below 2^52"
            )]
            let max = peak.max() as f64;
            println!(
                "{:<width$} {:<16} {:>9.1}{} {:>10.1}",
                row.runtime,
                row.workload,
                mb_of(peak.stats.median),
                if gro_bench::report::near_rss_floor(peak, record.rss_floor_kib) {
                    "*"
                } else {
                    " "
                },
                mb_of(max),
            );
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "peaks in KiB are far below 2^52"
        )]
        let floor = record.rss_floor_kib.map_or_else(
            || "-".to_owned(),
            |kib| format!("{:.1} MB", mb_of(kib as f64)),
        );
        println!(
            "(peak resident memory per process, median and max, as reaped; the floor every sample reads at least is {floor}, and `*` marks a row within {:.0}% of it, so not the runtime's own)",
            (gro_bench::report::RSS_FLOOR_MARGIN - 1.0) * 100.0
        );
    }
    // the version column fits the longest line, fgro's with its full commit
    let version_width = record
        .runtimes
        .iter()
        .map(|runtime| runtime.version.len())
        .fold("version".len(), usize::max);
    println!(
        "\n{:<width$} {:<version_width$} {:>10} {:>10}",
        "runtime", "version", "MB", "gzip MB"
    );
    for runtime in &record.runtimes {
        println!(
            "{:<width$} {:<version_width$} {:>10.1} {:>10.1}",
            runtime.name,
            runtime.version,
            mb(runtime.bytes),
            mb(runtime.gzip_bytes),
        );
    }
    if let Some(bytes) = record.snapshot_bytes {
        println!("fgro's startup snapshot: {:.1} MB", mb(bytes));
    }
    let noise = &record.noise;
    println!(
        "\nnoise ({} {} against its copy): the medians sit {:.2}% apart{}",
        noise.runtime,
        noise.workload,
        noise.median_deviation * 100.0,
        noise
            .instructions_median_deviation
            .map_or_else(String::new, |deviation| format!(
                ", the instruction medians {:.3}%",
                deviation * 100.0
            )),
    );
}

/// The incumbent a warm row gates against, Node at its best configuration.
const GATE_WARM: &str = "node_compile_cache";
/// The incumbent a cold row gates against.
const GATE_COLD: &str = "node";
/// Reported beside the gate.
const BESIDE: &str = "deno";

/// Prints the gate table, report only; nothing fails on it: the suite's own
/// `gate` when it has one ([`print_suite_gate`]), else each `fgro` row's
/// median ([`is_fgro`] runtimes only) against its gate row's, same-session:
/// warm rows gate against Node with its compile cache, cold rows against
/// plain Node, with Deno beside.
fn print_gate(suite: &Suite, record: &RunRecord) {
    if let Some(gate) = &suite.gate {
        print_suite_gate(suite, gate, record);
        return;
    }
    let median = |runtime: &str, workload: &str| {
        record
            .rows
            .iter()
            .find(|row| row.runtime == runtime && row.workload == workload)
            .map(|row| row.wall_ns.stats.median / 1e6)
    };
    let ms =
        |value: Option<f64>| value.map_or_else(|| "-".to_owned(), |value| format!("{value:.3}"));
    let mut lines = Vec::new();
    for (workload, runtime) in suite.rows().filter(|(_, runtime)| is_fgro(runtime)) {
        let Some(fgro) = median(&runtime.name, &workload.name) else {
            continue;
        };
        let gate = if workload.cold { GATE_COLD } else { GATE_WARM };
        let ratio = |other: Option<f64>| {
            other.map_or_else(|| "-".to_owned(), |other| format!("{:.2}x", fgro / other))
        };
        lines.push(format!(
            "{:<10} {:<16} {:>10.3} {:<18} {:>10} {:>9} {:>10} {:>9}",
            runtime.name,
            workload.name,
            fgro,
            gate,
            ms(median(gate, &workload.name)),
            ratio(median(gate, &workload.name)),
            ms(median(BESIDE, &workload.name)),
            ratio(median(BESIDE, &workload.name)),
        ));
    }
    if lines.is_empty() {
        return;
    }
    println!(
        "\n{:<10} {:<16} {:>10} {:<18} {:>10} {:>9} {:>10} {:>9}",
        "runtime", "workload", "median ms", "gate", "gate ms", "x gate", "deno ms", "x deno"
    );
    for line in lines {
        println!("{line}");
    }
    println!(
        "(medians, same session; the noise row ({} {} against its copy) sat {:.2}% apart, and each row's mad above bounds its own spread)",
        record.noise.runtime,
        record.noise.workload,
        record.noise.median_deviation * 100.0
    );
}

/// The median of the row `runtime` × `workload`, in milliseconds.
fn median_ms(record: &RunRecord, runtime: &str, workload: &str) -> Option<f64> {
    record
        .rows
        .iter()
        .find(|row| row.runtime == runtime && row.workload == workload)
        .map(|row| row.wall_ns.stats.median / 1e6)
}

/// Prints a suite's own `gate`: each of its runtimes on every workload
/// against `gate.against`, marked `gate` on the workloads it names and
/// `report` on the rest, then the runtimes reported only. Report only.
fn print_suite_gate(suite: &Suite, gate: &Gate, record: &RunRecord) {
    let ms =
        |value: Option<f64>| value.map_or_else(|| "-".to_owned(), |value| format!("{value:.3}"));
    println!(
        "\n{:<10} {:<16} {:>10} {:<10} {:>10} {:>9} {:<6}",
        "runtime", "workload", "median ms", "against", "its ms", "x it", "row"
    );
    for runtime in &gate.runtimes {
        for workload in &suite.workloads {
            let median = median_ms(record, runtime, &workload.name);
            let against = median_ms(record, &gate.against, &workload.name);
            let ratio = median
                .zip(against)
                .map_or_else(|| "-".to_owned(), |(a, b)| format!("{:.2}x", a / b));
            let role = if gate.workloads.contains(&workload.name) {
                "gate"
            } else {
                "report"
            };
            println!(
                "{:<10} {:<16} {:>10} {:<10} {:>10} {:>9} {:<6}",
                runtime,
                workload.name,
                ms(median),
                gate.against,
                ms(against),
                ratio,
                role,
            );
        }
    }
    let reported: Vec<&str> = suite
        .runtimes
        .iter()
        .map(|runtime| runtime.name.as_str())
        .filter(|name| *name != gate.against && !gate.runtimes.iter().any(|gated| gated == name))
        .collect();
    if !reported.is_empty() {
        println!(
            "reported only, in the summary above: {}",
            reported.join(", ")
        );
    }
    println!(
        "(medians, same session; the noise row ({} {} against its copy) sat {:.2}% apart, and each row's mad above bounds its own spread)",
        record.noise.runtime,
        record.noise.workload,
        record.noise.median_deviation * 100.0
    );
}

/// Bytes in megabytes (10^6), for printing.
#[expect(
    clippy::cast_precision_loss,
    reason = "a binary's size is far below 2^52 bytes"
)]
fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1e6
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
    run_program_to(root, env, program, display, args, Stdio::inherit())
}

/// [`run_program`] with the program's stdout sent to `stdout`; its stderr,
/// where failures print, is always inherited.
fn run_program_to(
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

    use gro_bench::check::Difference;

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
    fn a_history_naming_a_directory_is_rejected() {
        let root = root();
        // cargo runs tests from `crates/xtask`, which holds `src/`
        assert!(Path::new("src").is_dir());
        let error = history_path(&root, Some(PathBuf::from("src"))).unwrap_err();
        assert!(error.to_string().ends_with("is a directory"), "{error}");
        assert!(history_path(&root, Some(PathBuf::from("src/"))).is_err());
    }

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

    #[test]
    fn an_fgro_runtime_must_be_the_build_by_name_or_by_program() {
        let suite = |runtimes: &str| -> Suite {
            serde_json::from_str(&format!(
                r#"{{"name": "s", "version": 1, "samples": 2, "runtimes": [{runtimes}],
                    "workloads": [{{"name": "w", "script": "w.js"}}]}}"#
            ))
            .unwrap()
        };
        let dir = root();
        // stands in for the built binary: any existing executable, canonical
        let built = gro_bench::run::find_program("cargo", &dir).expect("cargo is on PATH");
        let other = gro_bench::run::find_program("sh", &dir).expect("sh is on PATH");
        assert_ne!(built, other);
        let refused = |runtimes: &str| {
            matches!(
                check_fgro_programs(&suite(runtimes), &dir, &built),
                Err(BenchError::SuiteInvalid(ref message)) if message.contains("release snapshot build")
            )
        };
        let program = other.display();
        // named fgro, another program
        assert!(refused(&format!(
            r#"{{"name": "fgro", "program": "{program}"}}"#
        )));
        // another name, a program named fgro that isn't the build
        assert!(refused(r#"{"name": "mine", "program": "./missing/fgro"}"#));
        // neither: not fgro, so not checked
        assert!(!refused(&format!(
            r#"{{"name": "sh", "program": "{program}"}}"#
        )));
        // the build itself passes
        let built_program = built.display();
        assert!(!refused(&format!(
            r#"{{"name": "fgro", "program": "{built_program}"}}"#
        )));
    }

    /// The fakes are committed executables, not written by the test, since
    /// a file just written can't run while another thread's fork holds it
    /// open for writing (`ETXTBSY`).
    #[test]
    fn a_measurement_build_is_refused_as_the_timed_release() {
        let fakes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/measurement_guard");
        assert_eq!(refuse_measurement_build(&fakes.join("release")), Ok(()));
        for build in ["phases", "prices"] {
            assert_eq!(
                refuse_measurement_build(&fakes.join(build)),
                Err(ExitCode::from(EXIT_INPUT)),
                "{build}"
            );
        }
        // and a priced build must say it is one
        assert_eq!(require_marker(&fakes.join("prices"), PRICES_MARKER), Ok(()));
        assert_eq!(
            require_marker(&fakes.join("release"), PRICES_MARKER),
            Err(ExitCode::from(EXIT_INPUT))
        );
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
            BenchError::ProjectLinkMissing {
                workload: "w".to_owned(),
                package: "p".to_owned(),
                path: path(),
                source: io_error(),
            },
            BenchError::ProjectLinkUnversioned {
                workload: "w".to_owned(),
                package: "p".to_owned(),
                path: path(),
            },
            BenchError::ProjectEntry {
                workload: "w".to_owned(),
                path: path(),
                reason: "r".to_owned(),
            },
            BenchError::CheckFailed {
                command: "c".to_owned(),
                reason: "r".to_owned(),
            },
            BenchError::ParityMismatch {
                workload: "w".to_owned(),
                runtime: "a".to_owned(),
                against: "b".to_owned(),
                difference: Box::new(Difference {
                    at: "$".to_owned(),
                    a: "1".to_owned(),
                    b: "2".to_owned(),
                }),
            },
            BenchError::CollectionNotFound {
                corpora: path(),
                tree: "t".to_owned(),
                name: "c".to_owned(),
            },
            BenchError::CollectionEntry {
                name: "c".to_owned(),
                path: path(),
                mode: "120000".to_owned(),
            },
            BenchError::EntryCollides {
                workload: "w".to_owned(),
                collection: "c".to_owned(),
                entry: path(),
            },
            BenchError::BudgetExceeded {
                runtime: "fgro".to_owned(),
                budget: "bytes",
                current: 2,
                limit: 1,
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
            BenchError::Write {
                path: path(),
                source: io_error(),
            },
        ];
        for error in &other {
            assert_eq!(bench_exit_code(error), 1, "{error:?}");
        }
    }
}
