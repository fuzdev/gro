//! The process runner: measures a suite's rows and returns the run's record.
//!
//! A run:
//!
//! 1. checks the suite's corpora checkout against its pin, resolves each
//!    runtime's executable, refusing a script (a file starting with `#!`),
//!    writes a fresh copy of each collection a workload names, with the
//!    workload's script at its root ([`collection`](crate::collection)), and
//!    of each project ([`project`](crate::project)), and hashes the artifacts
//!    (every executable, script, file argument, stdin file, and file of a
//!    copy) with blake3, and reads the git state of each checkout outside the
//!    suite's own that a workload reads: each project link's target, which
//!    must be in a checkout, and the checkout of a script outside the
//!    suite's;
//! 2. probes each runtime's version and measures its executable's size, as
//!    is and gzipped;
//! 3. runs each row once, untimed, for correctness: it must exit
//!    successfully, with stdout of the shape its runtime's `stdout` names,
//!    and the values of the suite's `parity` runtimes must be equal on each
//!    workload ([`check`](crate::check));
//! 4. runs each row's warmup processes, then its timed samples, in the
//!    interleaved order of [`schedule`], with the
//!    first row's A/A copy as one more row (a row is a workload and a
//!    runtime);
//! 5. hashes the artifacts, lists each copy, and reads the corpora's tree
//!    and the other checkouts' states again, and discards the run if any
//!    changed (a copy gaining or losing a path counts), since a rebuild or an
//!    edit mid-run means the samples timed different code: the copies cover
//!    what a collection's workloads run, the corpora's tree what the others
//!    read from it, and a checkout's state what a link or script reaches in
//!    it;
//! 6. computes each row's statistics.
//!
//! The context's [`Measure`] says what a sample measures beside wall time:
//! nothing, the phase timings a phases build of fgro writes
//! ([`phases`]), or hardware counters under `perf stat`
//! ([`counters`](crate::counters)), which the run checks work before
//! anything else. Only the timed samples are counted; the correctness run,
//! the warmups, and the version probes run as they are. In a wall run, on
//! Linux, each sample's peak memory is read as it's reaped (`wait4`'s
//! `ru_maxrss`), and after the samples the floor that peak can't go below
//! (a trivial program's, run the same way; [`RunRecord`]'s `rss_floor_kib`);
//! a counter run's spawner is `perf`, whose own peak would mask a small
//! runtime's, so it records none, nor does a phases run.
//!
//! After the probes, and before any process runs, the suite's `budgets` are
//! checked against the sizes measured, and the startup snapshot's the
//! context names, failing the run on the first one crossed; a phases run,
//! whose `fgro` is a measurement build, checks none.
//!
//! Every sample is a fresh process, timed from just before its spawn to just
//! after its exit is reaped, with stdout and stderr on the null device, and
//! stdin on its runtime's `stdin` file, opened before the timed span, or the
//! null device. A process that exits unsuccessfully fails the run, naming the
//! command to re-run alone. Nothing prints during the run, so the terminal is
//! no part of any sample.
//!
//! Every process, samples and version probes alike, runs the canonical
//! executable (the file hashed) with the suite's `program` as written as its
//! `argv[0]`, as a shell would pass it, and with a cleared environment holding
//! only the caller's `PATH` and `HOME` and the variables the suite sets. Its
//! working directory is the suite's directory, a collection's workloads
//! included.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime};

use crate::check::{first_difference, stdout_value, truncated};
use crate::collection::{list_paths, materialize};
use crate::counters::{Counts, Perf};
use crate::error::BenchError;
use crate::phases::{self, PHASES_FILE_ENV, SampleSpan};
use crate::project::materialize_project;
use crate::record::{
    CheckoutState, GitState, Machine, MachineQuiet, Metric, Mode, NoiseRecord, PhaseMetric,
    RECORD_VERSION, RowRecord, RunRecord, RuntimeInfo, SuiteId, checkout_root, read_corpora_tree,
};
use crate::schedule::schedule;
use crate::stats::{Stats, ratio_deviation};
use crate::suite::{Arg, Budget, FileArg, Runtime, Stdout, Suite, Workload};

/// What a run takes from the caller: what it records but can't learn itself,
/// and where it writes.
#[derive(Debug, Clone)]
pub struct RunContext {
    pub git: GitState,
    pub machine: Machine,
    pub machine_quiet: MachineQuiet,
    /// The directory the run writes its collection copies under, at
    /// `corpora/<tree>/<collection>`, each replaced on every run.
    /// `cargo xtask bench` passes the gro checkout's gitignored
    /// `target/bench/`, which is trusted like the rest of `target/` (a
    /// symlink already in it is followed), since anyone who can write there
    /// can already change build output. A phases or counter run also writes
    /// each sample's file here, under `phases/` or `counters/`.
    pub work_dir: PathBuf,
    /// What each sample measures beside wall time.
    pub measure: Measure,
    /// The startup snapshot the suite's `fgro` embeds, whose size the record
    /// carries and a `snapshot_bytes` budget limits.
    pub snapshot: Option<PathBuf>,
}

/// What a run's samples measure beside wall time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Measure {
    /// Nothing: each process runs as is.
    Wall,
    /// The phase timings of the runtimes named, each a phases build of fgro,
    /// whose processes get [`PHASES_FILE_ENV`] naming the file to write.
    Phases { runtimes: BTreeSet<String> },
    /// User-space instructions and cycles, each sample under `perf stat`.
    Counters,
}

/// How the run takes its samples, set up from the [`Measure`].
#[derive(Debug)]
enum Sampler {
    Wall,
    Phases {
        /// The file each sample of a phases runtime writes.
        file: PathBuf,
    },
    Counters {
        perf: Perf,
        /// The file each sample's counts go to.
        out: PathBuf,
    },
}

/// One timed process.
#[derive(Debug, Clone)]
struct Sample {
    wall_ns: u64,
    /// Its counts, under [`Sampler::Counters`].
    counts: Option<Counts>,
    /// Its phases' durations, under [`Sampler::Phases`] for a phases runtime.
    phases: Vec<(String, u64)>,
    /// Its peak resident set size in KiB, where the harness reads one
    /// ([`reap`]).
    max_rss_kib: Option<u64>,
}

/// The environment variables a measured process keeps from the caller's.
const KEPT_ENV: [&str; 2] = ["PATH", "HOME"];

/// A runtime's executable, resolved.
#[derive(Debug, Clone)]
struct Program {
    /// The canonical file, which the run hashes and executes.
    path: PathBuf,
    /// The suite's `program` as written, passed as `argv[0]`, which a
    /// multi-call binary dispatches on.
    #[cfg_attr(not(unix), expect(dead_code, reason = "`argv[0]` is set on unix only"))]
    argv0: String,
    /// The runtime's `env`.
    env: BTreeMap<String, String>,
}

impl Program {
    /// A command running this program from `dir`, in the isolated
    /// environment, with stdin on the null device.
    fn command(&self, dir: &Path) -> Command {
        let mut command = Command::new(&self.path);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.arg0(&self.argv0);
        }
        self.isolate(&mut command, dir);
        command
    }

    /// Sets `command`'s environment to the isolated one with this program's
    /// variables, its directory to `dir`, and its stdin to the null device.
    fn isolate(&self, command: &mut Command, dir: &Path) {
        command.env_clear();
        for key in KEPT_ENV {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .envs(&self.env)
            .current_dir(dir)
            .stdin(Stdio::null());
    }
}

/// A row's command, resolved.
#[derive(Debug)]
struct RowCommand {
    runtime: String,
    workload: String,
    program: Program,
    args: Vec<OsString>,
    /// The runtime's `cold` env on a cold row, over the runtime's `env`.
    env: BTreeMap<String, String>,
    warmup: u32,
    /// The file each process reads on stdin, the runtime's `stdin`.
    stdin: Option<PathBuf>,
    /// What the runtime's stdout holds, for the correctness run.
    stdout: Option<Stdout>,
    /// Whether its samples write phase timings, under [`Sampler::Phases`].
    phases: bool,
}

impl RowCommand {
    /// A process of this row, its stdin opened fresh, with stdout and
    /// stderr on the null device.
    fn command(&self, dir: &Path) -> Result<Command, BenchError> {
        let mut command = self.program.command(dir);
        command.args(&self.args);
        self.finish(command)
    }

    /// A process of this row under `perf stat`, writing its counts to
    /// `out`, set up as [`command`](Self::command) sets one up.
    fn counted(&self, perf: &Perf, out: &Path, dir: &Path) -> Result<Command, BenchError> {
        let mut command = perf.command(out, &self.program.path, &self.args);
        self.program.isolate(&mut command, dir);
        self.finish(command)
    }

    /// Adds the row's variables to `command`, puts its stdout and stderr on
    /// the null device, and opens its stdin.
    fn finish(&self, mut command: Command) -> Result<Command, BenchError> {
        command
            .envs(&self.env)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(stdin) = &self.stdin {
            let file = File::open(stdin).map_err(|source| BenchError::Read {
                path: stdin.clone(),
                source,
            })?;
            command.stdin(file);
        }
        Ok(command)
    }

    /// The command line to re-run alone, with its stdin redirect.
    fn display(&self) -> String {
        let line = display_command(&self.program.path, &self.args);
        match &self.stdin {
            Some(stdin) => format!("{line} < {}", stdin.display()),
            None => line,
        }
    }
}

/// Measures `suite`, whose files are relative to `dir`.
///
/// # Errors
///
/// Fails, recording nothing, when the suite is invalid, a program or file is
/// missing, a version probe or any process fails, or an artifact changed
/// during the run.
pub fn run(suite: &Suite, dir: &Path, context: RunContext) -> Result<RunRecord, BenchError> {
    suite.validate()?;
    let sampler = sampler(suite, &context)?;
    let corpora = suite
        .corpora
        .as_ref()
        .map(|pin| -> Result<_, BenchError> {
            let path = resolve_file(dir, &pin.path)?;
            let found = read_corpora_tree(&path)?;
            if found != pin.tree {
                return Err(BenchError::CorporaTree {
                    path,
                    pinned: pin.tree.clone(),
                    found,
                });
            }
            Ok((path, found))
        })
        .transpose()?;
    let started_unix_s = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());

    let programs = suite
        .runtimes
        .iter()
        .map(|runtime| resolve_program(runtime, dir))
        .collect::<Result<Vec<_>, _>>()?;

    let mut artifacts: BTreeSet<PathBuf> = programs
        .iter()
        .map(|program| program.path.clone())
        .collect();
    // the scripts and file arguments, by their paths as the suite writes them
    let mut suite_files: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut resolve = |file: &Path| -> Result<PathBuf, BenchError> {
        let resolved = resolve_file(dir, file)?;
        artifacts.insert(resolved.clone());
        suite_files.insert(file.to_string_lossy().into_owned(), resolved.clone());
        Ok(resolved)
    };
    // every workload's script
    let scripts = suite
        .workloads
        .iter()
        .map(|workload| Ok((workload.name.as_str(), resolve(&workload.script)?)))
        .collect::<Result<BTreeMap<_, _>, BenchError>>()?;
    // each runtime's stdin file, hashed like a file argument
    let stdins = suite
        .runtimes
        .iter()
        .map(|runtime| {
            runtime
                .stdin
                .as_ref()
                .map(|FileArg { file }| resolve(file))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut copies = materialize_collections(suite, corpora.as_ref(), &context.work_dir, &scripts)?;
    materialize_projects(suite, dir, &context.work_dir, &mut copies)?;
    let program_of: BTreeMap<&str, (&Program, &Option<PathBuf>)> = suite
        .runtimes
        .iter()
        .map(|runtime| runtime.name.as_str())
        .zip(programs.iter().zip(&stdins))
        .collect();
    let mut rows = Vec::new();
    for (workload, runtime) in suite.rows() {
        let name = workload.name.as_str();
        let script = copies.entries.get(name).unwrap_or_else(|| &scripts[name]);
        let (args, mut env) = row_args(runtime, workload, script, &mut resolve)?;
        let (program, stdin) = program_of[runtime.name.as_str()];
        let phases = match (&context.measure, &sampler) {
            (Measure::Phases { runtimes }, Sampler::Phases { file })
                if runtimes.contains(&runtime.name) =>
            {
                env.insert(PHASES_FILE_ENV.to_owned(), file.display().to_string());
                true
            }
            _ => false,
        };
        rows.push(RowCommand {
            runtime: runtime.name.clone(),
            workload: workload.name.clone(),
            program: program.clone(),
            args,
            env,
            warmup: workload.warmup,
            stdin: stdin.clone(),
            stdout: runtime.stdout,
            phases,
        });
    }
    artifacts.extend(copies.files.iter().cloned());
    let before = hash_artifacts(&artifacts)?;
    let listed = copies
        .roots
        .iter()
        .map(|root| list_paths(root))
        .collect::<Result<Vec<_>, _>>()?;
    let artifact_hashes: BTreeMap<String, String> = suite_files
        .into_iter()
        .map(|(file, resolved)| (file, before[&resolved].clone()))
        .collect();
    let checkouts = read_checkouts(suite, dir)?;

    // one thread per runtime, since gzipping each executable takes seconds
    let runtimes = std::thread::scope(|scope| {
        #[expect(
            clippy::needless_collect,
            reason = "every thread spawns before the first join, or the probes run one at a time"
        )]
        let probes: Vec<_> = suite
            .runtimes
            .iter()
            .zip(&programs)
            .map(|(runtime, program)| {
                let hash = &before[&program.path];
                scope.spawn(move || probe(runtime, program, dir, hash))
            })
            .collect();
        probes
            .into_iter()
            .map(|probe| {
                probe
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect::<Result<Vec<_>, _>>()
    })?;

    let snapshot_bytes = context
        .snapshot
        .as_ref()
        .map(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.len())
                .map_err(|source| BenchError::Read {
                    path: path.clone(),
                    source,
                })
        })
        .transpose()?;
    // a measurement build isn't the size that ships
    if !matches!(context.measure, Measure::Phases { .. }) {
        check_budgets(suite, &runtimes, snapshot_bytes)?;
    }

    check_rows(suite, &rows, dir)?;

    // the noise row: the first row again, measured as a row of its own
    let commands: Vec<&RowCommand> = rows.iter().chain(rows.first()).collect();
    let warmups: Vec<usize> = commands.iter().map(|row| row.warmup as usize).collect();
    for index in schedule(&warmups) {
        run_once(commands[index], dir, &Sampler::Wall)?;
    }
    let sample_count = suite.samples.get() as usize;
    let mut samples: Vec<Vec<Sample>> = vec![Vec::with_capacity(sample_count); commands.len()];
    for index in schedule(&vec![sample_count; commands.len()]) {
        samples[index].push(run_once(commands[index], dir, &sampler)?);
    }
    // only a wall run's spawner is the harness alone ([`RunRecord`])
    let rss_floor_kib = if context.measure == Measure::Wall {
        rss_floor(dir, &sampler)?
    } else {
        None
    };

    let after = hash_artifacts(&artifacts)?;
    let mut changed = changed_artifacts(&before, &after);
    for (root, before) in copies.roots.iter().zip(&listed) {
        // a path written or removed in a copy, such as a lockfile or an install
        let after = list_paths(root)?;
        changed.extend(before.symmetric_difference(&after).cloned());
    }
    if let Some((path, tree)) = &corpora {
        // a re-pin or a local change mid-run, which the tree id wouldn't name
        if read_corpora_tree(path).ok().as_ref() != Some(tree) {
            changed.push(path.clone());
        }
    }
    let roots: BTreeMap<&PathBuf, &CheckoutState> = checkouts
        .values()
        .map(|(root, state)| (root, state))
        .collect();
    for (root, state) in roots {
        // a commit, a switch, or an edit mid-run in another checkout
        if CheckoutState::read(root).ok().as_ref() != Some(state) {
            changed.push(root.clone());
        }
    }
    if !changed.is_empty() {
        return Err(BenchError::ArtifactsChanged { paths: changed });
    }

    let rows_record: Vec<RowRecord> = rows
        .iter()
        .zip(&samples)
        .map(|(row, samples)| {
            Ok(RowRecord {
                runtime: row.runtime.clone(),
                workload: row.workload.clone(),
                warmup: row.warmup,
                wall_ns: wall_metric(samples),
                instructions: count_metric(samples, |counts| counts.instructions),
                cycles: count_metric(samples, |counts| counts.cycles),
                kernel_instructions: samples
                    .iter()
                    .map(|sample| sample.counts.and_then(|counts| counts.kernel_instructions))
                    .collect::<Option<Vec<u64>>>()
                    .map(metric),
                phases: phase_metrics(row, samples)?,
                peak_rss_kib: if context.measure == Measure::Wall {
                    rss_metric(samples)
                } else {
                    None
                },
            })
        })
        .collect::<Result<_, BenchError>>()?;
    let noise = noise_record(&rows_record[0], &samples[rows.len()]);

    Ok(RunRecord {
        record_version: RECORD_VERSION,
        mode: match context.measure {
            Measure::Wall => Mode::Wall,
            Measure::Phases { .. } => Mode::Phases,
            Measure::Counters => Mode::Counters,
        },
        counters: match &sampler {
            Sampler::Counters { perf, .. } => Some(perf.tool.clone()),
            Sampler::Wall | Sampler::Phases { .. } => None,
        },
        suite: SuiteId {
            name: suite.name.clone(),
            version: suite.version,
        },
        config_hash: suite.config_hash(),
        started_unix_s,
        git: context.git,
        machine: context.machine,
        machine_quiet: context.machine_quiet,
        corpora: corpora.map(|(_, tree)| tree),
        runtimes,
        artifacts: artifact_hashes,
        checkouts: checkouts
            .into_iter()
            .map(|(path, (_, state))| (path, state))
            .collect(),
        rows: rows_record,
        noise,
        rss_floor_kib,
        snapshot_bytes,
    })
}

/// Fails the run when a runtime's size crosses one of the suite's budgets
/// for it, naming the budget, the size, and the limit.
fn check_budgets(
    suite: &Suite,
    runtimes: &[RuntimeInfo],
    snapshot_bytes: Option<u64>,
) -> Result<(), BenchError> {
    for (name, budget) in &suite.budgets {
        let Some(runtime) = runtimes.iter().find(|runtime| &runtime.name == name) else {
            continue;
        };
        let sizes = Sizes {
            bytes: Some(runtime.bytes),
            gzip_bytes: Some(runtime.gzip_bytes),
            snapshot_bytes,
        };
        check_budget(name, budget, &sizes)?;
    }
    Ok(())
}

/// A runtime's sizes as measured, each in bytes, for [`check_budget`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sizes {
    /// The executable as installed.
    pub bytes: Option<u64>,
    /// The executable gzipped ([`gzip_size`]).
    pub gzip_bytes: Option<u64>,
    /// Its startup snapshot.
    pub snapshot_bytes: Option<u64>,
}

/// Fails when one of `sizes` crosses its limit in `budget`, the runtime
/// `runtime`'s, naming the budget, the size, and the limit.
///
/// # Errors
///
/// [`BenchError::BudgetExceeded`] for the first size over its limit, and
/// [`BenchError::SuiteInvalid`] for a limit with no size measured.
pub fn check_budget(runtime: &str, budget: &Budget, sizes: &Sizes) -> Result<(), BenchError> {
    let checks = [
        ("bytes", budget.bytes, sizes.bytes),
        ("gzip_bytes", budget.gzip_bytes, sizes.gzip_bytes),
        (
            "snapshot_bytes",
            budget.snapshot_bytes,
            sizes.snapshot_bytes,
        ),
    ];
    for (field, limit, current) in checks {
        let Some(limit) = limit else {
            continue;
        };
        let Some(current) = current else {
            return Err(BenchError::SuiteInvalid(format!(
                "the runtime `{runtime}` has a `{field}` budget, but no size was measured for it (a snapshot budget needs the snapshot named)"
            )));
        };
        if current > limit {
            return Err(BenchError::BudgetExceeded {
                runtime: runtime.to_owned(),
                budget: field,
                current,
                limit,
            });
        }
    }
    Ok(())
}

/// How many times [`rss_floor`] runs its trivial program.
const FLOOR_RUNS: usize = 4;

/// The peak a sample reads at least, whatever it runs: the largest of a
/// trivial program's (`true`, found on `PATH`) over a few runs as `sampler`
/// runs the samples, taken after them, when the harness's own peak is
/// highest. `None` where the harness reads no peak.
fn rss_floor(dir: &Path, sampler: &Sampler) -> Result<Option<u64>, BenchError> {
    if !cfg!(target_os = "linux") {
        return Ok(None);
    }
    let program = find_program("true", dir).ok_or_else(|| BenchError::ProgramNotFound {
        runtime: "the peak memory floor".to_owned(),
        program: "true".to_owned(),
    })?;
    let row = RowCommand {
        runtime: "true".to_owned(),
        workload: "floor".to_owned(),
        program: Program {
            path: program,
            argv0: "true".to_owned(),
            env: BTreeMap::new(),
        },
        args: Vec::new(),
        env: BTreeMap::new(),
        warmup: 0,
        stdin: None,
        stdout: None,
        phases: false,
    };
    let mut floor = None;
    for _ in 0..FLOOR_RUNS {
        let peak = run_once(&row, dir, sampler)?.max_rss_kib;
        floor = floor.max(peak);
    }
    Ok(floor)
}

/// The collections a run copied: what each workload with a collection runs,
/// and what the run checks didn't change.
#[derive(Debug, Default)]
struct Copies {
    /// Each copy's root, canonical.
    roots: Vec<PathBuf>,
    /// Every file of every copy, entries included.
    files: Vec<PathBuf>,
    /// The entry each workload with a collection runs, by workload name.
    entries: BTreeMap<String, PathBuf>,
}

/// Copies each collection the suite's workloads name, once, from the pinned
/// tree into `<work_dir>/corpora/<tree>/<collection>`, and writes each of its
/// workloads' scripts (by their resolved paths in `scripts`) at its root
/// under the script's own file name, refusing one the collection already has.
fn materialize_collections(
    suite: &Suite,
    corpora: Option<&(PathBuf, String)>,
    work_dir: &Path,
    scripts: &BTreeMap<&str, PathBuf>,
) -> Result<Copies, BenchError> {
    let mut copies = Copies::default();
    // each collection's copy root, by name, and the entries written
    let mut roots: BTreeMap<&str, PathBuf> = BTreeMap::new();
    let mut entries: BTreeSet<PathBuf> = BTreeSet::new();
    for workload in &suite.workloads {
        let Some(collection) = workload.collection.as_deref() else {
            continue;
        };
        let Some((corpora_path, tree)) = corpora else {
            return Err(BenchError::SuiteInvalid(format!(
                "the workload `{}` names a collection, but the suite pins no `corpora`",
                workload.name
            )));
        };
        let root = if let Some(root) = roots.get(collection) {
            root.clone()
        } else {
            // canonical, so the paths written are the paths hashed and run
            fs::create_dir_all(work_dir).map_err(|source| BenchError::Write {
                path: work_dir.to_owned(),
                source,
            })?;
            let work_dir = work_dir.canonicalize().map_err(|source| BenchError::Read {
                path: work_dir.to_owned(),
                source,
            })?;
            // the tree id is git's own output, a hex id, matched to the pin
            let root = work_dir.join("corpora").join(tree).join(collection);
            copies
                .files
                .extend(materialize(corpora_path, tree, collection, &root)?);
            copies.roots.push(root.clone());
            roots.insert(collection, root.clone());
            root
        };
        // validation gives every collection workload's script a file name
        let file_name = workload.script.file_name().unwrap_or_default();
        let entry = root.join(file_name);
        if entries.insert(entry.clone()) {
            if fs::symlink_metadata(&entry).is_ok() {
                return Err(BenchError::EntryCollides {
                    workload: workload.name.clone(),
                    collection: collection.to_owned(),
                    entry: PathBuf::from(file_name),
                });
            }
            fs::copy(&scripts[workload.name.as_str()], &entry).map_err(|source| {
                BenchError::Write {
                    path: entry.clone(),
                    source,
                }
            })?;
            copies.files.push(entry.clone());
        }
        copies.entries.insert(workload.name.clone(), entry);
    }
    Ok(copies)
}

/// Copies each workload's project into `<work_dir>/projects/<workload>`
/// ([`materialize_project`]), adding the copies to `copies`.
fn materialize_projects(
    suite: &Suite,
    dir: &Path,
    work_dir: &Path,
    copies: &mut Copies,
) -> Result<(), BenchError> {
    for workload in &suite.workloads {
        let Some(project) = &workload.project else {
            continue;
        };
        let copy = materialize_project(&workload.name, project, &workload.script, dir, work_dir)?;
        copies.roots.push(copy.root);
        copies.files.extend(copy.files);
        copies.entries.insert(workload.name.clone(), copy.entry);
    }
    Ok(())
}

/// Reads the state of each checkout outside the suite's own (the one holding
/// `dir`, if any) that the workloads read, by the path as the suite writes
/// it, beside the checkout's root: each project link's target, refused when
/// it's in no checkout, and the checkout holding a script outside the
/// suite's (a collection's script runs from its copy, which the run covers).
fn read_checkouts(
    suite: &Suite,
    dir: &Path,
) -> Result<BTreeMap<String, (PathBuf, CheckoutState)>, BenchError> {
    let own = checkout_root(dir);
    let other = |root: &PathBuf| own.as_ref() != Some(root);
    let mut checkouts = BTreeMap::new();
    for workload in &suite.workloads {
        if let Some(project) = &workload.project {
            for (package, target) in &project.links {
                let path = dir.join(target);
                let root =
                    checkout_root(&path).ok_or_else(|| BenchError::ProjectLinkUnversioned {
                        workload: workload.name.clone(),
                        package: package.clone(),
                        path: path.clone(),
                    })?;
                if other(&root) {
                    let state = CheckoutState::read(&root)?;
                    checkouts.insert(target.to_string_lossy().into_owned(), (root, state));
                }
            }
        } else if workload.collection.is_none() {
            let script = resolve_file(dir, &workload.script)?;
            if let Some(root) = checkout_root(&script).filter(other) {
                let state = CheckoutState::read(&root)?;
                checkouts.insert(
                    workload.script.to_string_lossy().into_owned(),
                    (root, state),
                );
            }
        }
    }
    Ok(checkouts)
}

/// Runs each row once, untimed, and checks it: it exits successfully, its
/// stdout is what its runtime's `stdout` says, and the values of the suite's
/// `parity` runtimes are equal on each workload.
fn check_rows(suite: &Suite, rows: &[RowCommand], dir: &Path) -> Result<(), BenchError> {
    // each checked row's value, by workload and then runtime
    let mut values: BTreeMap<&str, BTreeMap<&str, serde_json::Value>> = BTreeMap::new();
    for row in rows {
        if let Some(value) = check_row(row, dir)? {
            values
                .entry(row.workload.as_str())
                .or_default()
                .insert(row.runtime.as_str(), value);
        }
    }
    let Some((first, others)) = suite.parity.split_first() else {
        return Ok(());
    };
    for workload in &suite.workloads {
        let values = &values[workload.name.as_str()];
        // validation gives every parity runtime a `stdout`, so each has a value
        let expected = &values[first.as_str()];
        for other in others {
            if let Some(difference) = first_difference(&values[other.as_str()], expected) {
                return Err(BenchError::ParityMismatch {
                    workload: workload.name.clone(),
                    runtime: other.clone(),
                    against: first.clone(),
                    difference: Box::new(difference),
                });
            }
        }
    }
    Ok(())
}

/// Runs one process of `row`, untimed, capturing its output: it must exit
/// successfully, and its stdout must be what `row.stdout` says, whose value
/// it returns.
fn check_row(row: &RowCommand, dir: &Path) -> Result<Option<serde_json::Value>, BenchError> {
    let spawn_error = |source| BenchError::Spawn {
        command: row.display(),
        source,
    };
    let mut child = row
        .command(dir)?
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_error)?;
    // stdout whole only when it's checked: a large output kept unchecked
    // would raise the harness's peak, which every later sample reads at least
    let stdout_kept = if row.stdout.is_some() {
        usize::MAX
    } else {
        UNCHECKED_KEPT
    };
    let (stdout, stderr) = std::thread::scope(|scope| {
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout =
            scope.spawn(move || stdout.map_or(Ok(Vec::new()), |out| read_kept(out, stdout_kept)));
        let stderr = stderr.map_or(Ok(Vec::new()), |err| read_kept(err, UNCHECKED_KEPT));
        let stdout = stdout
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        (stdout, stderr)
    });
    let status = child.wait().map_err(spawn_error)?;
    let (stdout, stderr) = (stdout.map_err(spawn_error)?, stderr.map_err(spawn_error)?);
    let failed = |reason: String| BenchError::CheckFailed {
        command: row.display(),
        reason,
    };
    if !status.success() {
        return Err(failed(format!(
            "it exited unsuccessfully ({status}); stdout: {}; stderr: {}",
            truncated(String::from_utf8_lossy(&stdout).trim()),
            truncated(String::from_utf8_lossy(&stderr).trim()),
        )));
    }
    row.stdout
        .map(|check| stdout_value(check, &stdout).map_err(failed))
        .transpose()
}

/// The bytes of an output the correctness run keeps when it checks nothing
/// in it, enough for its message: 64 KiB.
const UNCHECKED_KEPT: usize = 64 << 10;

/// Reads `reader` to its end, keeping its first `kept` bytes.
fn read_kept(mut reader: impl Read, kept: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut reader)
        .take(u64::try_from(kept).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    io::copy(&mut reader, &mut io::sink())?;
    Ok(bytes)
}

/// Finds a runtime's executable, canonicalized so the file hashed is the file
/// run: a `program` containing a `/` is relative to `dir`, any other is looked
/// up on `PATH`.
pub fn find_program(program: &str, dir: &Path) -> Option<PathBuf> {
    let found = if program.contains('/') {
        Some(dir.join(program)).filter(|path| is_executable(path))
    } else {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|entry| entry.join(program))
                .find(|candidate| is_executable(candidate))
        })
    };
    found.and_then(|path| path.canonicalize().ok())
}

/// Finds a runtime's executable and refuses a script.
fn resolve_program(runtime: &Runtime, dir: &Path) -> Result<Program, BenchError> {
    let path = find_program(&runtime.program, dir).ok_or_else(|| BenchError::ProgramNotFound {
        runtime: runtime.name.clone(),
        program: runtime.program.clone(),
    })?;
    if starts_with_shebang(&path)? {
        return Err(BenchError::ProgramIsScript {
            runtime: runtime.name.clone(),
            path,
        });
    }
    Ok(Program {
        path,
        argv0: runtime.program.clone(),
        env: runtime.env.clone(),
    })
}

/// A row's arguments after the program, and the variables it sets over the
/// runtime's `env`: the runtime's `args`, its `cold` args and env when the
/// workload is cold, then the script. `resolve` turns a file argument into the
/// absolute path passed.
fn row_args(
    runtime: &Runtime,
    workload: &Workload,
    script: &Path,
    resolve: &mut impl FnMut(&Path) -> Result<PathBuf, BenchError>,
) -> Result<(Vec<OsString>, BTreeMap<String, String>), BenchError> {
    let cold = runtime.cold.as_ref().filter(|_| workload.cold);
    let cold_args = cold.map_or(&[][..], |cold| &cold.args);
    let mut args = Vec::new();
    for arg in runtime.args.iter().chain(cold_args) {
        args.push(match arg {
            Arg::Text(text) => OsString::from(text),
            Arg::File(FileArg { file }) => resolve(file)?.into_os_string(),
        });
    }
    args.push(script.as_os_str().to_owned());
    let env = cold.map(|cold| cold.env.clone()).unwrap_or_default();
    Ok((args, env))
}

fn starts_with_shebang(path: &Path) -> Result<bool, BenchError> {
    let read_error = |source| BenchError::Read {
        path: path.to_owned(),
        source,
    };
    let mut start = Vec::with_capacity(2);
    File::open(path)
        .map_err(read_error)?
        .take(2)
        .read_to_end(&mut start)
        .map_err(read_error)?;
    Ok(start == b"#!")
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Finds a file the suite names, canonicalized, relative to `dir`.
fn resolve_file(dir: &Path, file: &Path) -> Result<PathBuf, BenchError> {
    let path = dir.join(file);
    path.canonicalize().map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            BenchError::SuiteFileMissing { path, source }
        } else {
            BenchError::Read { path, source }
        }
    })
}

fn hash_artifacts(artifacts: &BTreeSet<PathBuf>) -> Result<BTreeMap<PathBuf, String>, BenchError> {
    artifacts
        .iter()
        .map(|path| Ok((path.clone(), hash_file(path)?)))
        .collect()
}

fn hash_file(path: &Path) -> Result<String, BenchError> {
    let read_error = |source| BenchError::Read {
        path: path.to_owned(),
        source,
    };
    let file = File::open(path).map_err(read_error)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(file).map_err(read_error)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// The artifacts whose hash differs between `before` and `after`, which hash
/// the same set of paths.
fn changed_artifacts(
    before: &BTreeMap<PathBuf, String>,
    after: &BTreeMap<PathBuf, String>,
) -> Vec<PathBuf> {
    before
        .iter()
        .filter(|&(path, hash)| after.get(path) != Some(hash))
        .map(|(path, _)| path.clone())
        .collect()
}

fn probe(
    runtime: &Runtime,
    program: &Program,
    dir: &Path,
    hash: &str,
) -> Result<RuntimeInfo, BenchError> {
    let version = probe_line(program, &runtime.version_args, dir)?;
    let v8 = runtime
        .v8_args
        .as_ref()
        .map(|args| probe_line(program, args, dir))
        .transpose()?;
    let bytes = fs::metadata(&program.path)
        .map_err(|source| BenchError::Read {
            path: program.path.clone(),
            source,
        })?
        .len();
    Ok(RuntimeInfo {
        name: runtime.name.clone(),
        version,
        v8,
        blake3: hash.to_owned(),
        bytes,
        gzip_bytes: gzip_size(&program.path)?,
    })
}

/// The size of `path` gzipped at gzip's default level, counted from `gzip`'s
/// output as it streams. `-n` leaves the file's name and time out of the
/// header, so the size depends on the bytes alone.
///
/// # Errors
///
/// Fails when `gzip` can't run or fails.
pub fn gzip_size(path: &Path) -> Result<u64, BenchError> {
    let command = display_command(
        Path::new("gzip"),
        &[OsStr::new("-c"), OsStr::new("-n"), path.as_os_str()],
    );
    let spawn_error = |source| BenchError::Spawn {
        command: command.clone(),
        source,
    };
    let mut child = Command::new("gzip")
        .args(["-c", "-n"])
        .arg(path)
        // gzip reads default options from it
        .env_remove("GZIP")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(spawn_error)?;
    let counted = child
        .stdout
        .take()
        .map_or(Ok(0), |mut stdout| io::copy(&mut stdout, &mut io::sink()));
    let status = child.wait().map_err(spawn_error)?;
    let size = counted.map_err(spawn_error)?;
    if !status.success() {
        return Err(BenchError::CommandFailed { command, status });
    }
    Ok(size)
}

/// Runs `program` with `args` and returns the first non-empty line it prints.
fn probe_line(program: &Program, args: &[String], dir: &Path) -> Result<String, BenchError> {
    let command = display_command(&program.path, args);
    let output = program
        .command(dir)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .map_err(|source| BenchError::Spawn {
            command: command.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(BenchError::CommandFailed {
            command,
            status: output.status,
        });
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
        .ok_or(BenchError::NoVersion { command })
}

/// Sets up the run's [`Sampler`] from the context's [`Measure`]: for a
/// phases run, the runtimes it names must be the suite's, and for a counter
/// run, `perf` must count ([`Perf::find`]).
fn sampler(suite: &Suite, context: &RunContext) -> Result<Sampler, BenchError> {
    let dir = |name: &str| -> Result<PathBuf, BenchError> {
        let dir = context.work_dir.join(name);
        fs::create_dir_all(&dir).map_err(|source| BenchError::Write {
            path: dir.clone(),
            source,
        })?;
        Ok(dir)
    };
    match &context.measure {
        Measure::Wall => Ok(Sampler::Wall),
        Measure::Phases { runtimes } => {
            if let Some(unknown) = runtimes
                .iter()
                .find(|name| !suite.runtimes.iter().any(|runtime| &runtime.name == *name))
            {
                return Err(BenchError::SuiteInvalid(format!(
                    "phases are asked of the runtime `{unknown}`, which the suite doesn't have"
                )));
            }
            Ok(Sampler::Phases {
                file: dir("phases")?.join("sample.txt"),
            })
        }
        Measure::Counters => {
            let dir = dir("counters")?;
            let perf = Perf::find(&dir.join("probe.csv"))?;
            Ok(Sampler::Counters {
                perf,
                out: dir.join("sample.csv"),
            })
        }
    }
}

/// The wall clock, in nanoseconds since the Unix epoch, for the spans a
/// phases sample prices outside the runtime's own.
fn unix_ns() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos())
}

/// Runs one fresh process of `row` and times it, measuring what `sampler`
/// says.
fn run_once(row: &RowCommand, dir: &Path, sampler: &Sampler) -> Result<Sample, BenchError> {
    let remove = |path: &Path| match fs::remove_file(path) {
        Err(source) if source.kind() != io::ErrorKind::NotFound => Err(BenchError::Write {
            path: path.to_owned(),
            source,
        }),
        _ => Ok(()),
    };
    // stdin is opened here, before the timed span
    let (mut command, phases_file, counts_out) = match sampler {
        Sampler::Counters { perf, out } => {
            remove(out)?;
            (row.counted(perf, out, dir)?, None, Some((perf, out)))
        }
        Sampler::Phases { file } if row.phases => {
            remove(file)?;
            (row.command(dir)?, Some(file), None)
        }
        Sampler::Wall | Sampler::Phases { .. } => (row.command(dir)?, None, None),
    };
    let spawn_unix_ns = phases_file.map(|_| unix_ns());
    let start = Instant::now();
    let reaped = command.spawn().and_then(reap);
    let elapsed = start.elapsed();
    let reap_unix_ns = phases_file.map(|_| unix_ns());
    let (status, max_rss_kib) = reaped.map_err(|source| BenchError::Spawn {
        command: row.display(),
        source,
    })?;
    if !status.success() {
        return Err(BenchError::CommandFailed {
            command: row.display(),
            status,
        });
    }
    let counts = counts_out.map(|(perf, out)| perf.read(out)).transpose()?;
    let phases = match (phases_file, spawn_unix_ns, reap_unix_ns) {
        (Some(file), Some(spawn_unix_ns), Some(reap_unix_ns)) => {
            let text = fs::read_to_string(file).map_err(|source| {
                if source.kind() == io::ErrorKind::NotFound {
                    BenchError::Phases(format!(
                        "`{}` wrote no phases file, so it isn't a phases build",
                        row.display()
                    ))
                } else {
                    BenchError::Read {
                        path: file.clone(),
                        source,
                    }
                }
            })?;
            phases::durations(
                &text,
                SampleSpan {
                    spawn_unix_ns,
                    reap_unix_ns,
                },
            )?
        }
        _ => Vec::new(),
    };
    Ok(Sample {
        wall_ns: u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
        counts,
        phases,
        max_rss_kib,
    })
}

/// Waits for `child` and reaps it, returning its exit status and, on Linux,
/// its peak resident set size in KiB (`gro_sys::wait_child`).
#[cfg(target_os = "linux")]
fn reap(child: std::process::Child) -> io::Result<(std::process::ExitStatus, Option<u64>)> {
    gro_sys::wait_child(child).map(|reaped| (reaped.status, Some(reaped.max_rss_kib)))
}

/// Waits for `child` and reaps it, returning its exit status; elsewhere no
/// peak is read.
#[cfg(not(target_os = "linux"))]
fn reap(mut child: std::process::Child) -> io::Result<(std::process::ExitStatus, Option<u64>)> {
    child.wait().map(|status| (status, None))
}

/// The peak memory of every sample, when each has one.
fn rss_metric(samples: &[Sample]) -> Option<Metric> {
    samples
        .iter()
        .map(|sample| sample.max_rss_kib)
        .collect::<Option<Vec<u64>>>()
        .map(metric)
}

fn wall_metric(samples: &[Sample]) -> Metric {
    metric(samples.iter().map(|sample| sample.wall_ns).collect())
}

/// The metric of `samples`, which a row always has.
fn metric(samples: Vec<u64>) -> Metric {
    #[expect(
        clippy::expect_used,
        reason = "every row gets `samples` samples, which the suite's type keeps above 0"
    )]
    let stats = Stats::from_samples(&samples).expect("a row has samples");
    Metric { stats, samples }
}

/// One count of every sample, when each was counted.
fn count_metric(samples: &[Sample], count: impl Fn(&Counts) -> u64) -> Option<Metric> {
    samples
        .iter()
        .map(|sample| sample.counts.as_ref().map(&count))
        .collect::<Option<Vec<u64>>>()
        .map(metric)
}

/// Each phase's durations over `row`'s samples, which must name the same
/// phases in the same order.
fn phase_metrics(row: &RowCommand, samples: &[Sample]) -> Result<Vec<PhaseMetric>, BenchError> {
    let Some(first) = samples.first() else {
        return Ok(Vec::new());
    };
    let names: Vec<&str> = first.phases.iter().map(|(name, _)| name.as_str()).collect();
    for sample in samples {
        if !sample
            .phases
            .iter()
            .map(|(name, _)| name.as_str())
            .eq(names.iter().copied())
        {
            return Err(BenchError::Phases(format!(
                "the samples of `{}` marked different phases",
                row.display()
            )));
        }
    }
    Ok(names
        .iter()
        .enumerate()
        .map(|(index, name)| PhaseMetric {
            name: (*name).to_owned(),
            ns: metric(
                samples
                    .iter()
                    .map(|sample| sample.phases[index].1)
                    .collect(),
            ),
        })
        .collect())
}

fn noise_record(first: &RowRecord, copy: &[Sample]) -> NoiseRecord {
    let wall_ns = wall_metric(copy);
    let instructions = count_metric(copy, |counts| counts.instructions);
    let instructions_median_deviation = first
        .instructions
        .as_ref()
        .zip(instructions.as_ref())
        .map(|(first, copy)| ratio_deviation(first.stats.median, copy.stats.median));
    NoiseRecord {
        runtime: first.runtime.clone(),
        workload: first.workload.clone(),
        median_deviation: ratio_deviation(first.wall_ns.stats.median, wall_ns.stats.median),
        wall_ns,
        instructions,
        instructions_median_deviation,
    }
}

/// A command line for messages: the program and its arguments, space-separated.
fn display_command(program: &Path, args: &[impl AsRef<OsStr>]) -> String {
    let mut line = program.display().to_string();
    for arg in args {
        line.push(' ');
        line.push_str(&arg.as_ref().to_string_lossy());
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_suite_file_is_an_input_error() {
        let dir = std::env::temp_dir();
        let error = resolve_file(&dir, Path::new("gro_bench_no_such_file.js")).unwrap_err();
        assert!(
            matches!(error, BenchError::SuiteFileMissing { .. }),
            "{error:?}"
        );
        assert!(error.is_input_error());
        // an existing file resolves
        assert!(resolve_file(Path::new("."), Path::new("Cargo.toml")).is_ok());
    }

    #[test]
    fn a_cold_row_adds_the_runtime_cold_switches() {
        let runtime: Runtime = serde_json::from_str(
            r#"{"name": "deno", "program": "deno", "args": ["run"], "env": {"A": "1"},
                "cold": {"args": ["--no-code-cache", {"file": "shim.js"}], "env": {"B": "0"}}}"#,
        )
        .unwrap();
        let workload = |cold: bool| -> Workload {
            serde_json::from_str(&format!(
                r#"{{"name": "w", "script": "w.js", "cold": {cold}}}"#
            ))
            .unwrap()
        };
        let mut resolve = |file: &Path| Ok(Path::new("/abs").join(file));
        let script = Path::new("/abs/w.js");
        let (warm_args, warm_env) =
            row_args(&runtime, &workload(false), script, &mut resolve).unwrap();
        assert_eq!(warm_args, ["run", "/abs/w.js"]);
        assert!(warm_env.is_empty());
        let (cold_args, cold_env) =
            row_args(&runtime, &workload(true), script, &mut resolve).unwrap();
        assert_eq!(
            cold_args,
            ["run", "--no-code-cache", "/abs/shim.js", "/abs/w.js"]
        );
        assert_eq!(cold_env, BTreeMap::from([("B".to_owned(), "0".to_owned())]));
    }

    #[test]
    fn gzip_size_counts_the_compressed_bytes() {
        let path = std::env::temp_dir().join(format!("gro_bench_gzip_{}", std::process::id()));
        fs::write(&path, "a".repeat(100_000)).unwrap();
        let size = gzip_size(&path);
        fs::remove_file(&path).unwrap();
        let size = size.unwrap();
        // a run of one byte compresses to a few hundred bytes, header included
        assert!((20..1000).contains(&size), "{size}");
    }

    #[test]
    fn changed_artifacts_names_each_change() {
        let hashes = |pairs: &[(&str, &str)]| -> BTreeMap<PathBuf, String> {
            pairs
                .iter()
                .map(|&(path, hash)| (PathBuf::from(path), hash.to_owned()))
                .collect()
        };
        let before = hashes(&[("/a", "1"), ("/b", "2"), ("/c", "3")]);
        assert!(changed_artifacts(&before, &before).is_empty());
        let after = hashes(&[("/a", "1"), ("/b", "9"), ("/c", "3")]);
        assert_eq!(changed_artifacts(&before, &after), [PathBuf::from("/b")]);
    }
}
