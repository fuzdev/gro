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
//!   there, on the process's one thread, for an engine with the JIT on or
//!   off ([`Jit`]), which the box records and the engine is checked against.
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
//!    would (`KILLED_CALLS`: `execve` and `execveat`, `ptrace`, the
//!    `process_vm_readv` and `process_vm_writev`, `bpf`, `perf_event_open`, `userfaultfd`,
//!    the `kexec` calls, the `io_uring` calls, whose operations bypass
//!    seccomp, and the kernel module calls). The second is an allowlist:
//!    the calls the runtime makes after the box goes up, and their named
//!    siblings (`ALLOWED_CALLS`), pass; `prctl`, `fcntl`, `ioctl`,
//!    `socketpair`, `faccessat2`, and `clone` pass only with the arguments
//!    the runtime uses (`KEPT_PRCTLS`, `KEPT_FCNTLS`, `KEPT_IOCTLS`,
//!    `KEPT_SOCKETPAIR`, `KEPT_ACCESS_MODE`, and a `clone` that starts a
//!    thread, setting no flag in `CLONE_NAMESPACES`); the mapping calls
//!    (`MAPPING_CALLS`: `mmap`, `mprotect`, and `pkey_mprotect`) pass
//!    whatever their arguments with the JIT on, whose compiled code needs
//!    executable memory, and with it off only when their `prot` has
//!    `PROT_EXEC` clear, so once the box is up a jitless process makes no
//!    mapping executable:
//!    what it mapped before, its own code and its libraries', stays
//!    executable, and `mremap`, which keeps a mapping's protection, can
//!    still move, copy, or grow those mappings over their files' own bytes;
//!    every other call fails with `ENOSYS`,
//!    an argument outside its rules included. `ENOSYS` is what a call the
//!    kernel lacks returns, so glibc and std fall back where they can
//!    (`clone3` to `clone`, `statx` to `fstat`), and a host making a call
//!    the list doesn't name degrades where its libc has a fallback, and
//!    otherwise fails with `ENOSYS`. The filter
//!    compares exact call numbers, so an x32 call (`nr | 0x40000000`)
//!    matches nothing and fails too, and since the allowlist doesn't allow
//!    `seccomp`, no filter can be added after it. Built for `x86_64` and
//!    `aarch64`; the `aarch64` list follows that architecture's call table
//!    and is unverified on a host. A library loaded after the box goes up
//!    (by `dlopen`, which nothing in the runtime is known to call) fails on
//!    either architecture when its file is outside the roots, which Landlock
//!    refuses, or jitless, when the rule refuses its code's `PROT_EXEC`
//!    mapping; a library inside the roots loads under `--jit`.
//!
//!    The `READ_IMPLIES_EXEC` personality, which would make every readable
//!    mapping executable past the rule, can't be inherited: the kernel
//!    clears it when a 64-bit binary executes (checked on `x86_64`:
//!    `setarch -X` leaves `/proc/self/personality` without it), and the
//!    allowlist refuses `personality`.
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

mod closure;
#[cfg(target_os = "linux")]
mod landlock;
#[cfg(target_os = "linux")]
mod seccomp;
#[cfg(test)]
mod tests;

use closure::{link_closure, pin};

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
    /// doc describes, for an engine whose JIT is `jit`: off, the box refuses
    /// executable mappings. The box goes up once: this takes the roots'
    /// handles, which are closed once its rules name them.
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
    pub fn confine(self, jit: Jit) -> Result<Confined, ConfineError> {
        alone()?;
        install(&self, jit)?;
        Ok(Confined {
            roots: self.roots,
            jit,
        })
    }
}

/// Whether the engine a box holds runs V8's JIT, which the box is installed
/// for: off, no mapping made or changed after the box goes up by `mmap`,
/// `mprotect`, or `pkey_mprotect` may be executable.
///
/// The engine takes the same choice as a flag (`EngineOptions::jitless`),
/// and a confined engine whose flag disagrees with its box's [`Jit`] is
/// refused before it starts (`engine::init_engine`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jit {
    /// Jitless: JS runs in V8's interpreter alone, and the box refuses
    /// executable mappings.
    Off,
    /// The JIT on: its compiled code needs executable mappings, which the
    /// box allows.
    On,
}

impl Jit {
    /// The JIT as an engine's `jitless` flag says.
    pub const fn from_jitless(jitless: bool) -> Self {
        if jitless { Self::Off } else { Self::On }
    }
}

/// The process confined to its box, from the thread that installed it.
/// Made only by [`Prepared::confine`].
#[derive(Debug, PartialEq, Eq)]
pub struct Confined {
    /// What the box admits.
    roots: BoxRoots,
    /// Whether the box was installed for an engine with the JIT on.
    jit: Jit,
}

impl Confined {
    /// A confinement no box backs, for a test of what checks one.
    #[cfg(test)]
    pub(crate) const fn for_test(roots: BoxRoots, jit: Jit) -> Self {
        Self { roots, jit }
    }

    /// The roots the box admits.
    pub const fn roots(&self) -> &BoxRoots {
        &self.roots
    }

    /// Whether the box was installed for an engine with the JIT on, which
    /// decides whether it allows executable mappings.
    pub const fn jit(&self) -> Jit {
        self.jit
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
        count: u64,
    },
    /// The process's thread count couldn't be read.
    #[error(transparent)]
    ThreadsUnread(io::Error),
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
    Landlock(#[source] ::landlock::RulesetError),
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

/// The Landlock ABI the box requires, for its scope walls: [`LANDLOCK_ABI`](landlock::LANDLOCK_ABI)'s
/// number, for the messages, which every platform builds.
const REQUIRED_LANDLOCK_ABI: u32 = 6;

/// Installs the box `prepared`, for an engine whose JIT is `jit`, on the
/// calling thread, the process's only one.
#[cfg(target_os = "linux")]
fn install(prepared: &Prepared, jit: Jit) -> Result<(), ConfineError> {
    use crate::prices::{self, EvalDefault};
    // a priced build can leave either wall out, to time it (`prices`)
    // compiled first, so a filter that can't be built fails before any wall
    let filters = if prices::off(EvalDefault::Seccomp) {
        None
    } else {
        Some(seccomp::seccomp_filters(jit)?)
    };
    if !prices::off(EvalDefault::Landlock) {
        landlock::landlock(&prepared.pins)?;
    }
    // the killing filter first, so it passes the allowlist's own `seccomp`
    // call; the allowlist doesn't allow `seccomp`, so it is the last filter
    // the process can take
    for filter in filters.iter().flatten() {
        seccompiler::apply_filter(filter).map_err(ConfineError::Seccomp)?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
const fn install(_prepared: &Prepared, _jit: Jit) -> Result<(), ConfineError> {
    Err(ConfineError::Unsupported)
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
        count = gro_sys::thread_count().map_err(ConfineError::ThreadsUnread)?;
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
