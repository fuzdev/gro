//! gro's V8 boundary: the calls into V8 that `rusty_v8` exposes only as
//! `unsafe`, each behind a safe API, kept in one crate so every gro crate
//! but this one and `gro_sys`, its twin for system calls, stays free of
//! `unsafe`: they forbid it, and this one denies it and allows it per
//! function, each `unsafe` block beside the argument for why it's sound.
//!
//! Its `v8` is the crate `deno_core` re-exports as `deno_core::v8`, so the
//! types here are the ones the runtime holds.
//!
//! - [`record_array_buffers`] gives an isolate an `ArrayBuffer` allocator
//!   that records whether its latest allocation failed
//!   ([`ArrayBufferRecord`]), which a near-heap-limit callback reads to
//!   tell an array buffer's failure from the heap's.

use std::alloc::{GlobalAlloc as _, Layout, System};
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the latest allocation of an isolate's array-buffer allocator
/// failed, shared with the allocator [`record_array_buffers`] installs.
///
/// An array buffer's bytes (a typed array's, a `SharedArrayBuffer`'s) come
/// from that allocator, outside the heap. When one can't be had, V8 collects
/// all it can and calls the near-heap-limit callback, as it does for a heap
/// at its cap, and only then tries the allocation once more, so the callback
/// finds the failure recorded. An allocation that succeeds clears it, so a
/// failure V8 got past (one a collection made room for) is never read as a
/// later stop's. A resizable `ArrayBuffer`'s bytes and a `WebAssembly`
/// memory's come from V8's page allocator instead, which records nothing.
#[derive(Debug, Clone)]
pub struct ArrayBufferRecord(Arc<AtomicBool>);

impl ArrayBufferRecord {
    /// Whether the allocator's latest allocation failed. Reads one atomic,
    /// so it's safe in a near-heap-limit callback or an out-of-memory
    /// handler, on any thread.
    pub fn latest_failed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// `params` with an `ArrayBuffer` allocator that records whether its latest
/// allocation failed, and the record it writes.
///
/// The allocator allocates as V8's default does on Linux: zeroed or not as
/// V8 asks, from the C allocator (std's `System`: `calloc` and `malloc`, or
/// `posix_memalign` for a length under the alignment), freed with `free`,
/// with V8's default largest allocation. It replaces any allocator `params`
/// held. The record is had only with the allocator
/// installed, so it always speaks for the isolate built from `params`.
#[expect(
    unsafe_code,
    reason = "rusty_v8 makes an allocator from Rust callbacks only by an `unsafe` call"
)]
pub fn record_array_buffers(params: v8::CreateParams) -> (v8::CreateParams, ArrayBufferRecord) {
    let record = ArrayBufferRecord(Arc::new(AtomicBool::new(false)));
    let handle = Arc::into_raw(Arc::clone(&record.0));
    // SAFETY: `handle` is a strong count of an `Arc<AtomicBool>`, live until
    // the allocator's `drop` (`drop_handle`) takes it back, which V8 calls
    // once, from the allocator's destructor, after its last allocation or
    // free; each callback in `RECORDING` reads `handle` as the `&AtomicBool`
    // that `new_rust_allocator` requires of a handle of type `AtomicBool`.
    // The handle is `Send + Sync`, as an allocator V8 may call on any thread
    // needs
    let allocator = unsafe { v8::new_rust_allocator(handle, &RECORDING) };
    (params.array_buffer_allocator(allocator), record)
}

/// The recording allocator's callbacks.
static RECORDING: v8::RustAllocatorVtable<AtomicBool> = v8::RustAllocatorVtable {
    allocate,
    allocate_uninitialized,
    free,
    drop: drop_handle,
};

/// The alignment the allocator gives: 16 bytes, `malloc`'s on the 64-bit
/// targets gro builds for, and what V8's default allocator gives, since
/// it calls `malloc`.
const ALIGN: usize = 16;

/// The layout of an array buffer of `len` bytes, or `None` when no
/// allocation can be that large. An empty buffer takes one byte, so every
/// layout has a size, which `System` requires; V8 asks for none (it leaves
/// an empty buffer's bytes unallocated), and `free` maps the length the
/// same way.
fn layout(len: usize) -> Option<Layout> {
    Layout::from_size_align(len.max(1), ALIGN).ok()
}

/// Allocates `len` zeroed bytes, recording whether that failed.
#[expect(
    unsafe_code,
    reason = "an allocator callback rusty_v8 calls through its vtable, and `System`'s \
              allocation, are `unsafe`"
)]
unsafe extern "C" fn allocate(record: &AtomicBool, len: usize) -> *mut c_void {
    let data = layout(len).map_or_else(std::ptr::null_mut, |layout| {
        // SAFETY: the layout has a nonzero size (`layout`)
        unsafe { System.alloc_zeroed(layout) }
    });
    record.store(data.is_null(), Ordering::SeqCst);
    data.cast()
}

/// Allocates `len` bytes, uninitialized, recording whether that failed.
#[expect(
    unsafe_code,
    reason = "an allocator callback rusty_v8 calls through its vtable, and `System`'s \
              allocation, are `unsafe`"
)]
unsafe extern "C" fn allocate_uninitialized(record: &AtomicBool, len: usize) -> *mut c_void {
    let data = layout(len).map_or_else(std::ptr::null_mut, |layout| {
        // SAFETY: the layout has a nonzero size (`layout`)
        unsafe { System.alloc(layout) }
    });
    record.store(data.is_null(), Ordering::SeqCst);
    data.cast()
}

/// Frees the `len` bytes at `data`, which [`allocate`] or
/// [`allocate_uninitialized`] gave V8.
#[expect(
    unsafe_code,
    reason = "an allocator callback rusty_v8 calls through its vtable, and `System`'s \
              deallocation, are `unsafe`"
)]
unsafe extern "C" fn free(_record: &AtomicBool, data: *mut c_void, len: usize) {
    if data.is_null() {
        return;
    }
    if let Some(layout) = layout(len) {
        // SAFETY: V8 frees only what this allocator gave it, once, with the
        // length it asked for (`ArrayBuffer::Allocator::Free`'s contract),
        // so `data` came from `System` with this same layout: `layout` maps
        // the length the allocation took it from the same way, and an
        // allocation was made only when that layout existed
        unsafe { System.dealloc(data.cast(), layout) };
    }
}

/// Takes back the strong count [`record_array_buffers`] gave the allocator,
/// as V8 destroys the allocator.
#[expect(
    unsafe_code,
    reason = "an allocator callback rusty_v8 calls through its vtable, and taking back an \
              `Arc`'s raw pointer, are `unsafe`"
)]
unsafe extern "C" fn drop_handle(handle: *const AtomicBool) {
    // SAFETY: `handle` is the pointer `Arc::into_raw` gave in
    // `record_array_buffers`, and V8 calls this once, from the allocator's
    // destructor, after which no callback reads it
    drop(unsafe { Arc::from_raw(handle) });
}
