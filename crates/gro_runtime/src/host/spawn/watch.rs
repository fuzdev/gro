//! A launched child, watched from its launch until it's reaped: the task
//! that waits for it on the event loop's own runtime, and the run's table
//! of the children not reaped yet, which the run kills when it ends.
//!
//! The task awaits, in one future, the child's pidfd, which the reactor
//! reports readable once it has exited, its stdout and stderr pipes when
//! they're captured, the write of its stdin's data when it's given, and
//! its timers, so neither pipe can fill while the other is waited on. It
//! reads how the child ended without reaping it ([`Pidfd::ended`]), so the
//! child's pid, and the id of the group it leads, stay its own until the
//! task is done with them: a group is signaled only while its leader is
//! held unreaped, so a signal never reaches a group that took its id over.
//! Then it drops the child's entry in the table and its slot of the signal
//! handler's tables (`gro_sys`'s `GroupGuard` and `TermForward`), and
//! reaps it, in that order; once the handler has begun ending the process,
//! which may have read the id and not signaled it yet, it reads how the
//! child ended without reaping it, so the id stays the child's until the
//! process ends.
//!
//! It keeps the Node module's rules and bounds: past `max_bytes` of either
//! stream the child is killed (`SIGKILL`) and both pipes dropped, the
//! result `truncated`; a stream's capture grows by doubling from
//! [`MIN_CAPTURE_BYTES`], never past `max_bytes`, and a growth that can't
//! be allocated is a `RangeError` once the child has ended, killed as for
//! the cap; `timeout_ms` stops it with `SIGTERM`, then `SIGKILL` once
//! [`STOP_GRACE`] has passed; once the child has exited, a pipe something
//! it left behind still holds is waited on for [`EXIT_DRAIN`], then its
//! group killed, the pipes dropped, and the result `truncated`. A group's
//! pending `SIGKILL` outlasts the call's result, as the Node module's
//! does, the leader held unreaped until it's sent.
//!
//! The table ([`Children`]) holds each child launched and not reaped: its
//! pidfd and the group it leads, if it leads one. A run's `exit` kills
//! every child in it before the embedder's step, and so does the run's end,
//! once its event loop returns ([`SpawnRun::kill_children`]): a group is
//! killed by its id, which its leader, not reaped, still holds, and the
//! child by its pidfd. A run that ends on its own has none left but a
//! group whose grace is pending, as the Node module's exit hook finds it.
//! A task its runtime drops before it's done kills its child the same way,
//! and reaps nothing.
//!
//! [`SpawnRun::kill_children`]: super::SpawnRun::kill_children

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use gro_sys::child::{Ended, Pidfd, PidfdWatch};
use gro_sys::{GroupGuard, TermForward};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::unix::pipe;
use tokio::time::Sleep;

use crate::host::later::Settler;

/// How long the call waits, once the child has exited, for what it left
/// behind to let go of its pipes, the Node module's `EXIT_DRAIN_MS`.
pub const EXIT_DRAIN: Duration = Duration::from_millis(2000);

/// How long a stopped child has between `SIGTERM` and `SIGKILL`, the Node
/// module's `STOP_GRACE_MS`.
pub const STOP_GRACE: Duration = Duration::from_millis(2000);

/// The first capacity a stream's capture grows to, the Node module's.
pub const MIN_CAPTURE_BYTES: usize = 4096;

/// The most bytes one read of a pipe takes.
const READ_CHUNK: usize = 64 * 1024;

/// A child in the run's table: its number, its pidfd, and the group it
/// leads, if any.
type Live = (u64, Arc<Pidfd>, Option<u32>);

/// The run's children not reaped yet, each by a number of its own.
#[derive(Debug, Default)]
pub struct Children {
    /// The number the next child takes.
    next: AtomicU64,
    /// The children.
    live: Mutex<Vec<Live>>,
}

impl Children {
    /// Adds a child, returning its number.
    fn track(&self, pidfd: Arc<Pidfd>, group: Option<u32>) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, pidfd, group));
        id
    }

    /// Drops the child numbered `id`.
    fn untrack(&self, id: u64) {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(held, ..)| *held != id);
    }

    /// Kills every child not reaped yet: each group it leads by the
    /// group's id, which its leader, not reaped, holds, and the child by
    /// its pidfd. Never fails: a kill that finds nothing is what an ended
    /// child gives.
    pub fn kill_all(&self) {
        for (_, pidfd, group) in self
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            if let Some(pgid) = group {
                let _ = gro_sys::kill_process_group(*pgid);
            }
            let _ = pidfd.send_signal(libc::SIGKILL);
        }
    }
}

/// The slot of the signal handler's tables a child holds: its group's
/// guard when it leads one, else its place among the children a
/// `SIGTERM` passes on to.
#[derive(Debug)]
pub enum Guard {
    /// The group it leads, killed by the handler.
    Group(GroupGuard),
    /// The child, a `SIGTERM` passed on to it.
    Forward(TermForward),
}

impl Guard {
    /// Frees the slot, returning whether the child may be reaped now:
    /// `false` once the handler has begun ending the process, which may
    /// have read its id and not signaled it yet.
    fn release(self) -> bool {
        match self {
            Self::Group(guard) => guard.release(),
            Self::Forward(forward) => forward.release(),
        }
    }
}

/// A child launched and held until it's reaped, killed if it's dropped
/// before.
#[derive(Debug)]
pub struct Held {
    /// Its pidfd, which the table holds too.
    pub(super) pidfd: Arc<Pidfd>,
    /// The group it leads, its pid, when it leads one.
    pub(super) group: Option<u32>,
    /// Its slot of the signal handler's tables, dropped before it's reaped.
    guard: Option<Guard>,
    /// The run's table, and its number there.
    children: Arc<Children>,
    /// Its number in the table.
    id: u64,
    /// Whether it's reaped, or abandoned, so dropping it kills nothing.
    pub(super) done: bool,
}

impl Held {
    /// The child of `pidfd`, leading the group `group` when it leads one,
    /// added to `children`, its slot `guard`.
    pub fn track(
        pidfd: Arc<Pidfd>,
        group: Option<u32>,
        guard: Guard,
        children: &Arc<Children>,
    ) -> Self {
        let id = children.track(Arc::clone(&pidfd), group);
        Self {
            pidfd,
            group,
            guard: Some(guard),
            children: Arc::clone(children),
            id,
            done: false,
        }
    }

    /// Sends `signal` to the child, or to the group it leads, which its
    /// leader, not reaped, still holds; nothing once it's reaped, when the
    /// group's id may be another's. A failure is a child gone.
    pub(super) fn send(&self, signal: libc::c_int) {
        if self.done {
            return;
        }
        let _ = self.group.map_or_else(
            || self.pidfd.send_signal(signal),
            |pgid| gro_sys::signal_process_group(pgid, signal),
        );
    }

    /// Drops it from the table and from the signal handler's, then reaps
    /// it, returning how it ended; once the handler has begun ending the
    /// process, it waits for the child's end without reaping it, so the id
    /// the handler may still signal stays the child's until the process
    /// ends.
    pub(super) fn reap(&mut self) -> io::Result<Ended> {
        self.children.untrack(self.id);
        let freed = self.guard.take().is_none_or(Guard::release);
        self.done = true;
        if freed {
            self.pidfd.reap()
        } else {
            self.pidfd.wait_ended()
        }
    }

    /// Kills it, its group with it, and reaps it: for a launch that failed
    /// once the child had started.
    pub fn abandon(mut self) {
        let _ = self.kill_and_reap(true);
    }

    /// Kills it and reaps it once its pidfd couldn't say how it ended,
    /// `read` what reading it gave: its group too only when the read found
    /// it running, unreaped, since an error may be that something else
    /// reaped it, and its group's id may be another's.
    pub(super) fn kill_lost(&mut self, read: &io::Result<Option<Ended>>) -> io::Result<Ended> {
        self.kill_and_reap(matches!(read, Ok(None)))
    }

    /// Kills it, through its pidfd alone unless it's known unreaped, which
    /// keeps the group's id its own, then reaps it.
    fn kill_and_reap(&mut self, unreaped: bool) -> io::Result<Ended> {
        if unreaped {
            self.send(libc::SIGKILL);
        }
        let _ = self.pidfd.send_signal(libc::SIGKILL);
        self.reap()
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        if let Some(pgid) = self.group {
            let _ = gro_sys::kill_process_group(pgid);
        }
        let _ = self.pidfd.send_signal(libc::SIGKILL);
        self.children.untrack(self.id);
    }
}

/// What a call makes of the child's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// None: `run`, its streams the host's.
    None,
    /// Text: `out`, decoded as UTF-8 whole.
    Text,
    /// Bytes: `out_bytes`.
    Bytes,
}

/// Why the call stopped the child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// `timeout_ms` passed.
    TimedOut,
}

/// How a call's child ended, for its `finish` to make its result of.
#[derive(Debug)]
pub struct Outcome {
    /// What the call makes of the output.
    pub capture: Capture,
    /// How it ended, or why that's unknown.
    pub ended: Result<Ended, String>,
    /// Why the call stopped it, if it did.
    pub stop: Option<Stop>,
    /// Its stdout, as captured.
    pub stdout: Vec<u8>,
    /// Its stderr, as captured.
    pub stderr: Vec<u8>,
    /// Whether output was cut: past `max_bytes`, or held past the drain.
    pub truncated: bool,
    /// Whether a capture's growth couldn't be allocated.
    pub unallocated: bool,
}

/// One captured stream: its pipe while it's read, and the bytes kept.
#[derive(Debug)]
struct Stream {
    /// The pipe, until its end, an error, the cap, or the drain.
    pipe: Option<pipe::Receiver>,
    /// The bytes kept.
    bytes: Vec<u8>,
}

/// Grows `bytes`' capacity to hold at least `needed`, doubling from
/// [`MIN_CAPTURE_BYTES`] and never past `max_bytes`, by `reserve`, which
/// takes the bytes and how many more to make room for.
///
/// # Errors
///
/// Returns `reserve`'s error, the capture unchanged.
pub fn grow<E>(
    bytes: &mut Vec<u8>,
    needed: usize,
    max_bytes: usize,
    reserve: impl FnOnce(&mut Vec<u8>, usize) -> Result<(), E>,
) -> Result<(), E> {
    if needed <= bytes.capacity() {
        return Ok(());
    }
    let mut capacity = bytes.capacity().max(MIN_CAPTURE_BYTES);
    while capacity < needed {
        capacity = capacity.saturating_mul(2);
    }
    let capacity = capacity.min(max_bytes).max(needed);
    reserve(bytes, capacity - bytes.len())
}

/// Copies `chunk` into `bytes`, keeping at most `max_bytes` of the stream,
/// its growth by `reserve` ([`grow`]): whether the stream went past
/// `max_bytes`, the rest dropped.
///
/// # Errors
///
/// Returns `reserve`'s error, nothing copied.
pub fn add_chunk<E>(
    bytes: &mut Vec<u8>,
    chunk: &[u8],
    max_bytes: usize,
    reserve: impl FnOnce(&mut Vec<u8>, usize) -> Result<(), E>,
) -> Result<bool, E> {
    let room = max_bytes.saturating_sub(bytes.len());
    let kept = chunk.len().min(room);
    grow(bytes, bytes.len() + kept, max_bytes, reserve)?;
    bytes.extend_from_slice(&chunk[..kept]);
    Ok(chunk.len() > room)
}

/// The fallible reservation a capture grows by.
fn try_reserve(bytes: &mut Vec<u8>, additional: usize) -> Result<(), ()> {
    bytes.try_reserve_exact(additional).map_err(|_| ())
}

/// Data written to the child's stdin, then the pipe closed.
#[derive(Debug)]
struct Feed {
    /// The pipe.
    pipe: pipe::Sender,
    /// The data, copied when the option was read.
    data: Vec<u8>,
    /// How much of it is written.
    written: usize,
}

/// A launched child's watch, from its launch until it's reaped: the task's
/// state, polled as one future.
#[derive(Debug)]
pub struct Watch {
    /// The child.
    held: Held,
    /// Its pidfd, registered with the reactor.
    exit: PidfdWatch,
    /// What the call makes of the output.
    capture: Capture,
    /// The most bytes kept of each stream.
    max_bytes: usize,
    /// Its stdout and stderr, when they're captured.
    streams: [Option<Stream>; 2],
    /// Its stdin's data, while it's written.
    feed: Option<Feed>,
    /// How it ended, once it has, or why that's unknown.
    ended: Option<Result<Ended, String>>,
    /// Why the call stopped it.
    stop: Option<Stop>,
    /// The timeout, while it's pending.
    timeout: Option<Pin<Box<Sleep>>>,
    /// The grace before a stopped child's `SIGKILL`, while it's pending.
    grace: Option<Pin<Box<Sleep>>>,
    /// The drain bound, while it's pending.
    drain: Option<Pin<Box<Sleep>>>,
    /// Whether a stream passed `max_bytes`.
    capped: bool,
    /// Whether a pipe was still held when the drain bound passed.
    drain_cut: bool,
    /// Whether a capture's growth couldn't be allocated.
    unallocated: bool,
    /// The settler, until the result is handed over.
    settler: Option<Settler<Outcome>>,
    /// The buffer a pipe is read into.
    chunk: Box<[u8]>,
}

/// What a launch hands the watch of the child it started.
#[derive(Debug)]
pub struct Started {
    /// The child.
    pub held: Held,
    /// Its pidfd, registered with the reactor.
    pub exit: PidfdWatch,
    /// What the call makes of the output.
    pub capture: Capture,
    /// The most bytes kept of each stream.
    pub max_bytes: usize,
    /// Its stdout and stderr pipes, when they're captured.
    pub pipes: Option<(pipe::Receiver, pipe::Receiver)>,
    /// Its stdin's pipe and the data to write there, when data is given.
    pub feed: Option<(pipe::Sender, Vec<u8>)>,
    /// The timeout, when one is given.
    pub timeout: Option<Duration>,
}

impl Watch {
    /// The watch of `started`, settling its call's promise by `settler`.
    pub fn new(started: Started, settler: Settler<Outcome>) -> Self {
        let Started {
            held,
            exit,
            capture,
            max_bytes,
            pipes,
            feed,
            timeout,
        } = started;
        let stream = |pipe| Stream {
            pipe: Some(pipe),
            bytes: Vec::new(),
        };
        let streams = match pipes {
            Some((stdout, stderr)) => [Some(stream(stdout)), Some(stream(stderr))],
            None => [None, None],
        };
        Self {
            held,
            exit,
            capture,
            max_bytes,
            streams,
            feed: feed.map(|(pipe, data)| Feed {
                pipe,
                data,
                written: 0,
            }),
            ended: None,
            stop: None,
            timeout: timeout.map(|after| Box::pin(tokio::time::sleep(after))),
            grace: None,
            drain: None,
            capped: false,
            drain_cut: false,
            unallocated: false,
            settler: Some(settler),
            chunk: vec![0; READ_CHUNK].into_boxed_slice(),
        }
    }

    /// Whether a pipe is still read.
    fn reading(&self) -> bool {
        self.streams
            .iter()
            .flatten()
            .any(|stream| stream.pipe.is_some())
    }

    /// Stops the child for `stop`, `SIGTERM` now and `SIGKILL` once the
    /// grace has passed, unless it has exited.
    fn stop(&mut self, stop: Stop) {
        if self.ended.is_some() {
            return;
        }
        self.stop.get_or_insert(stop);
        self.held.send(libc::SIGTERM);
        if self.grace.is_none() {
            self.grace = Some(Box::pin(tokio::time::sleep(STOP_GRACE)));
        }
    }

    /// Kills the child, its group with it, and drops both pipes, for a
    /// stream past its cap or a capture that couldn't grow.
    fn kill_for_cap(&mut self) {
        self.capped = true;
        self.held.send(libc::SIGKILL);
        for stream in self.streams.iter_mut().flatten() {
            stream.pipe = None;
        }
    }

    /// The child has exited, ending so, or why that's unknown: no timeout
    /// now, nor a pending `SIGKILL` of a child alone, and the drain bound
    /// set while a pipe is held.
    fn on_exit(&mut self, ended: Result<Ended, String>) {
        self.ended = Some(ended);
        self.timeout = None;
        if self.held.group.is_none() {
            self.grace = None;
        }
        if self.reading() {
            self.drain = Some(Box::pin(tokio::time::sleep(EXIT_DRAIN)));
        }
    }

    /// The drain bound passed with a pipe still held: kills what's left
    /// of the group and drops the pipes.
    fn on_drain_bound(&mut self) {
        self.drain_cut = self.reading();
        if self.held.group.is_some() {
            self.held.send(libc::SIGKILL);
        }
        for stream in self.streams.iter_mut().flatten() {
            stream.pipe = None;
        }
    }

    /// Writes what it can of the stdin's data, closing the pipe once it's
    /// all written or the child stopped reading: no error, as the Node
    /// module's write ends.
    fn poll_feed(&mut self, cx: &mut Context<'_>) {
        let Some(feed) = &mut self.feed else {
            return;
        };
        loop {
            if feed.written == feed.data.len() {
                self.feed = None;
                return;
            }
            match Pin::new(&mut feed.pipe).poll_write(cx, &feed.data[feed.written..]) {
                Poll::Ready(Ok(0) | Err(_)) => {
                    self.feed = None;
                    return;
                }
                Poll::Ready(Ok(written)) => feed.written += written,
                Poll::Pending => return,
            }
        }
    }

    /// Reads what the pipes hold, keeping at most `max_bytes` of each,
    /// killing the child past it. A pipe that ends or fails is done.
    fn poll_streams(&mut self, cx: &mut Context<'_>) {
        for at in 0..self.streams.len() {
            while let Some(stream) = &mut self.streams[at] {
                let Some(pipe) = &mut stream.pipe else {
                    break;
                };
                let mut buffer = ReadBuf::new(&mut self.chunk);
                match Pin::new(pipe).poll_read(cx, &mut buffer) {
                    Poll::Ready(Ok(())) if buffer.filled().is_empty() => stream.pipe = None,
                    Poll::Ready(Ok(())) => {
                        match add_chunk(
                            &mut stream.bytes,
                            buffer.filled(),
                            self.max_bytes,
                            try_reserve,
                        ) {
                            Ok(false) => {}
                            Ok(true) => self.kill_for_cap(),
                            Err(()) => {
                                self.unallocated = true;
                                self.kill_for_cap();
                            }
                        }
                    }
                    Poll::Ready(Err(_)) => stream.pipe = None,
                    Poll::Pending => break,
                }
            }
        }
    }

    /// Reads how the child ended once the reactor reports its pidfd
    /// readable, without reaping it; `true` when it changed something.
    fn poll_exit(&mut self, cx: &mut Context<'_>) -> bool {
        if self.ended.is_some() {
            return false;
        }
        match self.exit.poll_exited(cx) {
            Poll::Pending => false,
            Poll::Ready(ready) => {
                let read = ready.and_then(|()| self.held.pidfd.ended());
                let ended = match read {
                    Ok(Some(ended)) => Ok(ended),
                    // readable only once it has exited, so neither is
                    // expected: it's killed, and the reap waited for
                    lost => self.lost(&lost),
                };
                self.on_exit(ended);
                true
            }
        }
    }

    /// How the child ended when its pidfd couldn't say, `read` what
    /// reading it gave: it's killed and reaped now, which says
    /// ([`Held::kill_lost`]).
    fn lost(&mut self, read: &io::Result<Option<Ended>>) -> Result<Ended, String> {
        let why = match read {
            Err(error) => error.to_string(),
            Ok(_) => "its pidfd was readable while it ran".to_owned(),
        };
        self.held
            .kill_lost(read)
            .map_err(|error| format!("{why}, and reaping it failed: {error}"))
    }

    /// Polls `timer`, clearing it and returning `true` once it fires.
    fn fired(timer: &mut Option<Pin<Box<Sleep>>>, cx: &mut Context<'_>) -> bool {
        let fired = timer
            .as_mut()
            .is_some_and(|sleep| sleep.as_mut().poll(cx).is_ready());
        if fired {
            *timer = None;
        }
        fired
    }

    /// Hands the call its result, once the child has ended and its pipes
    /// are done with.
    fn settle(&mut self) {
        let (Some(ended), Some(settler)) = (&self.ended, self.settler.take()) else {
            return;
        };
        self.feed = None;
        let mut taken = self
            .streams
            .iter_mut()
            .map(|stream| stream.take().map(|stream| stream.bytes).unwrap_or_default());
        let stdout = taken.next().unwrap_or_default();
        let stderr = taken.next().unwrap_or_default();
        settler.settle(Outcome {
            capture: self.capture,
            ended: ended.clone(),
            stop: self.stop,
            stdout,
            stderr,
            truncated: self.capped || self.drain_cut,
            unallocated: self.unallocated,
        });
    }
}

impl Future for Watch {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        // each step polled registers its wakeup; a step that changed
        // something can start a timer or end a stream, so the steps run
        // again until none does
        loop {
            this.poll_feed(cx);
            this.poll_streams(cx);
            let exited = this.poll_exit(cx);
            let timed_out = Self::fired(&mut this.timeout, cx);
            if timed_out {
                this.stop(Stop::TimedOut);
            }
            let graced = Self::fired(&mut this.grace, cx);
            // a group's leader, held unreaped, keeps the group's id
            if graced && (this.held.group.is_some() || this.ended.is_none()) {
                this.held.send(libc::SIGKILL);
            }
            let drained = Self::fired(&mut this.drain, cx);
            if drained {
                this.on_drain_bound();
            }
            let settled = this.ended.is_some() && !this.reading() && this.settler.is_some();
            if settled {
                this.settle();
            }
            if !(exited || timed_out || graced || drained || settled) {
                break;
            }
        }
        if this.settler.is_some() || (this.grace.is_some() && !this.held.done) {
            return Poll::Pending;
        }
        // settled, and no `SIGKILL` pending for its group: done with it
        if !this.held.done {
            let _ = this.held.reap();
        }
        Poll::Ready(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader};
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gro_sys::GroupGuard;
    use gro_sys::child::Pidfd;

    use super::{Children, Guard, Held, MIN_CAPTURE_BYTES, add_chunk, try_reserve};

    /// Whether `pid` is a process that hasn't exited.
    fn is_running(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z'))
        })
    }

    /// Waits, bounded, for `pid` to be gone, `false` when it outlives the
    /// bound.
    fn gone(pid: u32) -> bool {
        let by = Instant::now() + Duration::from_secs(10);
        while is_running(pid) {
            if Instant::now() >= by {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }

    /// A child leading a group of its own, held as a launch holds it, and
    /// the pid of a member of its group, which outlives the leader's end
    /// and holds no pipe.
    #[expect(
        clippy::zombie_processes,
        reason = "its pidfd reaps it, as a launch's does"
    )]
    fn held_leader_with_member() -> (Held, u32) {
        let mut child = Command::new("sh")
            .args([
                "-c",
                "sleep 60 < /dev/null > /dev/null 2>&1 & echo $!; exec sleep 60 > /dev/null",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let pid = child.id();
        let pidfd = Arc::new(Pidfd::open(pid).unwrap());
        let guard = GroupGuard::reserve().unwrap().arm(pid).unwrap();
        let children = Arc::new(Children::default());
        let held = Held::track(pidfd, Some(pid), Guard::Group(guard), &children);
        (held, line.trim().parse().unwrap())
    }

    /// Kills `pid`, a process a test left running on purpose.
    fn kill(pid: u32) {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
    }

    /// A child reaped is never signaled again, nor the group it led, whose
    /// id may be another's by then: the member of its group lives.
    #[test]
    fn a_reaped_child_s_group_is_never_signaled() {
        let (mut held, member) = held_leader_with_member();
        held.pidfd.send_signal(libc::SIGKILL).unwrap();
        held.reap().unwrap();
        held.send(libc::SIGKILL);
        std::thread::sleep(Duration::from_millis(100));
        let lived = is_running(member);
        kill(member);
        assert!(lived, "the member of a reaped leader's group was killed");
    }

    /// A child whose pidfd couldn't say how it ended is killed with its
    /// group when it was read running, unreaped, and alone, through its
    /// pidfd, when the read failed, which may be that something else
    /// reaped it.
    #[test]
    fn a_lost_child_s_group_is_killed_only_when_it_was_read_unreaped() {
        let (mut held, member) = held_leader_with_member();
        let failed = Err(std::io::Error::from_raw_os_error(libc::ECHILD));
        held.kill_lost(&failed).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let lived = is_running(member);
        kill(member);
        assert!(lived, "the member was killed though the read failed");

        let (mut held, member) = held_leader_with_member();
        held.kill_lost(&Ok(None)).unwrap();
        let killed = gone(member);
        kill(member);
        assert!(killed, "the member outlived its group's kill");
    }

    /// A capture grows by doubling from its first capacity, never past
    /// `max_bytes`, keeps at most `max_bytes` of the stream, and says when
    /// the stream went past it.
    #[test]
    fn a_capture_grows_by_doubling_and_keeps_at_most_its_cap() {
        let mut bytes = Vec::new();
        assert_eq!(
            add_chunk(&mut bytes, b"abc", 10_000, try_reserve),
            Ok(false)
        );
        assert_eq!(bytes.capacity(), MIN_CAPTURE_BYTES);
        let chunk = vec![b'x'; MIN_CAPTURE_BYTES];
        assert_eq!(
            add_chunk(&mut bytes, &chunk, 10_000, try_reserve),
            Ok(false)
        );
        assert_eq!(bytes.capacity(), 2 * MIN_CAPTURE_BYTES);
        assert_eq!(
            add_chunk(&mut bytes, &chunk, 10_000, try_reserve),
            Ok(false)
        );
        assert_eq!(bytes.len(), 2 * MIN_CAPTURE_BYTES + 3);
        assert_eq!(add_chunk(&mut bytes, &chunk, 10_000, try_reserve), Ok(true));
        assert_eq!(bytes.len(), 10_000);
        assert_eq!(bytes.capacity(), 10_000);
        assert_eq!(add_chunk(&mut bytes, b"y", 10_000, try_reserve), Ok(true));
        assert_eq!(bytes.len(), 10_000);
        let mut none = Vec::new();
        assert_eq!(add_chunk(&mut none, b"z", 0, try_reserve), Ok(true));
        assert!(none.is_empty());
    }

    /// A growth that can't be allocated fails, its capture unchanged, and
    /// nothing is copied; one that needs none never asks.
    #[test]
    fn a_growth_that_cant_be_allocated_fails_and_copies_nothing() {
        let refuse = |_: &mut Vec<u8>, _: usize| Err("refused");
        let mut bytes = Vec::new();
        assert_eq!(add_chunk(&mut bytes, b"abc", 100, refuse), Err("refused"));
        assert!(bytes.is_empty());
        let mut roomy = Vec::with_capacity(8);
        assert_eq!(add_chunk(&mut roomy, b"abc", 100, refuse), Ok(false));
        assert_eq!(roomy, b"abc");
    }
}
