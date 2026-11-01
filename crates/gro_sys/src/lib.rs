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
//! - [`wait_child`] reaps a child process by `wait4`, returning its exit
//!   status with its peak resident set size, which std's `wait` drops.
//!
//! With the `testing` feature, which only tests enable (`gro_cli`'s, from its
//! dev-dependencies), it also has:
//!
//! - `signal_thread`, which sends a signal to one thread by `tgkill`, the call
//!   the box allows and Landlock's signal scope keeps inside it.
//! - `map_anonymous` and `protect_anonymous`, which map an anonymous page
//!   with a protection, or map one and change it to that protection, and
//!   unmap it, so a box's tests reach its rule on executable mappings.

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

/// A child reaped by [`wait_child`].
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reaped {
    /// How it exited.
    pub status: std::process::ExitStatus,
    /// Its peak resident set size in KiB, the kernel's `ru_maxrss`: the
    /// largest of its own and that of each descendant it reaped.
    ///
    /// Not the child's own peak alone: `exec` folds the peak of the memory
    /// the child ran in before it into `ru_maxrss`. Spawned sharing its
    /// parent's memory (`vfork`, or `posix_spawn`, std's path), that is the
    /// parent's own peak, so a child whose peak is below its parent's reads
    /// the parent's; forked, it is the copy's, which starts at the size of
    /// what was copied.
    pub max_rss_kib: u64,
}

/// Waits for `child` to exit and reaps it by `wait4`, returning its exit
/// status and peak resident set size ([`Reaped`]), retrying an interrupted
/// wait. It takes the child by value, so nothing reaps it twice.
///
/// # Errors
///
/// Returns the call's error, such as `ECHILD` when the child was reaped
/// already some other way.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "std's `wait` drops the resource usage, and a raw system call is `unsafe`"
)]
pub fn wait_child(child: std::process::Child) -> io::Result<Reaped> {
    use std::os::unix::process::ExitStatusExt as _;

    let pid = libc::pid_t::try_from(child.id())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a pid past pid_t"))?;
    // its handles close here, as std's `wait` closes stdin first
    drop(child);
    let mut status: libc::c_int = 0;
    // SAFETY: an all-zero `rusage` is a valid value of the plain-integer
    // struct
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: `wait4` writes only to the two pointers, each to a live
        // local of the type it expects, and reaps only `pid`, a child of this
        // process that `child`, now consumed, named
        let result = unsafe { libc::wait4(pid, &raw mut status, 0, &raw mut usage) };
        if result == pid {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(Reaped {
        status: std::process::ExitStatus::from_raw(status),
        max_rss_kib: u64::try_from(usage.ru_maxrss).unwrap_or(0),
    })
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

/// The length [`map_anonymous`] and [`protect_anonymous`] map, which the
/// kernel rounds up to a whole page where pages are larger.
#[cfg(all(target_os = "linux", feature = "testing"))]
const TEST_MAPPING_LEN: usize = 4096;

/// Maps an anonymous private page with the protection `prot` (`PROT_READ`,
/// `PROT_EXEC`, ...) by `mmap`, then unmaps it, exposing none of its memory.
///
/// For tests of a box, from inside it: whether a mapping with `prot` may be
/// made, the jitless box refusing `PROT_EXEC`.
///
/// # Errors
///
/// Returns the `mmap` call's error (`ENOSYS` when a box's allowlist refuses
/// it), or the `munmap` call's.
#[cfg(all(target_os = "linux", feature = "testing"))]
#[expect(
    unsafe_code,
    reason = "std has no `mmap` or `munmap`, and a raw system call is `unsafe`"
)]
pub fn map_anonymous(prot: libc::c_int) -> io::Result<()> {
    // SAFETY: with no address hint and `MAP_ANONYMOUS`, `mmap` makes a new
    // mapping where nothing in the process is mapped, so no memory the
    // process uses changes; the pointer it returns is never read or written
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            TEST_MAPPING_LEN,
            prot,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if mapped == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `mapped` is the mapping made above, `TEST_MAPPING_LEN` long,
    // whose pointer was never handed out, so nothing references it
    if unsafe { libc::munmap(mapped, TEST_MAPPING_LEN) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Maps an anonymous private page readable and writable by `mmap`, changes
/// its protection to `prot` by `mprotect`, then unmaps it, exposing none of
/// its memory.
///
/// For tests of a box, from inside it: whether a mapping's protection may
/// become `prot`, the jitless box refusing `PROT_EXEC`, as a JIT asks for it
/// after writing its code.
///
/// # Errors
///
/// Returns the `mmap` call's error, the `mprotect` call's (`ENOSYS` when a
/// box's allowlist refuses it), or the `munmap` call's. The page is unmapped
/// whatever `mprotect` returns.
#[cfg(all(target_os = "linux", feature = "testing"))]
#[expect(
    unsafe_code,
    reason = "std has no `mmap`, `mprotect`, or `munmap`, and a raw system call is `unsafe`"
)]
pub fn protect_anonymous(prot: libc::c_int) -> io::Result<()> {
    // SAFETY: as `map_anonymous`'s: a new anonymous mapping, its pointer
    // never read or written
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            TEST_MAPPING_LEN,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if mapped == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `mapped` is the mapping made above, `TEST_MAPPING_LEN` long,
    // which nothing else in the process references, so changing its
    // protection affects no memory the process uses
    let protected = if unsafe { libc::mprotect(mapped, TEST_MAPPING_LEN, prot) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    };
    // SAFETY: as above, the mapping made here, whose pointer was never
    // handed out, so nothing references it
    let unmapped = if unsafe { libc::munmap(mapped, TEST_MAPPING_LEN) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    };
    protected.and(unmapped)
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

    /// A reaped child's status is its exit's, and its peak is counted.
    #[test]
    fn wait_child_reaps_with_the_status_and_the_peak() {
        let child = std::process::Command::new("sh")
            .args(["-c", "exit 3"])
            .spawn()
            .unwrap();
        let reaped = super::wait_child(child).unwrap();
        assert_eq!(reaped.status.code(), Some(3));
        assert!(reaped.max_rss_kib > 0, "{reaped:?}");
    }
}
