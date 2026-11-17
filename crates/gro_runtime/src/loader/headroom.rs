//! The stack a builtin's first import needs to run its backing script on
//! the importer's stack: `deno_core` builds a builtin's module the first
//! time a run imports it, and an `import()`'s inside the call.
//!
//! V8 holds JS to a limit `V8_STACK_SIZE` below the thread's stack start and
//! lets the native code JS calls run 40 KiB past it; a garbage collection
//! that starts further down ends the process (V8's check that the heap is
//! collected on the central stack). A first import started near the limit
//! would cut the script short, and `deno_core`'s report of that
//! failure, native frames that allocate, can run past the 40 KiB. So with
//! less than [`BUILTIN_IMPORT_STACK`] left above the limit, the loader has
//! `deno_core` build the module from the event loop, with nearly the whole
//! stack left, and the `import()` settles then: a backing script runs
//! whole whatever its importer left it, and no first import of a builtin
//! is cut short where the limit is read (off glibc nothing is deferred).
//!
//! The loader reads the limit as V8 does, from the thread's stack start
//! ([`gro_sys::stack_start`]), the first time `deno_core` asks, and what's
//! left from the address of a frame of its own, which lies below the
//! `import()` call's.

use std::cell::OnceCell;

/// V8's stack size, `--stack-size`, in bytes: its default on x64 and arm64
/// (`V8_DEFAULT_STACK_SIZE_KB`, 984), which gro doesn't change (no flag the
/// engine sets names it). V8 holds JS to this far below the thread's stack
/// start.
#[cfg(target_env = "gnu")]
const V8_STACK_SIZE: usize = 984 * 1024;

/// The stack, in bytes, the first `import()` of a builtin needs left above
/// V8's limit to build the module on the importer's stack: with less, the
/// module is built from the event loop. More than the need costs only that
/// a first import with less left settles a turn later; less would cut a
/// first import short.
///
/// What a first import needs, measured from the frame `deno_core` asks
/// from, each row imported once from far above the limit: V8 compiles an
/// backing script's outer function lazily, on its first call, and refuses
/// to compile with less than 40 KiB left
/// (`kStackSpaceRequiredForCompilation`), which would cut the import short,
/// so the need is the frames down to that compile plus 40 KiB. With the
/// amount swept in steps of 1 KiB, the sweep named below fails at 59,392
/// bytes in a debug build and passes at 60,416, and in the release build
/// fails at 43,008 and passes at 44,032. Every row measures the same, the
/// compile being the deepest step, and a realm a program patched (a `cause`
/// chain on every error class, an aggregate, `Error.prepareStackTrace`)
/// changes nothing, since no code of the program runs in a load that
/// succeeds. 96 KiB leaves 37 KiB over the debug build's need and 53 KiB
/// over the release build's. The sweep, `gro_cli`'s test
/// `a_first_import_short_of_stack_is_built_from_the_event_loop`, starts
/// every row's first import a frame apart up from the limit in both builds,
/// so a build that needs more than this fails it.
///
/// `deno_core` asks from the event loop too, as a load reaches a builtin,
/// with over 950 KiB left there in either build, and a yes would send the
/// builtin to the loader's `load`, which refuses it: the amount stays far
/// below what the event loop has.
///
/// A failed load's report would need more than the first import does:
/// `deno_core` converts the error natively, following `cause` and an
/// aggregate's errors 32 deep, which reached 54 KiB below the limit in a
/// release build with such a realm, and past V8's 40 KiB margin in a debug
/// build. A backing script throws nothing at load, so none fails with this
/// much left, and one built from the event loop would be reported with
/// nearly the whole stack.
pub const BUILTIN_IMPORT_STACK: usize = 96 * 1024;

/// What's left of the stack above V8's limit, read at the caller.
#[derive(Debug, Default)]
pub(super) struct Headroom {
    /// V8's limit on the thread the loader runs on, read on first need;
    /// `None` where the stack start can't be read.
    limit: OnceCell<Option<usize>>,
}

impl Headroom {
    /// Whether the stack left above V8's limit at the caller is less than
    /// [`BUILTIN_IMPORT_STACK`]; never where the limit can't be read.
    pub(super) fn is_short(&self) -> bool {
        self.left()
            .is_some_and(|left| left < BUILTIN_IMPORT_STACK.cast_signed())
    }

    /// The stack, in bytes, left above V8's limit at the caller, negative
    /// below it; `None` where the limit can't be read.
    fn left(&self) -> Option<isize> {
        let limit = (*self.limit.get_or_init(read_limit))?;
        let marker = 0u8;
        let here = std::ptr::from_ref(std::hint::black_box(&marker)).addr();
        Some(here.wrapping_sub(limit).cast_signed())
    }
}

/// V8's limit on the calling thread's stack: the stack start as V8 reads it,
/// less [`V8_STACK_SIZE`].
#[cfg(target_env = "gnu")]
fn read_limit() -> Option<usize> {
    gro_sys::stack_start().checked_sub(V8_STACK_SIZE)
}

/// Off glibc the stack start isn't read, and every module is built
/// where it's first imported.
#[cfg(not(target_env = "gnu"))]
fn read_limit() -> Option<usize> {
    None
}

#[cfg(test)]
mod tests;
