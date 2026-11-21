//! The private user and network namespace a process enters before its box
//! goes up, and the loopback interface brought up in it.
//!
//! [`enter_user_net_namespace`] makes one `unshare(CLONE_NEWUSER |
//! CLONE_NEWNET)`, which the kernel makes whole or not at all, then writes
//! the new user namespace's maps: `setgroups` denied, as the kernel requires
//! of a map its writer holds no privilege for, and the process's effective
//! user and group each mapped to itself, one id each. The process keeps its
//! ids, as files and programs see them, and is in a network namespace of
//! its own, which holds only a loopback interface, down until
//! [`UserNetNamespace::loopback_up`] brings it up.
//!
//! The namespace is a boundary, the kernel's, for the network: an internet
//! socket (loopback's included) or an abstract unix socket that code in the
//! process, or a program it starts, opens after entering reaches nothing
//! outside it, whatever it's built to do, short of a kernel bug. Two things
//! it doesn't reach: a socket opened before entering, or inherited, which
//! keeps the namespace it was made in (closing the inherited descriptors
//! first covers those), and a unix socket reached by its path in the
//! filesystem, which connects across namespaces (Landlock's and the
//! seccomp filter's to refuse).
//!
//! # Refused or stranded
//!
//! The call fails three ways, which a caller must tell apart
//! ([`NamespaceError`]):
//!
//! - [`NamespaceError::Threads`]: the process has another thread, a
//!   caller's bug; nothing changed.
//! - [`NamespaceError::Refused`]: nothing changed, the process is as it
//!   was, and the [`NamespaceRefused`] names the likely cause
//!   ([`Refusal`]) from the error and the kernel's settings: user
//!   namespaces restricted by a sysctl or by `AppArmor`, a seccomp filter,
//!   none left to make. A caller may fall back to another form of its box;
//!   only this arm carries a [`NamespaceRefused`], and only this module
//!   makes one, so a fallback that keeps its cause can't be built from
//!   another arm.
//! - [`NamespaceError::Stranded`]: `unshare` succeeded and a map's write
//!   failed after it. The process is in the new user namespace with its
//!   ids not all mapped, which nothing in the process can undo: no later
//!   step of a box is sound there, and the caller refuses to go on.
//!
//! `AppArmor`'s restriction of unprivileged user namespaces
//! (`kernel.apparmor_restrict_unprivileged_userns`) is told before
//! `unshare`, as [`Refusal::AppArmor`]: under it, `unshare` succeeds for an
//! unconfined process and the policy then denies every capability in the
//! new namespace, so the maps' writes would fail and strand it (inferred
//! from `AppArmor`'s design: no test here runs with the setting on).
//!
//! # Precondition and order
//!
//! The process has one thread, once a thread just joined has left the
//! count ([`crate::settled_thread_count`]); a thread started after shares
//! the namespaces. Its steps, in order, each needing the ones before it
//! but the last, which needs only to follow the first:
//!
//! 1. [`enter_user_net_namespace`], while the process is dumpable: the
//!    mark makes its `/proc/self` files root's, and the maps unwritable.
//! 2. [`UserNetNamespace::loopback_up`] and
//!    [`UserNetNamespace::refuse_nested_user_namespaces`], each taking a
//!    capability the process holds in the namespace until the next step.
//! 3. [`crate::capabilities::drop_capabilities`].
//! 4. [`crate::set_not_dumpable`].

use std::ffi::CStr;
use std::fs::File;
use std::io::{self, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, FromRawFd as _, OwnedFd};

/// The new user namespace's files the process writes after `unshare`, in
/// the order it writes them, which [`NamespaceError::Stranded`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapStep {
    /// `/proc/self/setgroups`, written `deny`, which the kernel requires
    /// before an unprivileged process writes its `gid_map`.
    Setgroups,
    /// `/proc/self/uid_map`, the effective user mapped to itself.
    UidMap,
    /// `/proc/self/gid_map`, the effective group mapped to itself.
    GidMap,
}

impl MapStep {
    /// The file's name in the process's `/proc` directory.
    const fn file(self) -> &'static CStr {
        match self {
            Self::Setgroups => c"setgroups",
            Self::UidMap => c"uid_map",
            Self::GidMap => c"gid_map",
        }
    }
}

impl std::fmt::Display for MapStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "/proc/self/{}", self.file().to_string_lossy())
    }
}

/// Why the namespaces were refused, as the error and the kernel's
/// settings, read once refused, tell it.
///
/// A cause is the likeliest one the settings show, never proven: an
/// `EPERM` under both a restricting sysctl and a seccomp filter is the
/// sysctl's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// `/proc/self` can't be read, for its thread count or to write the
    /// maps; found before `unshare`.
    #[error("/proc/self can't be read to write the namespace's maps")]
    NoProc,
    /// `AppArmor` restricts unprivileged user namespaces
    /// (`kernel.apparmor_restrict_unprivileged_userns` is 1) for this
    /// process: unconfined, and holding no `CAP_SYS_ADMIN`. Found before
    /// `unshare`, which would succeed and strand the process, as the
    /// [module doc](self) says; or, past that, an `unshare` refused under
    /// the setting. Inferred from `AppArmor`'s design: no test here runs
    /// on a kernel with the setting on.
    #[error(
        "AppArmor restricts unprivileged user namespaces \
         (kernel.apparmor_restrict_unprivileged_userns is 1)"
    )]
    AppArmor,
    /// Unprivileged user namespaces are off
    /// (`kernel.unprivileged_userns_clone` is 0, a Debian kernel's switch).
    #[error("unprivileged user namespaces are off (kernel.unprivileged_userns_clone is 0)")]
    Disabled,
    /// No namespace is left to make (`ENOSPC`): `user.max_user_namespaces`
    /// or `user.max_net_namespaces` is 0 or that many exist, or the process
    /// is nested as deep as the kernel allows (32).
    #[error(
        "no namespace is left to make (user.max_user_namespaces or user.max_net_namespaces, \
         or nested 32 deep)"
    )]
    NoneLeft,
    /// A seccomp filter on the process refuses `unshare`, as a container
    /// runtime's profile does.
    #[error("a seccomp filter on the process refuses unshare")]
    Filtered,
    /// None of the above shows: the call's error says what it can.
    #[error("unshare refused a user and network namespace")]
    Other,
}

/// The namespaces refused with nothing changed: the cause, and the
/// error of the call refused, if one was made. Only this module makes one.
#[derive(Debug, thiserror::Error)]
#[error("{cause}")]
pub struct NamespaceRefused {
    /// The likely cause.
    cause: Refusal,
    /// The refused call's error; none for a refusal found before any call.
    #[source]
    source: Option<io::Error>,
}

impl NamespaceRefused {
    /// The likely cause.
    pub const fn cause(&self) -> Refusal {
        self.cause
    }
}

/// Why [`enter_user_net_namespace`] failed.
#[derive(Debug, thiserror::Error)]
pub enum NamespaceError {
    /// The process has more than one thread, which a user namespace
    /// refuses; nothing changed.
    #[error("the process has {count} threads, and a user namespace takes one")]
    Threads {
        /// The thread count read.
        count: u64,
    },
    /// Nothing changed: the process is as it was.
    #[error(transparent)]
    Refused(NamespaceRefused),
    /// The process is in a new user namespace whose ids weren't all
    /// mapped, which can't be undone in it: it must not go on.
    #[error(
        "the process entered a user namespace and failed to write {step}, so its ids aren't all \
         mapped, which can't be undone"
    )]
    Stranded {
        /// The write that failed.
        step: MapStep,
        /// Its error.
        #[source]
        source: io::Error,
    },
}

/// The process's new user and network namespace, entered by
/// [`enter_user_net_namespace`]; holding one is the proof it was.
#[derive(Debug)]
#[must_use = "the namespace's loopback is down until it's brought up"]
pub struct UserNetNamespace {
    /// Made only here, so a value is the proof.
    _entered: (),
}

/// Why [`UserNetNamespace::loopback_up`] failed, the interface left as it
/// was.
#[derive(Debug, thiserror::Error)]
pub enum LoopbackError {
    /// The socket the interface is asked through couldn't be made.
    #[error("failed to open a socket to bring loopback up")]
    Socket(#[source] io::Error),
    /// Its flags couldn't be read (`SIOCGIFFLAGS`).
    #[error("failed to read loopback's flags")]
    ReadFlags(#[source] io::Error),
    /// Its flags couldn't be set (`SIOCSIFFLAGS`).
    #[error("failed to bring loopback up")]
    SetFlags(#[source] io::Error),
}

/// Enters a new user namespace and a new network namespace, as the
/// [module doc](self) says, by one `unshare(CLONE_NEWUSER | CLONE_NEWNET)`
/// and the three writes of the user namespace's maps.
///
/// Before `unshare` it reads the settled thread count, opens `/proc/self`,
/// so a process without `/proc` is refused with nothing changed, and tells
/// `AppArmor`'s restriction ([`Refusal::AppArmor`]). Each map file opens
/// beneath `/proc/self` after `unshare`, as the kernel binds a map file to
/// the namespace its opener is in, and is written whole by one `write`.
///
/// # Errors
///
/// Returns [`NamespaceError::Threads`] for another thread and
/// [`NamespaceError::Refused`] when `/proc/self` can't be read, `AppArmor`
/// restricts the process, or `unshare` fails, each with nothing changed;
/// [`NamespaceError::Stranded`] when a map's open or write fails after it.
#[expect(
    unsafe_code,
    reason = "std has no `unshare` or effective ids, and a libc call is `unsafe`"
)]
pub fn enter_user_net_namespace() -> Result<UserNetNamespace, NamespaceError> {
    let refused = |cause, source| NamespaceError::Refused(NamespaceRefused { cause, source });
    let count =
        crate::settled_thread_count().map_err(|error| refused(Refusal::NoProc, Some(error)))?;
    if count != 1 {
        return Err(NamespaceError::Threads { count });
    }
    // SAFETY: `geteuid` and `getegid` take nothing, always succeed, and
    // touch no memory of the process's
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    let proc_self =
        File::open("/proc/self").map_err(|error| refused(Refusal::NoProc, Some(error)))?;
    if AppArmorInputs::read().restricts() {
        return Err(refused(Refusal::AppArmor, None));
    }
    // SAFETY: `unshare` takes an integer and touches no memory of the
    // process's; the kernel makes both namespaces or neither
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } != 0 {
        let error = io::Error::last_os_error();
        let settings = Settings::read();
        // a thread started since the count, which the kernel refuses
        if let (Some(libc::EINVAL), Some(count)) = (error.raw_os_error(), settings.threads)
            && count > 1
        {
            return Err(NamespaceError::Threads { count });
        }
        return Err(refused(
            refusal(error.raw_os_error(), &settings),
            Some(error),
        ));
    }
    for (step, contents) in [
        (MapStep::Setgroups, "deny".to_owned()),
        (MapStep::UidMap, format!("{uid} {uid} 1\n")),
        (MapStep::GidMap, format!("{gid} {gid} 1\n")),
    ] {
        write_map(proc_self.as_fd(), step, contents.as_bytes())
            .map_err(|source| NamespaceError::Stranded { step, source })?;
    }
    Ok(UserNetNamespace { _entered: () })
}

/// `CAP_SYS_ADMIN`'s number.
const CAP_SYS_ADMIN: u32 = 21;

/// What tells whether `AppArmor` restricts this process's user namespaces,
/// each `None` when it couldn't be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AppArmorInputs {
    /// `kernel.apparmor_restrict_unprivileged_userns` reads 1.
    restricting: Option<bool>,
    /// The process's `AppArmor` label (`/proc/self/attr/apparmor/current`,
    /// or `/proc/self/attr/current` on a kernel without the first).
    label: Option<String>,
    /// The calling thread's effective capabilities.
    effective: Option<u64>,
}

impl AppArmorInputs {
    /// Reads them: the sysctl first, and the rest only where it reads 1.
    fn read() -> Self {
        let restricting =
            std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
                .ok()
                .map(|value| value.trim() == "1");
        if restricting != Some(true) {
            return Self {
                restricting,
                ..Self::default()
            };
        }
        let label = std::fs::read_to_string("/proc/self/attr/apparmor/current")
            .or_else(|_| std::fs::read_to_string("/proc/self/attr/current"))
            .ok();
        let effective = crate::capabilities::capability_sets()
            .ok()
            .map(|sets| sets.effective);
        Self {
            restricting,
            label,
            effective,
        }
    }

    /// Whether the restriction applies: the sysctl is 1, the process is
    /// unconfined, which the restriction covers, and it holds no
    /// `CAP_SYS_ADMIN`, which the restriction exempts. Anything unread
    /// isn't taken to restrict, so `unshare` is tried.
    fn restricts(&self) -> bool {
        self.restricting == Some(true)
            && self
                .label
                .as_deref()
                .is_some_and(|label| label.trim_end_matches(['\n', '\0']) == "unconfined")
            && self
                .effective
                .is_some_and(|effective| effective & (1 << CAP_SYS_ADMIN) == 0)
    }
}

/// Writes `contents` to `step`'s file beneath `proc_self`, by one `write`,
/// as the kernel takes a map: whole, in one call.
#[expect(
    unsafe_code,
    reason = "std opens only by a path from the working directory, and a libc call is `unsafe`"
)]
fn write_map(proc_self: BorrowedFd<'_>, step: MapStep, contents: &[u8]) -> io::Result<()> {
    // SAFETY: `openat` reads only the name, a live NUL-terminated string,
    // and takes the directory's descriptor, borrowed for the call
    let fd = unsafe {
        libc::openat(
            proc_self.as_raw_fd(),
            step.file().as_ptr(),
            libc::O_WRONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and is owned by nothing else, so the
    // `File` takes it, closing it once
    let mut file = File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    let written = file.write(contents)?;
    if written == contents.len() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::WriteZero,
            format!("{written} of {} bytes written", contents.len()),
        ))
    }
}

/// The kernel's settings that tell why `unshare` refused, each `None`
/// when it couldn't be read, as a kernel without the setting has it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Settings {
    /// The process's thread count, read as refused.
    threads: Option<u64>,
    /// `kernel.apparmor_restrict_unprivileged_userns`.
    apparmor_restricts: Option<bool>,
    /// `kernel.unprivileged_userns_clone`.
    userns_clone: Option<bool>,
    /// Whether a seccomp filter is installed (`Seccomp: 2`).
    filtered: Option<bool>,
}

impl Settings {
    /// Reads them, once `unshare` has refused.
    fn read() -> Self {
        let flag = |path: &str| {
            std::fs::read_to_string(path)
                .ok()
                .map(|value| value.trim() != "0")
        };
        let status = std::fs::read_to_string("/proc/self/status").ok();
        let field = |name: &str| {
            status.as_deref().and_then(|status| {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .and_then(|value| value.trim().parse::<u64>().ok())
            })
        };
        Self {
            threads: field("Threads:"),
            apparmor_restricts: flag("/proc/sys/kernel/apparmor_restrict_unprivileged_userns"),
            userns_clone: flag("/proc/sys/kernel/unprivileged_userns_clone"),
            filtered: field("Seccomp:").map(|mode| mode == 2),
        }
    }
}

/// The likely cause of `unshare`'s refusal with `errno`, given `settings`.
fn refusal(errno: Option<libc::c_int>, settings: &Settings) -> Refusal {
    match errno {
        Some(libc::ENOSPC) => Refusal::NoneLeft,
        Some(libc::EPERM | libc::EACCES) if settings.apparmor_restricts == Some(true) => {
            Refusal::AppArmor
        }
        Some(libc::EPERM) if settings.userns_clone == Some(false) => Refusal::Disabled,
        _ if settings.filtered == Some(true) => Refusal::Filtered,
        _ => Refusal::Other,
    }
}

impl UserNetNamespace {
    /// Brings the namespace's loopback interface up, so a program in it
    /// binds and connects to `127.0.0.1` and `::1`, which reach only the
    /// namespace.
    ///
    /// It asks through a datagram socket of its own by `ioctl`: the
    /// interface's flags read (`SIOCGIFFLAGS`), then set again with
    /// `IFF_UP` (`SIOCSIFFLAGS`), which takes `CAP_NET_ADMIN` in the
    /// namespace: call it before dropping the capabilities, which refuses
    /// it (`EPERM`). That's two calls on one socket, where netlink, the
    /// other way to ask, is a socket family, a message built and its answer
    /// parsed.
    ///
    /// # Errors
    ///
    /// Returns the step that failed ([`LoopbackError`]), the interface as
    /// it was.
    #[expect(
        unsafe_code,
        reason = "std has no socket without an address, no `ioctl`, and no `ifreq`"
    )]
    pub fn loopback_up(&self) -> Result<(), LoopbackError> {
        // SAFETY: `socket` takes integers and touches no memory of the
        // process's
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(LoopbackError::Socket(io::Error::last_os_error()));
        }
        // SAFETY: `fd` was just opened and is owned by nothing else
        let socket = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: an all-zero `ifreq` is a valid value: a name of NULs and
        // a union of integers, addresses, and a pointer, null when zero
        let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
        for (to, from) in request.ifr_name.iter_mut().zip(c"lo".to_bytes()) {
            *to = libc::c_char::from_ne_bytes([*from]);
        }
        ifreq_call(socket.as_fd(), libc::SIOCGIFFLAGS, &mut request)
            .map_err(LoopbackError::ReadFlags)?;
        // SAFETY: `SIOCGIFFLAGS` filled the union's flags, its member read
        let flags = unsafe { request.ifr_ifru.ifru_flags };
        // `IFF_UP` is 1, within a short
        #[expect(clippy::cast_possible_truncation, reason = "`IFF_UP` is 1")]
        let up = libc::IFF_UP as libc::c_short;
        request.ifr_ifru.ifru_flags = flags | up;
        ifreq_call(socket.as_fd(), libc::SIOCSIFFLAGS, &mut request)
            .map_err(LoopbackError::SetFlags)
    }
}

impl UserNetNamespace {
    /// Refuses the process, and every program it starts, any user
    /// namespace nested in this one, by setting its
    /// `user.max_user_namespaces` to 0: a nested `unshare` or `clone` of
    /// one then fails with `ENOSPC`, which the kernel decides, whatever
    /// filter is or isn't installed.
    ///
    /// A nested user namespace would hand back every capability
    /// [`crate::capabilities::drop_capabilities`] dropped, inside it. The
    /// setting is this namespace's own, which its owner may set while it
    /// holds `CAP_SYS_RESOURCE` in it: call it before dropping the
    /// capabilities.
    ///
    /// # Errors
    ///
    /// Returns the write's error: `EACCES` once the capabilities are
    /// dropped.
    pub fn refuse_nested_user_namespaces(&self) -> io::Result<()> {
        std::fs::OpenOptions::new()
            .write(true)
            .open("/proc/sys/user/max_user_namespaces")
            .and_then(|mut file| file.write_all(b"0"))
    }
}

/// `ioctl(socket, call, request)` for an `ifreq` call.
#[expect(
    unsafe_code,
    reason = "std has no `ioctl`, and a raw system call is `unsafe`"
)]
fn ifreq_call(
    socket: BorrowedFd<'_>,
    call: libc::c_ulong,
    request: &mut libc::ifreq,
) -> io::Result<()> {
    // SAFETY: the two calls made here, `SIOCGIFFLAGS` and `SIOCSIFFLAGS`,
    // read and write only the `ifreq` the pointer names, a live, writable
    // value of the size they expect; the socket is borrowed for the call.
    // It's made by `syscall`, whose request is any integer, as libc's
    // `ioctl` types it differently by C library
    let result = unsafe {
        libc::syscall(
            libc::SYS_ioctl,
            socket.as_raw_fd(),
            call,
            std::ptr::from_mut(request),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::{AppArmorInputs, CAP_SYS_ADMIN, Refusal, Settings, refusal};

    /// Each cause comes from its error and the setting that shows it, and
    /// an error no setting explains is `Other`.
    #[test]
    fn a_refusal_names_the_setting_that_shows_it() {
        let none = Settings::default();
        let apparmor = Settings {
            apparmor_restricts: Some(true),
            ..none
        };
        let apparmor_off = Settings {
            apparmor_restricts: Some(false),
            ..none
        };
        let clone_off = Settings {
            userns_clone: Some(false),
            ..none
        };
        let clone_on = Settings {
            userns_clone: Some(true),
            ..none
        };
        let filtered = Settings {
            filtered: Some(true),
            ..none
        };
        let both = Settings {
            apparmor_restricts: Some(true),
            filtered: Some(true),
            ..none
        };
        for (errno, settings, expected) in [
            (libc::EINVAL, none, Refusal::Other),
            (libc::ENOSPC, filtered, Refusal::NoneLeft),
            (libc::EPERM, apparmor, Refusal::AppArmor),
            (libc::EACCES, apparmor, Refusal::AppArmor),
            (libc::EPERM, apparmor_off, Refusal::Other),
            (libc::EPERM, clone_off, Refusal::Disabled),
            (libc::EPERM, clone_on, Refusal::Other),
            (libc::EPERM, filtered, Refusal::Filtered),
            (libc::ENOSYS, filtered, Refusal::Filtered),
            (libc::ENOSYS, none, Refusal::Other),
            (libc::EPERM, both, Refusal::AppArmor),
            (libc::EPERM, none, Refusal::Other),
        ] {
            assert_eq!(
                refusal(Some(errno), &settings),
                expected,
                "{errno} {settings:?}"
            );
        }
    }

    /// `ENOSPC`'s message names both settings that can cause it.
    #[test]
    fn none_left_names_both_limits() {
        let message = Refusal::NoneLeft.to_string();
        assert!(message.contains("user.max_user_namespaces"), "{message}");
        assert!(message.contains("user.max_net_namespaces"), "{message}");
    }

    /// `AppArmor` restricts only with the sysctl at 1, an unconfined label,
    /// and no `CAP_SYS_ADMIN`; anything unread doesn't restrict.
    #[test]
    fn apparmor_restricts_an_unconfined_process_without_cap_sys_admin() {
        let restricting = AppArmorInputs {
            restricting: Some(true),
            label: Some("unconfined\n".to_owned()),
            effective: Some(0),
        };
        assert!(restricting.restricts());
        for (inputs, expected) in [
            (
                AppArmorInputs {
                    label: Some("unconfined".to_owned()),
                    ..restricting.clone()
                },
                true,
            ),
            (
                AppArmorInputs {
                    effective: Some(!(1 << CAP_SYS_ADMIN)),
                    ..restricting.clone()
                },
                true,
            ),
            (
                AppArmorInputs {
                    restricting: Some(false),
                    ..restricting.clone()
                },
                false,
            ),
            (
                AppArmorInputs {
                    restricting: None,
                    ..restricting.clone()
                },
                false,
            ),
            (
                AppArmorInputs {
                    label: Some("/usr/bin/fgro (enforce)\n".to_owned()),
                    ..restricting.clone()
                },
                false,
            ),
            (
                AppArmorInputs {
                    label: None,
                    ..restricting.clone()
                },
                false,
            ),
            (
                AppArmorInputs {
                    effective: Some(1 << CAP_SYS_ADMIN),
                    ..restricting.clone()
                },
                false,
            ),
            (
                AppArmorInputs {
                    effective: None,
                    ..restricting
                },
                false,
            ),
            (AppArmorInputs::default(), false),
        ] {
            assert_eq!(inputs.restricts(), expected, "{inputs:?}");
        }
    }
}
