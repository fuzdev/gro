//! A deadline on each process a run spawns, so a hung runtime fails the run
//! instead of hanging it.
//!
//! One thread watches every process of the run. A caller runs a process
//! through [`Watchdog::run`], which spawns it as the leader of a process
//! group of its own (`process_group(0)`), arms a watch on the group, waits
//! as the caller says, reaping the process, and disarms the watch, failing
//! with [`BenchError::TimedOut`] when the deadline passed: the watchdog then
//! killed the group (`SIGKILL`), every process in it, and the caller's own
//! wait still reaped the leader. Arming takes a lock and a push onto a list
//! with room reserved (and, on Linux, a compare-and-swap for its guard,
//! below), and wakes nothing, so in a timed sample it costs the
//! span almost nothing; the watchdog wakes on its own at the earliest
//! deadline it saw, and otherwise once a timeout, which is at or before any
//! deadline armed since, each one being a timeout after its arming.
//!
//! Every process gets its own group, timed samples too, so a kill reaches
//! what it spawned: the untimed runs read a child's pipes, which its own
//! children would otherwise hold open past the kill, and a counter run's
//! sample is `perf stat`, whose workload would otherwise outlive it. The
//! group is set in the child before its `exec`, the same on every row, so
//! rows stay comparable in time and in instructions.
//!
//! A child in a group of its own doesn't get the terminal's signals, so on
//! Linux the watchdog also guards each group it watches
//! (`gro_sys::guard_group`), and [`Watchdog::start`] installs the handler
//! that kills every guarded group when a Ctrl-C, a hangup, or a `SIGTERM`
//! ends the run (`gro_sys::kill_guarded_groups_on_signal`), the run then
//! ending by that signal. A signal in the moment between a spawn and its
//! guard misses that group, and a `SIGKILL` of the run orphans them all;
//! elsewhere nothing kills them.
//!
//! The kill names the group by its leader's id, which is free for reuse once
//! the leader is reaped, and the watch is disarmed just after: a deadline
//! passing in that window, microseconds after a process that ran for the
//! whole timeout exited, would signal whatever took the id, which Linux
//! hands out in order, so only after wrapping around every id. Such a kill
//! finds no process (`ESRCH`) unless the id was reused, and only a kill that
//! succeeds marks the watch fired, so a process that exited in time isn't
//! reported timed out.

use std::io;
use std::process::{Child, Command};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::BenchError;

/// The room the list of watches starts with, past the most processes a run
/// waits on at once (one probe per runtime).
const WATCHES: usize = 16;

/// The thread that kills a process past its deadline, stopped and joined on
/// drop.
#[derive(Debug)]
pub struct Watchdog {
    shared: Arc<Shared>,
    timeout: Duration,
    thread: Option<JoinHandle<()>>,
}

#[derive(Debug)]
struct Shared {
    state: Mutex<State>,
    /// Notified only to stop.
    wake: Condvar,
}

#[derive(Debug)]
struct State {
    next: u64,
    watches: Vec<Armed>,
    stopped: bool,
}

#[derive(Debug)]
struct Armed {
    token: u64,
    /// The leader of the group, its id the group's.
    pid: u32,
    deadline: Instant,
    fate: Fate,
}

/// What became of a watched group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fate {
    /// Its deadline is ahead.
    Pending,
    /// Past its deadline, the watchdog killed it.
    Killed,
    /// Past its deadline, the kill found no process, so it had exited.
    Gone,
}

/// A group being watched, until [`disarm`](Self::disarm)ed or dropped.
#[derive(Debug)]
struct Watch<'a> {
    watchdog: &'a Watchdog,
    token: u64,
    /// Kills the group should a signal end the run, until dropped.
    #[cfg(target_os = "linux")]
    _guard: Option<gro_sys::GroupGuard>,
}

impl Watchdog {
    /// Starts the watchdog's thread, each process it watches given
    /// `timeout`, and, on Linux, installs the handler that kills every
    /// watched group when a signal ends the process.
    ///
    /// # Errors
    ///
    /// Fails when the handler can't be installed or the thread can't start.
    pub fn start(timeout: Duration) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        gro_sys::kill_guarded_groups_on_signal()?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                next: 0,
                watches: Vec::with_capacity(WATCHES),
                stopped: false,
            }),
            wake: Condvar::new(),
        });
        let thread = std::thread::Builder::new()
            .name("gro_bench_watchdog".to_owned())
            .spawn({
                let shared = Arc::clone(&shared);
                move || watch_loop(&shared, timeout)
            })?;
        Ok(Self {
            shared,
            timeout,
            thread: Some(thread),
        })
    }

    /// Each process's deadline, after its arming.
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Spawns `command` as the leader of a process group of its own, watches
    /// the group, and runs `wait` on the child, which must reap it: the inner
    /// result is the spawn's or `wait`'s. Should the child still run a
    /// timeout after its spawn, the watchdog kills its group, every process
    /// in it, and `wait`'s reaping returns.
    ///
    /// # Errors
    ///
    /// Fails with [`BenchError::TimedOut`], naming the command as `display`
    /// writes it, when the watchdog killed the group.
    pub fn run<T>(
        &self,
        command: &mut Command,
        display: impl FnOnce(&Command) -> String,
        wait: impl FnOnce(Child) -> io::Result<T>,
    ) -> Result<io::Result<T>, BenchError> {
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(command, 0);
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => return Ok(Err(error)),
        };
        // inside a timed span, arming is a lock, a push, and a guard's
        // compare-and-swap, waking no thread
        let watch = self.watch_group(child.id());
        let waited = wait(child);
        if watch.disarm() {
            return Err(BenchError::TimedOut {
                command: display(command),
                timeout_ms: self.timeout.as_millis(),
            });
        }
        Ok(waited)
    }

    /// Watches the process group the child `pid` leads, which the caller
    /// spawned with `process_group(0)` and will reap, killing every process
    /// in it should the child still run a timeout from now.
    fn watch_group(&self, pid: u32) -> Watch<'_> {
        let deadline = Instant::now() + self.timeout;
        let mut state = self.shared.lock();
        let token = state.next;
        state.next += 1;
        state.watches.push(Armed {
            token,
            pid,
            deadline,
            fate: Fate::Pending,
        });
        drop(state);
        Watch {
            watchdog: self,
            token,
            // a run waits on fewer groups at once than there are slots; one
            // past them goes unguarded
            #[cfg(target_os = "linux")]
            _guard: gro_sys::guard_group(pid),
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.shared.lock().stopped = true;
        self.shared.wake.notify_all();
        if let Some(thread) = self.thread.take() {
            // the loop never panics but by a poisoned lock it recovers from
            let _ = thread.join();
        }
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Watch<'_> {
    /// Stops watching, the leader reaped, and says whether its deadline
    /// passed with the group still there, so the watchdog killed it.
    fn disarm(self) -> bool {
        self.remove()
    }

    fn remove(&self) -> bool {
        let mut state = self.watchdog.shared.lock();
        state
            .watches
            .iter()
            .position(|armed| armed.token == self.token)
            .is_some_and(|index| state.watches.swap_remove(index).fate == Fate::Killed)
    }
}

impl Drop for Watch<'_> {
    fn drop(&mut self) {
        // a no-op after `disarm`
        self.remove();
    }
}

/// Kills each watched group past its deadline, sleeping until the earliest
/// one ahead, or a timeout when none is, until stopped.
fn watch_loop(shared: &Shared, timeout: Duration) {
    let mut state = shared.lock();
    while !state.stopped {
        let now = Instant::now();
        let mut wait = timeout;
        for armed in state
            .watches
            .iter_mut()
            .filter(|armed| armed.fate == Fate::Pending)
        {
            if armed.deadline <= now {
                // a group already gone, reaped in time but not yet disarmed,
                // fails with `ESRCH`, and isn't reported as killed
                armed.fate = if kill_group(armed.pid).is_ok() {
                    Fate::Killed
                } else {
                    Fate::Gone
                };
            } else {
                wait = wait.min(armed.deadline - now);
            }
        }
        state = shared
            .wake
            .wait_timeout(state, wait)
            .unwrap_or_else(PoisonError::into_inner)
            .0;
    }
}

/// Kills the process group `pgid`.
#[cfg(target_os = "linux")]
fn kill_group(pgid: u32) -> io::Result<()> {
    gro_sys::kill_process_group(pgid)
}

/// Elsewhere, by the `kill` utility, its failure an error.
#[cfg(not(target_os = "linux"))]
fn kill_group(pgid: u32) -> io::Result<()> {
    let status = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("`kill` failed ({status})")))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::CommandExt as _;

    use super::*;

    fn sleep(seconds: &str) -> Command {
        let mut command = Command::new("sleep");
        command.arg(seconds);
        command
    }

    #[test]
    fn a_process_past_its_deadline_is_killed_and_one_within_it_isnt() {
        let watchdog = Watchdog::start(Duration::from_millis(200)).unwrap();
        let display = |_: &Command| "sleep 60".to_owned();
        let error = watchdog
            .run(&mut sleep("60"), display, |mut child| child.wait())
            .unwrap_err();
        assert!(
            matches!(
                error,
                BenchError::TimedOut {
                    timeout_ms: 200,
                    ..
                }
            ),
            "{error}"
        );
        let status = watchdog
            .run(&mut Command::new("true"), display, |mut child| child.wait())
            .unwrap()
            .unwrap();
        assert!(status.success());
        drop(watchdog);
    }

    /// A kill of a group gone by its deadline, reaped but not yet disarmed,
    /// finds no process, and the watch doesn't report it killed.
    #[test]
    fn a_group_gone_before_its_deadline_isnt_reported_killed() {
        let watchdog = Watchdog::start(Duration::from_millis(50)).unwrap();
        let mut child = Command::new("true").process_group(0).spawn().unwrap();
        child.wait().unwrap();
        let watch = watchdog.watch_group(child.id());
        let watched = Instant::now() + Duration::from_secs(10);
        while watchdog.shared.lock().watches[0].fate == Fate::Pending {
            assert!(Instant::now() < watched, "the watchdog never got to it");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!watch.disarm());
    }

    /// A spawn that fails is the inner error, not a timeout.
    #[test]
    fn a_failed_spawn_is_the_inner_error() {
        let watchdog = Watchdog::start(Duration::from_secs(60)).unwrap();
        let result = watchdog
            .run(
                &mut Command::new("/nonexistent/gro_bench"),
                |_| String::new(),
                |mut child| child.wait(),
            )
            .unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    }
}
