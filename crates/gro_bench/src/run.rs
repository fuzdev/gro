//! The process runner: measures a suite's rows and returns the run's record.
//!
//! A run:
//!
//! 1. resolves each runtime's executable, refusing a script (a file starting
//!    with `#!`), and hashes the artifacts (every executable, script, and file
//!    argument) with blake3;
//! 2. probes each runtime's version;
//! 3. runs each row's warmup processes, then its timed samples, in the
//!    interleaved order of [`schedule`], with the
//!    first row's A/A copy as one more row;
//! 4. hashes the artifacts again and discards the run if any changed, since a
//!    rebuild mid-run means the samples timed different code;
//! 5. computes each row's statistics.
//!
//! Every sample is a fresh process, timed from just before its spawn to just
//! after its exit is reaped, with stdin, stdout and stderr on the null
//! device. A process that exits unsuccessfully fails the run, naming the
//! command to re-run alone. Nothing prints during the run, so the terminal is
//! no part of any sample.
//!
//! Every process, samples and version probes alike, runs the canonical
//! executable (the file hashed) with the suite's `program` as written as its
//! `argv[0]`, as a shell would pass it, and with a cleared environment holding
//! only the caller's `PATH` and `HOME`.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime};

use crate::error::BenchError;
use crate::record::{
    GitState, Machine, MachineQuiet, Metric, NoiseRecord, RECORD_VERSION, RowRecord, RunRecord,
    RuntimeInfo, SuiteId,
};
use crate::schedule::schedule;
use crate::stats::{Stats, ratio_deviation};
use crate::suite::{Arg, FileArg, Runtime, Suite};

/// What a run records but can't learn itself, from the caller.
#[derive(Debug, Clone)]
pub struct RunContext {
    pub git: GitState,
    pub machine: Machine,
    pub machine_quiet: MachineQuiet,
    /// The corpora tree id, when the suite reads the corpora.
    pub corpora: Option<String>,
    /// The workspace repos the suite reads, at their commit SHAs.
    pub workspace: BTreeMap<String, String>,
}

/// One timed process. Counter mode adds its counts (`instructions:u`,
/// `cycles:u`) here.
#[derive(Debug, Clone, Copy)]
struct Sample {
    wall_ns: u64,
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
        command.env_clear();
        for key in KEPT_ENV {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.current_dir(dir).stdin(Stdio::null());
        command
    }
}

/// A row's command, resolved.
#[derive(Debug)]
struct RowCommand {
    runtime: String,
    workload: String,
    program: Program,
    args: Vec<OsString>,
    warmup: u32,
}

impl RowCommand {
    fn command(&self, dir: &Path) -> Command {
        let mut command = self.program.command(dir);
        command
            .args(&self.args)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn display(&self) -> String {
        display_command(&self.program.path, &self.args)
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
    let mut rows = Vec::new();
    for workload in &suite.workloads {
        let script = resolve(&workload.script)?;
        for (runtime, program) in suite.runtimes.iter().zip(&programs) {
            let mut args = Vec::new();
            for arg in &runtime.args {
                args.push(match arg {
                    Arg::Text(text) => OsString::from(text),
                    Arg::File(FileArg { file }) => resolve(file)?.into_os_string(),
                });
            }
            args.push(script.clone().into_os_string());
            rows.push(RowCommand {
                runtime: runtime.name.clone(),
                workload: workload.name.clone(),
                program: program.clone(),
                args,
                warmup: workload.warmup,
            });
        }
    }
    let before = hash_artifacts(&artifacts)?;
    let artifact_hashes: BTreeMap<String, String> = suite_files
        .into_iter()
        .map(|(file, resolved)| (file, before[&resolved].clone()))
        .collect();

    let runtimes = suite
        .runtimes
        .iter()
        .zip(&programs)
        .map(|(runtime, program)| probe(runtime, program, dir, &before[&program.path]))
        .collect::<Result<Vec<_>, _>>()?;

    // the noise row: the first row again, measured as a row of its own
    let commands: Vec<&RowCommand> = rows.iter().chain(rows.first()).collect();
    let warmups: Vec<usize> = commands.iter().map(|row| row.warmup as usize).collect();
    for index in schedule(&warmups) {
        run_once(commands[index], dir)?;
    }
    let sample_count = suite.samples.get() as usize;
    let mut samples: Vec<Vec<Sample>> = vec![Vec::with_capacity(sample_count); commands.len()];
    for index in schedule(&vec![sample_count; commands.len()]) {
        samples[index].push(run_once(commands[index], dir)?);
    }

    let after = hash_artifacts(&artifacts)?;
    let changed = changed_artifacts(&before, &after);
    if !changed.is_empty() {
        return Err(BenchError::ArtifactsChanged { paths: changed });
    }

    let rows_record: Vec<RowRecord> = rows
        .iter()
        .zip(&samples)
        .map(|(row, samples)| RowRecord {
            runtime: row.runtime.clone(),
            workload: row.workload.clone(),
            warmup: row.warmup,
            wall_ns: wall_metric(samples),
        })
        .collect();
    let noise = noise_record(&rows_record[0], wall_metric(&samples[rows.len()]));

    Ok(RunRecord {
        record_version: RECORD_VERSION,
        suite: SuiteId {
            name: suite.name.clone(),
            version: suite.version,
        },
        config_hash: suite.config_hash(),
        started_unix_s,
        git: context.git,
        machine: context.machine,
        machine_quiet: context.machine_quiet,
        corpora: context.corpora,
        workspace: context.workspace,
        runtimes,
        artifacts: artifact_hashes,
        rows: rows_record,
        noise,
    })
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
    })
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
        std::fs::metadata(path)
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
    Ok(RuntimeInfo {
        name: runtime.name.clone(),
        version,
        v8,
        blake3: hash.to_owned(),
    })
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

/// Runs one fresh process of `row` and times it.
fn run_once(row: &RowCommand, dir: &Path) -> Result<Sample, BenchError> {
    let mut command = row.command(dir);
    let start = Instant::now();
    let status = command.status();
    let elapsed = start.elapsed();
    let status = status.map_err(|source| BenchError::Spawn {
        command: row.display(),
        source,
    })?;
    if !status.success() {
        return Err(BenchError::CommandFailed {
            command: row.display(),
            status,
        });
    }
    Ok(Sample {
        wall_ns: u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
    })
}

fn wall_metric(samples: &[Sample]) -> Metric {
    let samples: Vec<u64> = samples.iter().map(|sample| sample.wall_ns).collect();
    #[expect(
        clippy::expect_used,
        reason = "every row gets `samples` samples, which the suite's type keeps above 0"
    )]
    let stats = Stats::from_samples(&samples).expect("a row has samples");
    Metric { stats, samples }
}

fn noise_record(first: &RowRecord, copy: Metric) -> NoiseRecord {
    NoiseRecord {
        runtime: first.runtime.clone(),
        workload: first.workload.clone(),
        median_deviation: ratio_deviation(first.wall_ns.stats.median, copy.stats.median),
        wall_ns: copy,
    }
}

/// A command line for messages: the program and its arguments, space-separated.
fn display_command(program: &Path, args: &[impl AsRef<std::ffi::OsStr>]) -> String {
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
