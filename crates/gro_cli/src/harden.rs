//! The steps `fgro eval` takes on the main thread before it opens anything:
//! closing the inherited descriptors, clearing the environment, and capping
//! memory. Each is Linux's, with a no-op elsewhere, where eval refuses
//! before it would start.

/// Closes every file descriptor above stderr the process inherited (a file,
/// a socket, a device its parent left open), which the box, gating only new
/// opens, wouldn't reach: `gro_sys::close_inherited_fds`, sound because it
/// runs first on the main thread, the only one, before anything in the
/// process opens a descriptor.
#[cfg(target_os = "linux")]
pub fn close_inherited() -> std::io::Result<()> {
    gro_sys::close_inherited_fds()
}

/// The variables eval keeps from its environment: `TZ` alone, which ICU
/// reads for the time zone eval then checks is UTC.
#[cfg(target_os = "linux")]
const KEPT_ENV: &[&str] = &["TZ"];

/// Clears the process's environment but [`KEPT_ENV`], so no library eval
/// starts (`deno_core`'s diagnostics and import graph file, ICU's locale,
/// std's backtrace and stack size, any other) reads a variable:
/// `gro_sys::clear_env_except`, sound because it runs on the main thread,
/// the only one, before anything that keeps a pointer into the environment
/// has run.
#[cfg(target_os = "linux")]
pub fn clear_env() -> std::io::Result<()> {
    gro_sys::clear_env_except(KEPT_ENV)
}

/// Lowers the stack of a thread started without a size of its own (V8's
/// platform workers) to `gro_runtime::eval::DEFAULT_THREAD_STACK`, from the
/// size glibc takes from the inherited `RLIMIT_STACK`, keeping a smaller one:
/// `gro_sys::set_default_thread_stack`. Then caps the process's data segment,
/// which counts those stacks, at `gro_runtime::eval::MEMORY_LIMIT`, soft and
/// hard alike, or keeps the lower limit the parent set: `gro_sys::limit_data`.
/// Returns the limit set, in bytes. The box's seccomp filter then refuses
/// `setrlimit` and `prlimit64`, so nothing the eval runs raises it.
#[cfg(target_os = "linux")]
pub fn limit_memory() -> std::io::Result<u64> {
    gro_sys::set_default_thread_stack(gro_runtime::eval::DEFAULT_THREAD_STACK)?;
    gro_sys::limit_data(gro_runtime::eval::MEMORY_LIMIT)
}

/// The process's data limit as it stands, its soft value in bytes
/// (`u64::MAX` for none), for a priced build that doesn't cap it:
/// `gro_sys::data_limit`.
#[cfg(target_os = "linux")]
pub fn data_limit() -> std::io::Result<u64> {
    gro_sys::data_limit().map(|(soft, _hard)| soft)
}

#[cfg(not(target_os = "linux"))]
pub use elsewhere::{clear_env, close_inherited, data_limit, limit_memory};

/// Each step elsewhere, a no-op with Linux's signature: there is no box to
/// escape, and eval refuses before it would start.
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signatures as Linux's, which can fail"
)]
mod elsewhere {
    /// Closes nothing.
    pub const fn close_inherited() -> std::io::Result<()> {
        Ok(())
    }

    /// Clears nothing.
    pub const fn clear_env() -> std::io::Result<()> {
        Ok(())
    }

    /// Caps nothing, and returns no limit.
    pub const fn limit_memory() -> std::io::Result<u64> {
        Ok(u64::MAX)
    }

    /// No limit.
    pub const fn data_limit() -> std::io::Result<u64> {
        Ok(u64::MAX)
    }
}
