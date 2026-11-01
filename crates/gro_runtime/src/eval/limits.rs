//! Eval's limits: the heap cap, with its near-heap-limit callback and the
//! out-of-memory backstop, and the timeout's watchdog.
//!
//! Both limits stop the run's JS the same way, by asking V8 to terminate
//! execution, and both say why in one [`Stopped`], which the envelope's
//! kind is read from, never the message V8 or `deno_core` reports. The
//! first to stop the run is the one reported.
//!
//! - **The heap.** The isolate's heap is capped at [`HEAP_LIMIT`]. Near the
//!   cap, V8 calls the callback [`watch_heap`] installs, which records
//!   [`Stop::Heap`], terminates execution, and raises the cap by
//!   [`HEAP_HEADROOM`] once, the room for the run to unwind; a second call
//!   raises nothing. A terminate reaches only JS V8 interrupts: one long
//!   builtin (`new Array(5e7).fill({})`) runs on, and under the JIT, V8
//!   doesn't act on a terminate the callback requests, so either reaches
//!   the cap again with nothing left to give, and V8 calls the
//!   out-of-memory handler: [`oom_backstop`] writes the envelope prepared
//!   before the run ([`set_backstop`]) and exits the process with code 1.
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

/// The heap's cap, in bytes: 512 MiB.
///
/// Provisional until it's measured against real configs. A config builds a
/// small object graph (its evaluation allocates a few MiB of heap), so the
/// cap is two orders of magnitude above that, and still well below a
/// machine's memory, so a runaway eval on the heap fails as `heap` long
/// before the kernel's out-of-memory killer would take it, and its process.
///
/// The cap is V8's heap only. Outside it, uncapped, is the backing memory
/// of an `ArrayBuffer`, a `SharedArrayBuffer`, or a typed array, which a
/// bounded array-buffer allocator would cap. A runaway allocation there
/// grows the process until the kernel ends it, with no envelope written, so
/// a caller treats a missing envelope as a failure, and a caller that wants
/// a hard bound on memory sets one on the process. Also outside it, and
/// bounded by byte caps rather than this one: a module's source, as read
/// and stripped, and its parse
/// ([`MAX_MODULE_SIZE`](crate::resolver::MAX_MODULE_SIZE) per module), and
/// the envelope's own bytes, which the serializer writes outside the heap
/// ([`MAX_ENVELOPE_SIZE`]; a large string is held a few times over while
/// it's written). Not bounded but by this cap on each copy: an uncaught
/// error's report (its message and stack, and each cause's and aggregated
/// error's, up to 256 errors) and the specifiers the loader records, copied
/// out of the heap, so a run that throws a long message, many times over,
/// can take many times this cap before it ends.
pub const HEAP_LIMIT: usize = 512 << 20;

/// The envelope's cap, in bytes: 64 MiB.
///
/// The serializer stops as soon as the value's JSON would pass it, and an
/// envelope that would pass it whole is counted, never written: a value's
/// is replaced by a small `serialize` failure with no `access` or graph,
/// and a failure's keeps its kind, dropping its `access` and graph, then,
/// if it's still past, cutting its long text fields to 64 KiB each. A
/// config's value is a few KiB.
pub const MAX_ENVELOPE_SIZE: usize = 64 << 20;

/// The room the cap is raised by, once, when the heap first nears it: 32
/// MiB, enough for the run to unwind and its envelope to be written.
pub const HEAP_HEADROOM: usize = 32 << 20;

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

/// Why the host stopped the run's JS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The heap reached its cap.
    Heap,
    /// The timeout passed.
    Timeout,
}

/// Why the host stopped the run's JS, if it did: the first stop recorded,
/// shared by the heap's callback, the watchdog's thread, and the
/// serializer's walk.
#[derive(Debug, Clone, Default)]
pub struct Stopped(Arc<AtomicU8>);

impl Stopped {
    const NONE: u8 = 0;
    const HEAP: u8 = 1;
    const TIMEOUT: u8 = 2;

    /// Records `stop`, unless a stop is recorded already.
    fn set(&self, stop: Stop) {
        let value = match stop {
            Stop::Heap => Self::HEAP,
            Stop::Timeout => Self::TIMEOUT,
        };
        // the first stop stands
        let _ = self
            .0
            .compare_exchange(Self::NONE, value, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// The stop recorded, if any.
    pub fn get(&self) -> Option<Stop> {
        match self.0.load(Ordering::SeqCst) {
            Self::HEAP => Some(Stop::Heap),
            Self::TIMEOUT => Some(Stop::Timeout),
            _ => None,
        }
    }
}

/// Installs the heap's near-limit callback and the out-of-memory backstop
/// on `runtime`, whose heap was built capped at [`HEAP_LIMIT`].
pub fn watch_heap(runtime: &mut JsRuntime, stopped: Stopped) {
    let handle = runtime.v8_isolate().thread_safe_handle();
    let mut raised = false;
    runtime.add_near_heap_limit_callback(move |current, _initial| {
        stopped.set(Stop::Heap);
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
    envelope: String,
}

/// The backstop's state, a process static because V8's out-of-memory
/// handler is a bare function with no user data: an exception to the rule
/// that the runtime keeps no process state beyond the engine's. Set once
/// per process, before the first eval that passes a stream.
static BACKSTOP: OnceLock<Backstop> = OnceLock::new();

/// Whether the backstop has written, so it writes once, whichever thread
/// V8 calls it on.
static BACKSTOP_WRITTEN: AtomicBool = AtomicBool::new(false);

/// Sets what the out-of-memory backstop writes: `envelope` (one line, its
/// newline included) to `out`, a handle to the envelope's stream opened
/// before the run, never `std::io::stdout()`'s lock, which another thread
/// may hold when V8 runs out of memory. Returns false when the process set
/// one already.
pub fn set_backstop(out: Arc<File>, envelope: String) -> bool {
    BACKSTOP.set(Backstop { out, envelope }).is_ok()
}

/// V8's out-of-memory handler for an eval's isolate: writes the prepared
/// envelope, once, when there is one, and exits the process with code 1.
/// V8 may call it on any thread, so it takes no lock.
extern "C" fn oom_backstop(_location: *const std::ffi::c_char, _details: &v8::OomDetails) {
    if !BACKSTOP_WRITTEN.swap(true, Ordering::SeqCst)
        && let Some(backstop) = BACKSTOP.get()
    {
        let mut out: &File = &backstop.out;
        // nothing is left to report a failed write to
        let _ = out.write_all(backstop.envelope.as_bytes());
        let _ = out.flush();
    }
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
    }
}
