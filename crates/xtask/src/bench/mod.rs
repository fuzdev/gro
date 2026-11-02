//! `cargo xtask bench`: run a measurement suite and append its record to the
//! history, or report on the history, or run an A/B against a base revision.

use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use gro_bench::error::BenchError;
use gro_bench::record::{GitState, Machine, MachineQuiet, Mode};
use gro_bench::run::{Measure, RunContext, run as run_suite};
use gro_bench::suite::{Runtime, Suite};

use crate::check::fetch_v8;
use crate::cmd::{StepResult, target_dir};
use crate::snapshot::{self, Measurement};
use crate::{Bench, EXIT_INPUT, REPORT};
use print::{print_base, print_gate, print_summary};

mod base;
mod print;

/// The measurement history, relative to the root: one JSON line per run,
/// append-only, beside the harness that writes it.
pub const HISTORY: &str = "crates/gro_bench/history.jsonl";

/// Where a bench writes, relative to the root: its copies of corpora
/// collections, under the gitignored `target/`.
const BENCH_DIR: &str = "target/bench";

/// Runs a suite, prints a summary, and appends the record to the history;
/// or, with `report`, reports on the history; or, with `--base`, runs the
/// A/B and prints it.
pub fn bench(root: &Path, args: Bench) -> StepResult {
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

#[cfg(test)]
mod tests {
    use std::process::ExitStatus;

    use gro_bench::check::Difference;

    use super::*;
    use crate::root;

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
