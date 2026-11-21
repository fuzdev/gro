//! Raw system calls the safe std API lacks, kept in one crate so every gro
//! crate but this one and `gro_v8`, its twin for the V8 calls `rusty_v8`
//! exposes only as `unsafe`, stays free of `unsafe`: they forbid it, and
//! this one denies it and allows it per function, each `unsafe` block
//! beside the argument for why it's sound.
//!
//! Linux-only: each call is Linux's, and built for another OS the crate
//! fails to compile.
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
//!   raising it; `default_thread_stack` reads it.
//! - [`stack_start`] reads the calling thread's stack start as V8 takes it,
//!   the address V8's limit on JS's stack is measured from.
//! - [`thread_count`] reads the process's thread count, which a box or a
//!   process-wide change installed from one thread needs to be 1.
//! - [`settled_thread_count`] reads it until it's 1, a while, so a thread
//!   just joined, still counted, doesn't count.
//! - [`wait_child`] reaps a child process by `wait4`, returning its exit
//!   status with its peak resident set size ([`Reaped`]), which std's `wait`
//!   drops.
//! - [`kill_process_group`] sends `SIGKILL` to a process group by its id,
//!   for a watchdog that doesn't own the child another thread is reaping,
//!   and [`signal_process_group`] any signal.
//! - [`GroupGuard`], [`TermForward`], and [`guard_children_on_signal`] kill
//!   the process groups a process spawned when a terminal's interrupt, a
//!   hangup, or a `SIGTERM` ends it, since a child in a group of its own
//!   doesn't get the terminal's signals, and pass a `SIGTERM` on to the
//!   children sharing its group, which a supervisor's `SIGTERM` of it alone
//!   wouldn't reach.
//! - [`cloexec_inherited_fds`] marks every descriptor above stderr
//!   close-on-exec, so a child is handed none the process inherited.
//! - [`raise_open_files_limit`] raises the soft limit on the process's open
//!   descriptors to its hard one, as Node does as it starts.
//! - [`exit_now`] ends the process by `_exit`, running none of its teardown.
//! - [`write_unbuffered`] writes to a descriptor by one `write`, where std's
//!   `Stdout` buffers what a write leaves and writes it ahead of the next.
//! - [`may_execute`] asks whether the process may execute a file, by
//!   `access`, as `execve` would judge it.
//! - [`beneath`] makes the filesystem calls confined beneath a directory
//!   descriptor, each path resolved by one `openat2` with `RESOLVE_BENEATH`:
//!   opening, `stat`, listing a directory, and making, removing, and
//!   renaming an entry.
//! - [`namespace`] enters a private user and network namespace, one
//!   `unshare` and the maps written after it, its refusals told apart from
//!   a failure after it, and brings its loopback interface up.
//! - [`capabilities`] drops every capability the process holds, and reads
//!   the sets back.
//! - [`set_not_dumpable`] marks the process not dumpable, so no other
//!   process of its user reads its memory or its environment through
//!   `/proc`; [`is_dumpable`] reads it.
//! - [`lower_hard_limit`] lowers a resource's hard limit, the soft one with
//!   it where it's higher, never raising either; [`resource_limit`] reads
//!   a resource's limits.
//! - [`private_dir`] makes a directory private to the process beneath a
//!   directory's descriptor, refusing any entry already at its name.
//!
//! With the `child` feature, which `gro_runtime` enables, so a crate without
//! it builds no tokio, it also has `child`, which holds a child process by
//! its pidfd: opening it, signaling through it, reading how the child ended
//! with or without reaping it, and waiting for its exit on a tokio
//! runtime's reactor; and sets `SIGCHLD` back to its default, so the kernel
//! never reaps a child itself.
//!
//! With the `testing` feature, which only tests enable (`gro_cli`'s, from its
//! dev-dependencies), it also has:
//!
//! - `signal_thread`, which sends a signal to one thread by `tgkill`, the call
//!   the box allows and Landlock's signal scope keeps inside it.
//! - `map_anonymous` and `protect_anonymous`, which map an anonymous page
//!   with a protection, or map one and change it to that protection, and
//!   unmap it, so a box's tests reach its rule on executable mappings.

#[cfg(not(target_os = "linux"))]
compile_error!("gro_sys is Linux-only: its system calls are Linux's");

use std::ffi::OsString;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

pub mod beneath;
pub mod capabilities;
#[cfg(feature = "child")]
pub mod child;
pub mod namespace;
pub mod private_dir;

/// Whether the process may execute the file at `path`, by `access(2)`.
///
/// It asks with `X_OK`, judged by the process's real user and group, as
/// Node's `accessSync(path, X_OK)` judges it, symlinks followed. A file it
/// may run has an execute bit for it, or, for root, any execute bit at all.
///
/// # Errors
///
/// Returns the call's error but for `EACCES`, which is `false`: `ENOENT`
/// for nothing at the path, `ENOTDIR`, `ELOOP`, and the like, and
/// `EINVAL` for a path holding a NUL byte.
#[expect(unsafe_code, reason = "`access` takes a raw C string")]
pub fn may_execute(path: &std::path::Path) -> io::Result<bool> {
    use std::os::unix::ffi::OsStrExt as _;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    // SAFETY: `path` is a NUL-terminated string that outlives the call,
    // which only reads it.
    let result = unsafe { libc::access(path.as_ptr(), libc::X_OK) };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EACCES) {
        Ok(false)
    } else {
        Err(error)
    }
}

/// The lowest descriptor [`close_inherited_fds`] closes, the one above
/// stderr.
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

/// Marks every file descriptor above stderr (2) the process holds
/// close-on-exec, so a program it starts is handed only stdin, stdout, and
/// stderr.
///
/// At the start of `main`, those are the ones it inherited from its
/// parent without close-on-exec, which would pass on to every child; what
/// the process opens itself is close-on-exec already, as std opens
/// everything. Unlike [`close_inherited_fds`], it closes nothing, so it
/// has no precondition: a descriptor a value owns stays open under it, and
/// only an exec drops it.
///
/// It marks them with one `close_range` call (`CLOSE_RANGE_CLOEXEC`); on a
/// kernel without it (before 5.9, `ENOSYS`), without the flag (before 5.11,
/// `EINVAL`), or under a seccomp filter that refuses it (`EPERM`), with
/// `/proc/self/fd` listed and each listed descriptor marked in turn.
///
/// # Errors
///
/// Returns an error when `close_range` fails otherwise; on the fallback,
/// when `/proc/self/fd` can't be listed or names something other than a
/// descriptor, or when marking one fails other than for the listing's own
/// handle, closed by then (`EBADF`), or one another thread closed between.
pub fn cloexec_inherited_fds() -> io::Result<()> {
    match cloexec_range_above_stderr() {
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOSYS | libc::EINVAL | libc::EPERM)
            ) =>
        {
            cloexec_listed()
        }
        result => result.map_err(|error| context("close_range failed", &error)),
    }
}

/// Marks the descriptors from [`FIRST_INHERITED`] up close-on-exec with one
/// `close_range` call.
#[expect(
    unsafe_code,
    reason = "std has no `close_range`, and a raw system call is `unsafe`"
)]
fn cloexec_range_above_stderr() -> io::Result<()> {
    // SAFETY: `close_range` takes three integers and reads or writes none of
    // this process's memory, and with `CLOSE_RANGE_CLOEXEC` it closes
    // nothing, so no value's descriptor changes under it
    let result = unsafe {
        libc::syscall(
            libc::SYS_close_range,
            FIRST_INHERITED,
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Lists `/proc/self/fd`, then marks each descriptor listed above stderr
/// close-on-exec: the fallback where the kernel's `close_range` can't. A
/// descriptor closed since the listing (its own handle among them) is
/// passed over.
fn cloexec_listed() -> io::Result<()> {
    let names = std::fs::read_dir("/proc/self/fd")
        .and_then(|listing| {
            listing
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect::<io::Result<Vec<_>>>()
        })
        .map_err(|error| context("failed to list /proc/self/fd", &error))?;
    for fd in above_stderr(names)? {
        match set_cloexec(fd) {
            Err(error) if error.raw_os_error() != Some(libc::EBADF) => {
                return Err(context(
                    &format!("failed to mark descriptor {fd} close-on-exec"),
                    &error,
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Sets `FD_CLOEXEC` on the descriptor `fd`, keeping its other flags.
#[expect(
    unsafe_code,
    reason = "`fcntl` on a descriptor by its number is a libc call, which is `unsafe`"
)]
fn set_cloexec(fd: libc::c_int) -> io::Result<()> {
    // SAFETY: `fcntl` with `F_GETFD` takes integers and reads or writes none
    // of this process's memory
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above, `F_SETFD` changes only the descriptor's
    // close-on-exec flag, never what it refers to
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
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

/// A resource whose use the kernel limits per process (`RLIMIT_*`): the
/// ones the process reads or lowers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
    /// The largest core dump written, in bytes (`RLIMIT_CORE`).
    Core,
    /// The data segment, in bytes (`RLIMIT_DATA`), which [`limit_data`]
    /// caps.
    Data,
}

/// The type `getrlimit` and `setrlimit` take a resource as, by C library.
#[cfg(target_env = "gnu")]
type RawResource = libc::__rlimit_resource_t;
/// The type `getrlimit` and `setrlimit` take a resource as, by C library.
#[cfg(not(target_env = "gnu"))]
type RawResource = libc::c_int;

impl Resource {
    /// The kernel's number for it.
    const fn raw(self) -> RawResource {
        match self {
            Self::Core => libc::RLIMIT_CORE,
            Self::Data => libc::RLIMIT_DATA,
        }
    }
}

/// A resource's soft and hard limits, `u64::MAX` for unlimited
/// (`RLIM_INFINITY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceLimit {
    /// The limit the kernel holds the process to, which the process may
    /// raise as far as the hard one.
    pub soft: u64,
    /// The ceiling of the soft limit, which only a process holding
    /// `CAP_SYS_RESOURCE` in the initial user namespace may raise.
    pub hard: u64,
}

/// Lowers the process's hard limit on `resource` to `hard`, and the soft
/// limit with it where it's higher, and returns the limits set.
///
/// It never raises either: a hard limit already lower stands, as does a
/// soft one. Once lowered, nothing in the process can raise the hard limit
/// again without `CAP_SYS_RESOURCE` in the initial user namespace (holding
/// every capability in a user namespace of its own doesn't do), and every
/// process it starts after inherits it. It sets the process's own limits (`setrlimit`), never
/// another's.
///
/// It touches no memory of the process's and holds no lock, so it's safe on
/// any thread; it governs every thread.
///
/// # Errors
///
/// Returns the error of `getrlimit` or `setrlimit`, the limits unchanged.
#[expect(
    unsafe_code,
    reason = "std has no `setrlimit`, and a libc call is `unsafe`"
)]
pub fn lower_hard_limit(resource: Resource, hard: u64) -> io::Result<ResourceLimit> {
    let current = resource_limit(resource)?;
    let hard = hard.min(current.hard);
    let lowered = ResourceLimit {
        soft: current.soft.min(hard),
        hard,
    };
    if lowered == current {
        return Ok(current);
    }
    let limit = libc::rlimit {
        rlim_cur: lowered.soft,
        rlim_max: lowered.hard,
    };
    // SAFETY: `setrlimit` reads only the pointer, to a live local of the
    // type it expects, and changes no memory of the process's
    if unsafe { libc::setrlimit(resource.raw(), &raw const limit) } != 0 {
        return Err(context(
            &format!("setrlimit({resource:?}) failed"),
            &io::Error::last_os_error(),
        ));
    }
    Ok(lowered)
}

/// The process's soft and hard limits on `resource`, from `getrlimit`.
///
/// # Errors
///
/// Returns the call's error.
#[expect(
    unsafe_code,
    reason = "std has no `getrlimit`, and a libc call is `unsafe`"
)]
pub fn resource_limit(resource: Resource) -> io::Result<ResourceLimit> {
    let mut current = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes only to the pointer, to a live local of the
    // type it expects
    if unsafe { libc::getrlimit(resource.raw(), &raw mut current) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(ResourceLimit {
        soft: current.rlim_cur,
        hard: current.rlim_max,
    })
}

/// Raises the soft limit on the process's open descriptors,
/// `RLIMIT_NOFILE`, to its hard limit, and returns the soft limit then; a
/// soft limit already at the hard one is left as it is.
///
/// Node raises it so as it starts, so a program that holds many children,
/// each with its pipes, or many files open, isn't refused at a soft limit
/// a shell set low (often 1024). A child started after inherits the raised
/// limit, as Node's does. Raising a soft limit to the hard one takes no
/// privilege, and Linux's hard limit is never unlimited for this resource.
///
/// It touches no memory of the process's and holds no lock, so it's safe on
/// any thread.
///
/// # Errors
///
/// Returns the error of `getrlimit` or `setrlimit`, the limit unchanged.
#[expect(
    unsafe_code,
    reason = "std has no `getrlimit` or `setrlimit`, and a libc call is `unsafe`"
)]
pub fn raise_open_files_limit() -> io::Result<u64> {
    let mut current = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` writes only to the pointer, to a live local of the
    // type it expects
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut current) } != 0 {
        return Err(context(
            "getrlimit(RLIMIT_NOFILE) failed",
            &io::Error::last_os_error(),
        ));
    }
    if current.rlim_cur >= current.rlim_max {
        return Ok(current.rlim_cur);
    }
    let raised = libc::rlimit {
        rlim_cur: current.rlim_max,
        rlim_max: current.rlim_max,
    };
    // SAFETY: `setrlimit` reads only the pointer, to a live local of the
    // type it expects, and changes no memory of the process's
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const raised) } != 0 {
        return Err(context(
            "setrlimit(RLIMIT_NOFILE) failed",
            &io::Error::last_os_error(),
        ));
    }
    Ok(raised.rlim_cur)
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
#[cfg(target_env = "gnu")]
#[expect(
    unsafe_code,
    reason = "std has no default thread attribute, and a libc call is `unsafe`"
)]
pub fn set_default_thread_stack(bytes: usize) -> io::Result<()> {
    let current = default_thread_stack()
        .map_err(|error| context("failed to read the default thread stack", &error))?;
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
#[cfg(not(target_env = "gnu"))]
pub const fn set_default_thread_stack(_bytes: usize) -> io::Result<()> {
    Ok(())
}

/// The stack size, in bytes, of a thread started without one, from
/// `pthread_getattr_default_np`.
///
/// # Errors
///
/// Returns the error of a call.
#[cfg(target_env = "gnu")]
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
#[cfg(target_env = "gnu")]
#[expect(
    unsafe_code,
    reason = "the `libc` crate lacks these two, so they're declared here, as glibc's headers do"
)]
unsafe extern "C" {
    fn pthread_setattr_default_np(attr: *const libc::pthread_attr_t) -> libc::c_int;
    fn pthread_getattr_default_np(attr: *mut libc::pthread_attr_t) -> libc::c_int;
}

/// The calling thread's stack start, its highest address, as V8 takes it
/// (`base::Stack::GetStackStart` on glibc).
///
/// That's the top of the stack `pthread_getattr_np` reports, or glibc's
/// `__libc_stack_end` when that lies inside it, as it does on the main
/// thread, or when the call fails, as it can on the main thread. V8 holds JS
/// to a limit below this address, so a caller on the same thread finds how
/// much of the limit's stack is left from it.
///
/// Every call asks again: it reads the thread's attributes, a system call
/// among them, so a caller that needs it more than once keeps it.
#[cfg(target_env = "gnu")]
#[expect(
    unsafe_code,
    reason = "std has no thread attributes, and a libc call and a C static are `unsafe`"
)]
pub fn stack_start() -> usize {
    // SAFETY: glibc's dynamic loader sets `__libc_stack_end` before any code
    // of the process runs and never writes it again, so the read races
    // nothing; only the address it holds is used, never what's there
    let libc_stack_end = unsafe { __libc_stack_end }.addr();
    let mut attr = std::mem::MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: `pthread_getattr_np` initializes the attribute the pointer
    // names, a live local of the type it expects, from the thread
    // `pthread_self` names, the calling one, alive for the call
    let code = unsafe { libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr()) };
    if code != 0 {
        // the attribute is uninitialized, so there's nothing to destroy
        return libc_stack_end;
    }
    let mut base: *mut libc::c_void = std::ptr::null_mut();
    let mut size: libc::size_t = 0;
    // SAFETY: the attribute was initialized above; the call writes only to
    // `base` and `size`, live locals of the types it expects
    let code = unsafe { libc::pthread_attr_getstack(attr.as_ptr(), &raw mut base, &raw mut size) };
    // SAFETY: the attribute was initialized above and is destroyed once
    unsafe { libc::pthread_attr_destroy(attr.as_mut_ptr()) };
    if code != 0 {
        // V8 aborts here, on a stack glibc reported without its bounds
        return libc_stack_end;
    }
    let base = base.addr();
    let start = base + size;
    if (base..=start).contains(&libc_stack_end) {
        libc_stack_end
    } else {
        start
    }
}

// glibc's dynamic loader's; the `libc` crate doesn't declare it
#[cfg(target_env = "gnu")]
#[expect(
    unsafe_code,
    reason = "the `libc` crate lacks it, so it's declared here, as glibc's own code does"
)]
unsafe extern "C" {
    static __libc_stack_end: *mut libc::c_void;
}

/// The process's thread count, its `Threads:` line in `/proc/self/status`,
/// read once.
///
/// A thread whose join has returned can stay counted for a moment longer,
/// while the kernel finishes its exit, so a caller that needs the count to
/// reach 1 after a join reads it again ([`settled_thread_count`]).
///
/// # Errors
///
/// Returns an error when the file can't be read or has no thread count.
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

/// How many times [`settled_thread_count`] reads the thread count before
/// it returns one above 1.
///
/// A thread whose join has returned can stay counted for a moment longer,
/// while the kernel finishes its exit (the joiner wakes when the thread's
/// id is cleared, before the thread is removed from its group).
pub const THREAD_COUNT_READS: usize = 1000;

/// The process's thread count once it has settled: read until it's 1,
/// yielding between reads, at most [`THREAD_COUNT_READS`] times, then the
/// last count read.
///
/// For a caller that needs the process to have one thread, its own, just
/// after joining another: the joined thread, still counted for a moment,
/// isn't another. A count above 1 it returns is most likely a thread
/// that's running.
///
/// # Errors
///
/// As [`thread_count`]'s.
pub fn settled_thread_count() -> io::Result<u64> {
    let mut count = thread_count()?;
    for _ in 1..THREAD_COUNT_READS {
        if count == 1 {
            break;
        }
        std::thread::yield_now();
        count = thread_count()?;
    }
    Ok(count)
}

/// Closes the descriptors from [`FIRST_INHERITED`] up with one `close_range`
/// call.
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

/// The descriptors above stderr among `names`, the names of `/proc/self/fd`'s
/// entries, each a descriptor's number.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidData`] when a name isn't
/// a descriptor's number.
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

/// Sends `SIGKILL` to every process of the group `pgid`, by `kill` with the
/// id negated ([`signal_process_group`]).
///
/// That is a child spawned as the leader of its own group
/// (`CommandExt::process_group(0)`) and what it spawned, which, holding the
/// leader's pipes, would otherwise keep a reader of them waiting. For a
/// watchdog that ends a child another thread is reaping with [`wait_child`],
/// which owns the child, so std's `Child::kill` can't reach it.
///
/// # Errors
///
/// As [`signal_process_group`]'s.
pub fn kill_process_group(pgid: u32) -> io::Result<()> {
    signal_process_group(pgid, libc::SIGKILL)
}

/// Sends `signal` to every process of the group `pgid`, by `kill` with the
/// id negated.
///
/// The id names the group only while its leader isn't reaped: a caller
/// signals only a group it knows isn't reaped yet, as far as it can know,
/// since the id is free for reuse once it is. A leader held by its pidfd
/// and read without reaping (`child::Pidfd::ended`) is known.
///
/// # Errors
///
/// Returns the call's error: `ESRCH` when no process is in the group,
/// `EPERM` when none may be signaled, or `EINVAL` for an invalid signal, or
/// an id past `pid_t` or not above 1.
pub fn signal_process_group(pgid: u32, signal: libc::c_int) -> io::Result<()> {
    // group 1 would be `kill(-1)`, every process the caller may signal
    if pgid <= 1 {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    send(-positive_pid(pgid)?, signal)
}

/// The most process groups [`GroupGuard`]s guard at once.
pub const GUARDED_GROUPS: usize = 1024;

/// The most children [`TermForward`]s name at once.
pub const FORWARDED_CHILDREN: usize = 1024;

/// The signals [`guard_children_on_signal`] handles: the ones that end a
/// process from a terminal (an interrupt, a hangup) or from a supervisor.
const GUARDING_SIGNALS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

/// What a slot reserved and not armed yet holds, which the handler waits
/// on and never signals: neither it nor a free slot's 0 is above 1.
const RESERVED: libc::pid_t = -1;

/// How long the handler waits for the slots reserved as it began to be
/// armed or freed, in nanoseconds: a launch arms its slot within
/// microseconds of its child's start, so the bound only ends a wait for a
/// thread that never resumes.
const ARMING_WAIT_NS: u64 = 1_000_000_000;

/// How long the handler sleeps between its looks at the slots reserved,
/// in nanoseconds.
const ARMING_POLL_NS: libc::c_long = 100_000;

/// The groups guarded, a slot each, 0 when free; lock-free, so the signal
/// handler reads them.
static GUARDED: [AtomicI32; GUARDED_GROUPS] = [const { AtomicI32::new(0) }; GUARDED_GROUPS];

/// The children a `SIGTERM` passes on to, a slot each, 0 when free;
/// lock-free, so the signal handler reads them.
static FORWARDED: [AtomicI32; FORWARDED_CHILDREN] =
    [const { AtomicI32::new(0) }; FORWARDED_CHILDREN];

/// Set by the handler as it begins, before it reads a slot, and never
/// cleared, since the handler ends the process: from then no slot is
/// reserved, and an id a slot frees isn't safe to free.
static ENDING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// How many slots this thread holds reserved and not armed: the
    /// handler, run on a thread that holds one, can't wait for it.
    static RESERVING: AtomicU32 = const { AtomicU32::new(0) };
}

/// Why no slot of the handler's tables was reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreserved {
    /// Every slot is taken.
    Full,
    /// A signal the handler guards against is ending the process.
    Ending,
}

/// A slot of one of the tables the handler reads, freed when dropped.
#[derive(Debug)]
struct Slot {
    /// The table.
    table: &'static [AtomicI32],
    /// Its place in the table.
    at: usize,
}

impl Slot {
    /// Stores `id`, checked above 1.
    fn store(&self, id: libc::pid_t) {
        debug_assert!(id > 1, "{id}");
        self.table[self.at].store(id, Ordering::SeqCst);
    }

    /// Frees the slot, returning whether the id it held is safe to free:
    /// `false` once the handler has begun, since it may have read the id
    /// and not signaled it yet, so the caller keeps the process holding
    /// the id unreaped until the process ends.
    fn release(self) -> bool {
        drop(self);
        // the handler sets `ENDING` before it reads a slot: either it reads
        // this one freed, or this reads the flag set
        !ENDING.load(Ordering::SeqCst)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.table[self.at].store(0, Ordering::SeqCst);
    }
}

/// A slot reserved, on the thread that reserved it and no other, and not
/// armed yet, counted in that thread's [`RESERVING`] while it lives; its
/// slot is freed, then the count lowered, if it's dropped unarmed.
#[derive(Debug)]
struct Reservation {
    /// The slot, holding [`RESERVED`]; dropped before `counted`.
    slot: Slot,
    /// Its place in the thread's count.
    counted: Counted,
}

/// One slot reserved in the calling thread's [`RESERVING`], until dropped;
/// neither `Send` nor `Sync`, so it's dropped on the thread it counts in.
#[derive(Debug)]
struct Counted(std::marker::PhantomData<*const ()>);

impl Counted {
    /// Raises the calling thread's count.
    fn new() -> Self {
        RESERVING.with(|count| count.fetch_add(1, Ordering::SeqCst));
        Self(std::marker::PhantomData)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        RESERVING.with(|count| count.fetch_sub(1, Ordering::SeqCst));
    }
}

impl Reservation {
    /// The first free slot of `table`, reserved by a compare-and-swap, no
    /// lock or system call.
    ///
    /// The thread's count is raised before the slot is taken, and the
    /// flag read after: either the handler reads the slot reserved, and
    /// waits for its arming, or this reads the flag set and backs out, so
    /// no child is started once the handler has begun.
    fn new(table: &'static [AtomicI32]) -> Result<Self, Unreserved> {
        let counted = Counted::new();
        let at = table
            .iter()
            .position(|slot| {
                slot.compare_exchange(0, RESERVED, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            })
            .ok_or(Unreserved::Full)?;
        let reservation = Self {
            slot: Slot { table, at },
            counted,
        };
        if ENDING.load(Ordering::SeqCst) {
            return Err(Unreserved::Ending);
        }
        Ok(reservation)
    }

    /// The slot, holding `id`, a process's or a group's, or `EINVAL` for
    /// an id past `pid_t` or not above 1, the slot freed.
    fn arm(self, id: u32) -> io::Result<Slot> {
        let Self { slot, counted } = self;
        let Some(id) = positive_pid(id).ok().filter(|id| *id > 1) else {
            // freed before the count falls, as a reservation's drop is
            drop(slot);
            drop(counted);
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        };
        slot.store(id);
        // armed before the count falls, so a handler on this thread never
        // reads the count fallen and the slot still reserved
        drop(counted);
        Ok(slot)
    }
}

/// A process group [`guard_children_on_signal`]'s handler kills, until
/// released.
///
/// The group is a child spawned as the leader of its own
/// (`CommandExt::process_group(0)`), and the kill happens once
/// [`guard_children_on_signal`] is installed. A child in a group of its own
/// isn't in the terminal's foreground group, so the terminal's interrupt
/// never reaches it, and it would outlive its parent.
///
/// A guard is reserved before the child is spawned ([`GroupGuard::reserve`]),
/// so a process that guards as many groups as it may refuses a spawn rather
/// than run a child unguarded, and armed with the group's id once it's
/// spawned ([`GroupReservation::arm`]). A signal that arrives between the two
/// waits for the arming, so the group is killed, and once the handler has
/// begun no reservation is made. Reserving and arming are each a
/// compare-and-swap or a store, no lock or system call, cheap enough for a
/// timed span.
///
/// The caller releases the guard before it reaps the child
/// ([`GroupGuard::release`]), since the id is free for reuse after, and
/// reaps it only when the release says it may.
#[derive(Debug)]
#[must_use = "the group is guarded only until the guard is released"]
pub struct GroupGuard(Slot);

/// A [`GroupGuard`] reserved and not armed yet, guarding nothing: armed
/// ([`GroupReservation::arm`]) or dropped on the thread that reserved it.
#[derive(Debug)]
#[must_use = "a reservation guards nothing until it's armed"]
pub struct GroupReservation(Reservation);

impl GroupGuard {
    /// A guard reserved, guarding nothing until it's armed.
    ///
    /// # Errors
    ///
    /// Returns [`Unreserved::Full`] when [`GUARDED_GROUPS`] are reserved
    /// already, and [`Unreserved::Ending`] once the handler has begun.
    pub fn reserve() -> Result<GroupReservation, Unreserved> {
        Reservation::new(&GUARDED).map(GroupReservation)
    }

    /// Stops guarding the group, returning whether its id may be freed
    /// now, its leader reaped: `false` once the handler has begun, which
    /// may have read the id and not killed the group yet, so the caller
    /// leaves the leader unreaped, holding the id, for the process's end.
    pub fn release(self) -> bool {
        self.0.release()
    }
}

impl GroupReservation {
    /// The guard of the group `pgid`.
    ///
    /// # Errors
    ///
    /// Returns `EINVAL`, the reservation freed, for an id past `pid_t` or
    /// not above 1.
    pub fn arm(self, pgid: u32) -> io::Result<GroupGuard> {
        self.0.arm(pgid).map(GroupGuard)
    }
}

/// Guards the process group `pgid` ([`GroupGuard`]), reserving and arming
/// at once, for a caller that spawned the group already.
///
/// Returns `None`, guarding nothing, when no guard can be reserved, or for
/// an id past `pid_t` or not above 1.
pub fn guard_group(pgid: u32) -> Option<GroupGuard> {
    GroupGuard::reserve().ok()?.arm(pgid).ok()
}

/// A child process [`guard_children_on_signal`]'s handler passes a
/// `SIGTERM` on to, until released.
///
/// That is a child sharing this process's group,
/// which a terminal's signals reach, but a supervisor's `SIGTERM` of this
/// process alone doesn't. Reserved and armed as a [`GroupGuard`] is, and
/// released before the child is reaped, which only a release that says it
/// may allows, since its pid is free for reuse after.
#[derive(Debug)]
#[must_use = "the child is named only until the slot is released"]
pub struct TermForward(Slot);

/// A [`TermForward`] reserved and not armed yet, naming nothing: armed
/// ([`ForwardReservation::arm`]) or dropped on the thread that reserved it.
#[derive(Debug)]
#[must_use = "a reservation names nothing until it's armed"]
pub struct ForwardReservation(Reservation);

impl TermForward {
    /// A slot reserved, naming nothing until it's armed.
    ///
    /// # Errors
    ///
    /// Returns [`Unreserved::Full`] when [`FORWARDED_CHILDREN`] are
    /// reserved already, and [`Unreserved::Ending`] once the handler has
    /// begun.
    pub fn reserve() -> Result<ForwardReservation, Unreserved> {
        Reservation::new(&FORWARDED).map(ForwardReservation)
    }

    /// Stops naming the child, returning whether its pid may be freed now,
    /// as [`GroupGuard::release`] does for a group.
    pub fn release(self) -> bool {
        self.0.release()
    }
}

impl ForwardReservation {
    /// The slot naming the child `pid`.
    ///
    /// # Errors
    ///
    /// Returns `EINVAL`, the reservation freed, for an id past `pid_t` or
    /// not above 1.
    pub fn arm(self, pid: u32) -> io::Result<TermForward> {
        self.0.arm(pid).map(TermForward)
    }
}

/// Installs a handler for `SIGINT`, `SIGTERM`, and `SIGHUP` that kills
/// every group a [`GroupGuard`] guards and, for a `SIGTERM`, sends it on
/// to every child a [`TermForward`] names.
///
/// Then the handler ends the process by the signal's default action, so
/// its parent sees it killed by that signal.
///
/// As it begins, the handler sets a flag no reservation is made past, then
/// waits, bounded, for every slot reserved before to be armed or freed, so
/// a child started between its reservation and its arming is killed too;
/// it sends each group `SIGKILL` and each named child `SIGTERM`, sets the
/// signal's default action back, and raises the signal again, which the
/// kernel delivers once the handler returns. Each of the three signals is
/// blocked while the handler runs, so a second one, of the same number or
/// another, waits for it rather than end the process mid-scan; one a
/// thread of the process handles meanwhile does all of it again.
///
/// The handler run on a thread that holds a slot reserved and not armed,
/// which can't arm it while the handler waits, sends the signal on to the
/// process's main thread (`tgkill`) and returns, so the thread arms its
/// slot and the main thread's handler waits for it. That needs the main
/// thread live, not blocking the three signals, and not itself holding a
/// reservation: where it's the thread that holds one, its handler doesn't
/// wait, and the child that slot is for, if started, isn't killed.
///
/// The handler is async-signal-safe: it reads the ids from fixed arrays of
/// atomics, and its calls are `kill`, `tgkill`, `nanosleep`,
/// `clock_gettime`, `sigaction`, and `raise`. A signal the process ignores
/// (`SIG_IGN`, as `nohup` leaves `SIGHUP`) stays ignored, and installing
/// again is harmless. A `SIGKILL` of this process still orphans the groups.
///
/// # Errors
///
/// Returns `sigaction`'s error, which a valid signal and action never get.
#[expect(
    unsafe_code,
    reason = "std installs no signal handler, and a libc call is `unsafe`"
)]
pub fn guard_children_on_signal() -> io::Result<()> {
    let handler: extern "C" fn(libc::c_int) = on_guarding_signal;
    for signal in GUARDING_SIGNALS {
        // SAFETY: an all-zero `sigaction` is a valid value of the struct of
        // integers and a signal set
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: a null action only reads the current one, into a live local
        // of the type `sigaction` expects
        if unsafe { libc::sigaction(signal, std::ptr::null(), &raw mut current) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if current.sa_sigaction == libc::SIG_IGN {
            continue;
        }
        // SAFETY: as above
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = handler as libc::sighandler_t;
        // no `SA_RESETHAND`: a second signal during the scan waits for it,
        // and the handler sets the default back itself before it raises
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: `sigemptyset` and `sigaddset` write only the live local's
        // signal set, each signal valid
        unsafe {
            libc::sigemptyset(&raw mut action.sa_mask);
            for blocked in GUARDING_SIGNALS {
                libc::sigaddset(&raw mut action.sa_mask, blocked);
            }
        }
        // SAFETY: the action points at `on_guarding_signal`, an
        // `extern "C" fn(c_int)` that lives as long as the process and is
        // async-signal-safe; the old action isn't asked for
        if unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// [`guard_children_on_signal`]'s handler: sets [`ENDING`], hands the
/// signal on to the main thread from a thread mid-launch, else waits for
/// the slots reserved, kills each guarded group, sends a `SIGTERM` on to
/// each named child, sets the signal's default back, and raises it again,
/// which ends the process when the handler returns.
#[expect(
    unsafe_code,
    reason = "a signal handler calls libc's async-signal-safe calls, which are `unsafe`"
)]
extern "C" fn on_guarding_signal(signal: libc::c_int) {
    ENDING.store(true, Ordering::SeqCst);
    // a const-initialized thread local with no destructor, read without
    // any lazy step, so reading it here is safe
    let reserving = RESERVING
        .try_with(|count| count.load(Ordering::SeqCst) > 0)
        .unwrap_or(false);
    if reserving {
        if hand_off_to_main_thread(signal) {
            return;
        }
    } else {
        await_arming();
    }
    for slot in &GUARDED {
        let pgid = slot.load(Ordering::SeqCst);
        // a slot holds 0, `RESERVED`, or a group above 1, never a caller's
        // group or every process
        if pgid > 1 {
            // SAFETY: `kill` is async-signal-safe and takes two integers,
            // touching none of this process's memory
            unsafe { libc::kill(-pgid, libc::SIGKILL) };
        }
    }
    if signal == libc::SIGTERM {
        for slot in &FORWARDED {
            let pid = slot.load(Ordering::SeqCst);
            // as above: a process above 1, never a group or every process
            if pid > 1 {
                // SAFETY: as above
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        }
    }
    // SAFETY: an all-zero `sigaction` is a valid value of the struct of
    // integers and a signal set, and its handler is `SIG_DFL`, 0
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    // SAFETY: `sigaction` is async-signal-safe and reads only the live
    // local; the old action isn't asked for
    unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) };
    // SAFETY: `raise` is async-signal-safe and takes an integer; the signal
    // stays blocked until the handler returns, then ends the process
    unsafe { libc::raise(signal) };
}

/// Sends `signal` to the process's main thread, by `tgkill`, for the
/// handler on another thread mid-launch; `false` when this is the main
/// thread, or the call failed, so the handler goes on here.
#[expect(
    unsafe_code,
    reason = "std has no `gettid` or `tgkill`, and libc's calls are `unsafe`"
)]
fn hand_off_to_main_thread(signal: libc::c_int) -> bool {
    // SAFETY: `getpid` and `gettid` are async-signal-safe and take nothing
    let (pid, tid) = unsafe { (libc::getpid(), libc::syscall(libc::SYS_gettid)) };
    if tid == libc::c_long::from(pid) {
        return false;
    }
    // SAFETY: `tgkill` is async-signal-safe and takes three integers,
    // touching none of this process's memory; the main thread's handler
    // then runs, this one's signal blocked there until it returns
    unsafe { libc::syscall(libc::SYS_tgkill, pid, pid, signal) == 0 }
}

/// Waits, at most [`ARMING_WAIT_NS`], for no slot of either table to be
/// reserved and not armed: each such one is a launch on another thread
/// between its reservation and its arming, which goes on while the handler
/// waits, and no slot is reserved from now.
#[expect(unsafe_code, reason = "`nanosleep` is a libc call, which is `unsafe`")]
fn await_arming() {
    let Some(deadline) = monotonic_ns().map(|now| now.saturating_add(ARMING_WAIT_NS)) else {
        return;
    };
    let reserved = || {
        GUARDED
            .iter()
            .chain(&FORWARDED)
            .any(|slot| slot.load(Ordering::SeqCst) == RESERVED)
    };
    while reserved() && monotonic_ns().is_some_and(|now| now < deadline) {
        let pause = libc::timespec {
            tv_sec: 0,
            tv_nsec: ARMING_POLL_NS,
        };
        // SAFETY: `nanosleep` is async-signal-safe and reads only the live
        // local; the time left isn't asked for
        unsafe { libc::nanosleep(&raw const pause, std::ptr::null_mut()) };
    }
}

/// `CLOCK_MONOTONIC`'s time in nanoseconds, `None` should the call fail;
/// async-signal-safe.
#[expect(
    unsafe_code,
    reason = "std's clock isn't async-signal-safe, and a libc call is `unsafe`"
)]
fn monotonic_ns() -> Option<u64> {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` is async-signal-safe and writes only the live
    // local, of the type it expects
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut now) } != 0 {
        return None;
    }
    let seconds = u64::try_from(now.tv_sec).ok()?;
    let nanos = u64::try_from(now.tv_nsec).ok()?;
    Some(seconds.saturating_mul(1_000_000_000).saturating_add(nanos))
}

/// `id` as a `pid_t` above 0, or `EINVAL`.
pub(crate) fn positive_pid(id: u32) -> io::Result<libc::pid_t> {
    libc::pid_t::try_from(id)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))
}

/// `kill(target, signal)`, for a `target` that is a process (above 0) or a
/// group (below -1), never 0 or -1, which name the caller's group or every
/// process.
#[expect(
    unsafe_code,
    reason = "std kills only a child it owns, and a libc call is `unsafe`"
)]
fn send(target: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    debug_assert!(!matches!(target, 0 | -1), "{target}");
    // SAFETY: `kill` takes two integers and reads or writes none of this
    // process's memory; the callers pass one process or one group, never 0
    // or -1
    let result = unsafe { libc::kill(target, signal) };
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
#[cfg(feature = "testing")]
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
#[cfg(feature = "testing")]
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
#[cfg(feature = "testing")]
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
#[cfg(feature = "testing")]
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

/// Marks the process not dumpable (`PR_SET_DUMPABLE` 0).
///
/// Then no process of its user may attach to it or read its memory, its
/// environment, its mappings, or its descriptors through `/proc/<pid>/`,
/// a program it starts inside its box among them, unless it holds
/// `CAP_SYS_PTRACE` over it; and no core is dumped of it. A debugger or
/// `perf` attaching from outside is refused the same way; one that starts
/// the process, attached or counting before the mark, keeps going.
///
/// The mark is the whole process's, every thread's. An `exec` clears it,
/// and a change of the process's ids or a gain of capabilities sets it to
/// `fs.suid_dumpable` (0 by default, not dumpable). Entering a user
/// namespace the process owns isn't a gain, and leaves the mark as it was.
///
/// # Order
///
/// Mark the process after [`namespace::enter_user_net_namespace`], never
/// before: the mark makes the process's own `/proc/self` files root's, so
/// the namespace's maps can't be written and the process is stranded in
/// it ([`namespace::NamespaceError::Stranded`]). Once marked, the process
/// can't read its own `/proc/self/environ`, `auxv`, or `mem` either; its
/// `status`, `stat`, `maps`, and `fd` it still reads.
///
/// # Errors
///
/// Returns the call's error, which a valid argument never gets.
#[expect(unsafe_code, reason = "`prctl` is a libc call, which is `unsafe`")]
pub fn set_not_dumpable() -> io::Result<()> {
    let zero: libc::c_ulong = 0;
    // SAFETY: `PR_SET_DUMPABLE` takes integers and touches no memory of the
    // process's
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, zero, zero, zero, zero) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Whether the process is dumpable (`PR_GET_DUMPABLE`): `false` once
/// [`set_not_dumpable`] marked it.
///
/// # Errors
///
/// Returns the call's error, which a valid argument never gets.
#[expect(unsafe_code, reason = "`prctl` is a libc call, which is `unsafe`")]
pub fn is_dumpable() -> io::Result<bool> {
    let zero: libc::c_ulong = 0;
    // SAFETY: `PR_GET_DUMPABLE` takes integers, touches no memory of the
    // process's, and returns the mark
    let mark = unsafe { libc::prctl(libc::PR_GET_DUMPABLE, zero, zero, zero, zero) };
    if mark < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(mark != 0)
    }
}

/// Ends the process now with `code`, by `_exit`.
///
/// No `atexit` handler, no C++ static destructor, and no stdio flush runs,
/// so a process whose other threads may be mid-operation (V8's workers, out
/// of memory) exits by its code rather than crashing in teardown. Anything
/// the caller wrote must be flushed first.
#[expect(
    unsafe_code,
    reason = "std has no `_exit`, and a libc call is `unsafe`"
)]
pub fn exit_now(code: i32) -> ! {
    // SAFETY: `_exit` takes an integer, reads no memory of the process's,
    // and never returns; skipping the process's teardown is its purpose
    unsafe { libc::_exit(code) }
}

/// Writes as much of `bytes` to `fd` as one `write(2)` takes, returning how
/// many bytes it took, buffering nothing.
///
/// std writes stdout through a line buffer that keeps what a failed or
/// partial write left and writes it ahead of the next write, so bytes a
/// caller was told failed could still land; a write here either took its
/// bytes or reports the call's error, and leaves nothing behind.
///
/// # Errors
///
/// Returns the call's error: `EAGAIN` (`WouldBlock`) on a descriptor
/// something made non-blocking, `EINTR` when a signal interrupted it before
/// it took anything, `EPIPE` when the reader has gone, and the like.
#[expect(
    unsafe_code,
    reason = "std writes a borrowed descriptor only through its buffer or a duplicate"
)]
pub fn write_unbuffered(fd: std::os::fd::BorrowedFd<'_>, bytes: &[u8]) -> io::Result<usize> {
    use std::os::fd::AsRawFd as _;
    // a count past `isize::MAX` is the implementation's to define, and
    // Linux takes less than that in one call anyway
    let length = bytes.len().min(isize::MAX.unsigned_abs());
    // SAFETY: `fd` is open for as long as it's borrowed, and `bytes` is
    // readable for `length` bytes, which `write` only reads
    let written = unsafe { libc::write(fd.as_raw_fd(), bytes.as_ptr().cast(), length) };
    usize::try_from(written).map_err(|_| io::Error::last_os_error())
}

/// `error`, of the same kind, its message prefixed with `what`.
fn context(what: &str, error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    /// Each table's slots are reserved until it's full, then none is,
    /// until one is freed; a slot reserved and not armed holds what the
    /// handler waits on, counted in the thread's reservations until it's
    /// armed or dropped, and an id not above 1 isn't armed, its slot freed.
    /// No other test of this binary reserves a slot, so the counts are the
    /// tables'.
    #[test]
    fn a_full_table_reserves_nothing_until_a_slot_is_freed() {
        use super::{
            FORWARDED, FORWARDED_CHILDREN, GUARDED, GUARDED_GROUPS, GroupGuard, RESERVED,
            RESERVING, TermForward, Unreserved,
        };
        use std::sync::atomic::Ordering;

        let reserving = || RESERVING.with(|count| count.load(Ordering::SeqCst));
        let mut groups: Vec<_> = std::iter::from_fn(|| GroupGuard::reserve().ok()).collect();
        assert_eq!(groups.len(), GUARDED_GROUPS);
        assert_eq!(reserving(), u32::try_from(GUARDED_GROUPS).unwrap());
        assert!(
            GUARDED
                .iter()
                .all(|slot| slot.load(Ordering::SeqCst) == RESERVED)
        );
        assert_eq!(GroupGuard::reserve().unwrap_err(), Unreserved::Full);
        assert!(groups.swap_remove(0).arm(1).is_err());
        assert_eq!(GUARDED[0].load(Ordering::SeqCst), 0);
        let guard = GroupGuard::reserve().unwrap().arm(4242).unwrap();
        assert_eq!(GUARDED[0].load(Ordering::SeqCst), 4242);
        assert_eq!(reserving(), u32::try_from(GUARDED_GROUPS - 1).unwrap());
        assert_eq!(GroupGuard::reserve().unwrap_err(), Unreserved::Full);
        assert!(guard.release());
        assert_eq!(GUARDED[0].load(Ordering::SeqCst), 0);
        drop(groups);
        assert_eq!(reserving(), 0);
        assert!(GUARDED.iter().all(|slot| slot.load(Ordering::SeqCst) == 0));

        let named: Vec<_> = std::iter::from_fn(|| TermForward::reserve().ok()).collect();
        assert_eq!(named.len(), FORWARDED_CHILDREN);
        assert_eq!(TermForward::reserve().unwrap_err(), Unreserved::Full);
        drop(named);
        assert_eq!(reserving(), 0);
        assert!(
            FORWARDED
                .iter()
                .all(|slot| slot.load(Ordering::SeqCst) == 0)
        );
    }

    /// A thread's stack start lies above every frame of the thread and no
    /// further above the first than the thread's stack holds, and a second
    /// read finds the same address.
    #[cfg(target_env = "gnu")]
    #[test]
    fn the_stack_start_is_the_top_of_the_thread_s_stack() {
        const SIZE: usize = 1 << 20;
        let (start, again, here) = std::thread::Builder::new()
            .stack_size(SIZE)
            .spawn(|| {
                let marker = 0u8;
                let here = std::ptr::from_ref(&marker).addr();
                (super::stack_start(), super::stack_start(), here)
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(start, again);
        assert!(here < start, "{here:#x} {start:#x}");
        assert!(start - here < SIZE, "{here:#x} {start:#x}");
    }

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

    /// An id past `pid_t`, or one that would name the caller's group or
    /// every process, is refused, and guards nothing.
    #[test]
    fn a_group_id_not_above_1_is_refused() {
        for pid in [0, 1, u32::MAX] {
            let error = super::kill_process_group(pid).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::EINVAL), "{pid}");
            assert!(super::guard_group(pid).is_none(), "{pid}");
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
