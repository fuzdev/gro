//! The process runner: measures a suite's rows and returns the run's record.
//!
//! A run:
//!
//! 1. checks each corpora collection the suite pins against its pinned tree
//!    (only those: a corpora commit elsewhere doesn't count), resolves each
//!    runtime's executable, refusing a script (a file starting with `#!`),
//!    writes a fresh copy of each collection a workload names, with the
//!    workload's script at its root ([`collection`](crate::collection)), and
//!    of each project ([`project`](crate::project)), refusing one whose files
//!    don't hash to the suite's pin of them, and hashes the artifacts
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
//!    interleaved order of [`schedule`](crate::schedule::schedule), with the
//!    first row's A/A copy as one more row (a row is a workload and a
//!    runtime);
//! 5. hashes the artifacts, lists each copy, and reads the pinned
//!    collections' trees and the other checkouts' states again, and discards
//!    the run if any changed (a copy gaining or losing a path counts), since
//!    a rebuild or an edit mid-run means the samples timed different code:
//!    the copies cover what a collection's workloads run, the collections'
//!    trees that the checkout still holds what was copied, and a checkout's
//!    state what a link or script reaches in it;
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
//! A run holds an exclusive lock on its work directory throughout
//! ([`lock`](crate::lock)), refusing to start while another run holds it,
//! since both would rewrite the same copies and sample files. Every process
//! of a runtime it spawns (probes, correctness runs, warmups, samples, and
//! perf) has the suite's deadline ([`Suite::timeout`]): one past it is
//! killed and reaped, and the run fails ([`watchdog`](crate::watchdog)).
//! The local `git` reads and `gzip` run without one.
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
//! `argv[0]`, as a shell would pass it (but under `perf stat`, which execs the
//! canonical path, [`counters`](crate::counters)), and with a cleared
//! environment holding only the caller's `PATH` and `HOME` and the variables
//! the suite sets. Its working directory is the suite's directory, a
//! collection's workloads included.

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

use artifacts::{changed_during_run, hash_artifacts, list_copies, read_checkouts};
use check::check_rows;
use command::probe_runtimes;
use metrics::{noise_record, row_records};
use prepare::{Plan, check_corpora_pin, plan_rows};
use sample::{Sampler, rss_floor, sample_rows, sampler};

use crate::budget::check_budgets;
use crate::error::BenchError;
use crate::lock::{DirLock, LockError, LockIoError, try_lock_dir};
use crate::record::{GitState, Machine, MachineQuiet, RECORD_VERSION, RunRecord, SuiteId};
use crate::suite::Suite;
use crate::watchdog::Watchdog;

pub(crate) use command::display_command;
pub use command::find_program;

/// What a run takes from the caller: what it records but can't learn itself,
/// and where it writes.
#[derive(Debug, Clone)]
pub struct RunContext {
    pub git: GitState,
    pub machine: Machine,
    pub machine_quiet: MachineQuiet,
    /// The directory the run writes its copies under, a collection's at
    /// `corpora/<tree>/<collection>` and a project's at
    /// `projects/<workload>`, each replaced on every run.
    /// `cargo xtask bench` passes the gro checkout's gitignored
    /// `target/bench/`, which is trusted like the rest of `target/` (a
    /// symlink already in it is followed), since anyone who can write there
    /// can already change build output. A phases or counter run also writes
    /// each sample's file here, under `phases/` or `counters/`. The run
    /// holds its `lock` file throughout.
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
    /// whose processes get [`PHASES_FILE_ENV`](crate::phases::PHASES_FILE_ENV)
    /// naming the file to write.
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
    // held to the end, so no other run rewrites the copies or sample files
    let _lock = lock_work_dir(&context.work_dir)?;
    let watchdog = Watchdog::start(suite.timeout()).map_err(|source| BenchError::Spawn {
        command: "the watchdog thread".to_owned(),
        source,
    })?;
    let sampler = sampler(suite, &context.measure, &context.work_dir, &watchdog)?;
    let corpora = check_corpora_pin(suite, dir)?;
    let started_unix_s = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());

    let Plan {
        programs,
        rows,
        copies,
        artifacts,
        suite_files,
    } = plan_rows(suite, dir, &context.work_dir, corpora.as_ref(), &sampler)?;
    let before = hash_artifacts(&artifacts)?;
    let listed = list_copies(&copies)?;
    let artifact_hashes: BTreeMap<String, String> = suite_files
        .into_iter()
        .map(|(file, resolved)| (file, before[&resolved].clone()))
        .collect();
    let checkouts = read_checkouts(suite, dir)?;

    let runtimes = probe_runtimes(&suite.runtimes, &programs, dir, &before, &watchdog)?;
    let snapshot_bytes = snapshot_size(context.snapshot.as_deref())?;
    // a measurement build isn't the size that ships
    if !matches!(sampler, Sampler::Phases { .. }) {
        check_budgets(suite, &runtimes, snapshot_bytes)?;
    }

    check_rows(suite, &rows, dir, &watchdog)?;

    let samples = sample_rows(suite, &rows, dir, &sampler, &watchdog)?;
    // only a wall run's spawner is the harness alone ([`RunRecord`])
    let wall = matches!(sampler, Sampler::Wall);
    let rss_floor_kib = if wall {
        rss_floor(dir, &sampler, &watchdog)?
    } else {
        None
    };

    let changed = changed_during_run(
        &artifacts,
        &before,
        &copies,
        &listed,
        corpora.as_ref(),
        &checkouts,
    )?;
    if !changed.is_empty() {
        return Err(BenchError::ArtifactsChanged { paths: changed });
    }

    let rows_record = row_records(&rows, &samples, wall)?;
    let noise = noise_record(&rows_record[0], &samples[rows.len()]);

    Ok(RunRecord {
        record_version: RECORD_VERSION,
        mode: sampler.mode(),
        counters: sampler.counter_tool(),
        suite: SuiteId {
            name: suite.name.clone(),
            version: suite.version,
        },
        config_hash: suite.config_hash(),
        started_unix_s,
        git: context.git,
        machine: context.machine,
        machine_quiet: context.machine_quiet,
        collections: corpora
            .map(|corpora| corpora.collections)
            .unwrap_or_default(),
        projects: suite.project_pins(),
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

/// Takes the exclusive lock on `work_dir`, refusing while another run holds
/// it.
fn lock_work_dir(work_dir: &Path) -> Result<DirLock, BenchError> {
    try_lock_dir(work_dir).map_err(|error| match error {
        LockError::Held { holder, .. } => BenchError::WorkDirLocked {
            dir: work_dir.to_owned(),
            holder,
        },
        LockError::Io(LockIoError { path, source }) => BenchError::Write { path, source },
    })
}

/// The size of the startup snapshot at `snapshot`, when the context names
/// one.
fn snapshot_size(snapshot: Option<&Path>) -> Result<Option<u64>, BenchError> {
    snapshot
        .map(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.len())
                .map_err(|source| BenchError::Read {
                    path: path.to_owned(),
                    source,
                })
        })
        .transpose()
}
