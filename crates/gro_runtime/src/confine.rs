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
//!    it is the process's only thread). The first kills the process on the
//!    calls no part of the runtime makes and code that escaped the engine
//!    would ([`KILLED_CALLS`]: `execve` and `execveat`, `ptrace`, the
//!    cross-process memory calls, `bpf`, `perf_event_open`, `userfaultfd`,
//!    the `kexec` calls, the `io_uring` calls, whose operations bypass
//!    seccomp, and the kernel module calls). The second is an allowlist:
//!    the calls the runtime makes after the box goes up, and their named
//!    siblings ([`ALLOWED_CALLS`]), pass; `prctl`, `fcntl`, `ioctl`,
//!    `socketpair`, and `clone` pass only with the arguments the runtime
//!    uses (`KEPT_PRCTLS`, `KEPT_FCNTLS`, `KEPT_IOCTLS`,
//!    [`KEPT_SOCKETPAIR`], and a `clone` that starts a thread, setting no
//!    flag in `CLONE_NAMESPACES`); every other call fails with `ENOSYS`,
//!    an argument outside its rules included. `ENOSYS` is what a call the
//!    kernel lacks returns, so glibc and std fall back where they can
//!    (`clone3` to `clone`, `statx` to `fstat`), and a host making a call
//!    the list doesn't name degrades where its libc has a fallback, and
//!    otherwise fails with `ENOSYS`. The filter
//!    compares exact call numbers, so an x32 call (`nr | 0x40000000`)
//!    matches nothing and fails too, and since the allowlist doesn't allow
//!    `seccomp`, no filter can be added after it. Built for `x86_64` and
//!    `aarch64`; the `aarch64` list follows that architecture's call table
//!    and is unverified on a host.
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

/// The calls that kill the process, by their own filter, installed before
/// the allowlist.
///
/// None is made by any part of the runtime, so reaching one means code
/// escaped the engine. The allowlist doesn't allow them either; this filter
/// is the tripwire that ends the process rather than let it carry on past
/// `ENOSYS`.
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

/// The calls the allowlist filter allows whatever their arguments.
///
/// They are the calls the runtime was seen to make after the box goes up,
/// and the named siblings a host may make in their place. Every call
/// missing from it and from the calls allowed by their arguments
/// (`KEPT_PRCTLS`, `KEPT_FCNTLS`, `KEPT_IOCTLS`, [`KEPT_SOCKETPAIR`], and a
/// `clone` that starts a thread, `CLONE_NAMESPACES`) fails with `ENOSYS`.
///
/// `sched_getaffinity`, `sched_getparam`, and `sched_getscheduler` take any
/// process, since glibc names the caller by its thread id, not 0: for
/// another process they tell whether it exists and how it's scheduled, not
/// its files or memory. Built for `x86_64` and `aarch64`; the `aarch64` list
/// is written from its call table and unverified on a host.
#[cfg(target_os = "linux")]
pub const ALLOWED_CALLS: &[libc::c_long] = &[
    libc::SYS_brk,
    libc::SYS_clock_gettime,
    libc::SYS_clock_nanosleep,
    libc::SYS_close,
    libc::SYS_epoll_create1,
    libc::SYS_epoll_ctl,
    libc::SYS_eventfd2,
    libc::SYS_exit,
    libc::SYS_exit_group,
    libc::SYS_futex,
    libc::SYS_getpid,
    libc::SYS_gettid,
    libc::SYS_lseek,
    libc::SYS_madvise,
    libc::SYS_mmap,
    libc::SYS_mprotect,
    libc::SYS_mremap,
    libc::SYS_munmap,
    libc::SYS_openat,
    libc::SYS_pkey_alloc,
    libc::SYS_pkey_mprotect,
    libc::SYS_read,
    // aarch64 has only `readlinkat`, which glibc's `readlink` makes there
    #[cfg(target_arch = "x86_64")]
    libc::SYS_readlink,
    #[cfg(target_arch = "aarch64")]
    libc::SYS_readlinkat,
    libc::SYS_rseq,
    libc::SYS_rt_sigaction,
    libc::SYS_rt_sigprocmask,
    libc::SYS_sched_getaffinity,
    libc::SYS_sched_getparam,
    libc::SYS_sched_getscheduler,
    libc::SYS_sched_yield,
    libc::SYS_set_robust_list,
    libc::SYS_sigaltstack,
    libc::SYS_statx,
    libc::SYS_uname,
    libc::SYS_write,
    // V8's entropy, whose failure aborts the process (`engine`): the vDSO's
    // on glibc 2.41 and Linux 6.11 and later, a call elsewhere
    libc::SYS_getrandom,
    // the event loop's wait, made only when it blocks: which one depends on
    // the architecture and the libc (aarch64 has no `epoll_wait`)
    #[cfg(target_arch = "x86_64")]
    libc::SYS_epoll_wait,
    libc::SYS_epoll_pwait,
    libc::SYS_epoll_pwait2,
    // std's fallback when `statx` fails
    libc::SYS_fstat,
    libc::SYS_newfstatat,
    // the return from a handled signal (`--jit`'s WebAssembly trap handler)
    // and the restart of a call it interrupted
    libc::SYS_rt_sigreturn,
    libc::SYS_restart_syscall,
    // `abort()`'s signal to its own thread; Landlock's signal scope keeps
    // it inside the box
    libc::SYS_tgkill,
    // the clock calls' fallbacks when the vDSO lacks one, and `nanosleep`,
    // which other libcs make for a sleep
    libc::SYS_clock_getres,
    libc::SYS_gettimeofday,
    libc::SYS_nanosleep,
];

/// The `prctl` options the box keeps, by the first argument: naming a
/// thread, and naming an anonymous mapping (glibc's and V8's). Every other
/// fails with `ENOSYS`, the ones that change what the process may do
/// (`PR_SET_NO_NEW_PRIVS`, `PR_SET_SECCOMP`, the capability options) or
/// who may reach it (`PR_SET_DUMPABLE`, `PR_SET_PTRACER`) among them.
#[cfg(target_os = "linux")]
const KEPT_PRCTLS: [libc::c_int; 2] = [libc::PR_SET_NAME, libc::PR_SET_VMA];

/// The `fcntl` commands the box keeps, by the second argument: reading and
/// setting the descriptor and status flags, and duplicating a descriptor
/// close-on-exec, the ones std and tokio use. Every other fails with
/// `ENOSYS`: the advisory locks, which would hold off another process
/// locking the same file inside the box, a lease, which stalls another
/// process's open for writing and tells the holder it was tried, directory
/// change notices, and setting a descriptor's owner among them.
#[cfg(target_os = "linux")]
const KEPT_FCNTLS: [libc::c_int; 5] = [
    libc::F_GETFD,
    libc::F_SETFD,
    libc::F_GETFL,
    libc::F_SETFL,
    libc::F_DUPFD_CLOEXEC,
];

/// The `ioctl` requests the box keeps: setting and clearing non-blocking
/// and close-on-exec, the bytes waiting, and reading a terminal's settings
/// and size. Every other request fails with `ENOSYS`, the ones that change
/// a file's attribute flags, generation, extended attributes, or verity, or
/// a filesystem's own (a btrfs subvolume), from a handle opened only to
/// read among them, which Landlock gates only on devices.
#[cfg(target_os = "linux")]
const KEPT_IOCTLS: [libc::Ioctl; 6] = [
    libc::FIONBIO,
    libc::FIOCLEX,
    libc::FIONCLEX,
    libc::FIONREAD,
    libc::TCGETS,
    libc::TIOCGWINSZ,
];

/// The one `socketpair` the box keeps, by domain and type: a unix stream
/// pair, which tokio uses, connected only to itself.
///
/// Every other fails with `ENOSYS`, a datagram pair above all, which can
/// send to any socket file by its path.
#[cfg(target_os = "linux")]
pub const KEPT_SOCKETPAIR: [libc::c_int; 2] = [libc::AF_UNIX, libc::SOCK_STREAM];

/// The bits of a socket type that name it, below the `SOCK_NONBLOCK` and
/// `SOCK_CLOEXEC` flags.
#[cfg(target_os = "linux")]
const SOCKET_TYPE_MASK: u64 = 0xf;

/// The namespace flags `clone` accepts, which the box's one kept `clone`
/// must not set: the flags under the mask of these and `CLONE_THREAD`, the
/// first argument on `x86_64` and `aarch64`, must be `CLONE_THREAD` alone, so a
/// thread starts, and no process is forked and no namespace made.
/// `CLONE_NEWTIME` isn't among them: `clone` reads its flags' low byte as
/// the exit signal, and takes that flag only from `clone3`.
///
/// `clone3` isn't allowed: its flags sit in memory, out of a filter's reach,
/// and its `ENOSYS` makes glibc start the thread with `clone` instead.
#[cfg(target_os = "linux")]
const CLONE_NAMESPACES: libc::c_int = libc::CLONE_NEWNS
    | libc::CLONE_NEWCGROUP
    | libc::CLONE_NEWUTS
    | libc::CLONE_NEWIPC
    | libc::CLONE_NEWUSER
    | libc::CLONE_NEWPID
    | libc::CLONE_NEWNET;

/// Installs the box `prepared` on the calling thread, the process's only
/// one.
#[cfg(target_os = "linux")]
fn install(prepared: &Prepared) -> Result<(), ConfineError> {
    // compiled first, so a filter that can't be built fails before any wall
    let filters = seccomp_filters()?;
    landlock(&prepared.pins)?;
    // the killing filter first, so it passes the allowlist's own `seccomp`
    // call; the allowlist doesn't allow `seccomp`, so it is the last filter
    // the process can take
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

/// The box's two seccomp filters, compiled for this architecture, in the
/// order they are installed: the calls that kill the process, then the
/// allowlist, which fails every call it doesn't allow with `ENOSYS`.
#[cfg(target_os = "linux")]
fn seccomp_filters() -> Result<[seccompiler::BpfProgram; 2], ConfineError> {
    use std::collections::BTreeMap;

    use seccompiler::{
        SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule,
    };
    let arch = seccompiler::TargetArch::try_from(std::env::consts::ARCH).map_err(|_| {
        ConfineError::SeccompArch {
            arch: std::env::consts::ARCH,
        }
    })?;
    let killed: BTreeMap<i64, Vec<SeccompRule>> = KILLED_CALLS
        .into_iter()
        .map(|call| (call, Vec::new()))
        .collect();
    // a call with rules is allowed when any rule matches, and a rule matches
    // when all its conditions hold; an int argument is compared at 32 bits,
    // the half the kernel reads
    let condition = |argument, length, op, value| {
        SeccompCondition::new(argument, length, op, value).map_err(ConfineError::SeccompFilter)
    };
    let rule = |conditions| SeccompRule::new(conditions).map_err(ConfineError::SeccompFilter);
    let each_value = |argument, values: &[u64]| {
        values
            .iter()
            .map(|value| {
                rule(vec![condition(
                    argument,
                    SeccompCmpArgLen::Dword,
                    SeccompCmpOp::Eq,
                    *value,
                )?])
            })
            .collect::<Result<Vec<_>, _>>()
    };
    let int = |value: libc::c_int| u64::from(value.unsigned_abs());
    // `prctl`'s option, the first argument, and `fcntl`'s command, the
    // second, ints
    let prctls = each_value(0, &KEPT_PRCTLS.map(int))?;
    let fcntls = each_value(1, &KEPT_FCNTLS.map(int))?;
    // `ioctl`'s request, the second argument, an unsigned int
    let ioctls = each_value(1, &KEPT_IOCTLS)?;
    // `socketpair`'s domain, the first argument, and its type, the second,
    // its flags masked off
    let [kept_domain, kept_type] = KEPT_SOCKETPAIR.map(int);
    let pairs = vec![rule(vec![
        condition(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, kept_domain)?,
        condition(
            1,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::MaskedEq(SOCKET_TYPE_MASK),
            kept_type,
        )?,
    ])?];
    // `clone`'s flags, the first argument, an unsigned long, compared whole:
    // a thread, and no namespace
    let thread = int(libc::CLONE_THREAD);
    let threads = vec![rule(vec![condition(
        0,
        SeccompCmpArgLen::Qword,
        SeccompCmpOp::MaskedEq(thread | int(CLONE_NAMESPACES)),
        thread,
    )?])?];
    let allowed: BTreeMap<i64, Vec<SeccompRule>> = ALLOWED_CALLS
        .iter()
        .map(|call| (*call, Vec::new()))
        .chain([
            (libc::SYS_prctl, prctls),
            (libc::SYS_fcntl, fcntls),
            (libc::SYS_ioctl, ioctls),
            (libc::SYS_socketpair, pairs),
            (libc::SYS_clone, threads),
        ])
        .collect();
    let compile = |rules, mismatch, matched| {
        SeccompFilter::new(rules, mismatch, matched, arch)
            .and_then(seccompiler::BpfProgram::try_from)
            .map_err(ConfineError::SeccompFilter)
    };
    Ok([
        compile(killed, SeccompAction::Allow, SeccompAction::KillProcess)?,
        compile(
            allowed,
            SeccompAction::Errno(libc::ENOSYS.unsigned_abs()),
            SeccompAction::Allow,
        )?,
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
        let [killing, allowing] = super::seccomp_filters().unwrap();
        assert!(!killing.is_empty());
        assert!(!allowing.is_empty());
    }

    /// The calls the allowlist allows whatever their arguments, listed apart
    /// from [`super::ALLOWED_CALLS`], so a call added or dropped there fails
    /// the tests: the calls the runtime was seen to make after the box goes
    /// up, then the named siblings.
    #[cfg(target_os = "linux")]
    fn allowed_calls() -> Vec<libc::c_long> {
        let mut expected = vec![
            libc::SYS_brk,
            libc::SYS_clock_gettime,
            libc::SYS_clock_nanosleep,
            libc::SYS_close,
            libc::SYS_epoll_create1,
            libc::SYS_epoll_ctl,
            libc::SYS_eventfd2,
            libc::SYS_exit,
            libc::SYS_exit_group,
            libc::SYS_futex,
            libc::SYS_getpid,
            libc::SYS_gettid,
            libc::SYS_lseek,
            libc::SYS_madvise,
            libc::SYS_mmap,
            libc::SYS_mprotect,
            libc::SYS_mremap,
            libc::SYS_munmap,
            libc::SYS_openat,
            libc::SYS_pkey_alloc,
            libc::SYS_pkey_mprotect,
            libc::SYS_read,
            libc::SYS_rseq,
            libc::SYS_rt_sigaction,
            libc::SYS_rt_sigprocmask,
            libc::SYS_sched_getaffinity,
            libc::SYS_sched_getparam,
            libc::SYS_sched_getscheduler,
            libc::SYS_sched_yield,
            libc::SYS_set_robust_list,
            libc::SYS_sigaltstack,
            libc::SYS_statx,
            libc::SYS_uname,
            libc::SYS_write,
            libc::SYS_getrandom,
            libc::SYS_epoll_pwait,
            libc::SYS_epoll_pwait2,
            libc::SYS_fstat,
            libc::SYS_newfstatat,
            libc::SYS_rt_sigreturn,
            libc::SYS_restart_syscall,
            libc::SYS_tgkill,
            libc::SYS_clock_getres,
            libc::SYS_gettimeofday,
            libc::SYS_nanosleep,
        ];
        #[cfg(target_arch = "x86_64")]
        expected.extend([libc::SYS_readlink, libc::SYS_epoll_wait]);
        #[cfg(target_arch = "aarch64")]
        expected.push(libc::SYS_readlinkat);
        expected
    }

    /// The calls the allowlist allows only by their arguments.
    #[cfg(target_os = "linux")]
    const RULED_CALLS: [libc::c_long; 5] = [
        libc::SYS_prctl,
        libc::SYS_fcntl,
        libc::SYS_ioctl,
        libc::SYS_socketpair,
        libc::SYS_clone,
    ];

    /// A sample of the calls the allowlist doesn't allow: every call the
    /// box once refused by name (the socket calls, the metadata calls, the
    /// watches, the System V and POSIX queue calls, and the calls that set
    /// a process's limits, priority, or scheduling), then the namespace,
    /// key, filter, process, and filesystem calls nothing in the runtime
    /// makes.
    #[cfg(target_os = "linux")]
    fn unlisted_calls() -> Vec<libc::c_long> {
        vec![
            libc::SYS_socket,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_accept4,
            libc::SYS_sendto,
            libc::SYS_sendmsg,
            libc::SYS_fchmod,
            libc::SYS_fchmodat,
            // `fchmodat2`, `setxattrat`, `removexattrat`, and `file_setattr`,
            // by their numbers in the table x86_64 and aarch64 share from 424
            452,
            463,
            466,
            469,
            libc::SYS_fchown,
            libc::SYS_fchownat,
            libc::SYS_utimensat,
            libc::SYS_setxattr,
            libc::SYS_lsetxattr,
            libc::SYS_fsetxattr,
            libc::SYS_removexattr,
            libc::SYS_lremovexattr,
            libc::SYS_fremovexattr,
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
            libc::SYS_prlimit64,
            libc::SYS_setpriority,
            libc::SYS_ioprio_set,
            libc::SYS_sched_setaffinity,
            libc::SYS_sched_setscheduler,
            libc::SYS_sched_setparam,
            libc::SYS_sched_setattr,
            libc::SYS_clone3,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_keyctl,
            libc::SYS_add_key,
            libc::SYS_request_key,
            libc::SYS_seccomp,
            libc::SYS_landlock_create_ruleset,
            libc::SYS_landlock_restrict_self,
            libc::SYS_kill,
            libc::SYS_tkill,
            libc::SYS_pidfd_open,
            libc::SYS_pidfd_send_signal,
            libc::SYS_process_madvise,
            libc::SYS_personality,
            libc::SYS_setuid,
            libc::SYS_capset,
            libc::SYS_flock,
            libc::SYS_mount,
            libc::SYS_pivot_root,
            libc::SYS_chdir,
            libc::SYS_getcwd,
            libc::SYS_getdents64,
            libc::SYS_pipe2,
            libc::SYS_memfd_create,
            libc::SYS_mknodat,
            libc::SYS_mkdirat,
            libc::SYS_unlinkat,
            libc::SYS_renameat2,
            libc::SYS_linkat,
            libc::SYS_symlinkat,
            libc::SYS_ftruncate,
            libc::SYS_fallocate,
            libc::SYS_name_to_handle_at,
            libc::SYS_open_by_handle_at,
            libc::SYS_pread64,
            libc::SYS_readv,
            libc::SYS_writev,
            libc::SYS_set_tid_address,
            libc::SYS_close_range,
            libc::SYS_clock_settime,
            libc::SYS_reboot,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_chmod,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_chown,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_lchown,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_utime,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_utimes,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_futimesat,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_open,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_fork,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_vfork,
            #[cfg(target_arch = "x86_64")]
            libc::SYS_readlinkat,
        ]
    }

    /// The allowlist is the list above, plus the calls allowed by their
    /// arguments, each with the values pinned here; none is a killed call,
    /// and the sample of unlisted calls is unlisted.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_allowlist_is_the_census() {
        let mut allowed = super::ALLOWED_CALLS.to_vec();
        let mut expected = allowed_calls();
        allowed.sort_unstable();
        expected.sort_unstable();
        assert_eq!(allowed, expected);
        allowed.dedup();
        assert_eq!(
            allowed.len(),
            super::ALLOWED_CALLS.len(),
            "a call listed twice"
        );
        for call in super::KILLED_CALLS.into_iter().chain(RULED_CALLS) {
            assert!(!allowed.contains(&call), "{call}");
        }
        for call in unlisted_calls() {
            assert!(!allowed.contains(&call), "{call}");
            assert!(!RULED_CALLS.contains(&call), "{call}");
        }
        assert_eq!(super::KEPT_PRCTLS, [libc::PR_SET_NAME, libc::PR_SET_VMA]);
        assert_eq!(
            super::KEPT_FCNTLS,
            [
                libc::F_GETFD,
                libc::F_SETFD,
                libc::F_GETFL,
                libc::F_SETFL,
                libc::F_DUPFD_CLOEXEC,
            ]
        );
        assert_eq!(super::KEPT_SOCKETPAIR, [libc::AF_UNIX, libc::SOCK_STREAM]);
    }

    /// The `ioctl` requests kept are the ones std, tokio, and the console
    /// make, and the allowlist compares the request with each; the
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
        let [_, allowing] = super::seccomp_filters().unwrap();
        for request in super::KEPT_IOCTLS {
            assert!(
                allowing
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

    /// The filters decide each call as the policy says, read as the kernel
    /// reads them. The allowlist allows every listed call, and the calls
    /// allowed by their arguments only with the kept values, an int's upper
    /// half, which the kernel drops, changing nothing; every other call, an
    /// x32 call among them, fails with `ENOSYS`. The killing filter allows
    /// all of those and kills on its own calls, and both kill a call from
    /// another architecture.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_filters_decide_by_the_allowlist() {
        const ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
        const ENOSYS: u32 = libc::SECCOMP_RET_ERRNO | libc::ENOSYS.unsigned_abs();
        const KILL: u32 = libc::SECCOMP_RET_KILL_PROCESS;
        let [killing, allowing] = super::seccomp_filters().unwrap();
        let arch = audit_arch();
        let high = 0xdead_0000_0000_0000_u64;
        let int = |value: libc::c_int| u64::from(value.unsigned_abs());
        let unix = int(libc::AF_UNIX);
        let [stream, datagram, seqpacket, cloexec, nonblock] = [
            libc::SOCK_STREAM,
            libc::SOCK_DGRAM,
            libc::SOCK_SEQPACKET,
            libc::SOCK_CLOEXEC,
            libc::SOCK_NONBLOCK,
        ]
        .map(int);
        // glibc's flags for a thread, and for `fork` and `posix_spawn`
        let thread = int(libc::CLONE_VM
            | libc::CLONE_FS
            | libc::CLONE_FILES
            | libc::CLONE_SIGHAND
            | libc::CLONE_THREAD
            | libc::CLONE_SYSVSEM
            | libc::CLONE_SETTLS
            | libc::CLONE_PARENT_SETTID
            | libc::CLONE_CHILD_CLEARTID);
        let sigchld = int(libc::SIGCHLD);
        let fork = int(libc::CLONE_CHILD_SETTID | libc::CLONE_CHILD_CLEARTID) | sigchld;
        let spawn = int(libc::CLONE_VM | libc::CLONE_VFORK) | sigchld;
        let mut cases = vec![
            // a thread, also with an upper half the kernel drops from
            // `clone`'s flags; a fork, a spawn, and a namespace refused
            (libc::SYS_clone, [thread, 0, 0, 0, 0, 0], ALLOW),
            (
                libc::SYS_clone,
                [int(libc::CLONE_THREAD), 0, 0, 0, 0, 0],
                ALLOW,
            ),
            (libc::SYS_clone, [high | thread, 0, 0, 0, 0, 0], ALLOW),
            (libc::SYS_clone, [fork, 0, 0, 0, 0, 0], ENOSYS),
            (libc::SYS_clone, [spawn, 0, 0, 0, 0, 0], ENOSYS),
            (libc::SYS_clone, [0, 0, 0, 0, 0, 0], ENOSYS),
            (
                libc::SYS_clone,
                [int(libc::CLONE_NEWUSER | libc::CLONE_THREAD), 0, 0, 0, 0, 0],
                ENOSYS,
            ),
            // the kept prctls, also with an upper half, and others refused
            (
                libc::SYS_prctl,
                [int(libc::PR_SET_NAME), 0, 0, 0, 0, 0],
                ALLOW,
            ),
            (
                libc::SYS_prctl,
                [high | int(libc::PR_SET_NAME), 0, 0, 0, 0, 0],
                ALLOW,
            ),
            (
                libc::SYS_prctl,
                [int(libc::PR_SET_VMA), 0, 0, 0, 0, 0],
                ALLOW,
            ),
            // the kept fcntls, also with an upper half, and others refused
            (libc::SYS_fcntl, [3, int(libc::F_GETFD), 0, 0, 0, 0], ALLOW),
            (
                libc::SYS_fcntl,
                [3, high | int(libc::F_SETFL), 0, 0, 0, 0],
                ALLOW,
            ),
            // the kept ioctls, also with an upper half, and others refused
            (
                libc::SYS_ioctl,
                [2, high | libc::FIONBIO, 0, 0, 0, 0],
                ALLOW,
            ),
            (libc::SYS_ioctl, [2, libc::TCGETS, 0, 0, 0, 0], ALLOW),
            (
                libc::SYS_ioctl,
                [2, high | libc::FS_IOC_SETFLAGS, 0, 0, 0, 0],
                ENOSYS,
            ),
            (
                libc::SYS_ioctl,
                [2, libc::FS_IOC_GETFLAGS, 0, 0, 0, 0],
                ENOSYS,
            ),
            (libc::SYS_ioctl, [2, libc::TIOCSTI, 0, 0, 0, 0], ENOSYS),
            (libc::SYS_ioctl, [2, 0, 0, 0, 0, 0], ENOSYS),
            // the kept socketpair, with its flags, and others refused
            (libc::SYS_socketpair, [unix, stream, 0, 0, 0, 0], ALLOW),
            (
                libc::SYS_socketpair,
                [unix, stream | cloexec | nonblock, 0, 0, 0, 0],
                ALLOW,
            ),
            (libc::SYS_socketpair, [unix, datagram, 0, 0, 0, 0], ENOSYS),
            (libc::SYS_socketpair, [unix, seqpacket, 0, 0, 0, 0], ENOSYS),
            (
                libc::SYS_socketpair,
                [int(libc::AF_INET), stream, 0, 0, 0, 0],
                ENOSYS,
            ),
        ];
        for namespace in [
            libc::CLONE_NEWNS,
            libc::CLONE_NEWCGROUP,
            libc::CLONE_NEWUTS,
            libc::CLONE_NEWIPC,
            libc::CLONE_NEWUSER,
            libc::CLONE_NEWPID,
            libc::CLONE_NEWNET,
        ] {
            cases.push((
                libc::SYS_clone,
                [thread | int(namespace), 0, 0, 0, 0, 0],
                ENOSYS,
            ));
        }
        for option in [
            libc::PR_SET_DUMPABLE,
            libc::PR_SET_NO_NEW_PRIVS,
            libc::PR_SET_SECCOMP,
            libc::PR_SET_PTRACER,
            libc::PR_SET_PDEATHSIG,
            libc::PR_CAP_AMBIENT,
            libc::PR_SET_MM,
            0,
        ] {
            cases.push((libc::SYS_prctl, [int(option), 0, 0, 0, 0, 0], ENOSYS));
        }
        for command in super::KEPT_FCNTLS {
            cases.push((libc::SYS_fcntl, [3, int(command), 0, 0, 0, 0], ALLOW));
        }
        for command in [
            libc::F_DUPFD,
            libc::F_SETLK,
            libc::F_SETLKW,
            libc::F_GETLK,
            libc::F_OFD_SETLK,
            libc::F_SETLEASE,
            libc::F_GETLEASE,
            libc::F_NOTIFY,
            libc::F_SETOWN,
            libc::F_SETPIPE_SZ,
            libc::F_ADD_SEALS,
        ] {
            cases.push((libc::SYS_fcntl, [3, int(command), 0, 0, 0, 0], ENOSYS));
            cases.push((
                libc::SYS_fcntl,
                [3, high | int(command), 0, 0, 0, 0],
                ENOSYS,
            ));
        }
        for request in super::KEPT_IOCTLS {
            cases.push((libc::SYS_ioctl, [2, request, 0, 0, 0, 0], ALLOW));
        }
        for call in allowed_calls() {
            cases.push((call, [0; 6], ALLOW));
        }
        for call in unlisted_calls() {
            cases.push((call, [0; 6], ENOSYS));
        }
        // an x32 call, the x86_64 call number with bit 30 set, matches no
        // number the filters compare
        #[cfg(target_arch = "x86_64")]
        for call in allowed_calls().into_iter().chain(RULED_CALLS) {
            cases.push((call | 0x4000_0000, [0; 6], ENOSYS));
        }
        for (call, args, expected) in cases {
            assert_eq!(
                decide(&allowing, arch, call, args),
                expected,
                "{call} {args:x?}"
            );
            assert_eq!(
                decide(&killing, arch, call, args),
                ALLOW,
                "{call} {args:x?}"
            );
        }
        for call in super::KILLED_CALLS {
            assert_eq!(decide(&killing, arch, call, [0; 6]), KILL, "{call}");
            assert_eq!(decide(&allowing, arch, call, [0; 6]), ENOSYS, "{call}");
            // by its x32 number, past the killing filter, the allowlist
            // still refuses it
            #[cfg(target_arch = "x86_64")]
            {
                assert_eq!(decide(&killing, arch, call | 0x4000_0000, [0; 6]), ALLOW);
                assert_eq!(decide(&allowing, arch, call | 0x4000_0000, [0; 6]), ENOSYS);
            }
        }
        for filter in [&killing, &allowing] {
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
