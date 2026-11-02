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
//!   cap, V8 calls the callback [`watch_heap`] installs, which reads the
//!   heap's use at that moment, records [`Stop::Heap`] when the heap is full
//!   ([`HEAP_FULL`]), terminates execution, and raises the cap by
//!   [`HEAP_HEADROOM`] once, the room for the run to unwind; a second call
//!   raises nothing. A terminate reaches only JS V8 interrupts: one long
//!   builtin (`new Array(5e7).fill({})`) runs on, and under the JIT, V8
//!   doesn't act on a terminate the callback requests, so either reaches
//!   the cap again with nothing left to give, and V8 calls the
//!   out-of-memory handler: [`oom_backstop`] writes the envelope prepared
//!   before the run ([`set_backstop`]) and exits the process with code 1.
//! - **The process's memory.** `fgro eval` caps the process's data segment
//!   at [`MEMORY_LIMIT`] before it starts a thread, which bounds what the
//!   heap's cap doesn't: an array buffer's bytes, the copies a module's
//!   parse and the envelope make, every thread's stack. Past it, an
//!   allocation fails: an array buffer's makes V8 collect all it can and
//!   call the near-heap-limit callback, which finds the heap well below its
//!   cap and records [`Stop::Memory`], stopping the run the same way; V8's
//!   own, on the run's thread, reaches the same out-of-memory handler, which
//!   writes a prepared `memory` envelope instead when the heap isn't at
//!   fault (V8 didn't report its heap out of memory, or the callback never
//!   found the heap full); V8's
//!   on one of its own threads, which calls no isolate's handler, and a Rust
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
use gro_v8::HeapReader;

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
/// `SharedArrayBuffer`, or a typed array. Also outside it, and bounded by
/// byte caps rather than this one: a module's source, as read
/// and stripped, and its parse
/// ([`MAX_MODULE_SIZE`](crate::resolver::MAX_MODULE_SIZE) per module), and
/// the envelope's own bytes, which the serializer writes outside the heap
/// ([`MAX_ENVELOPE_SIZE`]; a large string is held a few times over while
/// it's written). Not bounded but by this cap on each copy: an uncaught
/// error's report (its message and stack, and each cause's and aggregated
/// error's, up to 256 errors) and the specifiers the loader records, copied
/// out of the heap, so a run that throws a long message, many times over,
/// can take many times this cap, up to the process's, before it ends.
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
/// Sized by measurement, not by guess: about twice the least cap at which
/// the largest legal runs (a module at the source cap, a run stopped at the
/// heap's cap, a value just under the envelope's cap, each jitless and with
/// `--jit`, which takes the most) still give the same envelope they give
/// uncapped, so a legal run never meets it. A run that does is one holding
/// memory outside the heap no config needs, and fails as `memory`, or,
/// where the failing allocation is one nothing reports, with no envelope; a
/// caller that wants a lower bound sets a lower limit on the process, soft
/// or hard, which stands.
pub const MEMORY_LIMIT: u64 = 2 << 30;

/// The most stack a thread started without a size of its own gets, in
/// bytes: 8 MiB, what glibc gives one under the common 8 MiB
/// `RLIMIT_STACK`.
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
pub enum Stop {
    /// The heap reached its cap.
    Heap = 1,
    /// An allocation outside the heap failed, the process's memory taken:
    /// V8 collects all it can before it gives up on one, and calls the
    /// near-heap-limit callback as it does, which finds the heap well below
    /// its cap ([`HEAP_FULL`]). Turned into [`Stop::Heap`] when the run's end
    /// finds the heap full after all ([`Stopped::check_heap`]).
    Memory = 3,
    /// The timeout passed.
    Timeout = 2,
}

impl Stop {
    /// The stop held as `value`, or `None` for none.
    const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Heap),
            2 => Some(Self::Timeout),
            3 => Some(Self::Memory),
            _ => None,
        }
    }
}

/// Why the host stopped the run's JS, if it did: the first stop recorded,
/// shared by the heap's callback, the watchdog's thread, the serializer's
/// walk, and the out-of-memory backstop, which also reads whether the
/// callback found the heap full at all, first or not.
#[derive(Debug, Clone, Default)]
pub struct Stopped(Arc<StopState>);

/// What [`Stopped`] shares.
#[derive(Debug, Default)]
struct StopState {
    /// The first stop recorded, as its byte, or [`Stopped::NONE`].
    first: AtomicU8,
    /// Whether the near-heap-limit callback found the heap full, whatever
    /// stopped the run first.
    heap: AtomicBool,
}

impl Stopped {
    /// No stop recorded.
    const NONE: u8 = 0;

    /// Records `stop`, unless a stop is recorded already.
    fn set(&self, stop: Stop) {
        if stop == Stop::Heap {
            self.0.heap.store(true, Ordering::SeqCst);
        }
        // the first stop stands
        let _ = self.0.first.compare_exchange(
            Self::NONE,
            stop as u8,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }

    /// Turns a recorded [`Stop::Memory`] into [`Stop::Heap`] when the heap,
    /// read once the run has stopped, uses `used_heap` bytes, as many as a
    /// heap that reached its cap holds ([`heap_full`]): the near-heap-limit
    /// callback ran for one large object the heap couldn't fit, before it
    /// was made, which the raised cap then let in.
    pub fn check_heap(&self, used_heap: usize) {
        if heap_full(used_heap)
            && self
                .0
                .first
                .compare_exchange(
                    Stop::Memory as u8,
                    Stop::Heap as u8,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
        {
            self.0.heap.store(true, Ordering::SeqCst);
        }
    }

    /// Whether the near-heap-limit callback found the heap full, first or
    /// after another stop.
    fn heap_neared(&self) -> bool {
        self.0.heap.load(Ordering::SeqCst)
    }

    /// The stop recorded, if any.
    pub fn get(&self) -> Option<Stop> {
        Stop::from_u8(self.0.first.load(Ordering::SeqCst))
    }
}

/// Whether a heap using `used_heap` bytes is one that reached its cap: past
/// [`HEAP_FULL`]. Read in the near-heap-limit callback, at the moment V8
/// calls it: V8 calls it when the old generation passes the cap, after a
/// full collection failed to bring it back, so the heap then holds near the
/// cap in live objects; V8 also calls it after collecting all it can for an
/// allocation outside the heap that failed, when the heap holds only what
/// the run kept. Read after the run, the heap can be all but empty, the
/// stopped run's objects freed, so the callback's reading decides, and the
/// run's end only adds the one case the callback can't see
/// ([`Stopped::check_heap`]).
const fn heap_full(used_heap: usize) -> bool {
    used_heap >= HEAP_FULL
}

/// The least a heap that reached its cap uses, in bytes: three quarters of
/// the cap. Read in the callback, a heap grown to its cap a small object
/// at a time (arrays, objects, maps, strings, closures, jitless or with
/// `--jit`) holds nearly all of it, and one stopped in one builtin call
/// more than all of it; a heap that failed an allocation outside it holds
/// what the run kept, a few MiB for a config. The margin is room for a heap
/// V8 lays out otherwise. A run that fails an allocation outside the heap
/// while holding more than three quarters of the cap in it is reported as
/// `heap`.
const HEAP_FULL: usize = HEAP_LIMIT / 4 * 3;

/// Installs the heap's near-limit callback and the out-of-memory backstop
/// on `runtime`, whose heap was built capped at [`HEAP_LIMIT`].
pub fn watch_heap(runtime: &mut JsRuntime, stopped: Stopped) {
    let handle = runtime.v8_isolate().thread_safe_handle();
    let heap = HeapReader::new(runtime.v8_isolate());
    let mut raised = false;
    runtime.add_near_heap_limit_callback(move |current, _initial| {
        stopped.set(if heap.used_heap_size().is_some_and(heap_full) {
            Stop::Heap
        } else {
            Stop::Memory
        });
        handle.terminate_execution();
        if raised {
            // once only: under the JIT, V8 doesn't act on the terminate, and
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
    /// The `heap` envelope, one line.
    heap: String,
    /// The `memory` envelope, one line.
    memory: String,
    /// The run's stops, which say whether the heap neared its cap.
    stopped: Stopped,
}

/// The envelopes the out-of-memory backstop chooses between, each one line
/// with its newline, prepared before the run.
#[derive(Debug)]
pub struct BackstopEnvelopes {
    /// Written when the heap is at fault: V8 reports its heap out of
    /// memory, and the near-heap-limit callback found the heap full.
    pub heap: String,
    /// Written otherwise: the process reached its [`MEMORY_LIMIT`].
    pub memory: String,
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
/// stops. Returns false when the process set one already.
pub fn set_backstop(out: Arc<File>, envelopes: BackstopEnvelopes, stopped: Stopped) -> bool {
    let BackstopEnvelopes { heap, memory } = envelopes;
    BACKSTOP
        .set(Backstop {
            out,
            heap,
            memory,
            stopped,
        })
        .is_ok()
}

/// Whether the heap is at fault for an out-of-memory: V8 reports its heap
/// out of memory (rather than an allocation outside it, a zone's or a
/// mapping's), and the near-heap-limit callback found the heap full ([`HEAP_FULL`]).
/// A heap that fails to grow below its cap, the process's memory taken,
/// isn't, whether or not V8 called the callback first.
const fn heap_at_fault(is_heap_oom: bool, heap_neared: bool) -> bool {
    is_heap_oom && heap_neared
}

/// V8's out-of-memory handler for an eval's isolate: writes the prepared
/// `heap` envelope when the heap is at fault ([`heap_at_fault`]), else the
/// `memory` one, once, when there are any, and exits the process with
/// code 1 at once, running none of its teardown (`gro_sys::exit_now`). V8
/// may call it on any thread, so it takes no lock, and it allocates
/// nothing, since memory may be what ran out.
extern "C" fn oom_backstop(_location: *const std::ffi::c_char, details: &v8::OomDetails) {
    if !BACKSTOP_WRITTEN.swap(true, Ordering::SeqCst)
        && let Some(backstop) = BACKSTOP.get()
    {
        let envelope = if heap_at_fault(details.is_heap_oom, backstop.stopped.heap_neared()) {
            &backstop.heap
        } else {
            &backstop.memory
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
pub struct Watchdog {
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
    pub fn start(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_stop_stands() {
        let stopped = Stopped::default();
        let shared = stopped.clone();
        assert_eq!(shared.get(), None);
        stopped.set(Stop::Timeout);
        stopped.set(Stop::Heap);
        assert_eq!(shared.get(), Some(Stop::Timeout));
        // the heap neared its cap, though the timeout stopped the run
        assert!(shared.heap_neared());
    }

    #[test]
    fn a_full_heap_read_at_the_end_turns_memory_into_heap() {
        let stopped = Stopped::default();
        stopped.set(Stop::Memory);
        stopped.check_heap(HEAP_FULL - 1);
        assert_eq!(stopped.get(), Some(Stop::Memory));
        assert!(!stopped.heap_neared());
        stopped.check_heap(HEAP_FULL);
        assert_eq!(stopped.get(), Some(Stop::Heap));
        assert!(stopped.heap_neared());
        // another stop stands
        let timed_out = Stopped::default();
        timed_out.set(Stop::Timeout);
        timed_out.check_heap(HEAP_LIMIT);
        assert_eq!(timed_out.get(), Some(Stop::Timeout));
    }

    #[test]
    fn the_heap_is_at_fault_only_when_v8_says_so_and_it_neared_its_cap() {
        assert!(heap_at_fault(true, true));
        assert!(!heap_at_fault(true, false));
        assert!(!heap_at_fault(false, true));
        assert!(!heap_at_fault(false, false));
    }
}
