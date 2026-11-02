//! `cargo xtask bench`: run a measurement suite and append its record to the
//! history, or report on the history, or run an A/B against a base revision.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::Command;

use gro_bench::error::BenchError;
use gro_bench::record::{GitState, Machine, MachineQuiet};
use gro_bench::run::{Measure, RunContext, run as run_suite};
use gro_bench::suite::{Runtime, Suite};

use crate::check::fetch_v8;
use crate::cmd::StepResult;
use crate::error::XtaskError;
use crate::snapshot::{self, Measurement};
use crate::{Bench, REPORT};

mod base;

/// The measurement history, relative to the root: one JSON line per run,
/// append-only, beside the harness that writes it.
pub const HISTORY: &str = "crates/gro_bench/history.jsonl";

/// Where a bench writes, relative to the root: its copies of corpora
/// collections, under the gitignored `target/`.
const BENCH_DIR: &str = "target/bench";

/// What `cargo xtask bench` was asked to do, parsed from its flags once, so
/// no combination they refuse reaches the run.
#[derive(Debug, PartialEq, Eq)]
enum BenchPlan {
    /// `report`: read the history.
    Report { history: Option<PathBuf> },
    /// Run the suite at `suite`.
    Run {
        suite: PathBuf,
        history: Option<PathBuf>,
        attest_quiet: bool,
        samples: Option<u32>,
        kind: RunKind,
    },
}

/// What a run measures and what becomes of its record.
#[derive(Debug, PartialEq, Eq)]
enum RunKind {
    /// A run whose record is appended, measuring as `Recorded` says.
    Record(Recorded),
    /// `--counters --base <rev>`: each fgro row beside a build at `rev`,
    /// built under `dir` (`--base-dir`), recording nothing.
    Ab { rev: String, dir: Option<PathBuf> },
    /// A prices suite's run, under `perf stat` when `counters`; its record
    /// is appended. Only [`RunKind::for_suite`] makes one, from the suite.
    Prices { counters: bool },
}

/// What a recorded run measures beside wall time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recorded {
    Wall,
    /// `--phases`.
    Phases,
    /// `--counters`.
    Counters,
}

impl BenchPlan {
    /// Parses the flags, refusing those that don't combine; `vars` are the
    /// names of the environment's variables, which an A/B checks.
    fn parse(args: Bench, vars: impl IntoIterator<Item = String>) -> Result<Self, XtaskError> {
        let usage = |message: &str| Err(XtaskError::BenchUsage(message.to_owned()));
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
            return Ok(Self::Report {
                history: args.history,
            });
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
        let kind = match args.base {
            Some(rev) => {
                let set = base::codegen_overrides(vars);
                if !set.is_empty() {
                    return usage(&format!(
                        "--base builds the base from a cleared environment, so the head's build would take \
                         {} and the base's wouldn't; unset them for the A/B",
                        set.join(", ")
                    ));
                }
                RunKind::Ab {
                    rev,
                    dir: args.base_dir,
                }
            }
            None if args.phases => RunKind::Record(Recorded::Phases),
            None if args.counters => RunKind::Record(Recorded::Counters),
            None => RunKind::Record(Recorded::Wall),
        };
        Ok(Self::Run {
            suite: args.suite,
            history: args.history,
            attest_quiet: args.attest_quiet,
            samples: args.samples,
            kind,
        })
    }
}

impl RunKind {
    /// The kind of run `suite` makes of this one: a prices suite's is
    /// [`RunKind::Prices`], every runtime of it fgro, and refuses `--phases`
    /// and `--base`, which would replace the priced build.
    fn for_suite(self, suite: &Suite) -> Result<Self, XtaskError> {
        if suite.prices.is_none() {
            return Ok(self);
        }
        if let Some(runtime) = suite.runtimes.iter().find(|runtime| !is_fgro(runtime)) {
            return Err(XtaskError::BenchUsage(format!(
                "a prices suite prices fgro's defaults, and its runtime `{}` isn't fgro",
                runtime.name
            )));
        }
        match self {
            Self::Record(Recorded::Wall) => Ok(Self::Prices { counters: false }),
            Self::Record(Recorded::Counters) => Ok(Self::Prices { counters: true }),
            Self::Record(Recorded::Phases) | Self::Ab { .. } | Self::Prices { .. } => {
                Err(XtaskError::BenchUsage(
                    "a prices suite runs the priced build, which --phases and --base would replace"
                        .to_owned(),
                ))
            }
        }
    }
}

/// Runs a suite, prints a summary, and appends the record to the history;
/// or, with `report`, reports on the history; or, with `--base`, runs the
/// A/B and prints it.
pub fn bench(root: &Path, args: Bench) -> StepResult {
    let plan = BenchPlan::parse(
        args,
        std::env::vars_os().map(|(name, _)| name.to_string_lossy().into_owned()),
    )?;
    let (suite_path, history, attest_quiet, samples, kind) = match plan {
        BenchPlan::Report { history } => return bench_report(root, history),
        BenchPlan::Run {
            suite,
            history,
            attest_quiet,
            samples,
            kind,
        } => (suite, history, attest_quiet, samples, kind),
    };
    let history = history_path(root, history)
        .map_err(|error| XtaskError::Input(format!("bench failed: invalid --history: {error}")))?;
    let suite_dir = suite_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut suite = Suite::load(&suite_path)?;
    if let Some(samples) = samples {
        let Some(samples) = NonZeroU32::new(samples) else {
            return Err(XtaskError::BenchUsage("--samples is at least 2".to_owned()));
        };
        suite.samples = samples;
        suite.validate()?;
    }
    let kind = kind.for_suite(&suite)?;
    let release = build_fgro(root, &suite, suite_dir)?;
    let fgro_names: BTreeSet<String> = suite
        .runtimes
        .iter()
        .filter(|runtime| is_fgro(runtime))
        .map(|runtime| runtime.name.clone())
        .collect();
    let snapshot = release
        .as_ref()
        // beside the release binary, as `cargo xtask build` writes it
        .map(|release| release.binary.with_file_name("snapshot.bin"));
    let measure = match &kind {
        RunKind::Record(Recorded::Wall) => Measure::Wall,
        RunKind::Record(Recorded::Counters) | RunKind::Ab { .. } => Measure::Counters,
        RunKind::Prices { counters } => {
            // a prices suite's runtimes are all fgro, so the release was built
            let Some(release) = &release else {
                return Err(XtaskError::Input("a prices suite runs no fgro".to_owned()));
            };
            measurement_build(root, release, Measurement::Prices, &fgro_names, &mut suite)?;
            if *counters {
                Measure::Counters
            } else {
                Measure::Wall
            }
        }
        RunKind::Record(Recorded::Phases) => {
            let Some(release) = &release else {
                return Err(XtaskError::BenchUsage(
                    "--phases times fgro's phases, and the suite runs no fgro".to_owned(),
                ));
            };
            measurement_build(root, release, Measurement::Phases, &fgro_names, &mut suite)?;
            Measure::Phases {
                runtimes: fgro_names.clone(),
            }
        }
    };
    let base = match kind {
        RunKind::Ab { rev, dir } => {
            if fgro_names.is_empty() {
                return Err(XtaskError::BenchUsage(
                    "--base compares fgro builds, and the suite runs no fgro".to_owned(),
                ));
            }
            let sha = base::resolve(root, &rev)?;
            let dir = base::base_dir(dir)?;
            let lock = base::lock(&dir)?;
            let built = base::build(root, &dir, &sha, &lock)?;
            suite = base_suite(&suite, &fgro_names, &built.binary);
            // the toolchain the head's build ran: this process's, which
            // `cargo xtask` started under the checkout's pin
            let mut rustc = Command::new("rustc");
            rustc.current_dir(root);
            let head_rustc = base::rustc_version(rustc)?;
            Some(BaseRun {
                rev,
                sha,
                rustc: built.rustc,
                head_rustc,
                _lock: lock,
            })
        }
        RunKind::Record(_) | RunKind::Prices { .. } => None,
    };
    let git = GitState::read(root, &history)?;
    let context = RunContext {
        git,
        machine: Machine::detect(),
        machine_quiet: if attest_quiet {
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
    print!("{}", gro_bench::summary::summary(&record));
    if let Some(base) = &base {
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
        let (table, flagged) = gro_bench::summary::ab_table(&record, BASE_SUFFIX);
        print!("{table}");
        if flagged > 0 {
            println!("[xtask] {flagged} row(s) moved past the perf rule");
        }
        println!("[xtask] an A/B records nothing");
        return Ok(());
    }
    if let Some(prices) = &suite.prices {
        println!();
        print!("{}", gro_bench::prices::table(&record, prices));
    } else if record.mode == gro_bench::record::Mode::Wall {
        print!("{}", gro_bench::summary::gate(&suite, &record, is_fgro));
    } else {
        println!("\n(no gate table: it reads wall times, which this mode's processes inflate)");
    }
    println!("[xtask] appended the record to {}", history.display());
    Ok(())
}

/// Builds the snapshot `fgro` with `measurement`'s feature and runs it in
/// place of each fgro runtime (`fgro_names`) of `suite`, refusing a priced
/// build whose `--version` doesn't say so.
fn measurement_build(
    root: &Path,
    release: &Release,
    measurement: Measurement,
    fgro_names: &BTreeSet<String>,
    suite: &mut Suite,
) -> StepResult {
    let built = snapshot::build(root, &release.target_dir, true, measurement)
        .map_err(XtaskError::in_bench_build)?
        .binary;
    if measurement == Measurement::Prices {
        require_marker(&built, PRICES_MARKER)?;
    }
    for runtime in &mut suite.runtimes {
        if fgro_names.contains(&runtime.name) {
            runtime.program = built.display().to_string();
        }
    }
    println!();
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
    let history = history_path(root, history)
        .map_err(|error| XtaskError::Input(format!("bench report: invalid --history: {error}")))?;
    let text = fs::read_to_string(&history).map_err(|error| {
        XtaskError::Input(format!(
            "bench report: can't read {}: {error}",
            history.display()
        ))
    })?;
    println!("[xtask] the history {}", history.display());
    print!("{}", gro_bench::report::report(&text));
    Ok(())
}

/// The runtime the bench builds before it runs: gro's own `fgro`.
const FGRO: &str = "fgro";

/// Whether a runtime is `fgro`, by its name or its program's file name. One
/// rule for both the build-and-refuse check and the gate report, so a runtime
/// can't pass as `fgro` in one and not the other.
fn is_fgro(runtime: &Runtime) -> bool {
    runtime.name == FGRO || Path::new(&runtime.program).file_name() == Some(OsStr::new(FGRO))
}

/// The release snapshot `fgro` a bench built, and cargo's target directory
/// it was built in, which a measurement build reuses.
#[derive(Debug)]
struct Release {
    /// The binary, canonical.
    binary: PathBuf,
    target_dir: PathBuf,
}

/// Builds the release snapshot `fgro` when the suite has an `fgro` runtime
/// ([`is_fgro`]), and refuses the run unless each one's program is the binary
/// just built: the only `fgro` worth timing is the release build with its
/// startup snapshot, the one path that verifies the release V8 library.
/// Building first (cached when nothing changed) means a stale or debug binary
/// is never timed. A compile that fails exits `2`, since the run can't go
/// ahead until it's fixed ([`XtaskError::in_bench_build`]); a failed fetch,
/// read, or write keeps its own code. Returns the build, when the suite has
/// an `fgro`.
fn build_fgro(root: &Path, suite: &Suite, suite_dir: &Path) -> Result<Option<Release>, XtaskError> {
    if !suite.runtimes.iter().any(is_fgro) {
        return Ok(None);
    }
    let target_dir = fetch_v8(root)?;
    let built = snapshot::build(root, &target_dir, true, Measurement::None)
        .map_err(XtaskError::in_bench_build)?
        .binary;
    let built = built.canonicalize().map_err(|error| {
        XtaskError::Failed(format!("bench failed: {}: {error}", built.display()))
    })?;
    check_fgro_programs(suite, suite_dir, &built)?;
    refuse_measurement_build(&built)?;
    println!();
    Ok(Some(Release {
        binary: built,
        target_dir,
    }))
}

/// The marker a phases build's `--version` line carries.
const PHASES_MARKER: &str = ", phases";

/// The marker a priced build's `--version` line carries.
const PRICES_MARKER: &str = ", prices";

/// `built --version`'s first line, failing when it doesn't run.
fn version_line(built: &Path) -> Result<String, XtaskError> {
    let output = Command::new(built)
        .arg("--version")
        .output()
        .map_err(|error| {
            XtaskError::Failed(format!(
                "bench failed: {} --version: {error}",
                built.display()
            ))
        })?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() {
        return Err(XtaskError::Input(format!(
            "bench failed: {} doesn't run ({}): {line}",
            built.display(),
            output.status,
        )));
    }
    Ok(line)
}

/// Refuses a release build that is a measurement build (phases or priced),
/// which a feature unified in from elsewhere in the build graph would make:
/// it would time what a release doesn't run.
fn refuse_measurement_build(built: &Path) -> StepResult {
    let line = version_line(built)?;
    if line.contains(PHASES_MARKER) || line.contains(PRICES_MARKER) {
        return Err(XtaskError::Input(format!(
            "bench failed: the release build {} is a measurement build: {line}",
            built.display(),
        )));
    }
    Ok(())
}

/// Refuses a measurement build whose `--version` lacks its `marker`, which
/// would time the release under its name: a priced build that reads no
/// switches prices every default at nothing.
fn require_marker(built: &Path, marker: &str) -> StepResult {
    let line = version_line(built)?;
    if !line.contains(marker) {
        return Err(XtaskError::Input(format!(
            "bench failed: {} isn't the measurement build it was built as (no `{marker}`): {line}",
            built.display(),
        )));
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

#[cfg(test)]
mod tests {
    use std::process::ExitStatus;

    use gro_bench::check::Difference;

    use super::*;
    use crate::{EXIT_INPUT, root};

    /// The exit code of `error` as the xtask returns it.
    fn bench_exit_code(error: BenchError) -> u8 {
        XtaskError::from(error).exit_code()
    }

    fn bench_args(args: &[&str]) -> Bench {
        let mut all = vec!["xtask", "bench"];
        all.extend(args);
        match crate::parse_args(&all.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>()) {
            Ok(crate::Xtask {
                command: crate::Subcommand::Bench(bench),
            }) => bench,
            _ => unreachable!("not a bench command line: {args:?}"),
        }
    }

    fn plan(args: &[&str]) -> Result<BenchPlan, String> {
        BenchPlan::parse(bench_args(args), Vec::new()).map_err(|error| error.to_string())
    }

    #[test]
    fn the_flags_parse_into_one_plan() {
        assert_eq!(
            plan(&["report", "--history", "h.jsonl"]),
            Ok(BenchPlan::Report {
                history: Some(PathBuf::from("h.jsonl"))
            })
        );
        let run = |kind| BenchPlan::Run {
            suite: PathBuf::from("s.json"),
            history: None,
            attest_quiet: false,
            samples: None,
            kind,
        };
        assert_eq!(plan(&["s.json"]), Ok(run(RunKind::Record(Recorded::Wall))));
        assert_eq!(
            plan(&["s.json", "--phases"]),
            Ok(run(RunKind::Record(Recorded::Phases)))
        );
        assert_eq!(
            plan(&["s.json", "--counters"]),
            Ok(run(RunKind::Record(Recorded::Counters)))
        );
        assert_eq!(
            plan(&["s.json", "--counters", "--base", "HEAD", "--base-dir", "d"]),
            Ok(run(RunKind::Ab {
                rev: "HEAD".to_owned(),
                dir: Some(PathBuf::from("d")),
            }))
        );
    }

    #[test]
    fn flags_that_dont_combine_are_refused_with_their_usage() {
        for (args, message) in [
            (
                &["report", "--counters"][..],
                "bench: `report` takes only --history",
            ),
            (
                &["report", "--samples", "4"],
                "bench: `report` takes only --history",
            ),
            (
                &["s.json", "--phases", "--counters"],
                "bench: --phases and --counters measure apart: a phases build counts more",
            ),
            (
                &["s.json", "--base", "HEAD"],
                "bench: --base compares instruction counts, so it needs --counters",
            ),
            (
                &["s.json", "--base-dir", "d"],
                "bench: --base-dir is where --base builds, so it needs --base",
            ),
        ] {
            assert_eq!(plan(args), Err(message.to_owned()), "{args:?}");
        }
        let error = BenchPlan::parse(
            bench_args(&["s.json", "--counters", "--base", "HEAD"]),
            ["RUSTC".to_owned(), "PATH".to_owned()],
        )
        .unwrap_err();
        assert_eq!(error.exit_code(), EXIT_INPUT);
        assert!(
            error.to_string().contains("would take RUSTC and"),
            "{error}"
        );
    }

    #[test]
    fn a_prices_suite_refuses_phases_and_base_and_a_runtime_not_fgro() {
        let suite = |runtime: &str| -> Suite {
            serde_json::from_str(&format!(
                r#"{{"name": "s", "version": 1, "samples": 2, "prices": {{"default": "{runtime}"}},
                    "runtimes": [{{"name": "{runtime}", "program": "{runtime}"}}],
                    "workloads": [{{"name": "w", "script": "w.js"}}]}}"#
            ))
            .unwrap()
        };
        let prices = suite("fgro");
        let kind = |kind: RunKind| kind.for_suite(&prices).map_err(|error| error.to_string());
        assert_eq!(
            kind(RunKind::Record(Recorded::Wall)),
            Ok(RunKind::Prices { counters: false })
        );
        assert_eq!(
            kind(RunKind::Record(Recorded::Counters)),
            Ok(RunKind::Prices { counters: true })
        );
        let replaced = Err(
            "bench: a prices suite runs the priced build, which --phases and --base would replace"
                .to_owned(),
        );
        assert_eq!(kind(RunKind::Record(Recorded::Phases)), replaced);
        assert_eq!(
            kind(RunKind::Ab {
                rev: "HEAD".to_owned(),
                dir: None
            }),
            replaced
        );
        let error = RunKind::Record(Recorded::Wall)
            .for_suite(&suite("node"))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "bench: a prices suite prices fgro's defaults, and its runtime `node` isn't fgro"
        );
    }

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
        let code = |result: StepResult| result.map_err(|error| error.exit_code());
        assert_eq!(
            code(refuse_measurement_build(&fakes.join("release"))),
            Ok(())
        );
        for build in ["phases", "prices"] {
            assert_eq!(
                code(refuse_measurement_build(&fakes.join(build))),
                Err(EXIT_INPUT),
                "{build}"
            );
        }
        // and a priced build must say it is one
        assert_eq!(
            code(require_marker(&fakes.join("prices"), PRICES_MARKER)),
            Ok(())
        );
        assert_eq!(
            code(require_marker(&fakes.join("release"), PRICES_MARKER)),
            Err(EXIT_INPUT)
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
        for error in input {
            let debug = format!("{error:?}");
            assert_eq!(bench_exit_code(error), EXIT_INPUT, "{debug}");
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
        for error in other {
            let debug = format!("{error:?}");
            assert_eq!(bench_exit_code(error), 1, "{debug}");
        }
    }
}
