//! Raw system calls the safe std API lacks, kept in one crate so every other
//! gro crate stays free of `unsafe`: they forbid it, and this one denies it
//! and allows it per function, each `unsafe` block beside the argument for
//! why it's sound.
//!
//! Linux only; elsewhere the crate is empty.
//!
//! - [`close_inherited_fds`] closes every file descriptor above stderr, the
//!   ones a process inherits from its parent, which a kernel box that gates
//!   only new opens doesn't reach.
//!
//! With the `testing` feature, which only tests enable (`gro_cli`'s, from its
//! dev-dependencies), it also has:
//!
//! - `signal_thread`, which sends a signal to one thread by `tgkill`, the call
//!   the box allows and Landlock's signal scope keeps inside it.

#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::io;

/// The lowest descriptor [`close_inherited_fds`] closes, the one above
/// stderr.
#[cfg(target_os = "linux")]
const FIRST_INHERITED: libc::c_uint = 3;

/// Closes every file descriptor above stderr (2) the process holds.
///
/// At the start of `main` those are the ones it inherited: a file, a socket,
/// or a device its parent left open without close-on-exec, which a Landlock
/// or seccomp box, gating only new opens, would let it keep using.
///
/// They close with one `close_range` call; on a kernel without it (before
/// 5.9, `ENOSYS`), or under a seccomp filter the parent installed that
/// refuses it (`EPERM`, what some container runtimes' seccomp profiles
/// return for a call they don't list), with `/proc/self/fd` listed and each
/// listed descriptor closed in turn.
///
/// # Precondition
///
/// No value in the process owns a descriptor above 2. The call closes
/// descriptors by number, so one a `File` or an `OwnedFd` held would close
/// under it, and the next open could reuse its number, which that value
/// would then read, write, or close: a break of std's I/O safety. That holds
/// on the main thread at the start of `main`, before the process opens
/// anything: std's start-up holds no descriptor above 2 (it reopens a closed
/// 0, 1, or 2 onto `/dev/null`, which fills only those numbers), and the one
/// handle std caches, `/dev/urandom`, opens only when the `getrandom` call is
/// refused and something has asked for randomness (a `HashMap`'s keys).
///
/// The function is safe, not `unsafe`, with the precondition documented
/// rather than typed: its one caller, the first step of `fgro eval`, is in a
/// crate that forbids `unsafe`. Before closing anything it reads the
/// process's thread count from `/proc/self/status` and refuses unless it's
/// 1, which turns the misuse most likely to matter, a call from a process
/// that has started threads holding handles of their own (a test harness
/// among them), into an error. The check can't see an owner on the calling
/// thread itself: a change that opens a descriptor and keeps it before the
/// call, in `main` or in what runs before it, breaks the precondition
/// silently.
///
/// # Errors
///
/// Returns an error, with nothing closed, when the thread count can't be
/// read or isn't 1, or when `close_range` fails other than by `ENOSYS` or
/// `EPERM`; on the fallback, when `/proc/self/fd` can't be listed or names
/// something other than a descriptor, or when a close fails, other than the
/// listing's own handle, closed already, failing with `EBADF`, and an
/// interrupted close, which Linux has released anyway. Some descriptors may
/// then be closed.
#[cfg(target_os = "linux")]
pub fn close_inherited_fds() -> io::Result<()> {
    let threads = thread_count()?;
    if threads != 1 {
        return Err(io::Error::other(format!(
            "the process has {threads} threads, not 1, so a descriptor may belong to another"
        )));
    }
    match close_range_above_stderr() {
        // a kernel before 5.9, or a parent's seccomp filter refusing the call
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOSYS | libc::EPERM)) => {
            close_listed()
        }
        result => result.map_err(|error| context("close_range failed", &error)),
    }
}

/// The process's thread count, from `/proc/self/status`.
#[cfg(target_os = "linux")]
fn thread_count() -> io::Result<u64> {
    let status = std::fs::read_to_string("/proc/self/status").map_err(|error| {
        context(
            "failed to read the thread count from /proc/self/status",
            &error,
        )
    })?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .and_then(|count| count.trim().parse().ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "/proc/self/status has no thread count",
            )
        })
}

/// Closes the descriptors from [`FIRST_INHERITED`] up with one `close_range`
/// call.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "std has no `close_range`, and a raw system call is `unsafe`"
)]
fn close_range_above_stderr() -> io::Result<()> {
    let flags: libc::c_uint = 0;
    // SAFETY: `close_range` takes three integers and reads or writes none of
    // this process's memory, so the call itself can't break memory safety;
    // that nothing owns the descriptors it closes is `close_inherited_fds`'s
    // precondition
    let result = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            FIRST_INHERITED,
            libc::c_uint::MAX,
            flags,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Lists `/proc/self/fd`, then closes each descriptor listed above stderr:
/// the fallback where the kernel has no `close_range` or a filter refuses
/// it. The listing's own handle is among them, closed when the listing ends,
/// so its close fails with `EBADF`; the process has one thread, so nothing
/// else opens or closes a descriptor between the listing and the closes, and
/// one `EBADF` is expected and a second is an error.
#[cfg(target_os = "linux")]
fn close_listed() -> io::Result<()> {
    let names = std::fs::read_dir("/proc/self/fd")
        .and_then(|listing| {
            listing
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<Vec<_>>>()
        })
        .map_err(|error| context("failed to list /proc/self/fd", &error))?;
    let mut listing_closed = false;
    for fd in above_stderr(names)? {
        match close(fd) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::EINTR) => {}
            Err(error) if error.raw_os_error() == Some(libc::EBADF) && !listing_closed => {
                listing_closed = true;
            }
            Err(error) => {
                return Err(context(&format!("failed to close descriptor {fd}"), &error));
            }
        }
    }
    Ok(())
}

/// The descriptors above stderr among `names`, the names of
/// `/proc/self/fd`'s entries, each a descriptor's number.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidData`] when a name isn't
/// a descriptor's number.
#[cfg(target_os = "linux")]
fn above_stderr(names: impl IntoIterator<Item = OsString>) -> io::Result<Vec<libc::c_int>> {
    let mut fds = Vec::new();
    for name in names {
        // a number a descriptor can be: not negative, and within an int
        let fd = name
            .to_str()
            .and_then(|name| name.parse::<libc::c_uint>().ok())
            .and_then(|fd| libc::c_int::try_from(fd).ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("/proc/self/fd lists {}, not a descriptor", name.display()),
                )
            })?;
        if fd > libc::STDERR_FILENO {
            fds.push(fd);
        }
    }
    Ok(fds)
}

/// Closes the descriptor `fd`.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "closing a descriptor by its number, which no value owns, is `unsafe`"
)]
fn close(fd: libc::c_int) -> io::Result<()> {
    // SAFETY: `close` takes an integer and reads or writes none of this
    // process's memory, so the call itself can't break memory safety; that
    // nothing owns `fd` is `close_inherited_fds`'s precondition
    if unsafe { libc::close(fd) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Sends `signal` to the thread `tid` of the process `tgid`, by `tgkill`;
/// a `signal` of 0 sends nothing and checks only that it could be sent.
///
/// For tests of a box, from inside it: std's `Child::kill` makes `kill`,
/// which the box's seccomp allowlist refuses, so only this call reaches what
/// Landlock's signal scope decides.
///
/// # Errors
///
/// Returns the call's error: `EPERM` when the signal may not be sent (a
/// Landlock scope among the reasons), `ESRCH` when no such thread exists, or
/// `EINVAL` for an invalid signal or a `tgid` or `tid` not above 0.
#[cfg(all(target_os = "linux", feature = "testing"))]
#[expect(
    unsafe_code,
    reason = "std has no `tgkill`, and a raw system call is `unsafe`"
)]
pub fn signal_thread(tgid: libc::pid_t, tid: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    // SAFETY: `tgkill` takes three integers and reads or writes none of this
    // process's memory, so the call itself can't break memory safety; what a
    // signal's handler then does is that handler's own concern, as with
    // std's `Child::kill`
    let result = unsafe { libc::syscall(libc::SYS_tgkill, tgid, tid, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `error`, of the same kind, its message prefixed with `what`.
#[cfg(target_os = "linux")]
fn context(what: &str, error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::ffi::OsString;

    /// The listing's names above stderr are the ones closed, in order; 0 to
    /// 2 stay.
    #[test]
    fn the_listed_descriptors_above_stderr_are_closed() {
        let names = ["0", "1", "2", "3", "17", "4"].map(OsString::from);
        assert_eq!(super::above_stderr(names).unwrap(), [3, 17, 4]);
    }

    /// A name that isn't a descriptor's number fails the listing, rather
    /// than be skipped.
    #[test]
    fn a_listed_name_that_is_no_descriptor_fails() {
        for name in ["x", "-1", "", "3 ", "2147483648"] {
            let error = super::above_stderr([OsString::from(name)]).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData, "{name:?}");
        }
    }
}
