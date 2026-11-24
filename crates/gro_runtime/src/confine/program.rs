//! A program's own box: a Landlock domain nested in the run's, its rules
//! named by the run's own rule handles, held across the run's box.
//!
//! The run's box keeps, as it goes up, the handles of the rules a program's
//! box takes (`ProgramRules::hold`, from `run::RunPrepared::confine`, for a
//! run that may start a program), so nothing a program's box names is opened
//! by its path once the run's box is up: each is the handle the run's own
//! rule opened before it, by its real path or beneath a directory's
//! descriptor with every symlink refused (`rules.rs`'s module doc). Its
//! private directory's handle is held too, as the parent each program's
//! temporary directory is made in. The rest are closed.
//!
//! A program's box ([`ProgramBox`]), built by [`ProgramRules::program_box`] once
//! the run's box is up, grants (`program_rights`):
//!
//! - every rule of the run's box but its private directory and any rule on
//!   a directory holding it, reading and executing as the run's grants,
//!   never writing, so a program reads the project, the system, and
//!   `node_modules` as the run does, and nothing of the private directory,
//!   where its siblings' temporary directories and the run's own lie. A
//!   rule holding the private directory would grant all of it, since
//!   Landlock grants beneath a rule whatever the rule's directory holds:
//!   the verb refuses a spawning run planned with one before its box
//!   (`BoxPlan::holding`), and the held rules leave one out besides;
//! - the devices as the run's box grants them, `/dev/null` read and
//!   written, which every redirect to it opens;
//! - what the program writes, by its write set ([`ProgramWrites`]): for a
//!   program of the grants table, the command's tool directories; for one
//!   whose `--allow-run-write=` names the root, the root's per-entry rules;
//!   and the directories `--allow-run-write=` names for it, read and
//!   written, each a rule of the run's that no other program's box takes;
//!   for one a flag grants, or one found beneath an exec root, nothing of
//!   the project but what that flag names for it;
//! - nothing of what `--allow-write=` names, not even its read: that flag
//!   is gro's own code's, and `--allow-read=` is what widens every box;
//! - a temporary directory of its own, granted as the run's rule grants the
//!   private directory (read, written, and executed), made beneath that
//!   directory's held handle by `mkdirat`, an entry already there refused,
//!   its mode then set to its user's alone whatever the umask, and named
//!   by the program's pin, `program_<pin>` (`temp_dir_name`), never by its
//!   name, which a program's own text could choose.
//!
//! Its ruleset is the run's shape (`landlock::ruleset`): every filesystem
//! right of ABI 6 handled, so it grants only what its rules do, both
//! scopes, so a program signals no process outside its own domain, the
//! run's and a sibling's included, and reaches no abstract unix socket
//! outside it, and TCP left unhandled (`TcpWall::Unhandled`), decided by
//! the run's layer beneath it: the kernel checks an access against every
//! layer of a domain, so a program binds and connects as the run does.
//! Built from the held handles once the run's box is up, a program's box
//! can wait for its first launch, so a run that starts nothing builds
//! none. [`ProgramBox::restrict_thread`] installs a duplicate of its ruleset
//! on the calling thread alone, nested in the run's domain: what that
//! thread starts afterward inherits it, and the run's other threads keep
//! the run's box. The held handles are close-on-exec, as every rule's is,
//! so no program inherits one.
//!
//! Kind: the nested domain is a boundary, the kernel's. Its adversary is a
//! program's own code (a linter's plugin, a test); its assets are other
//! programs' write roots and temporary directories, other processes'
//! environment, memory, and working directory, and signalling the run or a
//! sibling. It is defeated by a kernel bug; by a program's own write roots,
//! which a later command runs; by the tool directories every
//! program of the table shares; by realm code, which holds the outer
//! domain and can start any program of the closure with arguments of its
//! own choosing, so a directory `--allow-run-write=` names for a program
//! is one deliberate project code writes; by everything the named program
//! runs (its config, its plugins, for a test runner the tests); by a
//! directory it's given that the same run later loads, as `gen --check`
//! loads a genfile under `src/` within one `check`, which carries the
//! program's write to every subject's grants; by what programs share
//! for reading (a sibling's roots, and what `/proc` shows of any process:
//! its command line, its status, and the numbers of its descriptors); by what Landlock doesn't gate: a
//! path's mode, times, and metadata, by name, a sibling's temporary
//! directory's included, since its name is its pin's, the run's loopback,
//! where a sibling's listener or the run's is reached, and `pidfd_open` on
//! a sibling or the run, a handle that can't signal; and by the terminal's
//! foreground, which a program can take.

use std::io;
use std::os::fd::{AsFd as _, OwnedFd};
use std::path::{Path, PathBuf};

use ::landlock::RulesetCreated;
use gro_sys::beneath::{self, Last, Links, OpenAs};

use super::ConfineError;
use super::landlock::{self, TcpWall};
use super::plan::{DirFlag, RuleSource, holds};
use super::rules::{InstalledRule, Kind, Rights};

/// What a program's box writes beyond its own temporary directory: its
/// write set, which `program_rights` reads beside each rule's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramWrites {
    /// Whether it writes the command's tool directories, as a program of
    /// the grants table does.
    pub tool_dirs: bool,
    /// Whether it writes the root's per-entry rules, as one whose
    /// `--allow-run-write=` names the root does.
    pub root: bool,
    /// The program `--allow-run-write=` names it as, by its index in the
    /// plan's (`BoxPlan::run_write`), whose rules it reads and writes.
    pub run_write: Option<usize>,
}

impl ProgramWrites {
    /// A program of the grants table no `--allow-run-write=` names: the
    /// command's tool directories.
    pub const TABLE: Self = Self {
        tool_dirs: true,
        root: false,
        run_write: None,
    };

    /// A program a flag grants, or one found beneath an exec root, that no
    /// `--allow-run-write=` names: nothing of the project.
    pub const FLAG: Self = Self {
        tool_dirs: false,
        root: false,
        run_write: None,
    };
}

/// The mode a program's temporary directory is set to, whatever the
/// process's umask: its user's alone, as the private directory's is.
const TEMP_MODE: u32 = 0o700;

/// What a program's box grants beneath a rule of the run's box from
/// `source` granting `rights`, for a program writing `writes`, as the
/// [module doc](self) says: none for the run's private directory, the
/// devices as the run has them, a tool directory as the run has it for a
/// program that writes the tool directories, an entry's rule for one that
/// writes the root, a rule `--allow-run-write=` made as the run has it for
/// the program it names and none for any other, none for what a write flag
/// names, and the rest without writing; none where that leaves nothing.
/// The private directory's arm is the derivation's alone:
/// `ProgramRules::hold` takes that rule apart before any rule is held, so
/// no box built reaches it.
pub(super) const fn program_rights(
    source: RuleSource,
    rights: Rights,
    writes: ProgramWrites,
) -> Option<Rights> {
    let rights = match source {
        RuleSource::PrivateTemp | RuleSource::Flag(DirFlag::Write) => return None,
        RuleSource::ToolDir(_) if writes.tool_dirs => rights,
        RuleSource::Entry if writes.root => rights,
        RuleSource::RunWrite(index) => match writes.run_write {
            Some(named) if named == index => rights,
            _ => return None,
        },
        RuleSource::Device => rights,
        _ => rights.without_write(),
    };
    if rights.is_empty() {
        None
    } else {
        Some(rights)
    }
}

/// The name of the temporary directory of the program pinned at `pin`, in
/// the run's private directory: `program_<pin>`.
fn temp_dir_name(pin: usize) -> String {
    format!("program_{pin}")
}

/// A rule of the run's box held past it: the rule as installed and its
/// handle, which a program's box names.
#[derive(Debug)]
struct HeldRule {
    /// The rule.
    rule: InstalledRule,
    /// Its handle, `O_PATH`, opened before the run's box went up.
    handle: OwnedFd,
}

/// The rules every program's box of a run is built from, each the handle of
/// a rule of the run's box, held past it, and the run's private directory,
/// held to make each program's temporary directory in.
#[derive(Debug)]
pub struct ProgramRules {
    /// The run's rules a program's box takes in some part, in the run's
    /// order.
    rules: Vec<HeldRule>,
    /// The run's rule on its private directory.
    private: HeldRule,
}

/// Equal as the same rules and private directory, the handles aside.
impl PartialEq for ProgramRules {
    fn eq(&self, other: &Self) -> bool {
        self.private.rule == other.private.rule
            && self
                .rules
                .iter()
                .map(|held| &held.rule)
                .eq(other.rules.iter().map(|held| &held.rule))
    }
}

impl Eq for ProgramRules {}

impl ProgramRules {
    /// The run's box's rules as installed, `rules`, each with its handle,
    /// split into what the run keeps of them, its rules as data, and, for a
    /// run whose programs write `programs` (`BoxPlan::program_writes`, none
    /// for a run that starts nothing), the rules its programs' boxes are
    /// built from, with the handles they name: those some program of the
    /// run's box takes (`program_rights` for any of `programs`) and the
    /// private directory's, never a rule on the private directory or a
    /// directory holding it (`plan::holds`), which would grant every
    /// program all of it, whatever else refused it first. Every other
    /// handle is closed, and every one for a run that starts nothing, or
    /// one with no private directory, which no program's box can be made
    /// for.
    pub(super) fn hold(
        rules: Vec<(InstalledRule, OwnedFd)>,
        programs: &[ProgramWrites],
    ) -> (Vec<InstalledRule>, Option<Self>) {
        // the private directory's rule is planned after the rules that
        // could hold it, so it's found first
        let private_path = rules
            .iter()
            .find(|(rule, _)| !programs.is_empty() && rule.source() == RuleSource::PrivateTemp)
            .map(|(rule, _)| rule.path().to_owned());
        let mut installed = Vec::with_capacity(rules.len());
        let mut held = Vec::new();
        let mut private = None;
        for (rule, handle) in rules {
            if let Some(private_path) = &private_path {
                if rule.source() == RuleSource::PrivateTemp && private.is_none() {
                    private = Some(HeldRule {
                        rule: rule.clone(),
                        handle,
                    });
                } else if programs
                    .iter()
                    .any(|writes| program_rights(rule.source(), rule.rights(), *writes).is_some())
                    && !holds(rule.path(), private_path)
                {
                    held.push(HeldRule {
                        rule: rule.clone(),
                        handle,
                    });
                }
            }
            installed.push(rule);
        }
        let programs = private.map(|private| Self {
            rules: held,
            private,
        });
        (installed, programs)
    }

    /// The rules a box for a program writing `writes` grants beneath the
    /// run's, each with what it grants there, in the run's order, its
    /// temporary directory aside.
    #[cfg(test)]
    pub(super) fn granted(
        &self,
        writes: ProgramWrites,
    ) -> impl Iterator<Item = (&InstalledRule, Rights)> {
        self.held_for(writes)
            .map(|(held, rights)| (&held.rule, rights))
    }

    /// The held rules a box for a program writing `writes` grants beneath,
    /// each with what it grants there, in the run's order.
    fn held_for(&self, writes: ProgramWrites) -> impl Iterator<Item = (&HeldRule, Rights)> {
        self.rules.iter().filter_map(move |held| {
            program_rights(held.rule.source(), held.rule.rights(), writes)
                .map(|rights| (held, rights))
        })
    }

    /// The box of the program pinned at `pin`, writing `writes`, built
    /// now, once the run's box is up, as the [module doc](self) says: its
    /// temporary directory, `temp_dir_name`, made in the private
    /// directory first.
    ///
    /// # Precondition
    ///
    /// One box per pin: a pin is granted one way, its box built once and
    /// reused for each of its launches. A second build for a pin fails, its
    /// temporary directory made already, so a pin held by programs granted
    /// two ways (a program of the table found again beneath an exec root,
    /// or two names `--allow-run-write=` gives different directories) is
    /// the caller's to tell apart.
    ///
    /// # Errors
    ///
    /// Returns [`ProgramBoxError::TempDir`] when the temporary directory
    /// can't be made, an entry there already (a symlink included) refused,
    /// or opened as the directory made, every symlink refused; or
    /// [`ProgramBoxError::Landlock`] when the kernel refuses the ruleset.
    pub fn program_box(
        &self,
        writes: ProgramWrites,
        pin: usize,
    ) -> Result<ProgramBox, ProgramBoxError> {
        let name = temp_dir_name(pin);
        // the rule's path is the private directory's joined with `.`
        let temp_dir: PathBuf = self.private.rule.path().components().collect();
        let temp_dir = temp_dir.join(&name);
        let temp =
            make_temp_dir(self.private.handle.as_fd(), Path::new(&name)).map_err(|source| {
                ProgramBoxError::TempDir {
                    path: temp_dir.clone(),
                    source,
                }
            })?;
        // granted as the run's rule grants the private directory
        let temp_rights = self.private.rule.rights();
        let rules = self
            .held_for(writes)
            .map(|(held, rights)| (held.handle.as_fd(), rights.access(held.rule.kind())))
            .chain(std::iter::once((
                temp.as_fd(),
                temp_rights.access(Kind::Directory),
            )));
        let ruleset = landlock::ruleset(rules, TcpWall::Unhandled)?;
        Ok(ProgramBox { ruleset, temp_dir })
    }
}

/// Makes the directory `name` in `parent`, by `mkdirat`, any entry there
/// already refused (`EEXIST`), and opens it beneath `parent` as a
/// directory, every symlink refused, so the handle is of the directory made
/// or of none; then sets its mode to [`TEMP_MODE`] through the handle's
/// `/proc/self/fd` link, since `mkdirat` took the umask's bits off, which
/// may have been the owner's own, and a handle opened as a path changes no
/// mode itself, as the private directory's mode is set.
fn make_temp_dir(parent: std::os::fd::BorrowedFd<'_>, name: &Path) -> io::Result<OwnedFd> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::PermissionsExt as _;
    Last::open(parent, name)?.mkdir(TEMP_MODE)?;
    let dir = beneath::open(parent, name, OpenAs::Directory, Links::Refused)?;
    std::fs::set_permissions(
        format!("/proc/self/fd/{}", dir.as_raw_fd()),
        std::fs::Permissions::from_mode(TEMP_MODE),
    )?;
    Ok(dir)
}

/// A program's box, built and not installed: its ruleset, and its
/// temporary directory, made. Made only by [`ProgramRules::program_box`].
#[derive(Debug)]
pub struct ProgramBox {
    /// The ruleset, which each install duplicates.
    ruleset: RulesetCreated,
    /// The temporary directory, by its absolute path.
    temp_dir: PathBuf,
}

impl ProgramBox {
    /// The program's temporary directory, by its absolute path.
    pub fn temp_dir(&self) -> &Path {
        &self.temp_dir
    }

    /// Installs the box on the calling thread alone, nested in whatever
    /// domain the thread is in, by a duplicate of its ruleset: every thread
    /// and process the thread starts afterward inherits it, and no other
    /// thread is changed. The kernel never widens a domain, so a thread
    /// restricted again keeps every layer it had.
    ///
    /// # Errors
    ///
    /// Returns [`ProgramBoxError::Duplicate`] when the ruleset's descriptor
    /// can't be duplicated, or [`ProgramBoxError::Landlock`] when the install
    /// fails or holds only in part.
    pub fn restrict_thread(&self) -> Result<(), ProgramBoxError> {
        let ruleset = self
            .ruleset
            .try_clone()
            .map_err(ProgramBoxError::Duplicate)?;
        Ok(landlock::restrict(ruleset)?)
    }
}

/// Why a program's box couldn't be built or installed.
#[derive(Debug, thiserror::Error)]
pub enum ProgramBoxError {
    /// The program's temporary directory couldn't be made, or opened as the
    /// directory made.
    #[error("failed to make the program's temporary directory {}", path.display())]
    TempDir {
        /// Its absolute path.
        path: PathBuf,
        /// The call's error: `EEXIST` for an entry already there.
        #[source]
        source: io::Error,
    },
    /// The ruleset's descriptor couldn't be duplicated for an install.
    #[error("failed to duplicate the program's box's ruleset")]
    Duplicate(#[source] io::Error),
    /// The kernel refused the ruleset, or its install.
    #[error(transparent)]
    Landlock(#[from] ConfineError),
}
