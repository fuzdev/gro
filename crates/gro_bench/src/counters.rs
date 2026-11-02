//! Counter mode: each timed sample runs under `perf stat`, counting its
//! user-space instructions and cycles, and its kernel-space instructions
//! apart.
//!
//! The counts cover the process, its threads, and any process it forks
//! (perf's counters are inherited), so a runtime that spawns counts its
//! children's work too.
//!
//! `perf` is an external tool, found on `PATH`. Before any sample, the run
//! checks it works: `perf --version` prints a line, and a probe of `true`
//! counts the user-space events for the whole of its run, and then the
//! kernel's instructions too where it can (`perf_event_paranoid` at 1 or
//! below); where it can't, the run counts user space alone, and the record's
//! events, and its rows, leave the kernel out. A run refuses, naming why,
//! when perf is missing, when the kernel won't count (`perf_event_paranoid`,
//! no hardware counters, as in most VMs), or when an event was multiplexed
//! (counted for less than all of the run, so perf scaled an estimate).
//!
//! A sample runs `perf stat -x , -o <file> -e instructions:u,cycles:u[,instructions:k] --
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
use crate::watchdog::Watchdog;

/// The events always counted, as `perf stat -e` takes them.
///
/// User space only, so the kernel's share of a process (exec, page faults,
/// system calls) doesn't move the headline counts.
pub const EVENTS: [&str; 2] = ["instructions:u", "cycles:u"];

/// The kernel's instructions, counted apart where the kernel lets this user.
///
/// That is `perf_event_paranoid` at 1 or below. It's where a cost paid in system calls
/// (a box installed, its filters run) shows; elsewhere the run counts user
/// space alone.
pub const KERNEL_EVENT: &str = "instructions:k";

/// One process's counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// User-space instructions.
    pub instructions: u64,
    /// User-space cycles.
    pub cycles: u64,
    /// Kernel-space instructions, when [`KERNEL_EVENT`] is counted.
    pub kernel_instructions: Option<u64>,
}

/// A working `perf`, checked by [`Perf::find`].
#[derive(Debug, Clone)]
pub struct Perf {
    /// The executable, canonical.
    pub path: PathBuf,
    /// What the record names, the events counted among it.
    pub tool: CounterTool,
    /// Whether it counts [`KERNEL_EVENT`], which `tool`'s events then name.
    kernel: bool,
}

impl Perf {
    /// Finds `perf` on `PATH` and checks it counts [`EVENTS`] in full, and
    /// [`KERNEL_EVENT`] beside them when it can (else the events counted leave
    /// it out), writing its probe's output to `out`, each command killed past
    /// `watchdog`'s deadline.
    ///
    /// # Errors
    ///
    /// Fails with [`BenchError::Counters`] saying why when perf is missing,
    /// prints no version, or can't count every event for a whole run, or
    /// with [`BenchError::TimedOut`] when a command hangs.
    pub fn find(out: &Path, watchdog: &Watchdog) -> Result<Self, BenchError> {
        let dir = Path::new("/");
        let path = find_program("perf", dir).ok_or_else(|| {
            BenchError::Counters(
                "counter mode runs `perf stat`, and `perf` isn't on PATH (on Debian, the \
                 `linux-perf` package)"
                    .to_owned(),
            )
        })?;
        let mut version = Command::new(&path);
        version
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let version = watched_output(version, watchdog)?
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
        let probe = find_program("true", dir).ok_or_else(|| {
            BenchError::Counters("`true` isn't on PATH, to probe perf with".to_owned())
        })?;
        let with_kernel = Self::with_events(&path, &version, true);
        match with_kernel.probe(out, &probe, watchdog) {
            Ok(()) => return Ok(with_kernel),
            Err(error @ BenchError::TimedOut { .. }) => return Err(error),
            Err(_) => {}
        }
        let user = Self::with_events(&path, &version, false);
        user.probe(out, &probe, watchdog)?;
        Ok(user)
    }

    /// A perf counting [`EVENTS`], and [`KERNEL_EVENT`] when `kernel`.
    fn with_events(path: &Path, version: &str, kernel: bool) -> Self {
        let mut events: Vec<String> = EVENTS.iter().map(|event| (*event).to_owned()).collect();
        if kernel {
            events.push(KERNEL_EVENT.to_owned());
        }
        Self {
            path: path.to_owned(),
            tool: CounterTool {
                version: version.to_owned(),
                events,
            },
            kernel,
        }
    }

    /// Whether this perf counts [`KERNEL_EVENT`].
    const fn counts_kernel(&self) -> bool {
        self.kernel
    }

    /// Runs `probe` (`true`) under this perf and reads its counts.
    fn probe(&self, out: &Path, probe: &Path, watchdog: &Watchdog) -> Result<(), BenchError> {
        let perf = self;
        let mut command = perf.command(out, probe, &[]);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let output = watched_output(command, watchdog)?.map_err(|source| BenchError::Spawn {
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
        Ok(())
    }

    /// A command running `program` with `args` under `perf stat`, writing its
    /// counts to `out`. The caller sets the environment, directory, and
    /// stdio.
    pub fn command(&self, out: &Path, program: &Path, args: &[OsString]) -> Command {
        let mut command = Command::new(&self.path);
        command
            .args(["stat", "-x", ",", "-o"])
            .arg(out)
            .args(["-e", &self.tool.events.join(","), "--"])
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
        parse_counts(&text, self.counts_kernel()).map_err(BenchError::Counters)
    }
}

/// Runs `command` to its end, its piped output collected, killed past
/// `watchdog`'s deadline: the inner result is the spawn's or the wait's.
///
/// # Errors
///
/// Fails with [`BenchError::TimedOut`] when it ran past the deadline.
fn watched_output(
    mut command: Command,
    watchdog: &Watchdog,
) -> Result<std::io::Result<std::process::Output>, BenchError> {
    watchdog.run(
        &mut command,
        |command| {
            crate::run::display_command(
                Path::new(command.get_program()),
                &command.get_args().collect::<Vec<_>>(),
            )
        },
        std::process::Child::wait_with_output,
    )
}

/// Parses `perf stat -x ,` output: per event, a line of the value, its unit,
/// the event, the time it ran, and the percent of that time it was counted,
/// then optional metric fields; `#` lines are comments.
///
/// # Errors
///
/// Says why when an event of [`EVENTS`] (or, with `kernel`, [`KERNEL_EVENT`])
/// is missing, its value isn't a count (`<not counted>`, `<not supported>`), or
/// it was counted for less than the whole run (multiplexed).
fn parse_counts(text: &str, kernel: bool) -> Result<Counts, String> {
    let mut counts = [None; EVENTS.len() + 1];
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let [value, _unit, event, _time, percent, ..] = fields[..] else {
            return Err(format!("perf printed a line with too few fields: {line}"));
        };
        let index = if kernel && event == KERNEL_EVENT {
            EVENTS.len()
        } else if let Some(index) = EVENTS.iter().position(|known| *known == event) {
            index
        } else {
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
        [Some(instructions), Some(cycles), kernel_instructions]
            if kernel_instructions.is_some() == kernel =>
        {
            Ok(Counts {
                instructions,
                cycles,
                kernel_instructions,
            })
        }
        _ => Err(format!(
            "perf's output lacks one of {}{}: {}",
            EVENTS.join(", "),
            if kernel {
                format!(", {KERNEL_EVENT}")
            } else {
                String::new()
            },
            text.trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_event() {
        let text = "# started on Sun Nov  1 11:51:41 2026\n\n\
                    151140,,instructions:u,247554,100.00,0.85,insn per cycle\n\
                    177678,,cycles:u,247554,100.00,,\n\
                    912345,,instructions:k,247554,100.00,,\n";
        assert_eq!(
            parse_counts(text, true),
            Ok(Counts {
                instructions: 151_140,
                cycles: 177_678,
                kernel_instructions: Some(912_345),
            })
        );
        // counting user space alone, the kernel's line is no count of it
        assert_eq!(
            parse_counts(text, false),
            Ok(Counts {
                instructions: 151_140,
                cycles: 177_678,
                kernel_instructions: None,
            })
        );
    }

    #[test]
    fn refuses_an_uncounted_or_multiplexed_or_missing_event() {
        for (text, says) in [
            (
                "<not counted>,,instructions:u,0,0.00,,\n177678,,cycles:u,247554,100.00,,\n1,,instructions:k,1,100.00,,\n",
                "didn't count instructions:u",
            ),
            (
                "<not supported>,,cycles:u,0,100.00,,\n151140,,instructions:u,1,100.00,,\n1,,instructions:k,1,100.00,,\n",
                "didn't count cycles:u",
            ),
            (
                "151140,,instructions:u,247554,62.50,,\n177678,,cycles:u,247554,100.00,,\n1,,instructions:k,1,100.00,,\n",
                "multiplexed",
            ),
            (
                "151140,,instructions:u,247554,100.00,,\n177678,,cycles:u,247554,100.00,,\n",
                "lacks one of",
            ),
            ("151140,instructions:u\n", "too few fields"),
            // a kernel the user may not count, which the probe then leaves out
            (
                "151140,,instructions:u,1,100.00,,\n177678,,cycles:u,1,100.00,,\n<not supported>,,instructions:k,0,100.00,,\n",
                "didn't count instructions:k",
            ),
        ] {
            let error = parse_counts(text, true).unwrap_err();
            assert!(error.contains(says), "{error}");
        }
    }
}
