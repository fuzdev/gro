//! A deadline on each process a run spawns, so a hung runtime fails the run
//! instead of hanging it.
//!
//! One thread watches every process of the run. A caller arms a watch on a
//! process it spawned ([`Watchdog::watch`]) and disarms it once the process
//! is reaped ([`Watch::disarm`]), which says whether the deadline passed: the
//! watchdog then killed the process (`SIGKILL`), and the caller's own wait
//! still reaps it. Arming takes a lock and a push onto a list with room
//! reserved, and wakes nothing, so in a timed sample it costs the span
//! almost nothing; the watchdog wakes on its own at the earliest deadline it
//! saw, and otherwise once a timeout, which is at or before any deadline
//! armed since, each one being a timeout after its arming.
//!
//! A watch on a process kills that process alone ([`Watchdog::watch`]), not
//! what it spawned, so a timed sample's process is spawned as it always is:
//! a `perf stat` killed leaves its workload running, which the failed run
//! names. A watch on a group ([`Watchdog::watch_group`]) kills the group a
//! child leads (spawned with `process_group(0)`), and every process in it:
//! the untimed runs read a child's pipes, which its own children would
//! otherwise hold open past the kill.
//!
//! The kill names the process by its id, which is free for reuse once the
//! process is reaped, and the watch is disarmed just after: a deadline
//! passing in that window, microseconds after a process that ran for the
//! whole timeout exited, would signal whatever took the id, which Linux
//! hands out in order, so only after wrapping around every id.

use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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
    pid: u32,
    /// Whether `pid` leads a group to kill whole.
    group: bool,
    deadline: Instant,
    /// Whether the watchdog killed it.
    fired: bool,
}

/// A process being watched, until [`disarm`](Self::disarm)ed or dropped.
#[derive(Debug)]
pub struct Watch<'a> {
    watchdog: &'a Watchdog,
    token: u64,
}

impl Watchdog {
    /// Starts the watchdog's thread, each process it watches given
    /// `timeout`.
    ///
    /// # Errors
    ///
    /// Fails when the thread can't start.
    pub fn start(timeout: Duration) -> io::Result<Self> {
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

    /// Watches the process `pid`, which the caller spawned and will reap,
    /// killing it should it still run a timeout from now.
    pub fn watch(&self, pid: u32) -> Watch<'_> {
        self.arm(pid, false)
    }

    /// Watches the process group the child `pid` leads, which the caller
    /// spawned with `process_group(0)` and will reap, killing every process
    /// in it should the child still run a timeout from now.
    pub fn watch_group(&self, pid: u32) -> Watch<'_> {
        self.arm(pid, true)
    }

    fn arm(&self, pid: u32, group: bool) -> Watch<'_> {
        let deadline = Instant::now() + self.timeout;
        let mut state = self.shared.lock();
        let token = state.next;
        state.next += 1;
        state.watches.push(Armed {
            token,
            pid,
            group,
            deadline,
            fired: false,
        });
        Watch {
            watchdog: self,
            token,
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
    /// Stops watching, the process reaped, and says whether its deadline
    /// passed, so the watchdog killed it.
    pub fn disarm(self) -> bool {
        self.remove()
    }

    fn remove(&self) -> bool {
        let mut state = self.watchdog.shared.lock();
        state
            .watches
            .iter()
            .position(|armed| armed.token == self.token)
            .is_some_and(|index| state.watches.swap_remove(index).fired)
    }
}

impl Drop for Watch<'_> {
    fn drop(&mut self) {
        // a no-op after `disarm`
        self.remove();
    }
}

/// Kills each watched process past its deadline, sleeping until the
/// earliest one ahead, or a timeout when none is, until stopped.
fn watch_loop(shared: &Shared, timeout: Duration) {
    let mut state = shared.lock();
    while !state.stopped {
        let now = Instant::now();
        let mut wait = timeout;
        for armed in state.watches.iter_mut().filter(|armed| !armed.fired) {
            if armed.deadline <= now {
                // a process already gone fails with `ESRCH`, which is no matter
                let _ = kill(armed.pid, armed.group);
                armed.fired = true;
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

/// Kills the process `pid`, or the group it leads when `group`.
#[cfg(target_os = "linux")]
fn kill(pid: u32, group: bool) -> io::Result<()> {
    if group {
        gro_sys::kill_process_group(pid)
    } else {
        gro_sys::kill_process(pid)
    }
}

/// Elsewhere, by the `kill` utility.
#[cfg(not(target_os = "linux"))]
fn kill(pid: u32, group: bool) -> io::Result<()> {
    let target = if group {
        format!("-{pid}")
    } else {
        pid.to_string()
    };
    std::process::Command::new("kill")
        .args(["-KILL", "--", &target])
        .status()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_past_its_deadline_is_killed_and_one_within_it_isnt() {
        let watchdog = Watchdog::start(Duration::from_millis(200)).unwrap();
        let mut hung = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let watch = watchdog.watch(hung.id());
        let status = hung.wait().unwrap();
        assert!(watch.disarm(), "{status}");
        assert!(!status.success());
        let mut quick = std::process::Command::new("true").spawn().unwrap();
        let watch = watchdog.watch(quick.id());
        assert!(quick.wait().unwrap().success());
        assert!(!watch.disarm());
        drop(watchdog);
    }
}
