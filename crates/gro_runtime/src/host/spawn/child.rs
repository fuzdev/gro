//! A started child: what `start` hands its `SpawnChild`, held from its
//! launch until the last handle of it is collected, and the long-lived
//! task on the event loop's own runtime that watches it.
//!
//! The state is one [`Child`], shared by the task and the handles'
//! methods, which lock it briefly, never across a call into V8, and wake
//! the task when they changed what it acts on. The task holds the
//! settler of the child's `closed`, and settles every other promise the
//! child's members hand it: each output stream's pending `next()`, each
//! `stdin.write` and `close`, and each `[Symbol.asyncDispose]`, so each
//! is a `later` settlement, counted while it's pending, and the event loop
//! waits for what a program awaits of the child (`later.rs`).
//!
//! It keeps the Node module's rules and bounds:
//!
//! - **The streams.** Each piped output is read from the start into a
//!   buffer, so nothing is lost before its reader comes, a chunk at a time
//!   as the pipe gives it; past [`OUTPUT_BUFFER_BYTES`] buffered the pipe
//!   is left unread (paused), so the child can block on a full pipe, until
//!   its reader takes the buffer to half that. A chunk goes straight to a
//!   reader waiting for one. A reader that leaves early (`return`) drops
//!   what's buffered and every later chunk, the pipe still read so the
//!   child can't block on it.
//! - **The end.** `closed` settles once the child has exited and each
//!   output pipe has ended, with how it ended, as `run`'s result, or, when
//!   a pipe is still held once [`EXIT_DRAIN`] has passed after the exit,
//!   then, its group killed: a stream nobody reads, or one not behind, ends
//!   there, what it buffered still readable, and one paused for a reader
//!   behind it keeps its data and is read on once the reader catches up,
//!   for [`EXIT_DRAIN`] past each time it's resumed. `closed` never
//!   rejects.
//! - **The stops.** `timeout_ms` stops the child, `SIGTERM` then
//!   `SIGKILL` after [`STOP_GRACE`], `closed` then of kind `timed_out`; a
//!   disposal stops it so, recording no reason, and settles once `closed`
//!   has; `kill` sends its signal once, to the child or the group it leads,
//!   while it hasn't exited.
//! - **Its stdin.** Each write is copied when it's made and written in
//!   order, settling `true` once the pipe took all of it, or `false` once
//!   the pipe failed (the child closed it) or the child exited, as every
//!   later write does: the exit lets the pipe go, as the Node module's
//!   stdin is destroyed then, though a process the child left behind may
//!   hold its other end; `close` closes the pipe once every write before it
//!   is done, and a write after it is `false`.
//! - **Its reaping.** As `run`'s: read how it ended without reaping it, so
//!   its pid and its group's id stay its own while a signal may reach
//!   them, then reaped once `closed` has settled and no grace is pending
//!   (`watch.rs`). The run's table of children holds it until then, so the
//!   kill at the run's end, the guards, and the drain bound cover it.
//!
//! [`EXIT_DRAIN`]: super::watch::EXIT_DRAIN
//! [`STOP_GRACE`]: super::watch::STOP_GRACE

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use gro_sys::child::{Ended, PidfdWatch};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::unix::pipe;
use tokio::time::Sleep;

use super::watch::{EXIT_DRAIN, Held, STOP_GRACE, Stop};
use crate::host::later::Settler;

/// The bytes a started child's output buffers ahead of its reader before
/// its pipe is left unread, the Node module's `OUTPUT_BUFFER_BYTES`.
pub const OUTPUT_BUFFER_BYTES: usize = 1024 * 1024;

/// The most bytes one read of a pipe takes.
const READ_CHUNK: usize = 64 * 1024;

/// A chunk a reader's `next()` settles with, or `None` once the stream is
/// done.
pub type Chunk = Option<Vec<u8>>;

/// How a started child ended, for `closed`'s record: how it ended, or why
/// that's unknown, and why the module stopped it, if it did.
#[derive(Debug)]
pub struct Closed {
    /// How it ended.
    pub ended: Result<Ended, String>,
    /// Why the module stopped it.
    pub stop: Option<Stop>,
}

/// A started child, shared by its task and its handles' methods.
#[derive(Debug)]
pub struct Child {
    /// Its state.
    state: Mutex<ChildState>,
}

/// What a started child launched with: the child, its pidfd's watch, its
/// piped streams, and its timeout.
#[derive(Debug)]
pub struct StartedChild {
    /// The child.
    pub held: Held,
    /// Its pidfd, registered with the reactor.
    pub exit: PidfdWatch,
    /// Its stdin, when it's piped.
    pub stdin: Option<pipe::Sender>,
    /// Its stdout and stderr, each when it's piped.
    pub outputs: [Option<pipe::Receiver>; 2],
    /// The timeout, when one is given.
    pub timeout: Option<Duration>,
}

/// A started child's state.
#[derive(Debug)]
struct ChildState {
    /// The child.
    held: Held,
    /// Its pidfd, registered with the reactor.
    exit: PidfdWatch,
    /// How it ended, once it has, or why that's unknown.
    ended: Option<Result<Ended, String>>,
    /// Why the module stopped it.
    stop: Option<Stop>,
    /// Whether a disposal asked for it to be stopped.
    stop_asked: bool,
    /// Whether `closed` has settled.
    closed: bool,
    /// The disposals waiting for `closed`.
    disposals: Vec<Settler<()>>,
    /// Its stdout and stderr, each when it's piped.
    outputs: [Option<Output>; 2],
    /// Its stdin, when it's piped.
    stdin: Option<Input>,
    /// The timeout, while it's pending.
    timeout: Option<Pin<Box<Sleep>>>,
    /// The grace before a stopped child's `SIGKILL`, while it's pending.
    grace: Option<Pin<Box<Sleep>>>,
    /// The drain bound, while it's pending.
    drain: Option<Pin<Box<Sleep>>>,
    /// Whether the drain bound has passed.
    drained: bool,
    /// The task's waker, which a method that changed what the task acts on
    /// wakes.
    waker: Option<Waker>,
}

/// One piped output stream.
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is a state of the Node module's feed, which it keeps as one"
)]
struct Output {
    /// The pipe, until it ends, fails, or is let go.
    pipe: Option<pipe::Receiver>,
    /// The chunks read and not yet taken, in order.
    chunks: VecDeque<Vec<u8>>,
    /// How many bytes they hold.
    buffered: usize,
    /// Whether it's done: no chunk will be added.
    ended: bool,
    /// Whether its reader left early, so every chunk is dropped.
    discarding: bool,
    /// Whether its pipe is left unread for its reader to catch up.
    paused: bool,
    /// Whether the drain bound found it paused for a reader behind it.
    bounded: bool,
    /// Whether a reader took it.
    claimed: bool,
    /// The readers waiting for a chunk, in order.
    waiters: VecDeque<Settler<Chunk>>,
    /// The bound past a bounded stream's resumption, while it's pending.
    resumed: Option<Pin<Box<Sleep>>>,
}

/// A started child's piped stdin.
#[derive(Debug)]
struct Input {
    /// The pipe, until it's closed or fails.
    pipe: Option<pipe::Sender>,
    /// The writes and closes asked for, in order, the first in progress.
    queue: VecDeque<StdinStep>,
    /// How much of the first write is written.
    written: usize,
    /// Whether a close was asked for, so a later write goes nowhere.
    closing: bool,
}

/// A step a started child's stdin was asked for.
#[derive(Debug)]
enum StdinStep {
    /// Write these bytes, settling whether the pipe took them.
    Write(Vec<u8>, Settler<bool>),
    /// Close the pipe, settling once it's closed.
    Close(Settler<()>),
}

impl Output {
    /// A stream reading `pipe`.
    const fn new(pipe: pipe::Receiver) -> Self {
        Self {
            pipe: Some(pipe),
            chunks: VecDeque::new(),
            buffered: 0,
            ended: false,
            discarding: false,
            paused: false,
            bounded: false,
            claimed: false,
            waiters: VecDeque::new(),
            resumed: None,
        }
    }

    /// Whether its pipe holds `closed` back: open, and not left to a
    /// reader behind at the drain bound.
    const fn holds_close(&self) -> bool {
        self.pipe.is_some() && !self.bounded
    }

    /// A chunk read from the pipe: dropped while discarding, handed to the
    /// first reader waiting, or buffered, the pipe paused past the buffer.
    fn add(&mut self, chunk: &[u8]) {
        if self.discarding {
            return;
        }
        if let Some(waiter) = self.waiters.pop_front() {
            waiter.settle(Some(chunk.to_vec()));
            return;
        }
        self.chunks.push_back(chunk.to_vec());
        self.buffered += chunk.len();
        if self.buffered > OUTPUT_BUFFER_BYTES {
            self.paused = true;
        }
    }

    /// The pipe ended, failed, or was let go: no chunk will be added, and
    /// every reader waiting is done.
    fn end(&mut self) {
        self.pipe = None;
        self.ended = true;
        self.resumed = None;
        for waiter in self.waiters.drain(..) {
            waiter.settle(None);
        }
    }

    /// Resumes a paused pipe once its reader took the buffer to half of
    /// it, returning whether it did: one the drain bound found behind gets
    /// the bound again.
    fn resume_if_room(&mut self) -> bool {
        if !self.paused || self.buffered > OUTPUT_BUFFER_BYTES / 2 {
            return false;
        }
        self.paused = false;
        if self.bounded && !self.ended {
            self.resumed = Some(Box::pin(tokio::time::sleep(EXIT_DRAIN)));
        }
        true
    }
}

impl Input {
    /// Writes what it can of its queue in order, settling each step done.
    fn poll(&mut self, cx: &mut Context<'_>) {
        while let Some(step) = self.queue.front_mut() {
            match step {
                StdinStep::Close(_) => {
                    self.pipe = None;
                    if let Some(StdinStep::Close(settler)) = self.queue.pop_front() {
                        settler.settle(());
                    }
                }
                StdinStep::Write(data, _) => {
                    let done = match &mut self.pipe {
                        None => Some(false),
                        Some(_) if self.written == data.len() => Some(true),
                        Some(pipe) => match Pin::new(pipe).poll_write(cx, &data[self.written..]) {
                            Poll::Ready(Ok(0) | Err(_)) => {
                                // the child closed it or ended: every later
                                // write goes nowhere
                                self.pipe = None;
                                Some(false)
                            }
                            Poll::Ready(Ok(written)) => {
                                self.written += written;
                                None
                            }
                            Poll::Pending => return,
                        },
                    };
                    if let Some(taken) = done {
                        self.written = 0;
                        if let Some(StdinStep::Write(_, settler)) = self.queue.pop_front() {
                            settler.settle(taken);
                        }
                    }
                }
            }
        }
    }

    /// Lets the pipe go, settling every step left: a write `false`, a
    /// close done.
    fn finish(&mut self) {
        self.pipe = None;
        for step in self.queue.drain(..) {
            match step {
                StdinStep::Write(_, settler) => settler.settle(false),
                StdinStep::Close(settler) => settler.settle(()),
            }
        }
    }
}

impl Child {
    /// The state of a child `started` launched, its streams read from now.
    pub fn new(started: StartedChild) -> Self {
        let StartedChild {
            held,
            exit,
            stdin,
            outputs: [stdout, stderr],
            timeout,
        } = started;
        Self {
            state: Mutex::new(ChildState {
                held,
                exit,
                ended: None,
                stop: None,
                stop_asked: false,
                closed: false,
                disposals: Vec::new(),
                outputs: [stdout.map(Output::new), stderr.map(Output::new)],
                stdin: stdin.map(|pipe| Input {
                    pipe: Some(pipe),
                    queue: VecDeque::new(),
                    written: 0,
                    closing: false,
                }),
                timeout: timeout.map(|after| Box::pin(tokio::time::sleep(after))),
                grace: None,
                drain: None,
                drained: false,
                waker: None,
            }),
        }
    }

    /// Its state, locked.
    fn lock(&self) -> MutexGuard<'_, ChildState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends `signal` to the child, or to the group it leads, unless it has
    /// exited: whether it was sent.
    pub fn kill(&self, signal: libc::c_int) -> bool {
        let state = self.lock();
        if state.ended.is_some() {
            return false;
        }
        state.held.send(signal);
        true
    }

    /// Kills it at once, its group with it, for a start whose handles
    /// couldn't be made: its task reaps it as it would any child.
    pub fn abandon(&self) {
        let state = self.lock();
        if state.ended.is_none() {
            state.held.send(libc::SIGKILL);
        }
    }

    /// A disposal: stops the child, unless it has exited, and settles by
    /// `settler` once `closed` has.
    pub fn dispose(&self, settler: Settler<()>) {
        let mut state = self.lock();
        if state.closed {
            drop(state);
            settler.settle(());
            return;
        }
        state.disposals.push(settler);
        state.stop_asked = true;
        state.wake();
    }

    /// Claims the output stream at `stream` for its one reader, `false`
    /// when it was claimed already.
    pub fn claim(&self, stream: usize) -> bool {
        let mut state = self.lock();
        let Some(output) = state.outputs.get_mut(stream).and_then(Option::as_mut) else {
            return false;
        };
        !std::mem::replace(&mut output.claimed, true)
    }

    /// A reader's `next()` on the stream at `stream`, settled by `settler`:
    /// the first chunk buffered, the pipe resumed once there's room, or
    /// none once it's done or its reader left, else once a chunk comes.
    pub fn next(&self, stream: usize, settler: Settler<Chunk>) {
        let mut state = self.lock();
        let Some(output) = state.outputs.get_mut(stream).and_then(Option::as_mut) else {
            drop(state);
            settler.settle(None);
            return;
        };
        if let Some(chunk) = output.chunks.pop_front() {
            output.buffered -= chunk.len();
            let resumed = output.resume_if_room();
            settler.settle(Some(chunk));
            if resumed {
                state.wake();
            }
            return;
        }
        if output.ended || output.discarding {
            settler.settle(None);
            return;
        }
        output.waiters.push_back(settler);
    }

    /// The reader of the stream at `stream` left early: what's buffered is
    /// dropped, and every later chunk, the pipe read on so the child can't
    /// block on it, and every reader waiting is done.
    pub fn discard(&self, stream: usize) {
        let mut state = self.lock();
        let Some(output) = state.outputs.get_mut(stream).and_then(Option::as_mut) else {
            return;
        };
        output.discarding = true;
        output.chunks.clear();
        output.buffered = 0;
        output.paused = false;
        for waiter in output.waiters.drain(..) {
            waiter.settle(None);
        }
        state.wake();
    }

    /// `stdin.write(data)`, settled by `settler`: queued behind what came
    /// before it, or `false` at once when the stdin is closed, closing, or
    /// failed, or the task is done with it.
    pub fn write(&self, data: Vec<u8>, settler: Settler<bool>) {
        let mut state = self.lock();
        match &mut state.stdin {
            Some(input) if !input.closing && input.pipe.is_some() => {
                input.queue.push_back(StdinStep::Write(data, settler));
                state.wake();
            }
            _ => {
                drop(state);
                settler.settle(false);
            }
        }
    }

    /// `stdin.close()`, settled by `settler` once every write before it is
    /// done and the pipe closed, or at once when there's nothing left to
    /// close.
    pub fn close(&self, settler: Settler<()>) {
        let mut state = self.lock();
        match &mut state.stdin {
            Some(input) if input.pipe.is_some() || !input.queue.is_empty() => {
                input.closing = true;
                input.queue.push_back(StdinStep::Close(settler));
                state.wake();
            }
            _ => {
                drop(state);
                settler.settle(());
            }
        }
    }
}

impl ChildState {
    /// Wakes the task, which acts on what changed.
    fn wake(&self) {
        if let Some(waker) = &self.waker {
            waker.wake_by_ref();
        }
    }

    /// Whether an output pipe holds `closed` back.
    fn holds_close(&self) -> bool {
        self.outputs.iter().flatten().any(Output::holds_close)
    }

    /// Stops the child for `stop`, `SIGTERM` now and `SIGKILL` once the
    /// grace has passed, unless it has exited or `closed` settled; `None`
    /// records no reason, as a disposal's.
    fn stop(&mut self, stop: Option<Stop>) {
        if self.ended.is_some() || self.closed {
            return;
        }
        if self.stop.is_none() {
            self.stop = stop;
        }
        self.held.send(libc::SIGTERM);
        if self.grace.is_none() {
            self.grace = Some(Box::pin(tokio::time::sleep(STOP_GRACE)));
        }
    }

    /// Reads what each piped output's pipe holds, unless it's paused;
    /// `true` when a stream ended.
    fn poll_outputs(&mut self, cx: &mut Context<'_>, chunk: &mut [u8]) -> bool {
        let mut ended = false;
        for output in self.outputs.iter_mut().flatten() {
            while !output.paused {
                let Some(pipe) = &mut output.pipe else {
                    break;
                };
                let mut buffer = ReadBuf::new(chunk);
                match Pin::new(pipe).poll_read(cx, &mut buffer) {
                    Poll::Ready(Ok(())) if buffer.filled().is_empty() => {
                        output.end();
                        ended = true;
                    }
                    Poll::Ready(Ok(())) => output.add(buffer.filled()),
                    Poll::Ready(Err(_)) => {
                        output.end();
                        ended = true;
                    }
                    Poll::Pending => break,
                }
            }
        }
        ended
    }

    /// Reads how the child ended once the reactor reports its pidfd
    /// readable, without reaping it; `true` when it has.
    fn poll_exit(&mut self, cx: &mut Context<'_>) -> bool {
        if self.ended.is_some() {
            return false;
        }
        let Poll::Ready(ready) = self.exit.poll_exited(cx) else {
            return false;
        };
        let read = ready.and_then(|()| self.held.pidfd.ended());
        let ended = match read {
            Ok(Some(ended)) => Ok(ended),
            // readable only once it has exited, so neither is expected:
            // it's killed, and the reap says how it ended
            lost => {
                let why = match &lost {
                    Err(error) => error.to_string(),
                    Ok(_) => "its pidfd was readable while it ran".to_owned(),
                };
                self.held
                    .kill_lost(&lost)
                    .map_err(|error| format!("{why}, and reaping it failed: {error}"))
            }
        };
        self.ended = Some(ended);
        // a write pending, and every later one, goes nowhere, though a
        // process the child left behind holds the pipe's other end
        if let Some(input) = &mut self.stdin {
            input.finish();
        }
        self.timeout = None;
        if self.held.group.is_none() {
            self.grace = None;
        }
        if self.holds_close() {
            self.drain = Some(Box::pin(tokio::time::sleep(EXIT_DRAIN)));
        }
        true
    }

    /// The drain bound passed with a pipe still held: what's left of the
    /// group killed, each stream nobody reads or not behind ended, and one
    /// paused for a reader behind it left to that reader.
    fn on_drain_bound(&mut self) {
        self.drained = true;
        if self.held.group.is_some() {
            self.held.send(libc::SIGKILL);
        }
        for output in self.outputs.iter_mut().flatten() {
            if output.ended {
                continue;
            }
            if output.paused && output.claimed {
                output.bounded = true;
            } else {
                output.end();
            }
        }
    }
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

/// A started child's task: its state, and the settler of its `closed`.
#[derive(Debug)]
pub struct ChildTask {
    /// The child.
    child: Arc<Child>,
    /// `closed`'s settler, until it settles.
    closed: Option<Settler<Closed>>,
    /// The buffer a pipe is read into.
    chunk: Box<[u8]>,
}

impl ChildTask {
    /// The task of `child`, settling its `closed` by `closed`.
    pub fn new(child: Arc<Child>, closed: Settler<Closed>) -> Self {
        Self {
            child,
            closed: Some(closed),
            chunk: vec![0; READ_CHUNK].into_boxed_slice(),
        }
    }
}

impl Future for ChildTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let mut state = this.child.lock();
        if !state
            .waker
            .as_ref()
            .is_some_and(|waker| waker.will_wake(cx.waker()))
        {
            state.waker = Some(cx.waker().clone());
        }
        // each step polled registers its wakeup; a step that changed
        // something can end a stream or start a timer, so the steps run
        // again until none does
        loop {
            if std::mem::take(&mut state.stop_asked) {
                state.stop(None);
            }
            if let Some(input) = &mut state.stdin {
                input.poll(cx);
            }
            let ended = state.poll_outputs(cx, &mut this.chunk);
            let exited = state.poll_exit(cx);
            let timed_out = fired(&mut state.timeout, cx);
            if timed_out {
                state.stop(Some(Stop::TimedOut));
            }
            let graced = fired(&mut state.grace, cx);
            // a group's leader, held unreaped, keeps the group's id
            if graced && (state.held.group.is_some() || state.ended.is_none()) {
                state.held.send(libc::SIGKILL);
            }
            let drained = fired(&mut state.drain, cx);
            if drained {
                state.on_drain_bound();
            }
            let mut bounded = false;
            for output in state.outputs.iter_mut().flatten() {
                if fired(&mut output.resumed, cx) && !output.paused {
                    output.end();
                    bounded = true;
                }
            }
            let closing =
                !state.closed && state.ended.is_some() && (state.drained || !state.holds_close());
            if closing {
                state.closed = true;
                state.drain = None;
                let ended = state.ended.clone().unwrap_or_else(|| Err(String::new()));
                if let Some(closed) = this.closed.take() {
                    closed.settle(Closed {
                        ended,
                        stop: state.stop,
                    });
                }
                for disposal in state.disposals.drain(..) {
                    disposal.settle(());
                }
            }
            if !(ended || exited || timed_out || graced || drained || bounded || closing) {
                break;
            }
        }
        // settled, and no `SIGKILL` pending for its group: reaped
        if state.closed && state.grace.is_none() && !state.held.done {
            let _ = state.held.reap();
        }
        // its stdin was let go at the exit, which the reap follows
        let reading = state
            .outputs
            .iter()
            .flatten()
            .any(|output| output.pipe.is_some());
        if !state.held.done || reading {
            return Poll::Pending;
        }
        state.waker = None;
        Poll::Ready(())
    }
}
