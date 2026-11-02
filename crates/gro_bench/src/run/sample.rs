//! One timed process: how a run takes its samples, and the sample each
//! process yields.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use super::Measure;
use super::command::{Program, RowCommand, find_program};
use crate::counters::{Counts, Perf};
use crate::error::BenchError;
use crate::phases::{self, SampleSpan};
use crate::record::{CounterTool, Mode};
use crate::suite::Suite;
use crate::watchdog::Watchdog;

/// How the run takes its samples, set up from the [`Measure`] with what
/// each measure needs, and the record's [`Mode`] read from it.
#[derive(Debug)]
pub(super) enum Sampler {
    Wall,
    Phases {
        /// The runtimes whose samples write phases, each a phases build.
        runtimes: BTreeSet<String>,
        /// The file each sample of a phases runtime writes.
        file: PathBuf,
    },
    Counters {
        perf: Perf,
        /// The file each sample's counts go to.
        out: PathBuf,
    },
}

impl Sampler {
    /// The record's mode for a run sampled this way.
    pub(super) const fn mode(&self) -> Mode {
        match self {
            Self::Wall => Mode::Wall,
            Self::Phases { .. } => Mode::Phases,
            Self::Counters { .. } => Mode::Counters,
        }
    }

    /// The counter tool a counter run reads its counts with.
    pub(super) fn counter_tool(&self) -> Option<CounterTool> {
        match self {
            Self::Counters { perf, .. } => Some(perf.tool.clone()),
            Self::Wall | Self::Phases { .. } => None,
        }
    }

    /// The file the samples of `runtime` write their phases to, in a phases
    /// run that names it.
    pub(super) fn phases_file(&self, runtime: &str) -> Option<&Path> {
        match self {
            Self::Phases { runtimes, file } if runtimes.contains(runtime) => Some(file),
            Self::Wall | Self::Phases { .. } | Self::Counters { .. } => None,
        }
    }
}

/// One timed process.
#[derive(Debug, Clone)]
pub(super) struct Sample {
    pub(super) wall_ns: u64,
    /// Its counts, under [`Sampler::Counters`].
    pub(super) counts: Option<Counts>,
    /// Its phases' durations, under [`Sampler::Phases`] for a phases runtime.
    pub(super) phases: Vec<(String, u64)>,
    /// Its peak resident set size in KiB, where the harness reads one
    /// ([`reap`]).
    pub(super) max_rss_kib: Option<u64>,
}

/// How many times [`rss_floor`] runs its trivial program.
const FLOOR_RUNS: usize = 4;

/// The peak a sample reads at least, whatever it runs: the largest of a
/// trivial program's (`true`, found on `PATH`) over a few runs as `sampler`
/// runs the samples, taken after them, when the harness's own peak is
/// highest. `None` where the harness reads no peak.
pub(super) fn rss_floor(
    dir: &Path,
    sampler: &Sampler,
    watchdog: &Watchdog,
) -> Result<Option<u64>, BenchError> {
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
        let peak = run_once(&row, dir, sampler, watchdog)?.max_rss_kib;
        floor = floor.max(peak);
    }
    Ok(floor)
}

/// Sets up the run's [`Sampler`] from `measure`, writing under `work_dir`:
/// for a phases run, the runtimes it names must be the suite's, and for a
/// counter run, `perf` must count ([`Perf::find`]).
pub(super) fn sampler(
    suite: &Suite,
    measure: &Measure,
    work_dir: &Path,
    watchdog: &Watchdog,
) -> Result<Sampler, BenchError> {
    let dir = |name: &str| -> Result<PathBuf, BenchError> {
        let dir = work_dir.join(name);
        fs::create_dir_all(&dir).map_err(|source| BenchError::Write {
            path: dir.clone(),
            source,
        })?;
        Ok(dir)
    };
    match measure {
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
                runtimes: runtimes.clone(),
                file: dir("phases")?.join("sample.txt"),
            })
        }
        Measure::Counters => {
            let dir = dir("counters")?;
            let perf = Perf::find(&dir.join("probe.csv"), watchdog)?;
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
/// says, the process killed should it pass `watchdog`'s deadline.
pub(super) fn run_once(
    row: &RowCommand,
    dir: &Path,
    sampler: &Sampler,
    watchdog: &Watchdog,
) -> Result<Sample, BenchError> {
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
        Sampler::Phases { file, .. } if row.phases => {
            remove(file)?;
            (row.command(dir)?, Some(file), None)
        }
        Sampler::Wall | Sampler::Phases { .. } => (row.command(dir)?, None, None),
    };
    let spawn_unix_ns = phases_file.map(|_| unix_ns());
    let start = Instant::now();
    let reaped = watchdog.run(
        &mut command,
        |_| row.display(),
        |child| {
            let reaped = reap(child)?;
            Ok((reaped, start.elapsed(), phases_file.map(|_| unix_ns())))
        },
    )?;
    let ((status, max_rss_kib), elapsed, reap_unix_ns) =
        reaped.map_err(|source| BenchError::Spawn {
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
pub(super) fn reap(
    child: std::process::Child,
) -> io::Result<(std::process::ExitStatus, Option<u64>)> {
    gro_sys::wait_child(child).map(|reaped| (reaped.status, Some(reaped.max_rss_kib)))
}

/// Waits for `child` and reaps it, returning its exit status; elsewhere no
/// peak is read.
#[cfg(not(target_os = "linux"))]
pub(super) fn reap(
    mut child: std::process::Child,
) -> io::Result<(std::process::ExitStatus, Option<u64>)> {
    child.wait().map(|status| (status, None))
}
