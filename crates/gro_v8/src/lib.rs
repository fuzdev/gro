//! gro's V8 boundary: the calls into V8 that `rusty_v8` exposes only as
//! `unsafe`, each behind a safe API, kept in one crate so every gro crate
//! but this one and `gro_sys`, its twin for system calls, stays free of
//! `unsafe`: they forbid it, and this one denies it and allows it per
//! function, each `unsafe` block beside the argument for why it's sound.
//!
//! Its `v8` is the crate `deno_core` re-exports as `deno_core::v8` (through
//! `deno_v8`, a wrapper that re-exports `v8`), so the types here are the ones
//! the runtime holds.
//!
//! - [`record_array_buffers`] gives an isolate an `ArrayBuffer` allocator
//!   that records whether its latest allocation failed
//!   ([`ArrayBufferRecord`]), which a near-heap-limit callback reads to
//!   tell an array buffer's failure from the heap's.
//! - [`PendingArrayBuffer`] is an `ArrayBuffer`'s bytes allocated by the
//!   isolate's own allocator, which V8 paces its collector by, written by
//!   the host as `&mut [u8]` before any JS can see them.

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

/// The bytes of an `ArrayBuffer` to be, from the isolate's own allocator.
///
/// They're held by Rust alone until
/// [`into_array_buffer`](Self::into_array_buffer) makes the buffer of them,
/// so the host writes them in place, as `&mut [u8]`, and V8 counts them as
/// it counts a buffer JS allocates.
///
/// V8 paces its collector by the array buffers it allocates itself: before
/// each one, a young collection once the young generation's buffers pass
/// twice a semi-space, and a full one, then another try, when the allocator
/// fails. Bytes Rust allocated and handed over (`rusty_v8`'s
/// `new_backing_store_from_vec`) go through none of that, so buffers JS
/// has dropped pile up until the heap's own allocations happen to collect
/// them.
///
/// It holds a backing store no `ArrayBuffer` has yet (V8's
/// `ArrayBuffer::NewBackingStore`), owned by this value alone: no handle,
/// no other store, and no JS reaches its bytes, and V8 never moves them, so
/// a borrow of them stays the only one while it lives, whatever runs in the
/// isolate meanwhile. Dropped without becoming a buffer, it frees them at
/// once. It isn't `Send`, as `rusty_v8`'s `UniqueRef` isn't, so it's
/// filled on the isolate's thread.
#[derive(Debug)]
pub struct PendingArrayBuffer(v8::UniqueRef<v8::BackingStore>);

impl PendingArrayBuffer {
    /// `len` zeroed bytes from `isolate`'s `ArrayBuffer` allocator, under
    /// V8's pacing.
    ///
    /// V8 may collect garbage first, and retries a failed allocation after
    /// collecting all it can; one that still fails ends the process as out
    /// of memory (V8's `FatalProcessOutOfMemory`, which an isolate's
    /// out-of-memory handler sees), since `rusty_v8` binds no form of the
    /// call that returns its failure. A `len` past V8's largest
    /// `ArrayBuffer` fails a check of V8's, which ends the process too, so
    /// a caller bounds it, as the fs reads do at `2 ** 31 - 1`. An empty
    /// buffer allocates nothing.
    pub fn new(isolate: &mut v8::Isolate, len: usize) -> Self {
        Self(v8::ArrayBuffer::new_backing_store(isolate, len))
    }

    /// How many bytes it holds.
    pub fn len(&self) -> usize {
        self.0.byte_length()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Its bytes.
    #[expect(
        unsafe_code,
        reason = "`rusty_v8` gives a backing store's bytes as a pointer or as cells only"
    )]
    pub fn bytes(&self) -> &[u8] {
        let Some(data) = self.0.data() else {
            // a store of no bytes has no allocation
            return &[];
        };
        // SAFETY: `data` is the start of the store's `len()` bytes, one
        // allocation of the isolate's allocator, which V8 neither moves nor
        // resizes (the store isn't resizable) and frees only when the store
        // is destroyed, which the borrow of `self` rules out while the slice
        // lives; every byte is initialized, zeroed by the allocation (V8's
        // default `kZeroInitialized`, which calls the allocator's
        // `Allocate`, whose contract zeroes them) and written only as `u8`
        // since then; no `&mut` to them exists while `self` is borrowed shared,
        // the store being this value's alone ([`PendingArrayBuffer`]); a
        // `u8` needs no alignment, and an allocation never exceeds
        // `isize::MAX` bytes
        unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), self.len()) }
    }

    /// Its bytes, to write.
    #[expect(
        unsafe_code,
        reason = "`rusty_v8` gives a backing store's bytes as a pointer or as cells only"
    )]
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        let Some(data) = self.0.data() else {
            // a store of no bytes has no allocation
            return &mut [];
        };
        // SAFETY: as in `bytes`, `data` starts the store's `len()`
        // initialized bytes, live and unmoved while `self` is borrowed;
        // and the borrow is exclusive: the store is this value's alone, so
        // no `ArrayBuffer`, no JS, and no other store can read or write
        // them, whatever runs in the isolate while the slice lives, and the
        // `&mut self` borrow rules out any other slice of them
        unsafe { std::slice::from_raw_parts_mut(data.as_ptr().cast::<u8>(), self.len()) }
    }

    /// The `ArrayBuffer` of its bytes, which JS can reach from then on and
    /// V8 frees once it's collected.
    pub fn into_array_buffer<'s>(
        self,
        scope: &v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::ArrayBuffer> {
        v8::ArrayBuffer::with_backing_store(scope, &self.0.make_shared())
    }
}
