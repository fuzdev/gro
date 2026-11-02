//! gro's V8 boundary: the calls into V8 that `rusty_v8` exposes only as
//! `unsafe`, each behind a safe API, kept in one crate so every gro crate
//! but this one and `gro_sys`, its twin for system calls, stays free of
//! `unsafe`: they forbid it, and this one denies it and allows it per
//! function, each `unsafe` block beside the argument for why it's sound.
//!
//! Its `v8` is the crate `deno_core` re-exports as `deno_core::v8`, so the
//! types here are the ones the runtime holds.
//!
//! - [`HeapReader`] reads an isolate's heap use where V8 gives no isolate:
//!   in a near-heap-limit callback.

use std::sync::{Arc, Weak};

/// A reader of an isolate's heap use, for a near-heap-limit callback, which
/// V8 calls with no isolate.
///
/// It keeps the isolate's raw pointer and a weak handle to a token the
/// isolate holds in a slot. Slots are dropped as the isolate is disposed,
/// before V8 frees it, so the reader reads only while the isolate lives,
/// and reads nothing once it's gone. It can't leave the thread that made
/// it, the isolate's (it's neither `Send` nor `Sync`).
#[derive(Debug)]
pub struct HeapReader {
    isolate: v8::UnsafeRawIsolatePtr,
    alive: Weak<IsolateAlive>,
}

/// The token a [`HeapReader`]'s isolate holds in a slot, one per isolate,
/// shared by its readers.
#[derive(Debug)]
struct IsolateAlive;

impl HeapReader {
    /// A reader of `isolate`'s heap.
    #[expect(
        unsafe_code,
        reason = "rusty_v8 gives an isolate's raw pointer only by an `unsafe` call"
    )]
    pub fn new(isolate: &mut v8::Isolate) -> Self {
        let existing = isolate.get_slot::<Arc<IsolateAlive>>().map(Arc::downgrade);
        let alive = existing.unwrap_or_else(|| {
            let alive = Arc::new(IsolateAlive);
            let weak = Arc::downgrade(&alive);
            isolate.set_slot(alive);
            weak
        });
        // SAFETY: the call copies the isolate's pointer, reading nothing
        // through it; `used_heap_size` turns it back into an isolate only
        // while `alive` says the isolate lives
        let isolate = unsafe { isolate.as_raw_isolate_ptr() };
        Self { isolate, alive }
    }

    /// The bytes the heap's objects use, live or not yet collected, from
    /// V8's heap statistics, or `None` once the isolate is gone. Allocates
    /// nothing in Rust.
    #[expect(
        unsafe_code,
        reason = "V8 gives a near-heap-limit callback no isolate, so the reader's raw pointer is \
                  turned back into one"
    )]
    pub fn used_heap_size(&self) -> Option<usize> {
        // the token lives in the isolate's slots, dropped before V8 frees
        // the isolate; holding it here keeps nothing alive past that, since
        // the slot's drop is what ends it. Its count is atomic: V8 can run a
        // near-heap-limit callback on a thread allocating in the background
        // (V8's `HeapAllocator::RetryCustomAllocateLight`, in
        // `src/heap/heap-allocator.cc`, checks the heap's limit on the
        // allocating thread), so the read can't be kept to the isolate's
        let _alive = self.alive.upgrade()?;
        // SAFETY: the isolate's memory is live for the whole read. The token
        // was in its slot when upgraded, and rusty_v8's `OwnedIsolate::drop`
        // drops the slots (`prepare_annex_for_dispose`) before it calls
        // `Isolate::Dispose`; a read that began before that drop finishes
        // before V8 frees anything, since `Isolate::Deinit` (V8's
        // `src/execution/isolate.cc`) waits out every background task and
        // compile job before it tears the heap down ("no more background
        // threads left in this isolate"), and the isolate's own thread is
        // the one running the drop. The call itself may run on any thread:
        // V8's `Isolate::GetHeapStatistics` (`src/api/api.cc`) states the
        // embedder may call it from any thread, and sets the thread's
        // current isolate for the call. The handle made here only copies the
        // pointer, forms no Rust reference to the isolate's memory, has no
        // `Drop`, and is gone before this returns
        let mut isolate = unsafe { v8::Isolate::from_raw_isolate_ptr(self.isolate) };
        Some(isolate.get_heap_statistics().used_heap_size())
    }
}

#[cfg(test)]
mod tests {
    use super::HeapReader;

    #[test]
    fn a_reader_reads_while_its_isolate_lives_and_not_after() {
        v8::V8::initialize_platform(v8::new_default_platform(0, false).make_shared());
        v8::V8::initialize();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let reader = HeapReader::new(&mut isolate);
        let second = HeapReader::new(&mut isolate);
        assert!(reader.used_heap_size().is_some_and(|used| used > 0));
        assert_eq!(second.used_heap_size(), reader.used_heap_size());
        drop(isolate);
        assert_eq!(reader.used_heap_size(), None);
        assert_eq!(second.used_heap_size(), None);
    }
}
