//! Counter mode: each timed sample runs under `perf stat`, counting its
//! user-space instructions and cycles.
//!
//! The counts cover the process, its threads, and any process it forks
//! (perf's counters are inherited), so a runtime that spawns counts its
//! children's work too.
//!
//! `perf` is an external tool, found on `PATH`. Before any sample, the run
//! checks it works: `perf --version` prints a line, and a probe of `true`
//! counts both events for the whole of its run. A run refuses, naming why,
//! when perf is missing, when the kernel won't count (`perf_event_paranoid`,
//! no hardware counters, as in most VMs), or when an event was multiplexed
//! (counted for less than all of the run, so perf scaled an estimate).
//!
//! A sample runs `perf stat -x , -o <file> -e instructions:u,cycles:u --
//! <program> <args>` with the row's environment, working directory, and
//! stdin (its runtime's `stdin` file opened fresh for each sample, so no
//! sample reads what an earlier one left), and reads the counts back from the
//! file. Counting starts at the program's exec, so perf's own startup isn't
//! counted, but it is timed: a counter run's wall times run long. The program
//! runs by its canonical path, so its `argv[0]` is that path, not the suite's
//! `program`.
//!
//! The record keeps every sample's counts with their statistics: the median
//! is the headline the summary, the report, and an A/B compare, and the
//! minimum is recorded beside it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::BenchError;
use crate::record::CounterTool;
use crate::run::find_program;

/// The events counted, as `perf stat -e` takes them: user space only, so the
/// kernel's share of a process (exec, page faults) doesn't move the counts.
pub const EVENTS: [&str; 2] = ["instructions:u", "cycles:u"];

/// One process's counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub instructions: u64,
    pub cycles: u64,
}

/// A working `perf`, checked by [`Perf::find`].
#[derive(Debug, Clone)]
pub struct Perf {
    /// The executable, canonical.
    pub path: PathBuf,
    /// What the record names.
    pub tool: CounterTool,
}

impl Perf {
    /// Finds `perf` on `PATH` and checks it counts both [`EVENTS`] in full,
    /// writing its probe's output to `out`.
    ///
    /// # Errors
    ///
    /// Fails with [`BenchError::Counters`] saying why when perf is missing,
    /// prints no version, or can't count both events for a whole run.
    pub fn find(out: &Path) -> Result<Self, BenchError> {
        let dir = Path::new("/");
        let path = find_program("perf", dir).ok_or_else(|| {
            BenchError::Counters(
                "counter mode runs `perf stat`, and `perf` isn't on PATH (on Debian, the \
                 `linux-perf` package)"
                    .to_owned(),
            )
        })?;
        let version = Command::new(&path)
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_owned)
            })
            .ok_or_else(|| {
                BenchError::Counters(format!("`{} --version` failed", path.display()))
            })?;
        let perf = Self {
            path,
            tool: CounterTool {
                version,
                events: EVENTS.iter().map(|event| (*event).to_owned()).collect(),
            },
        };
        let probe = find_program("true", dir).ok_or_else(|| {
            BenchError::Counters("`true` isn't on PATH, to probe perf with".to_owned())
        })?;
        let mut command = perf.command(out, &probe, &[]);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let output = command.output().map_err(|source| BenchError::Spawn {
            command: format!("{} stat", perf.path.display()),
            source,
        })?;
        if !output.status.success() {
            return Err(BenchError::Counters(format!(
                "`perf stat` failed on `true` ({}): {}; the kernel may refuse counters to this \
                 user (`/proc/sys/kernel/perf_event_paranoid` above 2)",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        perf.read(out)?;
        Ok(perf)
    }

    /// A command running `program` with `args` under `perf stat`, writing its
    /// counts to `out`. The caller sets the environment, directory, and
    /// stdio.
    pub fn command(&self, out: &Path, program: &Path, args: &[OsString]) -> Command {
        let mut command = Command::new(&self.path);
        command
            .args(["stat", "-x", ",", "-o"])
            .arg(out)
            .args(["-e", &EVENTS.join(","), "--"])
            .arg(program)
            .args(args);
        command
    }

    /// Reads the counts `perf stat` wrote to `out`.
    ///
    /// # Errors
    ///
    /// Fails when the file can't be read, or with [`BenchError::Counters`]
    /// when it lacks an event or an event wasn't counted for the whole run.
    pub fn read(&self, out: &Path) -> Result<Counts, BenchError> {
        let text = std::fs::read_to_string(out).map_err(|source| BenchError::Read {
            path: out.to_owned(),
            source,
        })?;
        parse_counts(&text).map_err(BenchError::Counters)
    }
}

/// Parses `perf stat -x ,` output: per event, a line of the value, its unit,
/// the event, the time it ran, and the percent of that time it was counted,
/// then optional metric fields; `#` lines are comments.
///
/// # Errors
///
/// Says why when an event of [`EVENTS`] is missing, its value isn't a count
/// (`<not counted>`, `<not supported>`), or it was counted for less than the
/// whole run (multiplexed).
pub fn parse_counts(text: &str) -> Result<Counts, String> {
    let mut counts = [None; EVENTS.len()];
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let [value, _unit, event, _time, percent, ..] = fields[..] else {
            return Err(format!("perf printed a line with too few fields: {line}"));
        };
        let Some(index) = EVENTS.iter().position(|known| *known == event) else {
            continue;
        };
        let value: u64 = value
            .parse()
            .map_err(|_| format!("perf didn't count {event}: it printed `{value}`"))?;
        let percent: f64 = percent
            .parse()
            .map_err(|_| format!("perf printed no coverage for {event}: `{percent}`"))?;
        if percent < 100.0 {
            return Err(format!(
                "perf counted {event} for {percent}% of the run (multiplexed, so the count is \
                 scaled); free the counters (another perf, the NMI watchdog) and re-run"
            ));
        }
        counts[index] = Some(value);
    }
    match counts {
        [Some(instructions), Some(cycles)] => Ok(Counts {
            instructions,
            cycles,
        }),
        _ => Err(format!(
            "perf's output lacks one of {}: {}",
            EVENTS.join(", "),
            text.trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_events() {
        let text = "# started on Sun Nov  1 11:51:41 2026\n\n\
                    151140,,instructions:u,247554,100.00,0.85,insn per cycle\n\
                    177678,,cycles:u,247554,100.00,,\n";
        assert_eq!(
            parse_counts(text),
            Ok(Counts {
                instructions: 151_140,
                cycles: 177_678
            })
        );
    }

    #[test]
    fn refuses_an_uncounted_or_multiplexed_or_missing_event() {
        for (text, says) in [
            (
                "<not counted>,,instructions:u,0,0.00,,\n177678,,cycles:u,247554,100.00,,\n",
                "didn't count instructions:u",
            ),
            (
                "<not supported>,,cycles:u,0,100.00,,\n151140,,instructions:u,1,100.00,,\n",
                "didn't count cycles:u",
            ),
            (
                "151140,,instructions:u,247554,62.50,,\n177678,,cycles:u,247554,100.00,,\n",
                "multiplexed",
            ),
            ("151140,,instructions:u,247554,100.00,,\n", "lacks one of"),
            ("151140,instructions:u\n", "too few fields"),
        ] {
            let error = parse_counts(text).unwrap_err();
            assert!(error.contains(says), "{error}");
        }
    }
}
