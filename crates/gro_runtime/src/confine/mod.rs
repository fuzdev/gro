//! Confinement: the boxes a process is held in, and the type state that
//! gates the engine on them.
//!
//! A process is confined or explicitly unconfined before the engine starts:
//! [`engine::init_engine`](crate::engine::init_engine) takes a [`Confinement`],
//! and its token ([`Engine`](crate::engine::Engine)) names which. A process
//! is held in one of two boxes: eval's, read-only, whose states are here, and
//! the run's, which spawns and writes, whose states are [`run`]'s. Eval's
//! states:
//!
//! - [`EvalPrepared`]: the box's roots, computed before anything is confined
//!   ([`EvalPrepared::for_project`]): the project's real directory, and the link
//!   closure below it, each opened as it is found, so the box goes up on the
//!   directories found, not on whatever their paths name later.
//! - [`EvalConfined`]: the process passed through [`EvalPrepared::confine`], made only
//!   there, on the process's one thread, for an engine with the JIT on or
//!   off ([`Jit`]), which the box records and the engine is checked against.
//!
//! The run's ends in [`run::RunConfined`], made only by
//! [`run::RunPrepared::confine`], the box every command of Gro's and `fgro run` run in
//! (`gro_cli`'s `start`). [`Confinement`] names one of the two, or
//! [`Confinement::Unconfined`]: named by the caller, for a process that
//! holds no box (`fgro run` in none, the snapshot builder, and tests).
//!
//! The rest of this doc is eval's box; [`run`]'s module doc has the run's.
//! A box must hold every thread the process has, so it is installed while
//! the process has one: what resolves eval's roots runs on a thread joined
//! before [`EvalPrepared::confine`], which checks the count first and refuses
//! otherwise, and every thread the engine starts comes after it, spawned from
//! the thread that installed it.
//!
//! The box, installed by [`EvalPrepared::confine`] in this order on the
//! calling thread, after the thread count is checked:
//!
//! 1. **`no_new_privs`**: nothing the process executes can gain privileges.
//! 2. **Landlock**, at ABI 6 (Linux 6.12) or later, with every rule
//!    enforced in full, or the confinement fails. The filesystem: of every
//!    access right ABI 6 knows, only reading files and listing directories,
//!    and only beneath the box's roots, named by the handles opened when
//!    each was found, so nothing is written or executed anywhere and
//!    nothing outside the roots is read (`/proc` and `/dev` included). The
//!    network: binding and connecting TCP sockets refused. The scopes: no
//!    signal to a process outside the box, and no connection to an abstract
//!    unix socket outside it. A kernel without Landlock, or with it below
//!    ABI 6, is refused, naming what is missing and the kernel's ABI.
//! 3. **seccomp**, two filters on the calling thread (no thread sync, since it
//!    is the process's only thread), their tables in `seccomp.rs`. The first
//!    is a tripwire: it kills the process on the calls no part of the runtime
//!    makes and code that escaped the engine would (`KILLED_CALLS`). The
//!    second is an allowlist: the calls the runtime makes after the box goes
//!    up, and their named siblings, pass (`ALLOWED_CALLS`); a few pass only
//!    with the arguments the runtime uses (`KEPT_PRCTLS`, `KEPT_FCNTLS`,
//!    `KEPT_IOCTLS`, `KEPT_SOCKETPAIR`, and a `clone` that starts a thread,
//!    setting no flag in `CLONE_NAMESPACES`); and the mapping calls
//!    (`MAPPING_CALLS`) pass whatever their arguments with the JIT on, whose
//!    compiled code needs executable memory, and with it off only when their
//!    `prot` has `PROT_EXEC` clear, so once the box is up a jitless process
//!    makes no mapping executable (what it mapped before, its own code and its
//!    libraries', stays executable). Every other call fails with `ENOSYS`, an
//!    argument outside its rules included. `ENOSYS` is what a call the kernel
//!    lacks returns, so glibc and std fall back where they can (`clone3` to
//!    `clone`, `statx` to `fstat`), and a host making a call the list doesn't
//!    name degrades where its libc has a fallback, and otherwise fails with
//!    `ENOSYS`. The filter compares exact call numbers, so an x32 call
//!    (`nr | 0x40000000`) matches nothing and fails too, and since the
//!    allowlist doesn't allow `seccomp`, no filter can be added after it.
//!    Written for `x86_64` and `aarch64`, but only `x86_64` builds (its V8
//!    archive is the one pinned) and is checked; the `aarch64` arms are never
//!    compiled. A library loaded after the box goes up (by `dlopen`, which
//!    nothing in the runtime is known to call) fails when its file is outside
//!    the roots, which Landlock refuses, or jitless, when the rule refuses its
//!    code's `PROT_EXEC` mapping; a library inside the roots loads under
//!    `--jit`.
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
//! Each root is opened as it is found (`O_PATH`, with
//! `O_DIRECTORY` and `O_NOFOLLOW`), and kept only when the kernel names the
//! handle by the root's real path, so a directory swapped in after the
//! closure read it isn't what the box admits. Its Landlock rule reads beneath
//! that handle: Landlock's rules are [`rules::BoxRules`]'s, each a handle and
//! the rights it grants, opened by real path or beneath a directory's
//! descriptor with every symlink refused (`rules.rs`'s module doc).

use std::io;
use std::path::{Path, PathBuf};

mod closure;
mod facts;
pub mod git;
mod landlock;
pub mod plan;
pub mod program;
pub mod rules;
pub mod run;
pub mod seccomp;
#[cfg(test)]
mod tests;
pub mod tool_dirs;

use closure::link_closure;
use rules::{BoxRules, open_real_dir};

/// What a confined read may reach, each a real path, a directory beneath
/// it and a file only itself.
///
/// Eval's box: the project's directory first, then each link closure root.
/// The reads made before an `fgro run` entry in no project's box goes up
/// (`loader::prepass`): its directory, the type markers above it, and the
/// directories `--allow-read=` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxRoots(Vec<PathBuf>);

impl BoxRoots {
    /// Roots as given, each a real path.
    pub(crate) const fn new(roots: Vec<PathBuf>) -> Self {
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
pub struct EvalPrepared {
    /// What the box admits.
    roots: BoxRoots,
    /// Landlock's rules: reading beneath each root, in the roots' order, on
    /// an `O_PATH` handle to its directory opened before anything is
    /// confined ([`open_real_dir`]).
    rules: BoxRules,
}

impl EvalPrepared {
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
        let pinned = open_real_dir(&project)?;
        let (roots, pins) = link_closure(vec![(project, pinned)]);
        let rules = BoxRules::read_dirs(roots.iter().cloned().zip(pins));
        Ok(Self {
            roots: BoxRoots(roots),
            rules,
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
    /// read, before anything is installed; or an error naming the wall the
    /// kernel can't hold (checked before any is installed) or that failed to
    /// install, after which the process may hold part of the box and must run
    /// nothing it was to confine.
    pub fn confine(self, jit: Jit) -> Result<EvalConfined, ConfineError> {
        alone()?;
        install(&self, jit)?;
        Ok(EvalConfined {
            roots: self.roots,
            jit,
        })
    }
}

/// Whether the engine a box holds runs V8's JIT, which the box is installed
/// for: off, no mapping made or changed after the box goes up by `mmap`,
/// `mprotect`, or `pkey_mprotect` may be executable.
///
/// The engine takes the same choice (`EngineOptions::jit`), and a confined
/// engine whose choice disagrees with its box's is refused before it starts
/// (`engine::init_engine`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jit {
    /// Jitless: JS runs in V8's interpreter alone, and the box refuses
    /// executable mappings.
    Off,
    /// The JIT on: its compiled code needs executable mappings, which the
    /// box allows.
    On,
}

/// The process confined to its box, from the thread that installed it.
/// Made only by [`EvalPrepared::confine`].
#[derive(Debug, PartialEq, Eq)]
pub struct EvalConfined {
    /// What the box admits.
    roots: BoxRoots,
    /// Whether the box was installed for an engine with the JIT on.
    jit: Jit,
}

impl EvalConfined {
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

/// How a run's box holds the network, which decides what sockets its
/// seccomp denylist lets a process make
/// (`seccomp::denylist::ArgRule::Socket`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetForm {
    /// In a network namespace of the run's own, whose only interface is its
    /// own loopback: internet sockets are made freely and reach nothing
    /// outside the run.
    Namespace,
    /// In the host's network, where user namespaces are refused: Landlock
    /// refuses binding and connecting TCP sockets, and the denylist refuses
    /// every socket but a pair, so no UDP socket or listener is made either.
    InetRefused,
}

/// The process has more than one thread where a step of a box needs it.
///
/// It's the one shape of that refusal, at every step of either box that
/// checks, which a caller exits 1 for, a failed step of the box.
///
/// Only at the run's first step (`run::RunClosed::close_inherited`) does
/// the message name a library preloaded into the process as a likely
/// cause: past it, another thread is one not yet gone after its join, or
/// gro's own bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "the process has {count} threads, and a kernel box goes up only while it has one, since it \
     would leave the others outside{}",
    if *.first_step {
        ": a library preloaded into the process (LD_PRELOAD) may have started one"
    } else {
        ""
    }
)]
pub struct Threads {
    /// How many threads it has.
    pub count: u64,
    /// Whether it's the run's first step that found them.
    pub first_step: bool,
}

/// Whether a process is confined, and in which box, which the engine
/// takes to start.
#[derive(Debug, PartialEq, Eq)]
pub enum Confinement {
    /// Held in eval's box.
    Eval(EvalConfined),
    /// Held in the run's box.
    Run(run::RunConfined),
    /// Held in no box, by the caller's choice.
    Unconfined,
}

/// Why a box couldn't go up: eval's ([`EvalPrepared::confine`]) or the run's
/// ([`run::RunPrepared::confine`]).
///
/// A variant wrapping the `landlock` or `seccompiler` crate's error, or the
/// denylist's, writes that error into its message and names no source:
/// those crates' messages already hold their own causes, and the
/// denylist's has none, so a reader that follows the chain of sources
/// would print each cause again.
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    /// The process has more than one thread, so a box installed now would
    /// leave the others outside it.
    #[error(transparent)]
    Threads(Threads),
    /// The process's thread count couldn't be read.
    #[error(transparent)]
    ThreadsUnread(io::Error),
    /// The process holds a socket above stderr, which the run's box would
    /// leave on the network it was made in: its walls gate only sockets made
    /// after them.
    #[error(
        "the process holds {}, which the box's network walls wouldn't reach: the run's box goes \
         up holding no socket above stderr",
        held("a socket", "sockets", .fds)
    )]
    SocketHeld {
        /// The sockets' descriptors, in ascending order.
        fds: Vec<i32>,
    },
    /// A socket on stdin, stdout, or stderr isn't a connected unix stream:
    /// an internet socket can be disconnected and reconnected or listen, an
    /// unconnected or listening socket reaches the network it was made in,
    /// and a datagram one any address, a unix one any socket file by its
    /// path, past the run's box.
    #[error(
        "{}, which reaches past the box's walls: the run's box takes only a connected unix \
         stream on stdio, which reaches its peer alone; give it a file, a pipe, or /dev/null",
        stdio(.fds)
    )]
    StdioSocket {
        /// The descriptors, of 0, 1, and 2, in ascending order.
        fds: Vec<i32>,
    },
    /// The process holds a descriptor that holds other files out of the
    /// box's sight and acts on them with the credentials it was made with:
    /// an `io_uring` instance.
    #[error(
        "the process holds {}, which acts on files it registered with the credentials it was \
         made with, past the box: the run's box goes up holding none",
        held("an io_uring", "io_urings", .fds)
    )]
    HidingHeld {
        /// The descriptors, in ascending order.
        fds: Vec<i32>,
    },
    /// The process's descriptors couldn't be listed and read.
    #[error("failed to read the process's descriptors before the box")]
    DescriptorsUnread(#[source] io::Error),
    /// The kernel has no Landlock, or has it switched off.
    #[error(
        "the kernel {}, and the box's filesystem, network, and scope walls are Landlock's: \
         the box needs Landlock ABI {REQUIRED_LANDLOCK_ABI} (Linux 6.12) or later",
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
    #[error("failed to make or install the box's Landlock rules: {0}")]
    Landlock(::landlock::RulesetError),
    /// The kernel installed only part of the Landlock rules.
    #[error("the kernel enforced the box's Landlock rules only in part, so none can be relied on")]
    LandlockPartial,
    /// The seccomp filters can't be built for this architecture: eval's are
    /// written for `x86_64` and `aarch64`, the run's denylist for `x86_64`.
    #[error("the box's seccomp filters can't be built for the {arch} architecture")]
    SeccompArch {
        /// The architecture, as Rust names it.
        arch: &'static str,
    },
    /// The seccomp filters couldn't be compiled.
    #[error("failed to compile the box's seccomp filters: {0}")]
    SeccompFilter(seccompiler::BackendError),
    /// The run's seccomp denylist couldn't be assembled from its table.
    #[error("failed to assemble the run's seccomp denylist: {0}")]
    Denylist(seccomp::tree::TreeError),
    /// The kernel refused the seccomp filters.
    #[error("failed to install the box's seccomp filters: {0}")]
    Seccomp(seccompiler::Error),
}

impl ConfineError {
    /// Whether the host can't hold the box (no Landlock, an older one, an
    /// architecture the seccomp filters aren't built for, a kernel refusing
    /// them), as opposed to the process breaking the order
    /// the box needs or failing to build it.
    pub const fn is_host_unsupported(&self) -> bool {
        match self {
            Self::NoLandlock { .. }
            | Self::LandlockAbi { .. }
            | Self::SeccompArch { .. }
            | Self::Seccomp(_) => true,
            Self::Landlock(_)
            | Self::SeccompFilter(_)
            | Self::Denylist(_)
            | Self::Threads(_)
            | Self::ThreadsUnread(_)
            | Self::SocketHeld { .. }
            | Self::StdioSocket { .. }
            | Self::HidingHeld { .. }
            | Self::DescriptorsUnread(_)
            | Self::LandlockPartial => false,
        }
    }
}

/// `one` or `many` with the descriptors `fds` named, for a message:
/// `a socket (descriptor 5)`, `sockets (descriptors 5 and 7)`.
fn held(one: &str, many: &str, fds: &[i32]) -> String {
    match fds {
        [fd] => format!("{one} (descriptor {fd})"),
        _ => format!(
            "{many} (descriptors {})",
            listed(fds.iter().map(ToString::to_string))
        ),
    }
}

/// The standard streams `fds` name, each a socket that isn't a connected
/// stream, for a message.
fn stdio(fds: &[i32]) -> String {
    let names = fds.iter().map(|fd| match fd {
        0 => "stdin".to_owned(),
        1 => "stdout".to_owned(),
        2 => "stderr".to_owned(),
        fd => format!("descriptor {fd}"),
    });
    let what = if fds.len() == 1 {
        "is a socket that isn't a connected unix stream"
    } else {
        "are sockets that aren't connected unix streams"
    };
    format!("{} {what}", listed(names))
}

/// `items` joined for prose: `a`, `a and b`, `a, b, and c`.
fn listed(items: impl IntoIterator<Item = String>) -> String {
    let items: Vec<String> = items.into_iter().collect();
    match items.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// The Landlock ABI the box requires, for its scope walls: the number of
/// the ABI its rules are written for, for the messages.
pub const REQUIRED_LANDLOCK_ABI: u32 = landlock::LANDLOCK_ABI as u32;

/// Installs the box `prepared`, for an engine whose JIT is `jit`, on the
/// calling thread, the process's only one.
fn install(prepared: &EvalPrepared, jit: Jit) -> Result<(), ConfineError> {
    use crate::prices::{self, EvalDefault};
    // a priced build can leave either wall out, to time it (`prices`)
    //
    // the filters are compiled first, so one that can't be built fails
    // before any wall goes up
    let filters = if prices::off(EvalDefault::Seccomp) {
        None
    } else {
        Some(seccomp::seccomp_filters(jit)?)
    };
    if !prices::off(EvalDefault::Landlock) {
        landlock::landlock(&prepared.rules, landlock::TcpWall::Refused)?;
    }
    // the killing filter is installed first, so the allowlist's own
    // `seccomp` call passes it; the allowlist doesn't allow `seccomp`, so it
    // is the last filter the process can take
    for filter in filters.iter().flatten() {
        seccompiler::apply_filter(filter).map_err(ConfineError::Seccomp)?;
    }
    Ok(())
}

/// Checks that the process has one thread, the caller, once a thread just
/// joined has left the count (`gro_sys::settled_thread_count`).
fn alone() -> Result<(), ConfineError> {
    let count = gro_sys::settled_thread_count().map_err(ConfineError::ThreadsUnread)?;
    if count == 1 {
        Ok(())
    } else {
        Err(ConfineError::Threads(Threads {
            count,
            first_step: false,
        }))
    }
}
