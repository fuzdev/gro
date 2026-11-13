//! Eval's limits: the heap cap, with its near-heap-limit callback and the
//! out-of-memory backstop, the process's memory cap, and the timeout's
//! watchdog.
//!
//! Both limits stop the run's JS the same way, by asking V8 to terminate
//! execution, and both say why in one [`Stopped`], which the envelope's
//! kind is read from, never the message V8 or `deno_core` reports. The
//! first to stop the run is the one reported.
//!
//! - **The heap.** The isolate's heap is capped at [`HEAP_LIMIT`]. Near the
//!   cap, V8 calls the callback [`watch_heap`] installs, which records why
//!   the heap can grow no more ([`exhaustion`]), terminates execution, and
//!   raises the cap by [`HEAP_HEADROOM`] once, the room for the run to
//!   unwind; a second call raises nothing. A terminate reaches only JS V8
//!   interrupts: one long builtin (`new Array(5e7).fill({})`) runs on, and
//!   under the JIT, V8 can run on past a terminate the callback requests, so
//!   either reaches the cap again with nothing left to give, and V8 calls
//!   the out-of-memory handler: [`oom_backstop`] writes the envelope
//!   prepared before the run ([`set_backstop`]) and exits the process with
//!   code 1.
//! - **The process's memory.** `fgro eval` caps the process's data segment at
//!   [`MEMORY_LIMIT`] before it starts a thread, which bounds what the heap's
//!   cap doesn't: an array buffer's bytes, the copies a module's parse and the
//!   envelope make, every thread's stack. Past it, an allocation fails: an
//!   array buffer's makes V8 collect all it can and call the near-heap-limit
//!   callback, which finds the failure the isolate's allocator recorded
//!   ([`ArrayBufferRecord`]) and records [`Stop::Memory`], stopping the run the
//!   same way; the heap's own pages make V8 call the callback, or its
//!   out-of-memory handler, as a heap at its cap does: under a caller's limit
//!   that leaves the heap's cap out of reach ([`HeapReach`]) either reports
//!   `memory`, naming the limit, and under one in reach the handler reports
//!   `memory` when no callback ran before it ([`backstop_stop`]), while a
//!   callback that finds nothing recorded reports `heap` ([`exhaustion`]
//!   names those cases); V8's own allocations elsewhere, on the run's thread,
//!   reach the same out-of-memory handler, which writes a prepared `memory`
//!   envelope when V8 doesn't report its heap out of memory; V8's on one of
//!   its own threads, which calls no isolate's handler, and a Rust
//!   allocation, whose failure handler stable Rust can't set, abort the
//!   process with no envelope.
//! - **The timeout.** A [`Watchdog`] thread holds the isolate's thread-safe
//!   handle and terminates execution at the deadline, recording
//!   [`Stop::Timeout`], unless the eval ends first.
//!
//! The serializer's walk is Rust, which no terminate interrupts: it checks
//! [`Stopped`] once per batch.

use std::fs::File;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use deno_core::{JsRuntime, v8};
use gro_v8::ArrayBufferRecord;

use super::EvalOptions;
use crate::confine::Jit;

/// The heap's cap, in bytes: 512 MiB.
///
/// Provisional until it's measured against real configs. A config builds a
/// small object graph (its evaluation allocates a few MiB of heap), so the
/// cap is two orders of magnitude above that, and still well below a
/// machine's memory, so a runaway eval on the heap fails as `heap` long
/// before the kernel's out-of-memory killer would take it, and its process.
///
/// The cap is V8's heap only. Outside it, bounded only by the process's
/// [`MEMORY_LIMIT`], is the backing memory of an `ArrayBuffer`, a
/// `SharedArrayBuffer`, or a typed array. Also outside it, and bounded by byte
/// caps rather than this one: a module's source, as read and stripped, and its
/// parse ([`MAX_MODULE_SIZE`](crate::resolver::MAX_MODULE_SIZE) per module),
/// and the envelope's own bytes, which the serializer writes outside the heap
/// ([`MAX_ENVELOPE_SIZE`]; a large string is held a few times over while it's
/// written). Not bounded but by this cap on each copy: an uncaught error's
/// report (its message and stack, and each cause's and aggregated error's, up
/// to 256 errors) and the specifiers the loader records, copied out of the
/// heap, so a run that throws a long message, many times over, can take many
/// times this cap, up to the process's, before it ends.
///
/// The cap bounds the heap's growth, not each allocation in it: V8 makes one
/// large young object (a large array's elements, as large as V8 makes one)
/// whatever the cap, so a heap holding one can pass the cap by its size
/// without the run being stopped, and only [`MEMORY_LIMIT`] bounds it.
pub const HEAP_LIMIT: usize = 512 << 20;

/// The cap on the process's data segment, `RLIMIT_DATA`, in bytes: 2 GiB.
///
/// `fgro eval` sets it, soft and hard alike, before it starts any thread,
/// so it bounds every allocation the process makes after, outside the heap
/// as well as in it: an array buffer's bytes, the source and parse of the
/// largest module the loader takes, the serializer's copies of the largest
/// envelope, each thread's stack ([`DEFAULT_THREAD_STACK`] for V8's
/// workers), and with `--jit` the code range V8 reserves for compiled
/// code, which the limit counts whole from the engine's start. Not the whole
/// address space (`RLIMIT_AS`), which V8's reservations made without write
/// access (the heap's cage) and glibc's arenas would fill long before any
/// of it is used.
///
/// A caller's limit too low for the engine to start (with `--jit`, one under
/// the code range) aborts the process before the out-of-memory handler is
/// installed, with no envelope.
///
/// Sized by measurement, not by guess: well above the least cap at which the
/// largest runs it was measured against (a module at the source cap, a run
/// stopped at the heap's cap, a value just under the envelope's cap, each
/// jitless and with `--jit`, which takes the most) still give the same
/// envelope they give uncapped. It isn't out of a legal run's reach: a run
/// that holds a large young object past the heap's cap ([`HEAP_LIMIT`]) beside
/// a heap near it can come close to it, and this cap is what bounds that run.
/// A run that meets it fails as `memory` or `heap`, by the allocation that
/// fails and how V8 retries it, or, where that allocation is one nothing
/// reports, with no envelope; a caller that wants a lower bound sets a
/// lower limit on the process, soft or hard, which stands.
pub const MEMORY_LIMIT: u64 = 2 << 30;

/// The most stack a thread started without a size of its own gets, in bytes:
/// 8 MiB, what glibc gives one under the common 8 MiB `RLIMIT_STACK`.
///
/// `fgro eval` lowers the process's default to it before it starts any
/// thread, and never raises it: a smaller default, under a smaller
/// `RLIMIT_STACK` or glibc's own under an unlimited one, stands. V8's
/// platform workers start with no size, and glibc would size each by the
/// `RLIMIT_STACK` the process inherited, every byte of it counted against
/// [`MEMORY_LIMIT`]: a parent's stack limit of a few hundred MiB would leave
/// too little for the engine to start. The threads gro starts set sizes of
/// their own, which stand.
pub const DEFAULT_THREAD_STACK: usize = 8 << 20;

/// The envelope's cap, in bytes: 64 MiB.
///
/// The serializer stops as soon as the value's JSON would pass it, and an
/// envelope that would pass it whole is counted, never written: a value's
/// is replaced by a small `serialize` failure with no `access` or graph,
/// and a failure's keeps its kind, dropping its `access` and graph, then,
/// if it's still past, cutting its long text fields to 64 KiB each. A
/// config's value is a few KiB.
pub const MAX_ENVELOPE_SIZE: usize = 64 << 20;
// gro_cli's tests state it as a literal, which keeps V8 out of their binaries
const _: () = assert!(MAX_ENVELOPE_SIZE == 64 << 20);

/// The room the cap is raised by, once, when the heap first nears it: 32
/// MiB, enough for the run to unwind and its envelope to be written.
const HEAP_HEADROOM: usize = 32 << 20;

/// The timeout when the caller sets none: 10 seconds.
///
/// A safety net, not the caller's limit: a caller passes its own, and a
/// config evaluates in well under a second, so this is far above any real
/// eval and still below the spawn timeouts a caller commonly kills its
/// child at, so the caller gets the envelope rather than a killed process.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The watchdog thread's stack: 128 KiB, set rather than left to
/// `RUST_MIN_STACK`, which a huge value would make the spawn fail, and the
/// eval with it. The thread waits on a channel and asks V8 to terminate, so
/// it needs little; std raises a size below the platform's least to it.
const WATCHDOG_STACK_SIZE: usize = 128 << 10;

/// Why the host stopped the run's JS, by the byte [`Stopped`] holds it as
/// (0 is none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Stop {
    /// The heap reached its cap.
    Heap = 1,
    /// The timeout passed.
    Timeout = 2,
    /// The process's memory ran out: an array buffer's allocation failed,
    /// or, at the out-of-memory backstop, an allocation V8 reports outside
    /// its heap, or the heap's own with no near-heap-limit callback before
    /// it ([`backstop_stop`]).
    Memory = 3,
    /// The heap stopped growing under a process limit that leaves its cap
    /// out of reach ([`HeapReach::Unreachable`]): the process's memory ran
    /// out, by that configuration, reported as `memory` with the limit named.
    HeapOutOfReach = 4,
}

impl Stop {
    /// The stop held as `value`, or `None` for none.
    const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Heap),
            2 => Some(Self::Timeout),
            3 => Some(Self::Memory),
            4 => Some(Self::HeapOutOfReach),
            _ => None,
        }
    }
}

/// Why the host stopped the run's JS, if it did: the first stop recorded,
/// shared by the heap's callback, the watchdog's thread, the serializer's
/// walk, and the out-of-memory backstop, which also reads what the
/// near-heap-limit callback found, first or not.
#[derive(Debug, Clone, Default)]
pub(super) struct Stopped(Arc<StopState>);

/// What [`Stopped`] shares.
#[derive(Debug, Default)]
struct StopState {
    /// The first stop recorded, as its byte, or [`Stopped::NONE`].
    first: AtomicU8,
    /// The first [`exhaustion`] the near-heap-limit callback recorded, as its
    /// byte, whatever stopped the run first, or [`Stopped::NONE`].
    exhausted: AtomicU8,
}

impl Stopped {
    /// No stop recorded.
    const NONE: u8 = 0;

    /// Records `stop`, unless a stop is recorded already.
    fn set(&self, stop: Stop) {
        // the first stop stands, and the first exhaustion
        if matches!(stop, Stop::Heap | Stop::Memory | Stop::HeapOutOfReach) {
            let _ = self.0.exhausted.compare_exchange(
                Self::NONE,
                stop as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }
        let _ = self.0.first.compare_exchange(
            Self::NONE,
            stop as u8,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }

    /// What the near-heap-limit callback first found stopped the heap, first
    /// or after another stop, if it ran.
    fn exhausted(&self) -> Option<Stop> {
        Stop::from_u8(self.0.exhausted.load(Ordering::SeqCst))
    }

    /// The stop recorded, if any.
    pub(super) fn get(&self) -> Option<Stop> {
        Stop::from_u8(self.0.first.load(Ordering::SeqCst))
    }
}

/// Whether the process's data limit leaves the heap room to reach its cap,
/// decided once, before the run, from the limit and the JIT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeapReach {
    /// The limit holds the heap at its cap, raised by [`HEAP_HEADROOM`], and
    /// the engine's own memory beside it ([`ENGINE_MEMORY`], and with the
    /// JIT [`JIT_MEMORY`]): a heap that stops growing stopped at its cap.
    Reachable,
    /// The limit is lower, so the process's memory runs out before the heap
    /// reaches its cap: a heap that stops growing stopped there.
    Unreachable,
}

impl HeapReach {
    /// The reach of the heap of an eval run as `options` say, by their
    /// data limit and JIT.
    pub(super) const fn of(options: &EvalOptions) -> Self {
        if options.data_limit >= heap_room(options.jit) {
            Self::Reachable
        } else {
            Self::Unreachable
        }
    }
}

/// The least data limit that holds the heap at its cap, in bytes: the cap
/// and its headroom, and the engine's own memory beside them, with the JIT's
/// with `--jit`. 800 MiB jitless, 1312 MiB with the JIT.
pub(super) const fn heap_room(jit: Jit) -> u64 {
    let room = HEAP_LIMIT as u64 + HEAP_HEADROOM as u64 + ENGINE_MEMORY;
    match jit {
        Jit::Off => room,
        Jit::On => room + JIT_MEMORY,
    }
}

/// The memory the engine takes beside the heap at its cap, jitless, in
/// bytes: 256 MiB.
///
/// Measured as the process's peak data segment (`VmData`, what
/// `RLIMIT_DATA` counts) under the default cap: a jitless start takes 68 MiB,
/// heaps grown to the cap a small object or string at a time peak at 549 to
/// 556 MiB, and one builtin call that fills past the cap
/// (`new Array(5e7).fill({})`) at 739 MiB, 195 MiB past the cap and its
/// headroom; rounded up, so a limit that admits the heap at its cap admits
/// the largest of these too. A limit between what a run takes and this
/// margin reports a heap at its cap as `memory`, by configuration, never by
/// what the heap reads.
const ENGINE_MEMORY: u64 = 256 << 20;

/// The memory the JIT adds, in bytes: 512 MiB, the code range V8 reserves
/// writable and the limit counts whole from the engine's start. Measured
/// the same way: a start takes 580 MiB with `--jit` against 68 MiB
/// jitless, and each heap fixture 512 to 530 MiB more than jitless, at
/// most 1251 MiB, under [`heap_room`]'s 1312.
const JIT_MEMORY: u64 = 512 << 20;

// the cap `fgro eval` sets leaves the heap room to reach its own, with the JIT
const _: () = assert!(MEMORY_LIMIT >= heap_room(Jit::On));

/// What stopped a heap that can grow no more, as the near-heap-limit
/// callback finds it, decided by what the run was configured with and one
/// recorded fact, never by reading the heap, which V8's collections change
/// under the read: [`Stop::Memory`] when the isolate's array-buffer
/// allocator's latest allocation failed (`array_buffer_failed`), since V8
/// calls the callback for that failure as for a heap at its cap;
/// [`Stop::HeapOutOfReach`] when the process's limit leaves the heap's cap
/// out of reach (`reach`); [`Stop::Heap`] otherwise.
///
/// So under a cap in reach, memory outside the heap that ran the process out
/// reads as [`Stop::Heap`] whenever the allocation that fails is one that
/// runs the callback with nothing recorded: the pages of a resizable
/// `ArrayBuffer` or a `WebAssembly.Memory`, made or grown, which V8 commits
/// outside that allocator and retries as it retries a heap allocation, and a
/// heap page V8 retries the same way after memory outside the heap, array
/// buffers made earlier included, took the rest of the limit, and one large
/// young object larger than what the limit has left.
const fn exhaustion(array_buffer_failed: bool, reach: HeapReach) -> Stop {
    match (array_buffer_failed, reach) {
        (true, _) => Stop::Memory,
        (false, HeapReach::Reachable) => Stop::Heap,
        (false, HeapReach::Unreachable) => Stop::HeapOutOfReach,
    }
}

/// Installs the heap's near-limit callback and the out-of-memory backstop
/// on `runtime`, whose heap was built capped at [`HEAP_LIMIT`], its array
/// buffers allocated by the allocator `array_buffers` records, under a
/// process limit that leaves the heap's cap as `reach` says.
pub(super) fn watch_heap(
    runtime: &mut JsRuntime,
    stopped: Stopped,
    array_buffers: ArrayBufferRecord,
    reach: HeapReach,
) {
    let handle = runtime.v8_isolate().thread_safe_handle();
    let mut raised = false;
    runtime.add_near_heap_limit_callback(move |current, _initial| {
        stopped.set(exhaustion(array_buffers.latest_failed(), reach));
        handle.terminate_execution();
        if raised {
            // once only: under the JIT, V8 can run on past the terminate, and
            // raising again would let the heap grow without bound
            current
        } else {
            raised = true;
            current + HEAP_HEADROOM
        }
    });
    runtime.v8_isolate().set_oom_error_handler(oom_backstop);
}

/// What the out-of-memory backstop writes, and where.
#[derive(Debug)]
struct Backstop {
    /// The stream the envelope goes to, opened before the run.
    out: Arc<File>,
    /// The envelopes it chooses between.
    envelopes: BackstopEnvelopes,
    /// The run's stops, which say what the near-heap-limit callback found.
    stopped: Stopped,
    /// Whether the process's limit leaves the heap's cap in reach, for an
    /// out-of-memory the callback didn't see.
    reach: HeapReach,
}

/// The envelopes the out-of-memory backstop chooses between, each one line
/// with its newline, prepared before the run.
#[derive(Debug)]
pub(super) struct BackstopEnvelopes {
    /// Written for [`Stop::Heap`] ([`backstop_stop`]).
    pub(super) heap: String,
    /// Written for [`Stop::HeapOutOfReach`]: the heap stopped growing under
    /// a process limit with no room for its cap.
    pub(super) heap_out_of_reach: String,
    /// Written otherwise: the process reached its [`MEMORY_LIMIT`], or a
    /// lower one its caller set.
    pub(super) memory: String,
}

/// The backstop's state, a process static because V8's out-of-memory
/// handler is a bare function with no user data: an exception to the rule
/// that the runtime keeps no process state beyond the engine's. Set once
/// per process, before the first eval that passes a stream.
static BACKSTOP: OnceLock<Backstop> = OnceLock::new();

/// Whether the backstop has written, so it writes once, whichever thread
/// V8 calls it on.
static BACKSTOP_WRITTEN: AtomicBool = AtomicBool::new(false);

/// Sets what the out-of-memory backstop writes: one of `envelopes` to
/// `out`, a handle to the envelope's stream opened before the run, never
/// `std::io::stdout()`'s lock, which another thread may hold when V8 runs
/// out of memory, chosen by what V8 reports and by `stopped`, the run's
/// stops, and `reach`, its heap's. Returns false when the process set one
/// already.
pub(super) fn set_backstop(
    out: Arc<File>,
    envelopes: BackstopEnvelopes,
    stopped: Stopped,
    reach: HeapReach,
) -> bool {
    BACKSTOP
        .set(Backstop {
            out,
            envelopes,
            stopped,
            reach,
        })
        .is_ok()
}

/// What the out-of-memory backstop reports. An allocation V8 reports outside
/// its heap (a zone's or a mapping's, `is_heap_oom` false) is
/// [`Stop::Memory`]. The heap's own is what the near-heap-limit callback
/// found when it ran (`exhausted`, [`exhaustion`]): a heap at its cap that
/// one operation V8 can't stop took past it reaches the callback first. With
/// no callback before it, the heap failed to commit a page, or V8 gave up on
/// a size it can't hold, before it neared its cap, so memory outside the
/// heap took the process's: [`Stop::Memory`],
/// or [`Stop::HeapOutOfReach`] under a limit with no room for the heap's cap
/// (`reach`).
const fn backstop_stop(is_heap_oom: bool, exhausted: Option<Stop>, reach: HeapReach) -> Stop {
    match (is_heap_oom, exhausted, reach) {
        (true, Some(stop), _) => stop,
        (true, None, HeapReach::Unreachable) => Stop::HeapOutOfReach,
        (false, _, _) | (true, None, HeapReach::Reachable) => Stop::Memory,
    }
}

/// V8's out-of-memory handler for an eval's isolate: writes the prepared
/// envelope of the stop [`backstop_stop`] gives, once, when there are any,
/// and exits the process with code 1 at once, running none of its teardown
/// (`gro_sys::exit_now`). V8 may call it on any thread, so it takes no lock,
/// and it allocates nothing, since memory may be what ran out.
extern "C" fn oom_backstop(_location: *const std::ffi::c_char, details: &v8::OomDetails) {
    if !BACKSTOP_WRITTEN.swap(true, Ordering::SeqCst)
        && let Some(backstop) = BACKSTOP.get()
    {
        let envelope = match backstop_stop(
            details.is_heap_oom,
            backstop.stopped.exhausted(),
            backstop.reach,
        ) {
            Stop::Heap => &backstop.envelopes.heap,
            Stop::HeapOutOfReach => &backstop.envelopes.heap_out_of_reach,
            Stop::Memory | Stop::Timeout => &backstop.envelopes.memory,
        };
        let mut out: &File = &backstop.out;
        // nothing is left to report a failed write to
        let _ = out.write_all(envelope.as_bytes());
        let _ = out.flush();
    }
    // no teardown: V8's workers are live and V8 is in its out-of-memory path
    #[cfg(target_os = "linux")]
    gro_sys::exit_now(1);
    #[cfg(not(target_os = "linux"))]
    std::process::exit(1);
}

/// The timeout's watchdog: a thread that terminates the isolate's
/// execution at the deadline, recording [`Stop::Timeout`], unless it's
/// dropped first. Dropping it stops the thread and waits for it.
#[derive(Debug)]
pub(super) struct Watchdog {
    /// Dropped to wake the thread before the deadline.
    cancel: Option<mpsc::Sender<()>>,
    /// The thread.
    thread: Option<JoinHandle<()>>,
}

impl Watchdog {
    /// Starts the watchdog over `runtime`'s isolate, `timeout` from now.
    ///
    /// # Errors
    ///
    /// Returns the error when the thread can't be spawned.
    pub(super) fn start(
        runtime: &mut JsRuntime,
        timeout: Duration,
        stopped: Stopped,
    ) -> std::io::Result<Self> {
        let handle = runtime.v8_isolate().thread_safe_handle();
        let (cancel, cancelled) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("fgro-watchdog".to_owned())
            .stack_size(WATCHDOG_STACK_SIZE)
            .spawn(move || {
                if cancelled.recv_timeout(timeout) == Err(RecvTimeoutError::Timeout) {
                    stopped.set(Stop::Timeout);
                    handle.terminate_execution();
                }
            })?;
        Ok(Self {
            cancel: Some(cancel),
            thread: Some(thread),
        })
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        // disconnects the channel, which wakes the thread
        drop(self.cancel.take());
        if let Some(thread) = self.thread.take() {
            // the thread can't panic
            let _ = thread.join();
        }
    }
}

/// `bytes` as a person reads a cap: whole MiB when it is some, else bytes.
pub(super) fn format_size(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB && bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_first_stop_stands() {
        let stopped = Stopped::default();
        let shared = stopped.clone();
        assert_eq!(shared.get(), None);
        assert_eq!(shared.exhausted(), None);
        stopped.set(Stop::Timeout);
        stopped.set(Stop::Heap);
        stopped.set(Stop::Memory);
        stopped.set(Stop::HeapOutOfReach);
        assert_eq!(shared.get(), Some(Stop::Timeout));
        // the heap's callback ran, though the timeout stopped the run, and
        // its first finding stands
        assert_eq!(shared.exhausted(), Some(Stop::Heap));
        let memory_first = Stopped::default();
        memory_first.set(Stop::Memory);
        memory_first.set(Stop::Heap);
        assert_eq!(memory_first.get(), Some(Stop::Memory));
        assert_eq!(memory_first.exhausted(), Some(Stop::Memory));
        let out_of_reach_first = Stopped::default();
        out_of_reach_first.set(Stop::HeapOutOfReach);
        out_of_reach_first.set(Stop::Heap);
        assert_eq!(out_of_reach_first.get(), Some(Stop::HeapOutOfReach));
        assert_eq!(out_of_reach_first.exhausted(), Some(Stop::HeapOutOfReach));
    }

    /// A heap that stops growing is `heap` unless an array buffer's
    /// allocation failed, or the process's limit leaves the heap's cap out of
    /// reach, which is `memory` with the limit named. Nothing here reads the
    /// heap: a heap stopped at its cap reads whatever V8's collections leave
    /// it, all but empty at times, and the kind doesn't move with it.
    #[test]
    fn a_stopped_heap_is_heap_unless_memory_ran_out_first() {
        assert_eq!(exhaustion(false, HeapReach::Reachable), Stop::Heap);
        assert_eq!(exhaustion(true, HeapReach::Reachable), Stop::Memory);
        assert_eq!(
            exhaustion(false, HeapReach::Unreachable),
            Stop::HeapOutOfReach
        );
        // the array buffer's failure is the fact, whatever the limit
        assert_eq!(exhaustion(true, HeapReach::Unreachable), Stop::Memory);
    }

    /// The heap's cap is in reach under `fgro eval`'s own cap, jitless or
    /// with the JIT, and out of it under a limit well below the cap and its
    /// engine; a limit between the two rooms reaches jitless only. Each
    /// limit sits well clear of the edge it's on the side of.
    #[test]
    fn the_heap_s_reach_comes_from_the_data_limit_and_the_jit() {
        let reach = |data_limit: u64, jit: Jit| {
            let mut options = super::super::options(Path::new("main.js"), Path::new("/"));
            options.data_limit = data_limit;
            options.jit = jit;
            HeapReach::of(&options)
        };
        for jit in [Jit::Off, Jit::On] {
            assert_eq!(reach(MEMORY_LIMIT, jit), HeapReach::Reachable);
            assert_eq!(reach(u64::MAX, jit), HeapReach::Reachable);
            assert_eq!(reach(256 << 20, jit), HeapReach::Unreachable);
            assert_eq!(reach(HEAP_LIMIT as u64, jit), HeapReach::Unreachable);
        }
        assert_eq!(reach(1 << 30, Jit::Off), HeapReach::Reachable);
        assert_eq!(reach(1 << 30, Jit::On), HeapReach::Unreachable);
        assert_eq!(heap_room(Jit::Off), 800 << 20);
        assert_eq!(heap_room(Jit::On), 1312 << 20);
    }

    /// The backstop writes `heap` only when V8 reports its heap out of
    /// memory and the near-heap-limit callback found the heap at its cap
    /// before it. A heap out of memory with no callback before it never
    /// reached its cap: memory outside the heap (array buffers made earlier,
    /// say) took the process's, so it's `memory`, with the limit named when
    /// it leaves the heap's cap out of reach.
    #[test]
    fn the_backstop_blames_the_heap_only_for_a_heap_stopped_at_its_cap() {
        use HeapReach::{Reachable, Unreachable};
        let found = [Stop::Heap, Stop::Memory, Stop::HeapOutOfReach];
        for reach in [Reachable, Unreachable] {
            for stop in found {
                assert_eq!(backstop_stop(true, Some(stop), reach), stop);
            }
            for exhausted in [None, Some(Stop::Heap), Some(Stop::HeapOutOfReach)] {
                assert_eq!(backstop_stop(false, exhausted, reach), Stop::Memory);
            }
        }
        assert_eq!(backstop_stop(true, None, Reachable), Stop::Memory);
        assert_eq!(backstop_stop(true, None, Unreachable), Stop::HeapOutOfReach);
    }

    /// The allocator an eval's isolate is built with records a failed
    /// allocation, one too large for any process (4 PiB, under V8's largest
    /// array buffer, so V8 asks the allocator), and a success after it,
    /// zeroed or not, clears the record. `gro_v8`'s own test, here so V8 links into one
    /// fewer test binary.
    #[test]
    fn the_array_buffer_record_holds_the_latest_allocation_s_failure() {
        std::thread::Builder::new()
            .stack_size(crate::STACK_SIZE)
            .spawn(|| {
                crate::engine::init_engine(
                    crate::confine::Confinement::Unconfined,
                    &crate::engine::EngineOptions::default(),
                )
                .unwrap();
                let tokio = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let (create_params, record) =
                    gro_v8::record_array_buffers(v8::Isolate::create_params());
                let mut runtime = {
                    let _entered = tokio.enter();
                    JsRuntime::new(deno_core::RuntimeOptions {
                        create_params: Some(create_params),
                        ..Default::default()
                    })
                };
                let mut run = |source: &'static str| {
                    runtime.execute_script("record", source).unwrap();
                    record.latest_failed()
                };
                assert!(!run("new ArrayBuffer(8);"));
                assert!(run(
                    "try { new ArrayBuffer(2 ** 52); throw new Error('made'); } \
                     catch (error) { if (!(error instanceof RangeError)) throw error; }"
                ));
                assert!(!run("new Uint8Array(1 << 20);"));
                // an uninitialized allocation (a transfer's) clears it too
                assert!(!run("const kept = new ArrayBuffer(8); \
                     try { new ArrayBuffer(2 ** 52); } catch {} \
                     kept.transfer(16);"));
                // a free leaves it: the buffers a collection frees after a
                // failure don't clear what the callback reads
                assert!(run(
                    "globalThis.dropped = Array.from({length: 64}, () => new ArrayBuffer(1 << 16)); \
                     try { new ArrayBuffer(2 ** 52); } catch {} \
                     globalThis.dropped = null;"
                ));
                runtime.v8_isolate().low_memory_notification();
                assert!(record.latest_failed());
                drop(runtime);
                // the allocator gone, the record still reads its last
                assert!(record.latest_failed());
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
