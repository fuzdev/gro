//! The commands a run spawns: a runtime's resolved executable, a row's
//! command, and the lookup and probes behind them.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::budget::gzip_size;
use crate::counters::Perf;
use crate::error::BenchError;
use crate::record::RuntimeInfo;
use crate::suite::{Arg, FileArg, Runtime, Stdout, Workload};
use crate::watchdog::Watchdog;

/// The environment variables a measured process keeps from the caller's.
const KEPT_ENV: [&str; 2] = ["PATH", "HOME"];

/// A runtime's executable, resolved.
#[derive(Debug, Clone)]
pub(super) struct Program {
    /// The canonical file, which the run hashes and executes.
    pub(super) path: PathBuf,
    /// The suite's `program` as written, passed as `argv[0]`, which a
    /// multi-call binary dispatches on.
    #[cfg_attr(not(unix), expect(dead_code, reason = "`argv[0]` is set on unix only"))]
    pub(super) argv0: String,
    /// The runtime's `env`.
    pub(super) env: BTreeMap<String, String>,
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
pub(super) struct RowCommand {
    pub(super) runtime: String,
    pub(super) workload: String,
    pub(super) program: Program,
    pub(super) args: Vec<OsString>,
    /// The runtime's `cold` env on a cold row, over the runtime's `env`.
    pub(super) env: BTreeMap<String, String>,
    pub(super) warmup: u32,
    /// The file each process reads on stdin, the runtime's `stdin`.
    pub(super) stdin: Option<PathBuf>,
    /// What the runtime's stdout holds, for the correctness run.
    pub(super) stdout: Option<Stdout>,
    /// Whether its samples write phase timings, under [`Sampler::Phases`](super::sample::Sampler::Phases).
    pub(super) phases: bool,
}

impl RowCommand {
    /// A process of this row, its stdin opened fresh, with stdout and
    /// stderr on the null device.
    pub(super) fn command(&self, dir: &Path) -> Result<Command, BenchError> {
        let mut command = self.program.command(dir);
        command.args(&self.args);
        self.finish(command)
    }

    /// A process of this row under `perf stat`, writing its counts to
    /// `out`, set up as [`command`](Self::command) sets one up.
    pub(super) fn counted(
        &self,
        perf: &Perf,
        out: &Path,
        dir: &Path,
    ) -> Result<Command, BenchError> {
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
    pub(super) fn display(&self) -> String {
        let line = display_command(&self.program.path, &self.args);
        match &self.stdin {
            Some(stdin) => format!("{line} < {}", stdin.display()),
            None => line,
        }
    }
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
pub(super) fn resolve_program(runtime: &Runtime, dir: &Path) -> Result<Program, BenchError> {
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
pub(super) fn row_args(
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

/// A command line for messages: the program and its arguments, space-separated.
pub fn display_command(program: &Path, args: &[impl AsRef<OsStr>]) -> String {
    let mut line = program.display().to_string();
    for arg in args {
        line.push(' ');
        line.push_str(&arg.as_ref().to_string_lossy());
    }
    line
}

/// Probes every runtime ([`probe`]), one thread each, since gzipping each
/// executable takes seconds; `hashes` holds each program's hash.
pub(super) fn probe_runtimes(
    runtimes: &[Runtime],
    programs: &[Program],
    dir: &Path,
    hashes: &BTreeMap<PathBuf, String>,
    watchdog: &Watchdog,
) -> Result<Vec<RuntimeInfo>, BenchError> {
    std::thread::scope(|scope| {
        #[expect(
            clippy::needless_collect,
            reason = "every thread spawns before the first join, or the probes run one at a time"
        )]
        let probes: Vec<_> = runtimes
            .iter()
            .zip(programs)
            .map(|(runtime, program)| {
                let hash = &hashes[&program.path];
                scope.spawn(move || probe(runtime, program, dir, hash, watchdog))
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
    })
}

/// Probes `runtime`'s version (and V8 version, when it names the args) and
/// measures its executable's size, as is and gzipped.
fn probe(
    runtime: &Runtime,
    program: &Program,
    dir: &Path,
    hash: &str,
    watchdog: &Watchdog,
) -> Result<RuntimeInfo, BenchError> {
    let version = probe_line(program, &runtime.version_args, dir, watchdog)?;
    let v8 = runtime
        .v8_args
        .as_ref()
        .map(|args| probe_line(program, args, dir, watchdog))
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

/// Runs `program` with `args` and returns the first non-empty line it
/// prints, killing it past `watchdog`'s deadline.
fn probe_line(
    program: &Program,
    args: &[String],
    dir: &Path,
    watchdog: &Watchdog,
) -> Result<String, BenchError> {
    let command = display_command(&program.path, args);
    let spawn_error = |source| BenchError::Spawn {
        command: command.clone(),
        source,
    };
    let mut probe = program.command(dir);
    probe
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = watchdog
        .run(
            &mut probe,
            |_| command.clone(),
            std::process::Child::wait_with_output,
        )?
        .map_err(spawn_error)?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
