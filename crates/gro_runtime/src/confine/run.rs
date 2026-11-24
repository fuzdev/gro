//! The run's box: the steps a run's process takes before its box goes up,
//! and the box installed.
//!
//! Each step consumes the token the step before it made and makes the next,
//! so the order is the compiler's to hold, and the last,
//! [`RunPrepared::confine`], makes the [`RunConfined`] the engine takes
//! ([`Confinement::Run`](super::Confinement::Run)). On the process's main
//! thread, its only one:
//!
//! 0. Before any step, nothing changed: [`probe_floor`], the kernel's
//!    Landlock held to the floor the box needs, whose [`LandlockFloor`] the
//!    first step takes, and [`check_stdio`], stdin, stdout, and stderr held
//!    to what the box allows, so a refusal of either leaves the process as
//!    it started.
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
//! 4. [`RunHardened::prepare`]: the Landlock rules ([`BoxRules`]) opened
//!    from the run's plan (`plan::BoxPlan`) once the process is hardened,
//!    after what the run makes for itself (its tool directories,
//!    [`RunHardened::make_tool_dirs`], and its private directory,
//!    [`RunHardened::make_private_dir`]), so each rule names what was made.
//!    The token is the only way a run's rules open, so none can open before
//!    the first step closes the inherited descriptors by number, which would
//!    close a rule's handle under it: a guardrail against gro's own bug,
//!    held by the compiler. The box then owns the private directory, its
//!    lock with it, for as long as the box lasts.
//! 5. [`RunPrepared::confine`]: the thread count checked, then the
//!    process's descriptors (`gro_sys::unboxed_descriptors`), then
//!    Landlock, then the seccomp denylist (`seccomp::denylist`), each
//!    thread the process starts afterward inheriting both. The rules'
//!    handles are closed once the box takes them, but for a run that may
//!    start a program those its programs' boxes name, held past the box
//!    ([`ProgramRules`], `program.rs`), so no rule a program's box names is
//!    opened again by its path once the run's box is up.
//!
//! A priced build (`crate::prices`, `RunDefault`) can leave out the second
//! step's namespace, its box then built for the run's own network form, and
//! the fifth's Landlock or denylist, each to be timed; without the `prices`
//! feature nothing reads the switches and every step is taken.
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
//! dumpable keeps a program the run starts from reading the run's own `/proc`
//! files (its environment, its memory): the plan grants `/proc` read, and
//! Landlock refuses those reads only to a process outside the domain, so the
//! mark holds them here, with the denylist refusing `ptrace` and
//! `process_vm_*`. Dropping the capabilities is defense in depth against an exploit
//! of the process itself, since a program it starts holds none after `exec`
//! anyway. The `core` limit is a guardrail against an accident, a crash writing
//! the process's memory and environment to disk, not a wall: deliberate code
//! writes what it holds into a write root. The open-files raise is
//! availability, no wall, and best-effort, as `fgro run`'s is.

use std::io;
use std::os::fd::{AsFd as _, BorrowedFd};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use gro_sys::beneath;
use gro_sys::capabilities::{self, CapabilityError};
use gro_sys::namespace::{self, MapStep, NamespaceError, NamespaceRefused, Refusal};
use gro_sys::private_dir::{self, PrivateDir, PrivateDirError};

use super::landlock::{self, TcpWall};
use super::plan::{Anchor, BoxPlan, Opening};
use super::program::{ProgramRules, ProgramWrites};
use super::rules::{BoxRules, InstalledRule, RuleError};
use super::{ConfineError, NetForm, Threads, alone};
use crate::prices::{self, RunDefault};

/// The kernel's Landlock, probed and found at the floor the box needs, with
/// nothing changed: what the run's first step takes, made only by
/// [`probe_floor`].
#[derive(Debug)]
pub struct LandlockFloor {
    /// Made only there, so a value is the proof.
    _probed: (),
}

/// The kernel's Landlock probed, changing nothing: the status read with no
/// flag set, so no `restrict_self` call is made and `no_new_privs` is left
/// as it was, then held to the floor the box needs.
///
/// # Errors
///
/// Returns [`ConfineError::NoLandlock`] or [`ConfineError::LandlockAbi`]
/// for a kernel below the floor, or [`ConfineError::Landlock`] when the
/// status can't be read.
pub fn probe_floor() -> Result<LandlockFloor, ConfineError> {
    landlock::probe_floor()?;
    Ok(LandlockFloor { _probed: () })
}

/// Checks stdin, stdout, and stderr as the box will, before anything
/// changes: each that is a socket must be a connected unix stream, which
/// reaches its peer alone. [`RunPrepared::confine`] checks them again.
///
/// # Errors
///
/// Returns [`ConfineError::StdioSocket`] naming each that isn't, or
/// [`ConfineError::DescriptorsUnread`] when one can't be read.
pub fn check_stdio() -> Result<(), ConfineError> {
    let fds = gro_sys::unboxed_stdio().map_err(ConfineError::DescriptorsUnread)?;
    if fds.is_empty() {
        Ok(())
    } else {
        Err(ConfineError::StdioSocket { fds })
    }
}

/// The process with every descriptor it inherited above stderr closed:
/// the run's first step, made only by [`RunClosed::close_inherited`].
#[derive(Debug)]
pub struct RunClosed {
    /// The floor probed before it, carried so a value is the proof of both.
    _floor: LandlockFloor,
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
    /// What each program the run may start writes, none for a run that
    /// starts none, whose boxes are built from the rules' handles, held
    /// past the run's.
    programs: Vec<ProgramWrites>,
    /// The network the box is for.
    network: RunNetwork,
    /// The run's private directory, if it has one, held, its lock with it,
    /// as long as the box is.
    private: Option<RunPrivateDir>,
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
    /// The run's private directory, if it has one, held, its lock with it,
    /// as long as the box is: a sweep by another run removes it only once
    /// this process is gone.
    private: Option<RunPrivateDir>,
    /// For a run that may start a program, the rules its programs' boxes
    /// are built from, their handles held past the run's box, until taken
    /// ([`RunConfined::take_program_rules`]).
    programs: Option<ProgramRules>,
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
    /// The process has another thread: the inherited descriptors close by
    /// number, a user namespace takes a process of one thread, and
    /// capabilities are each thread's own.
    #[error(transparent)]
    Threads(#[from] Threads),
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
    /// It takes the [`LandlockFloor`] [`probe_floor`] made, so the floor is
    /// known before anything changes: a kernel below it is told while the
    /// process is still as it started, when running the Node gro in its
    /// place hands that the process a Node gro would have.
    ///
    /// # Errors
    ///
    /// Returns [`RunStepError::Threads`] or [`RunStepError::ThreadsUnread`]
    /// for a thread count other than 1 or unread, settled
    /// (`gro_sys::settled_thread_count`), with nothing closed, or
    /// [`RunStepError::Descriptors`] when the close fails.
    pub fn close_inherited(floor: LandlockFloor) -> Result<Self, RunStepError> {
        let count = gro_sys::settled_thread_count().map_err(RunStepError::ThreadsUnread)?;
        if count != 1 {
            return Err(Threads {
                count,
                first_step: true,
            }
            .into());
        }
        gro_sys::close_inherited_fds().map_err(RunStepError::Descriptors)?;
        Ok(Self { _floor: floor })
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
        // a priced build can stay in the host's namespaces, its box built
        // for the run's own network all the same (`prices`)
        if prices::off(RunDefault::Namespace) {
            return Ok(RunNetworked {
                network: RunNetwork::Namespace,
            });
        }
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
        NamespaceError::Threads { count } => Err(Threads {
            count,
            first_step: false,
        }
        .into()),
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
            CapabilityError::Threads { count } => Threads {
                count,
                first_step: false,
            }
            .into(),
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

/// The run's private directory, made only once the process is hardened.
///
/// [`RunHardened::make_private_dir`] makes it, so its descriptor is never
/// one the first step closes under it, and it's the only anchor
/// [`RunHardened::prepare`] opens the private directory's rule beneath.
#[derive(Debug)]
pub struct RunPrivateDir {
    /// The directory, held by its shared lock.
    dir: PrivateDir,
    /// Its absolute path, for messages.
    path: PathBuf,
}

impl RunPrivateDir {
    /// Its absolute path, as made.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Equal as the same directory made at the same path, the lock aside.
impl PartialEq for RunPrivateDir {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.dir.id() == other.dir.id()
    }
}

impl Eq for RunPrivateDir {}

impl RunHardened {
    /// Makes the run's private directory beneath `parent`, whose absolute
    /// path is `parent_path` (`gro_sys::private_dir::make_private_dir`),
    /// once the process is hardened.
    ///
    /// # Errors
    ///
    /// Returns why it couldn't be made, as `gro_sys` names it.
    pub fn make_private_dir(
        &self,
        parent: BorrowedFd<'_>,
        parent_path: &Path,
    ) -> Result<RunPrivateDir, PrivateDirError> {
        let dir = private_dir::make_private_dir(parent)?;
        let path = parent_path.join(dir.name());
        Ok(RunPrivateDir { dir, path })
    }

    /// The box `plan` plans, its rules opened now, once the process is
    /// hardened, for the network it's in: each in the plan's order, by its
    /// real path or beneath its anchor, the project's root and `/` each
    /// opened by its real path with every symlink refused, the private
    /// directory by `private`'s descriptor, which only this token makes,
    /// held to the directory it made by its device and inode numbers. The
    /// box then owns `private`, so its lock lasts as long as the box. The
    /// token is the only way a run's rules open, so none opens before the
    /// inherited descriptors close, which would close it under its owner: a
    /// guardrail against gro's own bug.
    ///
    /// # Errors
    ///
    /// Returns the first rule refused ([`RuleError`]): one that can't open
    /// as the [rules' module doc](super::rules) says, an anchor that can't
    /// open, refused as the rules beneath it, or a rule beneath the private
    /// directory when `private` is none.
    pub fn prepare(
        self,
        plan: &BoxPlan,
        private: Option<RunPrivateDir>,
    ) -> Result<RunPrepared, RuleError> {
        let root = Path::new("/");
        let open_anchor = |path: &Path| {
            beneath::open_root(path).map_err(|source| RuleError::Open {
                path: path.to_owned(),
                source,
            })
        };
        let project = open_anchor(plan.root())?;
        let slash = open_anchor(root)?;
        let mut rules = BoxRules::new();
        for rule in plan.rules() {
            match &rule.opening {
                Opening::Real { path, rights } => {
                    rules.real(path, *rights, rule.need, rule.source)?;
                }
                Opening::Beneath {
                    anchor,
                    path,
                    rights,
                } => {
                    let (dir, dir_path) = match (anchor, private.as_ref()) {
                        (Anchor::Project, _) => (project.as_fd(), plan.root()),
                        (Anchor::Slash, _) => (slash.as_fd(), root),
                        (Anchor::PrivateTemp, Some(private)) => {
                            held_private(private)?;
                            (private.dir.dir(), private.path.as_path())
                        }
                        (Anchor::PrivateTemp, None) => {
                            return Err(RuleError::NoPrivateDir { path: path.clone() });
                        }
                    };
                    rules.beneath(dir, dir_path, path, *rights, rule.need, rule.source)?;
                }
            }
        }
        Ok(RunPrepared {
            rules,
            programs: plan.program_writes(),
            network: self.network,
            private,
        })
    }
}

/// That `private`'s descriptor still names the directory made for the run:
/// a second chain of tokens closes descriptors by number, and a number
/// reused since would name another directory.
fn held_private(private: &RunPrivateDir) -> Result<(), RuleError> {
    let named = private
        .dir
        .dir()
        .try_clone_to_owned()
        .and_then(|handle| std::fs::File::from(handle).metadata());
    match named {
        Ok(metadata) if (metadata.dev(), metadata.ino()) == private.dir.id() => Ok(()),
        _ => Err(RuleError::PrivateDirChanged {
            path: private.path.clone(),
        }),
    }
}

impl RunPrepared {
    /// Confines the process to the run's box, from its one thread, as the
    /// [module doc](self) says. The box goes up once: this takes the rules'
    /// handles, which are closed once Landlock names them, but for a run
    /// that may start a program those its programs' boxes name, held past
    /// the run's box (`ProgramRules::hold`).
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
        // any wall goes up; a priced build can leave either wall out
        // (`prices`)
        let denylist = if prices::off(RunDefault::Filter) {
            None
        } else {
            Some(denylist(form)?)
        };
        let tcp = match form {
            NetForm::Namespace => TcpWall::Unhandled,
            NetForm::InetRefused => TcpWall::Refused,
        };
        if !prices::off(RunDefault::Landlock) {
            landlock::landlock(&self.rules, tcp)?;
        }
        // an unprivileged filter needs `no_new_privs`, which Landlock set,
        // and the filter's install sets it again (`seccompiler` does)
        if let Some(denylist) = &denylist {
            seccompiler::apply_filter(denylist).map_err(ConfineError::Seccomp)?;
        }
        let (rules, absent) = self.rules.into_installed();
        let (rules, programs) = ProgramRules::hold(rules, &self.programs);
        Ok(RunConfined {
            rules,
            absent,
            network: self.network,
            private: self.private,
            programs,
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
    /// The run's private directory, if it has one.
    pub const fn private_dir(&self) -> Option<&RunPrivateDir> {
        self.private.as_ref()
    }

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

    /// The rules its programs' boxes are built from, taken: for a run that
    /// may start a program, once, and none after, or for one that starts
    /// none.
    pub const fn take_program_rules(&mut self) -> Option<ProgramRules> {
        self.programs.take()
    }
}
