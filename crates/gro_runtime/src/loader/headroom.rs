//! The stack an `import()` of a builtin needs left: `deno_core` runs a
//! builtin's implementation the first time a run imports it, inside the
//! `import()` call, on the importer's stack, and would report a failure of
//! it there too.
//!
//! V8 holds JS to a limit `V8_STACK_SIZE` below the thread's stack start and
//! lets the native code JS calls run 40 KiB past it; a garbage collection
//! that starts further down ends the process (V8's check that the heap is
//! collected on the central stack). A first import started near the limit
//! would cut the implementation short, and `deno_core`'s report of that
//! failure, native frames that allocate, can run past the 40 KiB. So the
//! loader refuses an `import()` of a builtin with less than
//! [`BUILTIN_IMPORT_STACK`] left above the limit, before anything of it
//! runs: an implementation runs whole or not at all, whatever its importer
//! left it, and no report of its failure is made on that stack.
//!
//! The loader reads the limit as V8 does, from the thread's stack start
//! ([`gro_sys::stack_start`]), the first time an `import()` of a builtin
//! asks, and what's left from the address of a frame of its own, which lies
//! below the `import()` call's.

use std::cell::OnceCell;

/// V8's stack size, `--stack-size`, in bytes: its default on x64 and arm64
/// (`V8_DEFAULT_STACK_SIZE_KB`, 984), which gro doesn't change (no flag the
/// engine sets names it). V8 holds JS to this far below the thread's stack
/// start.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const V8_STACK_SIZE: usize = 984 * 1024;

/// The stack, in bytes, an `import()` of a builtin needs left above V8's
/// limit where the loader resolves it: with less, the import is refused,
/// whether or not the run imported the builtin already, which the loader
/// can't tell when it resolves.
///
/// What a first import needs, measured from the check's own frame, each row
/// imported once from far above the limit: V8 compiles an implementation's
/// outer function lazily, on its first call, and refuses to compile with
/// less than 40 KiB left (`kStackSpaceRequiredForCompilation`), which would
/// cut the import short, so the need is the frames down to that compile
/// plus 40 KiB. In a debug build with full debuginfo the compile's frame
/// is 8,608 bytes below the check's, a need of 49,568, and the deepest byte
/// the import writes is 43,664 below the check, in that compile; with the
/// amount at 48 KiB the sweep named below fails, at 50 KiB it passes. In the
/// release build the deepest byte is about 35 KiB below the check, and the
/// sweep fails with the amount at 40,960 bytes and passes at 41,472.
/// Every row measures the same, the compile being the deepest step, and a
/// realm a program patched (a `cause` chain on every error class, an
/// aggregate, `Error.prepareStackTrace`) changes nothing, since no code of
/// the program runs in a load that succeeds. 64 KiB leaves 14 KiB over the
/// debug build's need and 23 KiB over the release build's.
/// The sweep, `gro_cli`'s test
/// `a_first_import_short_of_stack_is_refused_and_run_whole_later`, starts
/// every row's first import a frame apart up from the limit in both builds,
/// so a build that needs more fails it.
///
/// A failed load's report would need more than this: `deno_core` converts
/// the error natively, following `cause` and an aggregate's errors 32 deep,
/// which reached 54 KiB below the limit in a release build with such a
/// realm, and past V8's 40 KiB margin in a debug build. With this much
/// left, an implementation, which throws nothing at load, doesn't fail.
pub const BUILTIN_IMPORT_STACK: usize = 64 * 1024;

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
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn read_limit() -> Option<usize> {
    gro_sys::stack_start().checked_sub(V8_STACK_SIZE)
}

/// Off glibc on Linux the stack start isn't read, and no import is refused.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn read_limit() -> Option<usize> {
    None
}

#[cfg(test)]
mod tests;
