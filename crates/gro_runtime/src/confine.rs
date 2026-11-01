//! Confinement: the box a run's process is held in, and the type state that
//! gates the engine on it.
//!
//! A run is either confined or explicitly unconfined before the engine starts:
//! [`engine::init_engine`](crate::engine::init_engine) takes a [`Confinement`],
//! and its token ([`Engine`](crate::engine::Engine)) names which. The states:
//!
//! - [`Prepared`]: the box's roots, computed before anything is confined
//!   ([`Prepared::for_project`]): the project's real directory, and the link
//!   closure below it, each opened as it is found, so the box goes up on the
//!   directories found, not on whatever their paths name later.
//! - [`Confined`]: the process passed through [`Prepared::confine`], made only
//!   there, on the process's one thread.
//! - [`Confinement::Unconfined`]: named by the caller, for a run that holds
//!   no box (`fgro run`, the snapshot builder, and tests).
//!
//! The box must hold every thread the process has, so it is installed while
//! the process has one: what resolves the run's roots runs on a thread joined
//! before [`Prepared::confine`], which checks the count first and refuses
//! otherwise, and every thread the engine starts comes after it, spawned from
//! the thread that installed it.
//!
//! The box, on Linux, installed by [`Prepared::confine`] in this order on the
//! calling thread, after the thread count is checked:
//!
//! 1. **`no_new_privs`**: nothing the process executes can gain privileges.
//! 2. **Landlock**, at ABI 6 (Linux 6.12) or later, with every rule
//!    enforced in full, or the confinement fails. The filesystem: of every access right ABI 6
//!    knows, only reading files and listing directories, and only beneath
//!    the box's roots, named by the handles opened when each was found, so
//!    nothing is written or executed anywhere and nothing
//!    outside the roots is read (`/proc` and `/dev` included). The network:
//!    binding and connecting TCP sockets refused. The scopes: no signal to a
//!    process outside the box, and no connection to an abstract unix socket
//!    outside it. A kernel without Landlock, or with it below ABI 6, is
//!    refused, naming what is missing and the kernel's ABI.
//! 3. **seccomp**, two filters on the calling thread (no thread sync, since
//!    it is the process's only thread): one kills the process on the calls
//!    no part of the runtime makes and code that escaped the engine would
//!    ([`KILLED_CALLS`]: `execve` and `execveat`, `ptrace`, the cross-process
//!    memory calls, `bpf`, `perf_event_open`, `userfaultfd`, the `kexec`
//!    calls, the `io_uring` calls, whose operations bypass seccomp, and the
//!    kernel module calls); the other refuses with `EACCES` what a library
//!    may try and handle failing ([`REFUSED_CALLS`]): every `socket`,
//!    `connect`, `bind`, and `listen`, every `socketpair` but
//!    [`KEPT_SOCKETPAIR`], the unix stream pair tokio uses (a datagram pair
//!    can send to any socket file by its path, which Landlock below ABI 9
//!    doesn't gate), the calls that change a file's mode, owner, times,
//!    extended attributes, or attribute flags, which Landlock at ABI 6
//!    doesn't gate, `inotify_add_watch` and `fanotify_init`, the System V
//!    IPC calls and `mq_open`/`mq_unlink`, the `fcntl` commands that take a
//!    lease or ask for directory change notices (`REFUSED_FCNTLS`; the
//!    advisory locks, `flock` and `F_SETLK`, are accepted: they write
//!    nothing, and only hold off another process locking the same file
//!    inside the box while the eval runs), the calls that set another process's
//!    limits, priority, CPUs, or scheduling (`SELF_ONLY_CALLS`), which
//!    Landlock's scopes don't reach, and every `ioctl` but `KEPT_IOCTLS`.
//!
//! Every thread the calling thread starts afterward inherits all three. The
//! reads the runtime makes at startup outside the roots (the CPU and cgroup
//! files `available_parallelism` reads, `/proc/self/maps`) fail and fall back
//! quietly; V8's entropy comes from the `getrandom` call, not a file
//! (`engine`).
//!
//! Off Linux there is no box, and [`Prepared::confine`] refuses.
//!
//! The link closure is the box's directory structure, read with no parsing:
//! starting from the project, every entry of a root's `node_modules` (each
//! `<name>`, and each `<name>` under an `@<scope>` directory, the dot-named
//! entries skipped) whose real path is a directory lying outside every root so
//! far, holding none, and holding a `package.json` that is a file (not a
//! symlink), becomes a root, unless a world-writable directory holds it, and
//! its own `node_modules` is read the same way, to a fixed point. An installed
//! package sits under the project's `node_modules`, already inside it; a
//! linked one (a symlinked path dependency, a workspace checkout) is where the
//! closure reaches outside. A link to a file, to an ancestor of the project,
//! to `/`, or to a directory that isn't a package (`/etc`, `~/.ssh`) adds
//! nothing.
//!
//! On Linux each root is opened as it is found (`O_PATH`, with
//! `O_DIRECTORY` and `O_NOFOLLOW`), and kept only when the kernel names the
//! handle by the root's real path, so a directory swapped in after the
//! closure read it isn't what the box admits.

use std::io;
use std::path::{Path, PathBuf};

use crate::resolver::{DiskTree, SourceTree};

/// A root, opened before anything is confined: on Linux, an `O_PATH` handle
/// to the directory ([`pin`]).
#[cfg(target_os = "linux")]
type Pin = std::os::fd::OwnedFd;

/// A root, opened before anything is confined: off Linux, where there is no
/// box, nothing.
#[cfg(not(target_os = "linux"))]
type Pin = ();

/// The directories a confined run may read: the project's real directory
/// first, then each link closure root, every one a real path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxRoots(Vec<PathBuf>);

impl BoxRoots {
    /// Roots as given, for a test of what reads them.
    #[cfg(test)]
    pub(crate) const fn for_test(roots: Vec<PathBuf>) -> Self {
        Self(roots)
    }

    /// Whether the real path `path` is one of the roots or lies under one,
    /// component by component.
    pub fn holds(&self, path: &Path) -> bool {
        self.0.iter().any(|root| path.starts_with(root))
    }

    /// The roots, the project's first.
    pub fn as_slice(&self) -> &[PathBuf] {
        &self.0
    }
}

/// The box, computed: its roots, before anything is confined, each opened.
#[derive(Debug)]
pub struct Prepared {
    /// What the box admits.
    roots: BoxRoots,
    /// Each root, opened, in the roots' order: what Landlock's rules name.
    #[cfg_attr(
        not(target_os = "linux"),
        expect(dead_code, reason = "off Linux there is no box to install")
    )]
    pins: Vec<Pin>,
}

impl Prepared {
    /// The box for the project whose real directory is `project`: it and the
    /// link closure below it, read from the disk now, each root opened as it
    /// is found. A `node_modules` or scope directory that can't be listed,
    /// an entry whose real path can't be read (a link to nothing), and a
    /// root that can't be opened as the directory found add nothing.
    ///
    /// # Errors
    ///
    /// Returns the error when `project` can't be opened as a directory
    /// without following a symlink, or the handle the kernel opened isn't
    /// `project` (a directory on its path swapped since it was read).
    pub fn for_project(project: PathBuf) -> io::Result<Self> {
        let pinned = pin(&project)?;
        let (roots, pins) = link_closure(project, pinned);
        Ok(Self {
            roots: BoxRoots(roots),
            pins,
        })
    }

    /// The roots the box admits.
    pub const fn roots(&self) -> &BoxRoots {
        &self.roots
    }

    /// Confines the process to the box, from its one thread, as the module
    /// doc describes.
    ///
    /// # Errors
    ///
    /// Returns [`ConfineError::Threads`] when the process has another thread,
    /// or [`ConfineError::ThreadsUnread`] when its thread count can't be
    /// read, before anything is installed; on Linux, an error naming the wall
    /// the kernel can't hold (checked before any is installed) or that failed
    /// to install, after which the process may hold part of the box and must
    /// run nothing it was to confine; and [`ConfineError::Unsupported`] off
    /// Linux.
    pub fn confine(&self) -> Result<Confined, ConfineError> {
        alone()?;
        install(self)?;
        Ok(Confined {
            roots: self.roots.clone(),
        })
    }
}

/// The process confined to its box, from the thread that installed it.
/// Made only by [`Prepared::confine`].
#[derive(Debug, PartialEq, Eq)]
pub struct Confined {
    /// What the box admits.
    roots: BoxRoots,
}

impl Confined {
    /// The roots the box admits.
    pub const fn roots(&self) -> &BoxRoots {
        &self.roots
    }
}

/// Whether a run's process is confined, which the engine takes to start.
#[derive(Debug, PartialEq, Eq)]
pub enum Confinement {
    /// Held in its box.
    Confined(Confined),
    /// Held in no box, by the caller's choice.
    Unconfined,
}

/// Why [`Prepared::confine`] refused.
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    /// The process has more than one thread, so a box installed now would
    /// leave the others outside it.
    #[error(
        "the process has {count} threads, so a box installed now would leave all but one \
         outside it: the box is installed while the process has one"
    )]
    Threads {
        /// How many threads it has.
        count: usize,
    },
    /// The process's thread count couldn't be read.
    #[error("failed to read the process's thread count from /proc/self/status: {0}")]
    ThreadsUnread(#[source] io::Error),
    /// The kernel has no Landlock, or has it switched off.
    #[error(
        "the kernel {}, and the box's filesystem, network, and scope walls are Landlock's: \
         confined eval needs Landlock ABI {REQUIRED_LANDLOCK_ABI} (Linux 6.12) or later",
        if *.built { "has Landlock but it isn't enabled (it is missing from the boot's \
         `lsm=` list)" } else { "has no Landlock" }
    )]
    NoLandlock {
        /// Whether the kernel is built with Landlock, though it isn't enabled.
        built: bool,
    },
    /// The kernel's Landlock is older than the box needs.
    #[error(
        "the kernel's Landlock is ABI {abi}, below the ABI {REQUIRED_LANDLOCK_ABI} (Linux \
         6.12) the box requires for its scope walls, which stop a signal to a process outside \
         the box and a connection to an abstract unix socket outside it"
    )]
    LandlockAbi {
        /// The kernel's ABI.
        abi: u32,
    },
    /// The Landlock rules couldn't be made or installed.
    #[cfg(target_os = "linux")]
    #[error("failed to install the box's Landlock rules: {0}")]
    Landlock(#[source] landlock::RulesetError),
    /// The kernel installed only part of the Landlock rules.
    #[error("the kernel enforced the box's Landlock rules only in part, so none can be relied on")]
    LandlockPartial,
    /// The seccomp filters can't be built for this architecture.
    #[error(
        "the box's seccomp filters can't be built for the {arch} architecture: they are built \
         for x86_64 and aarch64"
    )]
    SeccompArch {
        /// The architecture, as Rust names it.
        arch: &'static str,
    },
    /// The seccomp filters couldn't be compiled.
    #[cfg(target_os = "linux")]
    #[error("failed to compile the box's seccomp filters: {0}")]
    SeccompFilter(#[source] seccompiler::BackendError),
    /// The kernel refused the seccomp filters.
    #[cfg(target_os = "linux")]
    #[error("failed to install the box's seccomp filters: {0}")]
    Seccomp(#[source] seccompiler::Error),
    /// The platform has no box: confinement is Linux's.
    #[error("confinement is built for Linux only, and this platform has no box")]
    Unsupported,
}

impl ConfineError {
    /// Whether the host can't hold the box (no Landlock, an older one, an
    /// architecture the seccomp filters aren't built for, a kernel refusing
    /// them, another platform), as opposed to the process breaking the order
    /// the box needs or failing to build it.
    pub const fn is_host_unsupported(&self) -> bool {
        match self {
            Self::NoLandlock { .. }
            | Self::LandlockAbi { .. }
            | Self::SeccompArch { .. }
            | Self::Unsupported => true,
            #[cfg(target_os = "linux")]
            Self::Seccomp(_) => true,
            #[cfg(target_os = "linux")]
            Self::Landlock(_) | Self::SeccompFilter(_) => false,
            Self::Threads { .. } | Self::ThreadsUnread(_) | Self::LandlockPartial => false,
        }
    }
}

/// The Landlock ABI the box requires, for its scope walls: [`LANDLOCK_ABI`]'s
/// number, for the messages, which every platform builds.
const REQUIRED_LANDLOCK_ABI: u32 = 6;

/// The calls that kill the process: none is made by any part of the runtime,
/// so reaching one means code escaped the engine.
#[cfg(target_os = "linux")]
pub const KILLED_CALLS: [libc::c_long; 16] = [
    libc::SYS_execve,
    libc::SYS_execveat,
    libc::SYS_ptrace,
    libc::SYS_process_vm_readv,
    libc::SYS_process_vm_writev,
    libc::SYS_bpf,
    libc::SYS_perf_event_open,
    libc::SYS_userfaultfd,
    libc::SYS_kexec_load,
    libc::SYS_kexec_file_load,
    libc::SYS_io_uring_setup,
    libc::SYS_io_uring_enter,
    libc::SYS_io_uring_register,
    libc::SYS_init_module,
    libc::SYS_finit_module,
    libc::SYS_delete_module,
];

/// The calls refused with `EACCES` whatever their arguments, beside every
/// `socketpair` but [`KEPT_SOCKETPAIR`]: what a library may try and handle
/// failing.
///
/// - Every `socket`, `connect`, `bind`, and `listen`: no socket is made or
///   reached, and the kept stream pair's sockets can't be bound to an
///   abstract name or listen on one.
/// - The calls that change a file's mode, owner, times, extended
///   attributes, or attribute flags, which Landlock at ABI 6 doesn't gate.
///   aarch64 has none of the older path calls (`chmod`, `chown`, `lchown`,
///   `utime`, `utimes`, `futimesat`); the calls `libc` doesn't name on every
///   architecture are numbered from the table both share (`SYS_FCHMODAT2`).
/// - `inotify_add_watch` and `fanotify_init`, whose events would tell the
///   process when a file outside the box changes.
/// - The System V IPC calls and the POSIX message queue calls that make or
///   remove a queue (`mq_open`, `mq_unlink`, which Landlock doesn't gate),
///   whose objects live outside the box, outlast the process, and are
///   shared with the user's other processes.
#[cfg(target_os = "linux")]
pub const REFUSED_CALLS: &[libc::c_long] = &[
    libc::SYS_socket,
    libc::SYS_connect,
    libc::SYS_bind,
    libc::SYS_listen,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_chmod,
    libc::SYS_fchmod,
    libc::SYS_fchmodat,
    SYS_FCHMODAT2,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_chown,
    libc::SYS_fchown,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_lchown,
    libc::SYS_fchownat,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_utime,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_utimes,
    libc::SYS_utimensat,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_futimesat,
    libc::SYS_setxattr,
    libc::SYS_lsetxattr,
    libc::SYS_fsetxattr,
    libc::SYS_removexattr,
    libc::SYS_lremovexattr,
    libc::SYS_fremovexattr,
    SYS_SETXATTRAT,
    SYS_REMOVEXATTRAT,
    SYS_FILE_SETATTR,
    libc::SYS_inotify_add_watch,
    libc::SYS_fanotify_init,
    libc::SYS_mq_open,
    libc::SYS_mq_unlink,
    libc::SYS_shmget,
    libc::SYS_shmat,
    libc::SYS_shmctl,
    libc::SYS_msgget,
    libc::SYS_msgsnd,
    libc::SYS_msgrcv,
    libc::SYS_msgctl,
    libc::SYS_semget,
    libc::SYS_semop,
    libc::SYS_semtimedop,
    libc::SYS_semctl,
];

/// `fchmodat2`, by its number in the syscall table `x86_64` and `aarch64`
/// share from 424 on, since `libc` names it on `x86_64` alone. A kernel too old to
/// have a call refuses it with `ENOSYS` anyway.
#[cfg(target_os = "linux")]
const SYS_FCHMODAT2: libc::c_long = 452;

/// `setxattrat`, by its number in the shared table ([`SYS_FCHMODAT2`]).
#[cfg(target_os = "linux")]
const SYS_SETXATTRAT: libc::c_long = 463;

/// `removexattrat`, by its number in the shared table ([`SYS_FCHMODAT2`]).
#[cfg(target_os = "linux")]
const SYS_REMOVEXATTRAT: libc::c_long = 466;

/// `file_setattr`, by its number in the shared table ([`SYS_FCHMODAT2`]).
#[cfg(target_os = "linux")]
const SYS_FILE_SETATTR: libc::c_long = 469;

/// The calls refused with `EACCES` when they name a process other than the
/// caller: setting another process's resource limits, priority, I/O
/// priority, CPUs, or scheduling. Landlock scopes signals and `ptrace`, not
/// these, and a limit is a signal by other means (a CPU limit's `SIGXCPU`).
///
/// Each is the call, the argument naming the process, refused unless 0 (the
/// caller), and for `setpriority` and `ioprio_set` the value the argument
/// before it must hold to say that names one process, not a group or a
/// user. The calls that only read these (`getpriority`, `ioprio_get`,
/// `sched_getaffinity`, `sched_getparam`, `sched_getscheduler`,
/// `sched_getattr`) pass: for another process they tell whether it exists
/// and how it's scheduled, not its files or memory. A `prlimit64` that only
/// reads names its process the same way and is refused with the rest.
#[cfg(target_os = "linux")]
const SELF_ONLY_CALLS: [(libc::c_long, u8, Option<u64>); 7] = [
    (libc::SYS_prlimit64, 0, None),
    // `PRIO_PROCESS`, 0
    (libc::SYS_setpriority, 1, Some(0)),
    // `IOPRIO_WHO_PROCESS`, 1
    (libc::SYS_ioprio_set, 1, Some(1)),
    (libc::SYS_sched_setaffinity, 0, None),
    (libc::SYS_sched_setscheduler, 0, None),
    (libc::SYS_sched_setparam, 0, None),
    (libc::SYS_sched_setattr, 0, None),
];

/// The `ioctl` requests the box keeps: setting and clearing non-blocking
/// and close-on-exec, the bytes waiting, and reading a terminal's settings
/// and size. Every other request is refused with `EACCES`, the ones that
/// change a file's attribute flags, generation, extended attributes, or
/// verity, or a filesystem's own (a btrfs subvolume), from a handle opened
/// only to read among them, which Landlock gates only on devices.
#[cfg(target_os = "linux")]
const KEPT_IOCTLS: [libc::Ioctl; 6] = [
    libc::FIONBIO,
    libc::FIOCLEX,
    libc::FIONCLEX,
    libc::FIONREAD,
    libc::TCGETS,
    libc::TIOCGWINSZ,
];

/// The `fcntl` commands refused with `EACCES`: taking a lease (`F_SETLEASE`),
/// which stalls another process's open for writing until the lease breaks
/// and tells the holder it was tried, and asking for directory change
/// notices (`F_NOTIFY`), the older form of `inotify_add_watch`. Every other
/// command passes, the flag and descriptor commands std and tokio use among
/// them, and the advisory locks (`F_SETLK` and the others, like `flock`),
/// which write nothing and only hold off another process locking the same
/// file inside the box, such as a build tool's under the project root, for
/// as long as the eval runs.
#[cfg(target_os = "linux")]
const REFUSED_FCNTLS: [libc::c_int; 2] = [libc::F_SETLEASE, libc::F_NOTIFY];

/// The one `socketpair` the box keeps, by domain and type: a unix stream
/// pair, which tokio uses, connected only to itself.
///
/// Every other is refused with `EACCES`, a datagram pair above all, which can
/// send to any socket file by its path.
#[cfg(target_os = "linux")]
pub const KEPT_SOCKETPAIR: [libc::c_int; 2] = [libc::AF_UNIX, libc::SOCK_STREAM];

/// The bits of a socket type that name it, below the `SOCK_NONBLOCK` and
/// `SOCK_CLOEXEC` flags.
#[cfg(target_os = "linux")]
const SOCKET_TYPE_MASK: u64 = 0xf;

/// Installs the box `prepared` on the calling thread, the process's only
/// one.
#[cfg(target_os = "linux")]
fn install(prepared: &Prepared) -> Result<(), ConfineError> {
    // compiled first, so a filter that can't be built fails before any wall
    let filters = seccomp_filters()?;
    landlock(&prepared.pins)?;
    for filter in &filters {
        seccompiler::apply_filter(filter).map_err(ConfineError::Seccomp)?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
const fn install(_prepared: &Prepared) -> Result<(), ConfineError> {
    Err(ConfineError::Unsupported)
}

/// Installs the Landlock rules, reading beneath each of `pins`, the roots'
/// handles opened before, after `no_new_privs`, which the kernel requires
/// first and Landlock sets.
#[cfg(target_os = "linux")]
fn landlock(pins: &[Pin]) -> Result<(), ConfineError> {
    use std::os::fd::AsFd as _;

    use landlock::{
        Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, RestrictSelf, Ruleset,
        RulesetAttr, RulesetCreatedAttr, RulesetError, RulesetStatus, Scope,
    };
    // the kernel's status, read with nothing installed: no flags, and
    // `no_new_privs` left to the ruleset
    let status = RestrictSelf::default()
        .no_new_privs(false)
        .apply()
        .map_err(ConfineError::Landlock)?;
    landlock_floor(status.landlock)?;
    let restricted = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(LANDLOCK_ABI))
        .and_then(|ruleset| ruleset.handle_access(AccessNet::from_all(LANDLOCK_ABI)))
        .and_then(|ruleset| ruleset.scope(Scope::from_all(LANDLOCK_ABI)))
        .and_then(Ruleset::create)
        .and_then(|ruleset| {
            ruleset.add_rules(pins.iter().map(|pin| {
                Ok::<_, RulesetError>(PathBeneath::new(
                    pin.as_fd(),
                    AccessFs::ReadFile | AccessFs::ReadDir,
                ))
            }))
        })
        .and_then(landlock::RulesetCreated::restrict_self)
        .map_err(ConfineError::Landlock)?;
    if restricted.ruleset != RulesetStatus::FullyEnforced || !restricted.no_new_privs {
        return Err(ConfineError::LandlockPartial);
    }
    Ok(())
}

/// The Landlock ABI the box's rules are written for, and the least the
/// kernel's may be.
#[cfg(target_os = "linux")]
const LANDLOCK_ABI: landlock::ABI = landlock::ABI::V6;

/// Refuses a kernel whose Landlock, as `status` reports it, can't hold the
/// box: none, switched off, or below [`LANDLOCK_ABI`].
#[cfg(target_os = "linux")]
fn landlock_floor(status: landlock::LandlockStatus) -> Result<(), ConfineError> {
    use landlock::LandlockStatus;
    match status {
        LandlockStatus::NotEnabled => Err(ConfineError::NoLandlock { built: true }),
        LandlockStatus::NotImplemented => Err(ConfineError::NoLandlock { built: false }),
        LandlockStatus::Available { effective_abi, .. } if effective_abi < LANDLOCK_ABI => {
            Err(ConfineError::LandlockAbi {
                abi: effective_abi as u32,
            })
        }
        LandlockStatus::Available { .. } => Ok(()),
    }
}

/// The box's two seccomp filters, compiled for this architecture: the calls
/// that kill the process, and the calls refused with `EACCES`.
#[cfg(target_os = "linux")]
fn seccomp_filters() -> Result<[seccompiler::BpfProgram; 2], ConfineError> {
    use std::collections::BTreeMap;

    use seccompiler::{
        SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule, TargetArch,
    };
    let arch =
        TargetArch::try_from(std::env::consts::ARCH).map_err(|_| ConfineError::SeccompArch {
            arch: std::env::consts::ARCH,
        })?;
    let killed: BTreeMap<i64, Vec<SeccompRule>> = KILLED_CALLS
        .into_iter()
        .map(|call| (call, Vec::new()))
        .collect();
    // `socketpair` is refused when any rule matches: its domain, the first
    // argument, an int, isn't the kept one, or its type, the second, its
    // flags masked off, isn't
    let [kept_domain, kept_type] = KEPT_SOCKETPAIR.map(|value| u64::from(value.unsigned_abs()));
    let rule = |argument, op, value| {
        SeccompCondition::new(argument, SeccompCmpArgLen::Dword, op, value)
            .and_then(|condition| SeccompRule::new(vec![condition]))
    };
    let other_pairs = std::iter::once(rule(0, SeccompCmpOp::Ne, kept_domain))
        .chain(
            (0..=SOCKET_TYPE_MASK)
                .filter(|socket_type| *socket_type != kept_type)
                .map(|socket_type| rule(1, SeccompCmpOp::MaskedEq(SOCKET_TYPE_MASK), socket_type)),
        )
        .collect::<Result<Vec<_>, _>>()
        .map_err(ConfineError::SeccompFilter)?;
    // `ioctl` is refused unless its request, the second argument, an
    // unsigned int and so compared at 32 bits, is one of the kept: one rule
    // whose conditions all hold, so all must differ
    let ioctls = KEPT_IOCTLS
        .into_iter()
        .map(|request| SeccompCondition::new(1, SeccompCmpArgLen::Dword, SeccompCmpOp::Ne, request))
        .collect::<Result<Vec<_>, _>>()
        .and_then(SeccompRule::new)
        .map(|other_requests| vec![other_requests])
        .map_err(ConfineError::SeccompFilter)?;
    // `fcntl` is refused when its command, the second argument, an unsigned
    // int, is one of the refused: one rule each, any matching
    let fcntls = REFUSED_FCNTLS
        .into_iter()
        .map(|command| rule(1, SeccompCmpOp::Eq, u64::from(command.unsigned_abs())))
        .collect::<Result<Vec<_>, _>>()
        .map_err(ConfineError::SeccompFilter)?;
    // the calls that set another process's limits, priority, CPUs, or
    // scheduling are refused when they name anything but the caller (0),
    // which Landlock's signal and ptrace scopes don't reach: a CPU limit of
    // one second kills the process it's set on
    let other_processes = SELF_ONLY_CALLS
        .into_iter()
        .map(|(call, who, which)| {
            // `setpriority` and `ioprio_set` also name what `who` is, so one
            // naming a whole process group or user is refused too
            let which = which.map(|process| rule(0, SeccompCmpOp::Ne, process));
            std::iter::once(rule(who, SeccompCmpOp::Ne, 0))
                .chain(which)
                .collect::<Result<Vec<_>, _>>()
                .map(|rules| (call, rules))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(ConfineError::SeccompFilter)?;
    let refused: BTreeMap<i64, Vec<SeccompRule>> = REFUSED_CALLS
        .iter()
        .map(|call| (*call, Vec::new()))
        .chain([
            (libc::SYS_socketpair, other_pairs),
            (libc::SYS_ioctl, ioctls),
            (libc::SYS_fcntl, fcntls),
        ])
        .chain(other_processes)
        .collect();
    let compile = |rules, action| {
        SeccompFilter::new(rules, SeccompAction::Allow, action, arch)
            .and_then(seccompiler::BpfProgram::try_from)
            .map_err(ConfineError::SeccompFilter)
    };
    Ok([
        compile(killed, SeccompAction::KillProcess)?,
        compile(refused, SeccompAction::Errno(libc::EACCES.unsigned_abs()))?,
    ])
}

/// The link closure of the project at `project`, opened as `pinned`: it
/// first, then each `node_modules` entry's real path that is a package
/// directory ([`is_package`]), neither under a root so far nor holding one,
/// held by no world-writable directory ([`held_by_world_writable`]), and
/// opened as found ([`pin`]), read in turn to a fixed point. Returns the
/// roots and their handles, in the same order.
fn link_closure(project: PathBuf, pinned: Pin) -> (Vec<PathBuf>, Vec<Pin>) {
    let mut roots = vec![project];
    let mut pins = vec![pinned];
    let mut next = 0;
    while let Some(root) = roots.get(next) {
        let node_modules = root.join("node_modules");
        let entries = package_entries(&node_modules);
        next += 1;
        for entry in entries {
            // a package is a directory with a package.json, and none holds a
            // root: a link to a file, to an ancestor of the project, to `/`,
            // or to a directory that isn't a package widens nothing
            if let Ok(target) = std::fs::canonicalize(&entry)
                && is_package(&target)
                && !roots
                    .iter()
                    .any(|root| target.starts_with(root) || root.starts_with(&target))
                && !held_by_world_writable(&node_modules, &entry, &target)
                && let Ok(pinned) = pin(&target)
            {
                roots.push(target);
                pins.push(pinned);
            }
        }
    }
    (roots, pins)
}

/// Whether the real path `target` is a package's directory: a directory
/// holding a `package.json` that is a regular file, not a symlink, which the
/// resolver would read at its own real path, possibly outside the box.
fn is_package(target: &Path) -> bool {
    target.is_dir()
        && std::fs::symlink_metadata(target.join("package.json"))
            .is_ok_and(|metadata| metadata.file_type().is_file())
}

/// Opens the real directory `dir` as a root's handle: `O_PATH`, with
/// `O_DIRECTORY` and `O_NOFOLLOW` (and std's `O_CLOEXEC`), so only a
/// directory opens and a symlink in its last component doesn't, then checks
/// that the kernel names the handle by `dir`, so a directory on its path
/// swapped since `dir` was read is refused.
///
/// # Errors
///
/// Returns the error of the open or of reading the handle's path, or one of
/// kind [`io::ErrorKind::Other`] when the handle names another path.
#[cfg(target_os = "linux")]
fn pin(dir: &Path) -> io::Result<Pin> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)?;
    let opened =
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to read the path of its handle from /proc/self/fd: {error}"),
            )
        })?;
    if opened != dir {
        return Err(io::Error::other(format!(
            "opened {} as {}, another directory",
            dir.display(),
            opened.display()
        )));
    }
    Ok(file.into())
}

#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature as Linux's, which can fail"
)]
const fn pin(_dir: &Path) -> io::Result<Pin> {
    Ok(())
}

/// Whether the package linked at `entry` in `node_modules`, whose real path
/// is `target`, is held by a world-writable directory, which any user can
/// plant files in: `node_modules` and the entry's `@scope` directory, the real
/// parent of each, `target`, or the directories holding `target`, one per
/// segment of the package's name. The resolver refuses a
/// package found through any of them, by the same predicate
/// ([`SourceTree::is_world_writable`]), so none is worth a root. A directory
/// whose mode can't be read counts as one.
fn held_by_world_writable(node_modules: &Path, entry: &Path, target: &Path) -> bool {
    let scope = entry.parent().filter(|parent| *parent != node_modules);
    let segments = if scope.is_some() { 2 } else { 1 };
    let real_parents = [Some(node_modules), scope]
        .into_iter()
        .flatten()
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .filter_map(|real| real.parent().map(Path::to_path_buf));
    let mut dirs: Vec<PathBuf> = [Some(node_modules), scope]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf)
        .collect();
    dirs.extend(real_parents);
    dirs.extend(target.ancestors().take(segments + 1).map(Path::to_path_buf));
    dirs.iter()
        .any(|dir| !matches!(DiskTree.is_world_writable(dir), Ok(false)))
}

/// The package entries of the `node_modules` at `node_modules`, sorted: each
/// `<name>` and each `@<scope>/<name>`, the dot-named ones (`.bin`, a
/// `.package-lock.json`, a `.deno` store) skipped.
fn package_entries(node_modules: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    for entry in list(node_modules) {
        let Some(name) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if name.starts_with('@') {
            entries.extend(list(&entry).into_iter().filter(|scoped| {
                !scoped
                    .file_name()
                    .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
            }));
        } else {
            entries.push(entry);
        }
    }
    entries.sort();
    entries
}

/// The paths of the entries of the directory `dir`, or none when it can't be
/// listed.
fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |entries| {
            entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect()
        },
    )
}

/// How many times [`alone`] reads the thread count before it refuses: a
/// thread whose join has returned can stay counted for a moment longer, while
/// the kernel finishes its exit (the joiner wakes when the thread's id is
/// cleared, before the thread is removed from its group).
#[cfg(target_os = "linux")]
const THREAD_COUNT_READS: usize = 1000;

/// Checks that the process has one thread, the caller.
#[cfg(target_os = "linux")]
fn alone() -> Result<(), ConfineError> {
    let mut count = 0;
    for _ in 0..THREAD_COUNT_READS {
        count = thread_count().map_err(ConfineError::ThreadsUnread)?;
        if count == 1 {
            return Ok(());
        }
        std::thread::yield_now();
    }
    Err(ConfineError::Threads { count })
}

#[cfg(not(target_os = "linux"))]
const fn alone() -> Result<(), ConfineError> {
    Err(ConfineError::Unsupported)
}

/// The process's thread count, its `Threads:` line in `/proc/self/status`.
#[cfg(target_os = "linux")]
fn thread_count() -> io::Result<usize> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .and_then(|count| count.trim().parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no Threads: line"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use std::os::unix::fs::PermissionsExt as _;

    use super::{BoxRoots, ConfineError, Prepared};

    /// A fresh directory under the system's temporary directory, removed on
    /// drop, its path canonical. Every directory it makes has the mode
    /// `0o755`, whatever the umask, so none is world-writable unless a test
    /// makes it so.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("gro_runtime_confine_{}_{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            set_mode(&dir, 0o755);
            Self(dir.canonicalize().unwrap())
        }

        fn dir(&self, path: &str) -> PathBuf {
            let dir = self.0.join(path);
            std::fs::create_dir_all(&dir).unwrap();
            let mut made = self.0.clone();
            for component in Path::new(path).components() {
                made.push(component);
                set_mode(&made, 0o755);
            }
            dir
        }

        fn file(&self, path: &str) {
            std::fs::write(self.0.join(path), "{}").unwrap();
        }

        fn link(&self, target: &Path, at: &str) {
            std::os::unix::fs::symlink(target, self.0.join(at)).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// The roots of the box for `project`.
    fn closure(project: &Path) -> Vec<PathBuf> {
        Prepared::for_project(project.to_owned())
            .unwrap()
            .roots()
            .as_slice()
            .to_vec()
    }

    /// Links out of the project are followed to a fixed point through each
    /// new root's own `node_modules`, scoped names included; a link back into
    /// a root, an installed package, a dot-named entry, a link to nothing, a
    /// link to a file, to an ancestor of the project, or to `/`, a link to a
    /// directory with no `package.json`, or whose `package.json` is a symlink
    /// or a directory, and a link to a directory a world-writable one holds
    /// or that is one add nothing.
    #[test]
    fn the_link_closure_follows_links_out_to_a_fixed_point() {
        let scratch = Scratch::new("closure");
        // below the scratch directory, whose own parent is world-writable
        let project = scratch.dir("w/project");
        let tool = scratch.dir("w/tool");
        let util = scratch.dir("w/util");
        let hidden = scratch.dir("w/hidden");
        let scoped = scratch.dir("w/scoped");
        let shared = scratch.dir("w/shared");
        let planted = scratch.dir("w/shared/planted");
        let open = scratch.dir("w/open");
        // not packages: no package.json (`/etc`, `~/.ssh`), one symlinked to
        // a file, and one that is a directory
        let bare = scratch.dir("w/bare");
        let linked_json = scratch.dir("w/linked_json");
        let dir_json = scratch.dir("w/dir_json");
        scratch.dir("w/dir_json/package.json");
        for dir in [
            "w",
            "w/tool",
            "w/util",
            "w/hidden",
            "w/scoped",
            "w/shared/planted",
            "w/open",
        ] {
            scratch.file(&format!("{dir}/package.json"));
        }
        scratch.link(
            &scratch.0.join("w/tool/package.json"),
            "w/linked_json/package.json",
        );
        for dir in [&shared, &open] {
            set_mode(dir, 0o777);
        }
        std::fs::write(scratch.0.join("w/file.txt"), "").unwrap();
        scratch.dir("w/project/node_modules/@fuzdev");
        scratch.dir("w/project/node_modules/installed");
        scratch.dir("w/project/node_modules/@scope/installed");
        scratch.dir("w/tool/node_modules");
        scratch.file("w/project/node_modules/installed/package.json");
        scratch.link(&tool, "w/project/node_modules/@fuzdev/tool");
        scratch.link(&scoped, "w/project/node_modules/@scope/linked");
        scratch.link(&hidden, "w/project/node_modules/.bin");
        scratch.link(&hidden, "w/project/node_modules/@fuzdev/.hidden");
        scratch.link(&scratch.0.join("absent"), "w/project/node_modules/dangling");
        scratch.link(&scratch.0.join("w/file.txt"), "w/project/node_modules/file");
        scratch.link(&scratch.0.join("w"), "w/project/node_modules/ancestor");
        scratch.link(Path::new("/"), "w/project/node_modules/root");
        scratch.link(&planted, "w/project/node_modules/planted");
        scratch.link(&open, "w/project/node_modules/open");
        scratch.link(&bare, "w/project/node_modules/bare");
        scratch.link(&linked_json, "w/project/node_modules/linked_json");
        scratch.link(&dir_json, "w/project/node_modules/dir_json");
        scratch.link(&util, "w/tool/node_modules/util");
        scratch.link(&project, "w/tool/node_modules/project");
        scratch.link(
            &project.join("node_modules/installed"),
            "w/tool/node_modules/back",
        );
        assert_eq!(closure(&project), [project, tool, scoped, util]);
    }

    /// Each root is opened as the directory found: the kernel names every
    /// handle by its root's real path.
    #[cfg(target_os = "linux")]
    #[test]
    fn each_root_is_opened_as_the_directory_found() {
        use std::os::fd::AsRawFd as _;

        let scratch = Scratch::new("pinned");
        let project = scratch.dir("project");
        let lib = scratch.dir("lib");
        scratch.file("lib/package.json");
        scratch.dir("project/node_modules");
        scratch.link(&lib, "project/node_modules/lib");
        let prepared = Prepared::for_project(project.clone()).unwrap();
        assert_eq!(prepared.roots().as_slice(), [project, lib]);
        let named: Vec<PathBuf> = prepared
            .pins
            .iter()
            .map(|pin| std::fs::read_link(format!("/proc/self/fd/{}", pin.as_raw_fd())).unwrap())
            .collect();
        assert_eq!(named, prepared.roots().as_slice());
    }

    /// A project path that isn't its real directory isn't opened: a
    /// symlink, a path through a symlinked directory, which the kernel names
    /// by another path once opened, or no directory.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_project_that_is_not_its_real_directory_is_refused() {
        let scratch = Scratch::new("unpinned");
        let project = scratch.dir("real/project");
        scratch.link(&project, "linked");
        scratch.link(&scratch.0.join("real"), "alias");
        assert!(Prepared::for_project(scratch.0.join("linked")).is_err());
        assert!(Prepared::for_project(scratch.0.join("alias/project")).is_err());
        assert!(Prepared::for_project(scratch.0.join("absent")).is_err());
        assert!(Prepared::for_project(project).is_ok());
    }

    /// A project with no `node_modules` is its own box.
    #[test]
    fn a_project_with_no_node_modules_is_its_own_box() {
        let scratch = Scratch::new("bare");
        let project = scratch.dir("project");
        let prepared = Prepared::for_project(project.clone()).unwrap();
        assert_eq!(prepared.roots().as_slice(), std::slice::from_ref(&project));
        assert!(prepared.roots().holds(&project.join("a/b.ts")));
        assert!(!prepared.roots().holds(&scratch.0.join("projectile/b.ts")));
    }

    #[test]
    fn box_roots_hold_by_component() {
        let roots = BoxRoots(vec![PathBuf::from("/p"), PathBuf::from("/q/lib")]);
        assert!(roots.holds(Path::new("/p")));
        assert!(roots.holds(Path::new("/p/x.js")));
        assert!(roots.holds(Path::new("/q/lib/node_modules/y/z.js")));
        assert!(!roots.holds(Path::new("/pp/x.js")));
        assert!(!roots.holds(Path::new("/q/x.js")));
    }

    /// The filters compile for this architecture; installing them is the
    /// confined-process tests' (a test process has threads of its own).
    #[cfg(target_os = "linux")]
    #[test]
    fn the_seccomp_filters_compile() {
        let [killed, refused] = super::seccomp_filters().unwrap();
        assert!(!killed.is_empty());
        assert!(!refused.is_empty());
    }

    /// The calls the box refuses whatever their arguments, listed apart from
    /// [`super::REFUSED_CALLS`], so a call dropped there fails the tests.
    #[cfg(target_os = "linux")]
    fn refused_calls() -> Vec<libc::c_long> {
        let mut expected = vec![
            libc::SYS_socket,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_fchmod,
            libc::SYS_fchmodat,
            libc::SYS_fchown,
            libc::SYS_fchownat,
            libc::SYS_utimensat,
            libc::SYS_setxattr,
            libc::SYS_lsetxattr,
            libc::SYS_fsetxattr,
            libc::SYS_removexattr,
            libc::SYS_lremovexattr,
            libc::SYS_fremovexattr,
            super::SYS_FCHMODAT2,
            super::SYS_SETXATTRAT,
            super::SYS_REMOVEXATTRAT,
            super::SYS_FILE_SETATTR,
            libc::SYS_inotify_add_watch,
            libc::SYS_fanotify_init,
            libc::SYS_mq_open,
            libc::SYS_mq_unlink,
            libc::SYS_shmget,
            libc::SYS_shmat,
            libc::SYS_shmctl,
            libc::SYS_msgget,
            libc::SYS_msgsnd,
            libc::SYS_msgrcv,
            libc::SYS_msgctl,
            libc::SYS_semget,
            libc::SYS_semop,
            libc::SYS_semtimedop,
            libc::SYS_semctl,
        ];
        #[cfg(target_arch = "x86_64")]
        expected.extend([
            libc::SYS_chmod,
            libc::SYS_chown,
            libc::SYS_lchown,
            libc::SYS_utime,
            libc::SYS_utimes,
            libc::SYS_futimesat,
        ]);
        expected
    }

    /// The calls refused whatever their arguments include every one the
    /// confined-process tests can't make from safe code: the extended
    /// attribute calls (`setxattrat` and `removexattrat` by number),
    /// `fchmodat2` and `file_setattr` by number, `inotify_add_watch`,
    /// `fanotify_init`, and the System V IPC calls, beside the socket calls
    /// and the metadata calls those tests make.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_refused_calls_cover_metadata_and_watches() {
        let mut expected = refused_calls();
        let mut refused = super::REFUSED_CALLS.to_vec();
        refused.sort_unstable();
        expected.sort_unstable();
        assert_eq!(refused, expected);
    }

    /// The calls that set another process's limits, priority, CPUs, or
    /// scheduling are refused unless they name the caller, by the argument
    /// that names the process, and for `setpriority` and `ioprio_set` unless
    /// that argument names one process; the refusing filter matches each.
    /// The confined-process tests can't make these from safe code.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_self_only_calls_name_the_caller() {
        assert_eq!(
            super::SELF_ONLY_CALLS,
            [
                (libc::SYS_prlimit64, 0, None),
                // `PRIO_PROCESS` and `IOPRIO_WHO_PROCESS`
                (libc::SYS_setpriority, 1, Some(0)),
                (libc::SYS_ioprio_set, 1, Some(1)),
                (libc::SYS_sched_setaffinity, 0, None),
                (libc::SYS_sched_setscheduler, 0, None),
                (libc::SYS_sched_setparam, 0, None),
                (libc::SYS_sched_setattr, 0, None),
            ]
        );
        let [_, refused] = super::seccomp_filters().unwrap();
        for (call, _, _) in super::SELF_ONLY_CALLS {
            assert!(
                refused
                    .iter()
                    .any(|instruction| i64::from(instruction.k) == call),
                "{call}"
            );
        }
    }

    /// The `ioctl` requests kept are the ones std, tokio, and the console
    /// make, and the refusing filter compares the request with each; the
    /// ones that set a file's attributes aren't kept.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_kept_ioctls_set_nothing() {
        assert_eq!(
            super::KEPT_IOCTLS,
            [
                libc::FIONBIO,
                libc::FIOCLEX,
                libc::FIONCLEX,
                libc::FIONREAD,
                libc::TCGETS,
                libc::TIOCGWINSZ,
            ]
        );
        for refused in [
            libc::FS_IOC_SETFLAGS,
            libc::FS_IOC32_SETFLAGS,
            libc::FS_IOC_SETVERSION,
            libc::FS_IOC32_SETVERSION,
            0x401c_5820,
        ] {
            assert!(!super::KEPT_IOCTLS.contains(&refused), "{refused:#x}");
        }
        let [_, refused] = super::seccomp_filters().unwrap();
        for request in super::KEPT_IOCTLS {
            assert!(
                refused
                    .iter()
                    .any(|instruction| u64::from(instruction.k) == request),
                "{request:#x}"
            );
        }
    }

    /// The `seccomp_data` architecture value of the kernel's ABI this test
    /// runs on: `AUDIT_ARCH_X86_64` or `AUDIT_ARCH_AARCH64`, the
    /// architectures the box is built for.
    #[cfg(target_os = "linux")]
    fn audit_arch() -> u32 {
        match std::env::consts::ARCH {
            "x86_64" => 0xc000_003e,
            "aarch64" => 0xc000_00b7,
            other => panic!("no seccomp_data architecture value for {other}"),
        }
    }

    /// What the kernel would decide for the call `nr` with `args`, from the
    /// architecture `arch`, under the compiled `filter`: a classic BPF
    /// interpreter over the instructions `seccompiler` emits, run on the
    /// `seccomp_data` the kernel hands a filter (`nr` and `arch`, 32 bits
    /// each, the instruction pointer, then the six arguments, 64 bits each,
    /// little-endian on both architectures), so a table can pin the policy
    /// as compiled. Returns the filter's return value, its action and data.
    /// An instruction it doesn't know panics, never passes.
    #[cfg(target_os = "linux")]
    fn decide(
        filter: &[seccompiler::sock_filter],
        arch: u32,
        nr: libc::c_long,
        args: [u64; 6],
    ) -> u32 {
        const LOAD: u32 = libc::BPF_LD | libc::BPF_W | libc::BPF_ABS;
        const AND: u32 = libc::BPF_ALU | libc::BPF_AND | libc::BPF_K;
        const JUMP: u32 = libc::BPF_JMP | libc::BPF_JA;
        const EQUAL: u32 = libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K;
        const ABOVE: u32 = libc::BPF_JMP | libc::BPF_JGT | libc::BPF_K;
        const AT_LEAST: u32 = libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K;
        const RETURN: u32 = libc::BPF_RET | libc::BPF_K;
        let mut data = [0_u8; 64];
        data[..4].copy_from_slice(&i32::try_from(nr).unwrap().to_le_bytes());
        data[4..8].copy_from_slice(&arch.to_le_bytes());
        for (slot, arg) in data[16..].as_chunks_mut::<8>().0.iter_mut().zip(args) {
            slot.copy_from_slice(&arg.to_le_bytes());
        }
        let (mut pc, mut accumulator) = (0, 0_u32);
        loop {
            let instruction = &filter[pc];
            pc += 1;
            let k = instruction.k;
            let taken = |taken: bool| {
                usize::from(if taken {
                    instruction.jt
                } else {
                    instruction.jf
                })
            };
            match u32::from(instruction.code) {
                LOAD => {
                    let at = usize::try_from(k).unwrap();
                    accumulator = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                }
                AND => accumulator &= k,
                JUMP => pc += usize::try_from(k).unwrap(),
                EQUAL => pc += taken(accumulator == k),
                ABOVE => pc += taken(accumulator > k),
                AT_LEAST => pc += taken(accumulator >= k),
                RETURN => return k,
                code => panic!("an instruction the evaluator doesn't know: {code:#x}"),
            }
        }
    }

    /// The refusing filter decides each call as the policy says, read as
    /// the kernel reads it: the requests, commands, and processes it keeps
    /// pass, every other is refused, and an argument's upper half, which the
    /// kernel drops from an int, changes nothing. The killing filter passes
    /// all of those and kills on its own calls, and both kill a call from
    /// another architecture.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_refusing_filter_decides_by_its_arguments() {
        const PASS: u32 = libc::SECCOMP_RET_ALLOW;
        const REFUSE: u32 = libc::SECCOMP_RET_ERRNO | libc::EACCES.unsigned_abs();
        const KILL: u32 = libc::SECCOMP_RET_KILL_PROCESS;
        let [killing, refusing] = super::seccomp_filters().unwrap();
        let arch = audit_arch();
        let high = 0xdead_0000_0000_0000_u64;
        let some_pid = 4242;
        let [prio_process, prio_pgrp, prio_user] =
            [libc::PRIO_PROCESS, libc::PRIO_PGRP, libc::PRIO_USER].map(u64::from);
        // `IOPRIO_WHO_PROCESS`, `IOPRIO_WHO_PGRP`, `IOPRIO_WHO_USER`
        let [who_process, who_pgrp, who_user] = [1, 2, 3];
        let unix = u64::from(libc::AF_UNIX.unsigned_abs());
        let [stream, datagram, cloexec] = [libc::SOCK_STREAM, libc::SOCK_DGRAM, libc::SOCK_CLOEXEC]
            .map(|value| u64::from(value.unsigned_abs()));
        let [
            set_lease,
            notify,
            get_lease,
            get_fl,
            set_fl,
            get_fd,
            set_fd,
            dup_cloexec,
            set_lock,
        ] = [
            libc::F_SETLEASE,
            libc::F_NOTIFY,
            libc::F_GETLEASE,
            libc::F_GETFL,
            libc::F_SETFL,
            libc::F_GETFD,
            libc::F_SETFD,
            libc::F_DUPFD_CLOEXEC,
            libc::F_SETLK,
        ]
        .map(|command| u64::from(command.unsigned_abs()));
        let read_lock = u64::from(libc::F_RDLCK.unsigned_abs());
        let mut cases = vec![
            // the kept ioctls, also with an upper half, and the refused
            (libc::SYS_ioctl, [2, high | libc::FIONBIO, 0, 0, 0, 0], PASS),
            (
                libc::SYS_ioctl,
                [2, libc::FS_IOC_GETFLAGS, 0, 0, 0, 0],
                REFUSE,
            ),
            (
                libc::SYS_ioctl,
                [2, libc::FS_IOC_SETFLAGS, 0, 0, 0, 0],
                REFUSE,
            ),
            (
                libc::SYS_ioctl,
                [2, high | libc::FS_IOC_SETFLAGS, 0, 0, 0, 0],
                REFUSE,
            ),
            (libc::SYS_ioctl, [2, libc::TIOCSTI, 0, 0, 0, 0], REFUSE),
            (libc::SYS_ioctl, [2, 0, 0, 0, 0, 0], REFUSE),
            // leases and directory notices refused, also with an upper half;
            // the flag, descriptor, and advisory lock commands pass
            (libc::SYS_fcntl, [3, set_lease, read_lock, 0, 0, 0], REFUSE),
            (
                libc::SYS_fcntl,
                [3, high | set_lease, read_lock, 0, 0, 0],
                REFUSE,
            ),
            (libc::SYS_fcntl, [3, notify, 0, 0, 0, 0], REFUSE),
            (libc::SYS_fcntl, [3, get_lease, 0, 0, 0, 0], PASS),
            (libc::SYS_fcntl, [3, get_fl, 0, 0, 0, 0], PASS),
            (libc::SYS_fcntl, [3, set_fl, 0, 0, 0, 0], PASS),
            (libc::SYS_fcntl, [3, get_fd, 0, 0, 0, 0], PASS),
            (libc::SYS_fcntl, [3, set_fd, 0, 0, 0, 0], PASS),
            (libc::SYS_fcntl, [3, dup_cloexec, 3, 0, 0, 0], PASS),
            (libc::SYS_fcntl, [3, set_lock, 0, 0, 0, 0], PASS),
            // one process, the caller, by 0 only
            (libc::SYS_setpriority, [prio_process, 0, 19, 0, 0, 0], PASS),
            (
                libc::SYS_setpriority,
                [prio_process, some_pid, 19, 0, 0, 0],
                REFUSE,
            ),
            (
                libc::SYS_setpriority,
                [prio_process, high | some_pid, 19, 0, 0, 0],
                REFUSE,
            ),
            (
                libc::SYS_setpriority,
                [prio_process, high, 19, 0, 0, 0],
                PASS,
            ),
            (libc::SYS_setpriority, [prio_pgrp, 0, 19, 0, 0, 0], REFUSE),
            (libc::SYS_setpriority, [prio_user, 0, 19, 0, 0, 0], REFUSE),
            (libc::SYS_ioprio_set, [who_process, 0, 0, 0, 0, 0], PASS),
            (
                libc::SYS_ioprio_set,
                [who_process, some_pid, 0, 0, 0, 0],
                REFUSE,
            ),
            (libc::SYS_ioprio_set, [who_pgrp, 0, 0, 0, 0, 0], REFUSE),
            (libc::SYS_ioprio_set, [who_user, 0, 0, 0, 0, 0], REFUSE),
            // the reads pass
            (libc::SYS_getpriority, [prio_user, 0, 0, 0, 0, 0], PASS),
            // the kept socketpair, with its flags, and two others
            (
                libc::SYS_socketpair,
                [unix, stream | cloexec, 0, 0, 0, 0],
                PASS,
            ),
            (libc::SYS_socketpair, [unix, datagram, 0, 0, 0, 0], REFUSE),
            (
                libc::SYS_socketpair,
                [u64::from(libc::AF_INET.unsigned_abs()), stream, 0, 0, 0, 0],
                REFUSE,
            ),
            (libc::SYS_read, [0; 6], PASS),
        ];
        for (call, who, _) in super::SELF_ONLY_CALLS {
            let mut args = [0; 6];
            if call == libc::SYS_ioprio_set {
                args[0] = who_process;
            }
            cases.push((call, args, PASS));
            args[usize::from(who)] = some_pid;
            cases.push((call, args, REFUSE));
        }
        for call in refused_calls() {
            cases.push((call, [0; 6], REFUSE));
        }
        for (call, args, expected) in cases {
            assert_eq!(
                decide(&refusing, arch, call, args),
                expected,
                "{call} {args:x?}"
            );
            assert_eq!(decide(&killing, arch, call, args), PASS, "{call} {args:x?}");
        }
        for call in super::KILLED_CALLS {
            assert_eq!(decide(&killing, arch, call, [0; 6]), KILL, "{call}");
        }
        for filter in [&killing, &refusing] {
            assert_eq!(decide(filter, arch ^ 1, libc::SYS_read, [0; 6]), KILL);
        }
    }

    /// A kernel with no Landlock, with it off, or below ABI 6 is refused,
    /// naming what is missing; ABI 6 and later pass.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_landlock_floor_is_abi_6() {
        use landlock::{ABI, LandlockStatus};
        let available = |abi| LandlockStatus::Available {
            effective_abi: abi,
            kernel_abi: None,
        };
        for abi in [ABI::V1, ABI::V4, ABI::V5] {
            let error = super::landlock_floor(available(abi)).unwrap_err();
            assert!(
                matches!(error, ConfineError::LandlockAbi { abi: found } if found == abi as u32),
                "{error:?}"
            );
            assert!(error.is_host_unsupported());
            assert!(
                error.to_string().starts_with(&format!(
                    "the kernel's Landlock is ABI {}, below the ABI 6",
                    abi as u32
                )),
                "{error}"
            );
        }
        for (status, says) in [
            (LandlockStatus::NotImplemented, "the kernel has no Landlock"),
            (
                LandlockStatus::NotEnabled,
                "the kernel has Landlock but it isn't enabled",
            ),
        ] {
            let error = super::landlock_floor(status).unwrap_err();
            assert!(error.is_host_unsupported());
            assert!(error.to_string().starts_with(says), "{error}");
        }
        for abi in [ABI::V6, ABI::V7, ABI::V9] {
            assert!(super::landlock_floor(available(abi)).is_ok());
        }
        // the number the messages name is the floor's
        assert_eq!(super::LANDLOCK_ABI as u32, super::REQUIRED_LANDLOCK_ABI);
    }

    /// A test runs beside the harness's own threads, so confining from it is
    /// refused with the count, before anything is installed.
    #[cfg(target_os = "linux")]
    #[test]
    fn confining_with_another_thread_alive_is_refused() {
        let (stop, wait) = std::sync::mpsc::channel::<()>();
        let other = std::thread::spawn(move || wait.recv());
        let scratch = Scratch::new("threads");
        let prepared = Prepared::for_project(scratch.0.clone()).unwrap();
        match prepared.confine() {
            Err(ConfineError::Threads { count }) => assert!(count >= 2, "{count}"),
            other => panic!("expected a refusal by the thread count, got {other:?}"),
        }
        stop.send(()).unwrap();
        other.join().unwrap().unwrap();
    }
}
