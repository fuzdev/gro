//! A child process held by its pidfd.
//!
//! A pidfd is a descriptor naming the one process, which, unlike its pid,
//! never names another once it's reaped, so a signal sent through it can't
//! reach a process that took the pid over.
//!
//! [`Pidfd::open`] takes a child's pid right after it's spawned, before
//! anything can reap it: the process that spawned it is the only one that
//! reaps it, so the pid names that child until it's reaped, and the
//! process's `SIGCHLD` must not be ignored ([`default_child_signal`]), or
//! the kernel reaps it on its own. [`Pidfd::ended`] reads how it ended
//! without reaping it (`WNOWAIT`), so its pid, and the id of the group it
//! leads, stay its own while it's a zombie, [`Pidfd::wait_ended`] waits
//! for that end without reaping it, and [`Pidfd::reap`] reaps it.
//! [`Pidfd::watch`] registers the descriptor with the current tokio
//! runtime's reactor, which reports it readable once the child has exited
//! ([`PidfdWatch::poll_exited`]).

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd as _, OwnedFd, RawFd};
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

/// How a child ended, as `waitid` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// It exited with this code.
    Exited(i32),
    /// A signal of this number killed it, `dumped` when it dumped core.
    Killed {
        /// The signal's number.
        signal: libc::c_int,
        /// Whether it dumped core.
        dumped: bool,
    },
}

/// A child process's pidfd, which names that process alone.
#[derive(Debug)]
pub struct Pidfd {
    /// The descriptor, private and never replaced, so a watch's
    /// registration holds the one it was made with.
    fd: OwnedFd,
}

impl Pidfd {
    /// The pidfd of `pid`, a child of this process not reaped yet, by
    /// `pidfd_open`, close-on-exec.
    ///
    /// # Errors
    ///
    /// Returns the call's error: `ESRCH` when no process has the pid,
    /// `EMFILE` or `ENFILE` when no descriptor is left, `ENOSYS` before
    /// Linux 5.3, and `EINVAL` for a pid past `pid_t` or not above 0.
    #[expect(
        unsafe_code,
        reason = "std has no `pidfd_open`, and a raw system call is `unsafe`"
    )]
    pub fn open(pid: u32) -> io::Result<Self> {
        let pid = crate::positive_pid(pid)?;
        let flags: libc::c_uint = 0;
        // SAFETY: `pidfd_open` takes two integers and reads or writes none
        // of this process's memory
        let result = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, flags) };
        let fd = RawFd::try_from(result).map_err(|_| io::Error::last_os_error())?;
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call returned a new descriptor, open and owned by
        // nothing else
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { fd })
    }

    /// Sends `signal` to the child by `pidfd_send_signal`, which reaches it
    /// until it's reaped and nothing after.
    ///
    /// # Errors
    ///
    /// Returns the call's error: `ESRCH` once the child has exited, `EINVAL`
    /// for an invalid signal.
    #[expect(
        unsafe_code,
        reason = "std has no `pidfd_send_signal`, and a raw system call is `unsafe`"
    )]
    pub fn send_signal(&self, signal: libc::c_int) -> io::Result<()> {
        let flags: libc::c_uint = 0;
        // SAFETY: the descriptor is this value's, open; a null `siginfo_t`
        // asks the kernel to fill in what `kill` would, so no memory of
        // this process's is read or written
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                flags,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// How the child ended, `None` while it runs, read without reaping it,
    /// so its pid stays its own.
    ///
    /// # Errors
    ///
    /// Returns `waitid`'s error: `ECHILD` when the child was reaped some
    /// other way.
    pub fn ended(&self) -> io::Result<Option<Ended>> {
        self.wait(libc::WEXITED | libc::WNOHANG | libc::WNOWAIT)
    }

    /// Reaps the child, waiting for it to exit, and returns how it ended.
    /// The pid is free for reuse once this returns.
    ///
    /// # Errors
    ///
    /// Returns `waitid`'s error: `ECHILD` when the child was reaped some
    /// other way.
    pub fn reap(&self) -> io::Result<Ended> {
        loop {
            match self.wait(libc::WEXITED) {
                Ok(Some(ended)) => return Ok(ended),
                // a blocking wait returns only once the child has ended
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// Waits for the child to exit and returns how it ended, without
    /// reaping it, so its pid stays its own.
    ///
    /// # Errors
    ///
    /// Returns `waitid`'s error: `ECHILD` when the child was reaped some
    /// other way.
    pub fn wait_ended(&self) -> io::Result<Ended> {
        loop {
            match self.wait(libc::WEXITED | libc::WNOWAIT) {
                Ok(Some(ended)) => return Ok(ended),
                // a blocking wait returns only once the child has ended
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// `waitid` on the descriptor with `options`, `None` for a child that
    /// hasn't ended, as `WNOHANG` reports one.
    #[expect(
        unsafe_code,
        reason = "std has no `waitid`, and a libc call is `unsafe`"
    )]
    fn wait(&self, options: libc::c_int) -> io::Result<Option<Ended>> {
        // SAFETY: an all-zero `siginfo_t` is a valid value of the struct of
        // integers and unions of integers
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let id = libc::id_t::try_from(self.fd.as_raw_fd())
            .map_err(|_| io::Error::from_raw_os_error(libc::EBADF))?;
        // SAFETY: `waitid` writes only the live local `info`, of the type it
        // expects, and waits on the one process the descriptor names
        let result = unsafe { libc::waitid(libc::P_PIDFD, id, &raw mut info, options) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `waitid` filled `info` for a child's state change, or left
        // it zeroed for none with `WNOHANG`, so its pid is 0; either way the
        // fields read are the `SIGCHLD` ones the kernel wrote, or zeros
        let (pid, status) = unsafe { (info.si_pid(), info.si_status()) };
        if pid == 0 {
            return Ok(None);
        }
        match info.si_code {
            libc::CLD_EXITED => Ok(Some(Ended::Exited(status))),
            libc::CLD_KILLED => Ok(Some(Ended::Killed {
                signal: status,
                dumped: false,
            })),
            libc::CLD_DUMPED => Ok(Some(Ended::Killed {
                signal: status,
                dumped: true,
            })),
            code => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("waitid reported a child's state change of code {code}, not an end"),
            )),
        }
    }

    /// The descriptor registered with the current tokio runtime's reactor,
    /// for readability, which a pidfd reaches once its child has exited.
    ///
    /// # Errors
    ///
    /// Returns an error outside a tokio runtime, and the reactor's error
    /// when it can't register the descriptor.
    ///
    /// # Panics
    ///
    /// Panics inside a tokio runtime built without its I/O driver, as
    /// tokio's registration does.
    #[expect(
        unsafe_code,
        reason = "tokio's registration of a descriptor is `unsafe`, its obligation the caller's"
    )]
    pub fn watch(self: &Arc<Self>) -> io::Result<PidfdWatch> {
        tokio::runtime::Handle::try_current().map_err(io::Error::other)?;
        let held = HeldPidfd(Arc::clone(self));
        // SAFETY: `HeldPidfd`'s descriptor is the `Pidfd`'s, which no code
        // replaces or closes while the `Arc` holding it lives, and the
        // `AsyncFd` holds the `Arc` for its whole life; `PidfdWatch` hands
        // out no `&mut` to it, so its `as_raw_fd` returns that one
        // descriptor while it's registered
        let registered = unsafe { AsyncFd::register_with_interest(held, Interest::READABLE) }
            .map_err(|error| error.into_parts().1)?;
        Ok(PidfdWatch { registered })
    }
}

impl AsFd for Pidfd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// A pidfd held by count, what a watch registers.
#[derive(Debug)]
struct HeldPidfd(Arc<Pidfd>);

impl AsRawFd for HeldPidfd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.fd.as_raw_fd()
    }
}

/// A pidfd registered with a tokio runtime's reactor, for the wait for its
/// child's exit; deregistered when dropped. It holds the [`Pidfd`] by
/// count, so the descriptor stays open while it's registered.
#[derive(Debug)]
pub struct PidfdWatch {
    /// The registration, never handed out mutably.
    registered: AsyncFd<HeldPidfd>,
}

impl PidfdWatch {
    /// Ready once the child has exited, its readiness kept, so a later poll
    /// is ready too.
    ///
    /// # Errors
    ///
    /// Returns the reactor's error.
    pub fn poll_exited(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.registered
            .poll_read_ready(cx)
            .map_ok(|mut guard| guard.retain_ready())
    }
}

/// Sets `SIGCHLD` to its default action when the process inherited it
/// ignored.
///
/// With it ignored, the kernel reaps an exited child itself, so its pidfd
/// could tell nothing of how it ended. A handler, or the default, is left
/// as it is, its flags with it: a caller that set `SA_NOCLDWAIT` has the
/// kernel reap its children as an ignored `SIGCHLD` would, which this
/// doesn't undo. Across `exec` the kernel clears every handler's flags, so
/// a process inherits `SIGCHLD` only ignored or at its default, which this
/// covers.
///
/// # Errors
///
/// Returns `sigaction`'s error, which a valid signal and action never get.
#[expect(
    unsafe_code,
    reason = "std sets no signal disposition, and a libc call is `unsafe`"
)]
pub fn default_child_signal() -> io::Result<()> {
    // SAFETY: an all-zero `sigaction` is a valid value of the struct of
    // integers and a signal set
    let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
    // SAFETY: a null action only reads the current one, into a live local
    // of the type `sigaction` expects
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &raw mut current) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if current.sa_sigaction != libc::SIG_IGN {
        return Ok(());
    }
    // SAFETY: as above
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    // SAFETY: `sigemptyset` writes only the live local's signal set
    unsafe { libc::sigemptyset(&raw mut action.sa_mask) };
    // SAFETY: the default action names no handler; the old one isn't asked
    // for
    if unsafe { libc::sigaction(libc::SIGCHLD, &raw const action, std::ptr::null_mut()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
