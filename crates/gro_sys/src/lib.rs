//! Raw system calls the safe std API lacks, kept in one crate so every gro
//! crate but this one and `gro_v8`, its twin for the V8 calls `rusty_v8`
//! exposes only as `unsafe`, stays free of `unsafe`: they forbid it, and
//! this one denies it and allows it per function, each `unsafe` block
//! beside the argument for why it's sound.
//!
//! Linux only; elsewhere the crate is empty.
//!
//! - [`close_inherited_fds`] closes every file descriptor above stderr, the
//!   ones a process inherits from its parent, which a kernel box that gates
//!   only new opens doesn't reach.
//! - [`clear_env_except`] clears the process's environment but the
//!   variables it names, so no library the process runs later reads one.
//! - [`limit_data`] caps the process's data segment (`RLIMIT_DATA`), its
//!   soft and hard limits alike, so it can't be raised again.
//! - [`set_default_thread_stack`] lowers the stack size of a thread started
//!   without one, which glibc otherwise takes from `RLIMIT_STACK`, never
//!   raising it.
//! - [`thread_count`] reads the process's thread count, which a box or a
//!   process-wide change installed from one thread needs to be 1.
//! - [`wait_child`] reaps a child process by `wait4`, returning its exit
//!   status with its peak resident set size, which std's `wait` drops.
//! - [`kill_process`] and [`kill_process_group`] send `SIGKILL` to a process
//!   or a process group by its id, for a watchdog that doesn't own the child
//!   another thread is reaping.
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

/// Clears the process's environment but the variables named in `kept`.
///
/// Each kept variable is set again to the value it had, so nothing the
/// process runs after reads any other: not its own code, and not a
/// library's (`getenv` from C, or `std::env` from Rust). It clears by
/// `clearenv`, so every entry goes, malformed ones included (a name with no
/// `=`, or one std can't remove), then sets each kept variable that was
/// set, its value read first.
///
/// # Precondition
///
/// Nothing in the process holds a pointer into the environment, such as a
/// string `getenv` returned: on `clearenv`, musl frees the strings `setenv`
/// made, and a C library that kept one would then read freed memory (glibc
/// never frees them). That holds on the main thread at the start of `main`,
/// before the process has started any library that keeps one: std copies
/// what it reads.
///
/// The function is safe, not `unsafe`, with the precondition documented
/// rather than typed, as [`close_inherited_fds`]'s: its one caller, in
/// `fgro eval`, is in a crate that forbids `unsafe`. Changing the
/// environment races with any other thread that reads or writes it, so
/// before changing anything it reads the process's thread count from
/// `/proc/self/status` and refuses unless it's 1; a thread can only start
/// from this one, which is in this call. The check can't see a pointer the
/// calling thread kept from an earlier read.
///
/// # Errors
///
/// Returns an error, with nothing changed, when the thread count can't be
/// read or isn't 1, or when a name in `kept` is empty or holds a `=` or a
/// NUL; when `clearenv` fails, or a kept variable can't be set again, the
/// environment may be left cleared.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "changing the environment is `unsafe`: another thread may read it"
)]
pub fn clear_env_except(kept: &[&str]) -> io::Result<()> {
    let threads = thread_count()?;
    if threads != 1 {
        return Err(io::Error::other(format!(
            "the process has {threads} threads, not 1, so another may read the environment"
        )));
    }
    if let Some(name) = kept
        .iter()
        .find(|name| name.is_empty() || name.contains(['=', '\0']))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} can't name an environment variable"),
        ));
    }
    // read before the clear, each value a copy; a value from the environment
    // holds no NUL, so setting it again can't panic
    let values: Vec<(&str, OsString)> = kept
        .iter()
        .filter_map(|&name| std::env::var_os(name).map(|value| (name, value)))
        .collect();
    // SAFETY: `clearenv` reads and writes only the environment, which no
    // other thread exists to touch (the count above is 1, and only this
    // thread could start one); that nothing holds a pointer into it is this
    // function's precondition
    if unsafe { libc::clearenv() } != 0 {
        return Err(io::Error::other("clearenv failed"));
    }
    for (name, value) in values {
        // SAFETY: as above, the process's one thread is this call's; the name
        // was checked to hold no `=` or NUL and isn't empty, and the value,
        // from the environment, holds no NUL, so std's checks pass
        unsafe { std::env::set_var(name, value) };
    }
    Ok(())
}

/// Caps the process's data segment, `RLIMIT_DATA`, at `bytes`, and returns
/// the cap set.
///
/// A limit already lower, soft or hard, stands, and the cap is set soft and
/// hard alike, so nothing in the process can raise it again (raising a hard
/// limit takes `CAP_SYS_RESOURCE`).
///
/// Since Linux 4.7 the limit counts the process's private writable
/// mappings: what `malloc` and anonymous `mmap` take, thread stacks
/// included, but not a reservation made without write access until it's
/// made writable, nor a file's mapped code. A mapping, or a growth of one,
/// that would pass it fails with `ENOMEM`, so `malloc` returns null.
///
/// It touches no memory of the process's and holds no lock, so it's safe on
/// any thread; it governs every thread, the ones started after included.
///
/// # Errors
///
/// Returns the error of `getrlimit` or `setrlimit`, the limit unchanged.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "std has no `getrlimit` or `setrlimit`, and a libc call is `unsafe`"
)]
pub fn limit_data(bytes: u64) -> io::Result<u64> {
    let mut current = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes only to the pointer, to a live local of the
    // type it expects
    if unsafe { libc::getrlimit(libc::RLIMIT_DATA, &raw mut current) } != 0 {
        return Err(context(
            "getrlimit(RLIMIT_DATA) failed",
            &io::Error::last_os_error(),
        ));
    }
    // a limit the parent set lower stands, the soft one included (it's never
    // above the hard one); `RLIM_INFINITY` is the largest value, so it never
    // wins
    let cap = bytes.min(current.rlim_cur);
    let limit = libc::rlimit {
        rlim_cur: cap,
        rlim_max: cap,
    };
    // SAFETY: `setrlimit` reads only the pointer, to a live local of the
    // type it expects, and changes no memory of the process's
    if unsafe { libc::setrlimit(libc::RLIMIT_DATA, &raw const limit) } != 0 {
        return Err(context(
            "setrlimit(RLIMIT_DATA) failed",
            &io::Error::last_os_error(),
        ));
    }
    Ok(cap)
}

/// The process's `RLIMIT_DATA`, soft and hard, from `getrlimit`.
///
/// # Errors
///
/// Returns the call's error.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "std has no `getrlimit`, and a libc call is `unsafe`"
)]
pub fn data_limit() -> io::Result<(u64, u64)> {
    let mut current = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes only to the pointer, to a live local of the
    // type it expects
    if unsafe { libc::getrlimit(libc::RLIMIT_DATA, &raw mut current) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((current.rlim_cur, current.rlim_max))
}

/// Lowers the stack size of a thread started without a size of its own to
/// `bytes`, never raising it.
///
/// It reads the default ([`default_thread_stack`]) and keeps it when it's no
/// larger, else sets `bytes` by `pthread_setattr_default_np`.
///
/// Such a thread is one a C or C++ library starts with a default attribute,
/// V8's platform workers among them, any time after the call.
///
/// glibc sizes such a thread's stack by the `RLIMIT_STACK` the process
/// started with, so a parent's large stack limit, a few hundred MiB, would
/// make each one that large, every byte of it counted by [`limit_data`]'s
/// cap. Under an unlimited `RLIMIT_STACK`, glibc takes a smaller default of
/// its own instead, which stands. A thread std starts sets its own size, and
/// keeps it.
///
/// The calls take glibc's lock on its default attribute and touch no other
/// memory of the process's, so they're safe on any thread; a thread starting
/// while they run gets the old size or the new one, and a default another
/// thread sets between the read and the write is replaced.
///
/// Elsewhere than glibc it does nothing: musl sizes such a thread by a
/// fixed default of its own, never by `RLIMIT_STACK`.
///
/// # Errors
///
/// Returns the error of `pthread_getattr_default_np`,
/// `pthread_attr_getstacksize`, `pthread_attr_init`,
/// `pthread_attr_setstacksize` (a size below `PTHREAD_STACK_MIN`), or
/// `pthread_setattr_default_np`, the default unchanged.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[expect(
    unsafe_code,
    reason = "std has no default thread attribute, and a libc call is `unsafe`"
)]
pub fn set_default_thread_stack(bytes: usize) -> io::Result<()> {
    let current = default_thread_stack()
        .map_err(|error| context("reading the default thread stack failed", &error))?;
    if current <= bytes {
        return Ok(());
    }
    let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: `pthread_attr_init` initializes the attribute the pointer
    // names, a live local of the type it expects
    let code = unsafe { libc::pthread_attr_init(attr.as_mut_ptr()) };
    if code != 0 {
        return Err(context(
            "pthread_attr_init failed",
            &io::Error::from_raw_os_error(code),
        ));
    }
    // SAFETY: the attribute was initialized above and isn't destroyed until
    // below; the call writes only its stack size
    let code = unsafe { libc::pthread_attr_setstacksize(attr.as_mut_ptr(), bytes) };
    let set = if code == 0 {
        // SAFETY: the attribute is initialized, with no stack address or CPU
        // set, which the call would refuse; glibc copies it under its own
        // lock, keeping no pointer to it
        let code = unsafe { pthread_setattr_default_np(attr.as_ptr()) };
        if code == 0 {
            Ok(())
        } else {
            Err(context(
                "pthread_setattr_default_np failed",
                &io::Error::from_raw_os_error(code),
            ))
        }
    } else {
        Err(context(
            "pthread_attr_setstacksize failed",
            &io::Error::from_raw_os_error(code),
        ))
    };
    // SAFETY: the attribute was initialized above and is destroyed once;
    // glibc kept no pointer to it
    unsafe { libc::pthread_attr_destroy(attr.as_mut_ptr()) };
    set
}

/// Elsewhere than glibc, nothing to lower: musl sizes a thread started
/// without a size by a fixed default, never by `RLIMIT_STACK`.
///
/// # Errors
///
/// Never: the signature is glibc's, which can fail.
#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
pub const fn set_default_thread_stack(_bytes: usize) -> io::Result<()> {
    Ok(())
}

/// The stack size, in bytes, of a thread started without one, from
/// `pthread_getattr_default_np`.
///
/// # Errors
///
/// Returns the error of a call.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[expect(
    unsafe_code,
    reason = "std has no default thread attribute, and a libc call is `unsafe`"
)]
pub fn default_thread_stack() -> io::Result<usize> {
    let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: `pthread_getattr_default_np` initializes the attribute the
    // pointer names, a live local of the type it expects, to a copy of the
    // default
    let code = unsafe { pthread_getattr_default_np(attr.as_mut_ptr()) };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code));
    }
    let mut bytes: libc::size_t = 0;
    // SAFETY: the attribute was initialized above; the call writes only to
    // `bytes`, a live local of the type it expects
    let code = unsafe { libc::pthread_attr_getstacksize(attr.as_ptr(), &raw mut bytes) };
    // SAFETY: the attribute was initialized above and is destroyed once
    unsafe { libc::pthread_attr_destroy(attr.as_mut_ptr()) };
    if code == 0 {
        Ok(bytes)
    } else {
        Err(io::Error::from_raw_os_error(code))
    }
}

// glibc's, since 2.18; the `libc` crate doesn't declare them
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[expect(
    unsafe_code,
    reason = "the `libc` crate lacks these two, so they're declared here, as glibc's headers do"
)]
unsafe extern "C" {
    fn pthread_setattr_default_np(attr: *const libc::pthread_attr_t) -> libc::c_int;
    fn pthread_getattr_default_np(attr: *mut libc::pthread_attr_t) -> libc::c_int;
}

/// The process's thread count, its `Threads:` line in `/proc/self/status`,
/// read once.
///
/// A thread whose join has returned can stay counted for a moment longer,
/// while the kernel finishes its exit, so a caller that needs the count to
/// reach 1 after a join reads it again.
///
/// # Errors
///
/// Returns an error when the file can't be read or has no thread count.
#[cfg(target_os = "linux")]
pub fn thread_count() -> io::Result<u64> {
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

/// Sends `SIGKILL` to the process `pid`, by `kill`.
///
/// For a watchdog that ends a child another thread is reaping with
/// [`wait_child`], which owns the child, so std's `Child::kill` can't reach
/// it. The id names the child only until it's reaped: a caller kills only a
/// child it knows isn't reaped yet, as far as it can know, since the id is
/// free for reuse once it is.
///
/// # Errors
///
/// Returns the call's error: `ESRCH` when no such process exists, `EPERM`
/// when it may not be signaled, or `EINVAL` for an id past `pid_t` or not
/// above 0, which would name a process group.
#[cfg(target_os = "linux")]
pub fn kill_process(pid: u32) -> io::Result<()> {
    kill(positive_pid(pid)?)
}

/// Sends `SIGKILL` to every process of the group `pgid`, by `kill` with the
/// id negated.
///
/// That is a child spawned as the leader of its own group
/// (`CommandExt::process_group(0)`) and what it spawned, which, holding the
/// leader's pipes, would otherwise keep a reader of them waiting. Like
/// [`kill_process`], the id names the group only while its leader isn't
/// reaped.
///
/// # Errors
///
/// Returns the call's error: `ESRCH` when no process is in the group,
/// `EPERM` when none may be signaled, or `EINVAL` for an id past `pid_t` or
/// not above 1.
#[cfg(target_os = "linux")]
pub fn kill_process_group(pgid: u32) -> io::Result<()> {
    // group 1 would be `kill(-1)`, every process the caller may signal
    if pgid <= 1 {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    kill(-positive_pid(pgid)?)
}

/// `id` as a `pid_t` above 0, or `EINVAL`.
#[cfg(target_os = "linux")]
fn positive_pid(id: u32) -> io::Result<libc::pid_t> {
    libc::pid_t::try_from(id)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))
}

/// `kill(target, SIGKILL)`, for a `target` that is a process (above 0) or a
/// group (below -1), never 0 or -1, which name the caller's group or every
/// process.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "std kills only a child it owns, and a libc call is `unsafe`"
)]
fn kill(target: libc::pid_t) -> io::Result<()> {
    debug_assert!(!matches!(target, 0 | -1), "{target}");
    // SAFETY: `kill` takes two integers and reads or writes none of this
    // process's memory; the callers pass one process or one group, never 0
    // or -1
    let result = unsafe { libc::kill(target, libc::SIGKILL) };
    if result == 0 {
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

/// Ends the process now with `code`, by `_exit`.
///
/// No `atexit` handler, no C++ static destructor, and no stdio flush runs,
/// so a process whose other threads may be mid-operation (V8's workers, out
/// of memory) exits by its code rather than crashing in teardown. Anything
/// the caller wrote must be flushed first.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "std has no `_exit`, and a libc call is `unsafe`"
)]
pub fn exit_now(code: i32) -> ! {
    // SAFETY: `_exit` takes an integer, reads no memory of the process's,
    // and never returns; skipping the process's teardown is its purpose
    unsafe { libc::_exit(code) }
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

    /// A killed child reaps as killed by `SIGKILL`, and an id that would
    /// name a group is refused.
    #[test]
    fn kill_process_kills_a_child_by_its_id() {
        use std::os::unix::process::ExitStatusExt as _;

        let child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        super::kill_process(child.id()).unwrap();
        let reaped = super::wait_child(child).unwrap();
        assert_eq!(reaped.status.signal(), Some(libc::SIGKILL));
        for pid in [0, u32::MAX] {
            let error = super::kill_process(pid).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::EINVAL), "{pid}");
            let error = super::kill_process_group(pid).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::EINVAL), "{pid}");
        }
    }

    /// A killed group takes the leader's children with it.
    #[test]
    fn kill_process_group_kills_the_leader_and_its_children() {
        use std::io::Read as _;
        use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};

        let mut leader = std::process::Command::new("sh")
            .args(["-c", "sleep 60 & echo started; wait"])
            .stdout(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut stdout = leader.stdout.take().unwrap();
        let mut started = [0; 8];
        stdout.read_exact(&mut started).unwrap();
        super::kill_process_group(leader.id()).unwrap();
        // the pipe closes only when `sleep`, which holds it too, is gone
        let mut rest = Vec::new();
        stdout.read_to_end(&mut rest).unwrap();
        assert_eq!(leader.wait().unwrap().signal(), Some(libc::SIGKILL));
    }
}
