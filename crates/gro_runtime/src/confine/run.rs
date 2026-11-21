//! The run's box: the steps a run's process takes before its box goes up,
//! and the box installed.
//!
//! Each step consumes the token the step before it made and makes the next,
//! so the order is the compiler's to hold, and the last,
//! [`RunPrepared::confine`], makes the [`RunConfined`] the engine takes
//! ([`Confinement::Run`](super::Confinement::Run)). On the process's main
//! thread, its only one:
//!
//! 1. [`RunClosed::close_inherited`]: every descriptor above stderr the
//!    process inherited is closed, since Landlock gates only what's opened
//!    after it, and a socket made before the namespace keeps the network it
//!    was made in.
//! 2. [`RunClosed::enter_network`]: the run's own user and network
//!    namespace entered, its loopback brought up, and user namespaces
//!    nested in it refused (`gro_sys::namespace`), so a test binds and
//!    connects to `127.0.0.1` and reaches nothing outside the run
//!    ([`RunNetwork::Namespace`]). Where the kernel refuses the namespaces
//!    with nothing changed, the run takes the host's network with every
//!    internet socket refused, keeping the refusal's cause
//!    ([`RunNetwork::InetRefused`]); a process left in a namespace whose
//!    ids weren't all mapped fails ([`RunStepError::Stranded`]).
//! 3. [`RunNetworked::harden`]: every capability dropped
//!    (`gro_sys::capabilities`), after the loopback and the nested refusal,
//!    which take capabilities the namespace gives; the process marked not
//!    dumpable, after the namespace, whose maps a process so marked can't
//!    write; its `core` hard limit lowered to 0; and its open-files soft
//!    limit raised to the hard one where it can be, as Node raises it,
//!    which a program the run starts inherits.
//! 4. [`RunHardened::prepare`]: the Landlock rules ([`BoxRules`]), opened
//!    by the caller once the process is hardened, after whatever it makes
//!    for the run beneath the project, so each rule names what it made.
//! 5. [`RunPrepared::confine`]: the thread count checked, then the
//!    process's descriptors (`gro_sys::unboxed_descriptors`), then
//!    Landlock, then the seccomp denylist (`seccomp::denylist`), each
//!    thread the process starts afterward inheriting both.
//!
//! Between two steps the caller may run work on a thread it joins before
//! the next, as a pre-pass that reads the project does: every step after
//! the first that needs the process to have one thread reads the count
//! once a thread just joined has left it (`gro_sys::settled_thread_count`),
//! and refuses otherwise. A step that fails refuses the run: its token is
//! spent, and the process may hold part of what the steps before did.
//!
//! The tokens hold the order of the steps, not what the caller opens between
//! them: a descriptor it holds across the box keeps the access it was opened
//! with. So [`RunPrepared::confine`] refuses a process holding what the walls
//! would miss: a socket above stderr, which keeps the network it was made in; a
//! socket on stdin, stdout, or stderr unless it's a connected unix stream,
//! which reaches only its peer (a connected internet stream can be
//! disconnected, then connect elsewhere or listen); and a descriptor that holds
//! other files out of the scan's sight, acting on them with the credentials it
//! was made with (an `io_uring` instance, whose registered files need no
//! descriptor). A file opened for writing isn't refused, and stays writable
//! past Landlock, which checks only an open: the caller holds none across the
//! box.
//!
//! The box, installed by [`RunPrepared::confine`] on the calling thread:
//!
//! - **Landlock**, at ABI 6 (Linux 6.12) or later, every rule enforced in
//!   full or the confinement fails, as eval's: of every filesystem right
//!   ABI 6 knows, only what the rules grant, each beneath the handle it
//!   opened; both scopes, so no signal reaches a process outside the box
//!   and no connection an abstract unix socket outside it; and in the
//!   host's network, binding and connecting TCP sockets refused. In the
//!   run's own namespace TCP is left to the namespace.
//! - **The denylist**, its socket rule by the network form (`NetForm`):
//!   every call number classified, a family refused by its reason, as its
//!   module doc says. It allows `execve`, so the run spawns its tools, and
//!   every program the run starts inherits the whole box.
//!
//! What holds, and what defeats it. Each wall is a boundary, the kernel's,
//! against everything in the process and every program it starts: the
//! project's code and tests, a dependency, the tools, and an exploit of the
//! engine. Landlock holds the user's files outside the rules (the home
//! directory among them, and `.git` or `node_modules` where the rules leave
//! them out); the namespace and, in the host's network, Landlock's TCP wall
//! with the denylist hold the network; the denylist holds the kernel
//! surface no rule reaches (unix sockets by path, nested user namespaces,
//! the keyrings, System V IPC and message queues, ownership and
//! extended-attribute changes, other processes' memory and limits). Each is
//! defeated by a kernel bug, and the box as a whole by what its rules
//! allow: a write root a later command runs unboxed, a secret the rules let
//! it read (`.env*` in a readable tree), mode and time changes to any file
//! the user owns, whatever the rules, which the denylist leaves allowed,
//! what the run prints, a program it started signalling it, since both
//! share the domain, and a socket on stdin, stdout, or stderr that the
//! launcher chose, a connected unix stream, which reaches only its peer.
//!
//! Closing the inherited descriptors is part of that boundary: the walls gate
//! only what's opened after them, and a descriptor without close-on-exec passes
//! to every program the run starts, a socket on the host's network among them.
//! What passes it is a descriptor the process opens between the steps, of which
//! the scan before the box refuses a socket and a descriptor that hides files,
//! never a file opened for writing, and stdio, which the launcher chose (a
//! socket there only as a connected unix stream). Marking the process not
//! dumpable is a second layer against a program the run starts reading the
//! run's own `/proc` files (its environment, its memory), behind Landlock,
//! which refuses those reads while `/proc` stays outside the rules, and the
//! denylist, which refuses `ptrace` and `process_vm_*`: rules that granted
//! `/proc` would let a program in the same domain read them, and the mark alone
//! would hold. Dropping the capabilities is defense in depth against an exploit
//! of the process itself, since a program it starts holds none after `exec`
//! anyway. The `core` limit is a guardrail against an accident, a crash writing
//! the process's memory and environment to disk, not a wall: deliberate code
//! writes what it holds into a write root. The open-files raise is
//! availability, no wall, and best-effort, as `fgro run`'s is.

use std::io;
use std::path::PathBuf;

use gro_sys::capabilities::{self, CapabilityError};
use gro_sys::namespace::{self, MapStep, NamespaceError, NamespaceRefused, Refusal};

use super::landlock::{self, TcpWall};
use super::rules::{BoxRules, InstalledRule};
use super::{ConfineError, NetForm, alone};

/// The process with every descriptor it inherited above stderr closed:
/// the run's first step, made only by [`RunClosed::close_inherited`].
#[derive(Debug)]
pub struct RunClosed {
    /// Made only here, so a value is the proof.
    _closed: (),
}

/// The process in its network: its own namespace, or the host's with
/// internet sockets refused. Made only by [`RunClosed::enter_network`].
#[derive(Debug)]
pub struct RunNetworked {
    /// The network it's in.
    network: RunNetwork,
}

/// The process hardened: no capability, not dumpable, its limits set.
/// Made only by [`RunNetworked::harden`].
#[derive(Debug)]
pub struct RunHardened {
    /// The network it's in.
    network: RunNetwork,
}

/// The run's box, computed: its rules, each opened, and the network it's
/// for. Made only by [`RunHardened::prepare`].
#[derive(Debug)]
pub struct RunPrepared {
    /// Landlock's rules.
    rules: BoxRules,
    /// The network the box is for.
    network: RunNetwork,
}

/// The process confined to the run's box, from the thread that installed
/// it. Made only by [`RunPrepared::confine`].
#[derive(Debug, PartialEq, Eq)]
pub struct RunConfined {
    /// The rules the box installed, in order, for messages.
    rules: Vec<InstalledRule>,
    /// The paths of the optional rules that had nothing to name.
    absent: Vec<PathBuf>,
    /// The network it's in.
    network: RunNetwork,
}

/// The network a run's box holds it in, which decides its TCP wall and its
/// denylist's socket rule ([`NetForm`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunNetwork {
    /// A user and network namespace of the run's own, its loopback up and
    /// nested user namespaces refused in it.
    Namespace,
    /// The host's network, every internet socket refused, since the kernel
    /// refused the namespaces with nothing changed: why, as the settings
    /// tell it, and the refused call's error number, if one was made.
    InetRefused {
        /// The likely cause.
        cause: Refusal,
        /// The error number the refusal carries
        /// (`gro_sys::namespace::NamespaceRefused::errno`).
        errno: Option<i32>,
    },
}

impl RunNetwork {
    /// The network form the box's walls are built for.
    pub const fn form(&self) -> NetForm {
        match self {
            Self::Namespace => NetForm::Namespace,
            Self::InetRefused { .. } => NetForm::InetRefused,
        }
    }
}

/// The host's network, for the namespaces `refused` with nothing changed.
fn inet_refused(refused: &NamespaceRefused) -> RunNetwork {
    RunNetwork::InetRefused {
        cause: refused.cause(),
        errno: refused.errno(),
    }
}

/// Why a step of the run's start before its box failed. The process may
/// hold part of what the steps before it did, and runs nothing.
#[derive(Debug, thiserror::Error)]
pub enum RunStepError {
    /// The inherited descriptors couldn't be closed.
    #[error("failed to close the descriptors the process inherited")]
    Descriptors(#[source] io::Error),
    /// The process has another thread, a caller's bug: a user namespace
    /// takes a process of one thread, and capabilities are each thread's
    /// own.
    #[error(
        "the process has {count} threads, and the run's box is made on its one thread, before \
         any other starts"
    )]
    Threads {
        /// How many it has.
        count: u64,
    },
    /// The thread count couldn't be read.
    #[error("failed to read the thread count")]
    ThreadsUnread(#[source] io::Error),
    /// The process entered a user namespace and a map's write failed, so
    /// its ids aren't all mapped, which nothing in it can undo.
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
    /// The namespace's loopback couldn't be brought up.
    #[error("failed to bring the run's loopback up")]
    Loopback(#[source] namespace::LoopbackError),
    /// User namespaces nested in the run's couldn't be refused.
    #[error("failed to refuse user namespaces nested in the run's")]
    NestedNamespaces(#[source] io::Error),
    /// The capabilities couldn't be dropped.
    #[error("failed to drop the process's capabilities")]
    Capabilities(#[source] CapabilityError),
    /// The process couldn't be marked not dumpable.
    #[error("failed to mark the process not dumpable")]
    Dumpable(#[source] io::Error),
    /// The `core` hard limit couldn't be lowered.
    #[error("failed to lower the core dump limit to 0")]
    CoreLimit(#[source] io::Error),
}

impl RunClosed {
    /// Closes every descriptor above stderr the process inherited
    /// (`gro_sys::close_inherited_fds`): the run's first step.
    ///
    /// # Precondition
    ///
    /// No value in the process owns a descriptor above 2: it's called on
    /// the main thread, the only one, before the process opens anything.
    /// The descriptors close by number, so one a `File` or an `OwnedFd`
    /// held would close under it, and the next open could reuse its number,
    /// a break of std's I/O safety. The thread count is checked; an owner
    /// on the calling thread isn't, as `gro_sys::close_inherited_fds`'s doc
    /// says.
    ///
    /// # Errors
    ///
    /// Returns [`RunStepError::Descriptors`] when the process has another
    /// thread, whose count is read once, or the close fails.
    pub fn close_inherited() -> Result<Self, RunStepError> {
        gro_sys::close_inherited_fds().map_err(RunStepError::Descriptors)?;
        Ok(Self { _closed: () })
    }

    /// Enters the run's network, as the [module doc](self) says: its own
    /// user and network namespace with loopback up and nested user
    /// namespaces refused, or where the kernel refuses the namespaces, the
    /// host's with internet sockets refused.
    ///
    /// # Errors
    ///
    /// Returns [`RunStepError::Threads`] for another thread, with nothing
    /// changed; [`RunStepError::Stranded`] for a namespace entered whose
    /// maps failed; [`RunStepError::Loopback`] or
    /// [`RunStepError::NestedNamespaces`] for a namespace entered whose
    /// next step failed.
    pub fn enter_network(self) -> Result<RunNetworked, RunStepError> {
        let network = match namespace::enter_user_net_namespace() {
            Ok(entered) => {
                entered.loopback_up().map_err(RunStepError::Loopback)?;
                entered
                    .refuse_nested_user_namespaces()
                    .map_err(RunStepError::NestedNamespaces)?;
                RunNetwork::Namespace
            }
            Err(error) => network_refused(error)?,
        };
        Ok(RunNetworked { network })
    }
}

/// The network a run takes when the namespaces failed with `error`: the
/// host's for a refusal, which changed nothing, and none otherwise.
pub(super) fn network_refused(error: NamespaceError) -> Result<RunNetwork, RunStepError> {
    match error {
        NamespaceError::Refused(refused) => Ok(inet_refused(&refused)),
        NamespaceError::Threads { count } => Err(RunStepError::Threads { count }),
        NamespaceError::Stranded { step, source } => Err(RunStepError::Stranded { step, source }),
    }
}

impl RunNetworked {
    /// The network the process is in.
    pub const fn network(&self) -> &RunNetwork {
        &self.network
    }

    /// Hardens the process, as the [module doc](self) says: every
    /// capability dropped, not dumpable, the `core` hard limit 0, the
    /// open-files soft limit at the hard one where it can be.
    ///
    /// # Errors
    ///
    /// Returns [`RunStepError::Threads`] or [`RunStepError::ThreadsUnread`]
    /// for a thread count other than 1 or unread, with nothing changed, or
    /// the step that failed after it.
    pub fn harden(self) -> Result<RunHardened, RunStepError> {
        capabilities::drop_capabilities().map_err(|error| match error {
            CapabilityError::Threads { count } => RunStepError::Threads { count },
            CapabilityError::ThreadsUnread(source) => RunStepError::ThreadsUnread(source),
            error => RunStepError::Capabilities(error),
        })?;
        gro_sys::set_not_dumpable().map_err(RunStepError::Dumpable)?;
        gro_sys::lower_hard_limit(gro_sys::Resource::Core, 0).map_err(RunStepError::CoreLimit)?;
        // best-effort, as `fgro run`'s is: a program the run starts runs
        // with the limit it was given, as under Node
        let _ = gro_sys::raise_open_files_limit();
        Ok(RunHardened {
            network: self.network,
        })
    }
}

impl RunHardened {
    /// The box with the Landlock rules `rules`, opened since the process
    /// was hardened, for the network it's in.
    pub const fn prepare(self, rules: BoxRules) -> RunPrepared {
        RunPrepared {
            rules,
            network: self.network,
        }
    }
}

impl RunPrepared {
    /// Confines the process to the run's box, from its one thread, as the
    /// [module doc](self) says. The box goes up once: this takes the rules'
    /// handles, which are closed once Landlock names them.
    ///
    /// # Errors
    ///
    /// Returns [`ConfineError::Threads`] when the process has another
    /// thread, or [`ConfineError::ThreadsUnread`] when its count can't be
    /// read, [`ConfineError::SocketHeld`], [`ConfineError::StdioSocket`],
    /// or [`ConfineError::HidingHeld`] when it holds a descriptor the walls
    /// would miss, or [`ConfineError::DescriptorsUnread`] when its
    /// descriptors can't be read, before anything is installed; or an error
    /// naming the wall the kernel can't hold (checked before any is installed) or that failed
    /// to install, after which the process may hold part of the box and
    /// must run nothing it was to confine.
    pub fn confine(self) -> Result<RunConfined, ConfineError> {
        alone()?;
        let unboxed = gro_sys::unboxed_descriptors().map_err(ConfineError::DescriptorsUnread)?;
        if !unboxed.sockets.is_empty() {
            return Err(ConfineError::SocketHeld {
                fds: unboxed.sockets,
            });
        }
        if !unboxed.stdio_sockets.is_empty() {
            return Err(ConfineError::StdioSocket {
                fds: unboxed.stdio_sockets,
            });
        }
        if !unboxed.hiding.is_empty() {
            return Err(ConfineError::HidingHeld {
                fds: unboxed.hiding,
            });
        }
        let form = self.network.form();
        // the filter is assembled first, so one that can't be fails before
        // any wall goes up
        let denylist = denylist(form)?;
        let tcp = match form {
            NetForm::Namespace => TcpWall::Unhandled,
            NetForm::InetRefused => TcpWall::Refused,
        };
        landlock::landlock(&self.rules, tcp)?;
        // Landlock set `no_new_privs`, which an unprivileged filter needs
        seccompiler::apply_filter(&denylist).map_err(ConfineError::Seccomp)?;
        let (rules, absent) = self.rules.into_installed();
        Ok(RunConfined {
            rules,
            absent,
            network: self.network,
        })
    }
}

/// The run's denylist for the network form `form`.
#[cfg(target_arch = "x86_64")]
fn denylist(form: NetForm) -> Result<seccompiler::BpfProgram, ConfineError> {
    super::seccomp::denylist::denylist_filter(form).map_err(ConfineError::Denylist)
}

/// The run's denylist, whose table is `x86_64`'s alone.
#[cfg(not(target_arch = "x86_64"))]
fn denylist(_form: NetForm) -> Result<seccompiler::BpfProgram, ConfineError> {
    Err(ConfineError::SeccompArch {
        arch: std::env::consts::ARCH,
    })
}

impl RunConfined {
    /// The rules the box installed, in order.
    pub fn rules(&self) -> &[InstalledRule] {
        &self.rules
    }

    /// The paths of the optional rules that had nothing to name.
    pub fn absent(&self) -> &[PathBuf] {
        &self.absent
    }

    /// The network the box holds the process in.
    pub const fn network(&self) -> &RunNetwork {
        &self.network
    }
}
