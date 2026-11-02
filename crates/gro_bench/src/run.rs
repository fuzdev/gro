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
//! ([`phases`](crate::phases)), or hardware counters under `perf stat`
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

mod artifacts;
mod check;
mod command;
mod metrics;
mod prepare;
mod sample;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use artifacts::{changed_artifacts, hash_artifacts, read_checkouts};
use check::check_rows;
use command::{Program, RowCommand, probe, resolve_program, row_args};
use metrics::{count_metric, metric, noise_record, phase_metrics, rss_metric, wall_metric};
use prepare::{materialize_collections, materialize_projects, resolve_file};
use sample::{Sample, Sampler, rss_floor, run_once, sampler};

use crate::budget::check_budgets;
use crate::collection::list_paths;
use crate::error::BenchError;
use crate::git::read_corpora_tree;
use crate::phases::PHASES_FILE_ENV;
use crate::record::{
    CheckoutState, GitState, Machine, MachineQuiet, Mode, RECORD_VERSION, RowRecord, RunRecord,
    SuiteId,
};
use crate::schedule::schedule;
use crate::suite::{FileArg, Suite};

pub(crate) use command::display_command;
pub use command::find_program;

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
