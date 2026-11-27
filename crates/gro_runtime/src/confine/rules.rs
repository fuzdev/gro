//! The box's Landlock rules as data: each a handle opened before the box
//! goes up and the [`Rights`] it grants beneath that handle.
//!
//! A rule names what it opened, never a path. Landlock checks an access
//! against the inodes its rules were opened on, so what a path names later
//! changes nothing the box admits. Its handle is opened one of two ways, and
//! the way decides what it may grant.
//!
//! - **By its real path** (`BoxRules::real`): `O_PATH | O_NOFOLLOW`, then
//!   the kernel's name for the handle checked against the path, so a path
//!   that isn't real (a symlink on it) or a directory on it swapped since it
//!   was read is refused rather than admitted where it now leads. It grants
//!   reading and executing only ([`RealRights`]). Eval's roots, the
//!   project's and the link closure's, open the same way as directories
//!   (`open_real_dir`, with `O_DIRECTORY`) and read beneath them
//!   (`BoxRules::read_dirs`).
//! - **Beneath a directory's descriptor** (`BoxRules::beneath`): one
//!   `openat2` with every symlink on the path refused
//!   (`gro_sys::beneath::Links::Refused`), decided by the kernel in the call
//!   that opens it, so a rule never passes through a symlink, whether it
//!   leads out of the directory (`node_modules/.vite` linked to the home
//!   directory) or stays inside (linked to the project's `.git`). It grants
//!   any [`Rights`], and it's the only way to a rule that writes.
//!
//! A rule on a directory grants its rights to everything beneath it. One on
//! anything else (a file, a device) grants only the rights Landlock applies
//! to a file, the rest cut (`Rights::access`).
//!
//! Every rule is [`Need::Required`], whose failure to open fails the box,
//! or [`Need::Optional`], whose absence alone is allowed and recorded
//! ([`BoxRules::absent`]). Anything else that keeps an optional rule from
//! opening (a symlink, a swap, an unreadable directory, an empty path, a
//! relative one by real path) fails it too, so no rule is dropped quietly
//! and the box is never narrower or wider than its plan without saying
//! why. One absence passes a symlink. An optional rule by real path whose
//! missing entry lies beyond a dangling symlink on the path is recorded
//! absent, since that open follows every symlink but the last, and the box
//! is only narrower for it.
//!
//! The kernel holds what the rules grant, a boundary defeated by a kernel
//! bug. The opening is part of that boundary, a native check made before
//! any of the run's code. Its adversary is a symlink or a swapped directory
//! planted before the box goes up by code that ran earlier in the tree (an
//! earlier run's write, a dependency's install script), aimed at a rule
//! that would then grant beyond its path. Its asset is everything a rule's
//! path doesn't name, like the home directory, or `.git` when a plan leaves
//! it out of a tree. A rule opened beneath refuses the symlink in the same
//! call, and one opened by real path refuses a name that isn't the path's.
//! Neither sees a hard link to a file outside the tree, which is the file
//! it links (`gro_sys::beneath`'s module doc), or a directory moved out of
//! the tree after its rule opened, which takes write access to both places
//! already.

use std::io;
use std::ops::BitOr;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use gro_sys::beneath::{self, Links, OpenAs};
use landlock::{AccessFs, BitFlags, make_bitflags};

use super::landlock::LANDLOCK_ABI;
use super::plan::RuleSource;

/// What a rule grants beneath its handle: any of reading, writing, and
/// executing, joined by `|`, writing whole or plain ([`Rights::WRITE_PLAIN`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "four independent rights, each a set of Landlock's, joined by `|`"
)]
pub struct Rights {
    /// Reading files and listing directories.
    read: bool,
    /// Writing files, and making, removing, and renaming entries.
    write: bool,
    /// Making symlinks and FIFOs, and moving or linking an entry in from
    /// another directory, where it writes.
    special: bool,
    /// Executing files.
    execute: bool,
}

impl Rights {
    /// Reading files and listing directories: Landlock's `ReadFile` and
    /// `ReadDir`.
    pub const READ: Self = Self {
        read: true,
        write: false,
        special: false,
        execute: false,
    };

    /// Writing files, and making, removing, and renaming entries:
    /// Landlock's `WriteFile`, `Truncate`, `MakeReg`, `MakeDir`,
    /// `MakeFifo`, `MakeSym`, `RemoveFile`, `RemoveDir`, and `Refer`. Never
    /// a device, a socket, or a device's `ioctl`.
    pub const WRITE: Self = Self {
        read: false,
        write: true,
        special: true,
        execute: false,
    };

    /// Writing plain entries alone: [`Rights::WRITE`] without making a
    /// symlink or a FIFO, Landlock's `MakeSym` and `MakeFifo`, or moving or
    /// linking an entry in from another directory, its `Refer`, which could
    /// bring a symlink along. A tool directory is written so, and a
    /// directory `--allow-write=` or `--allow-run-write=` names that is a
    /// tool directory or where a linked `node_modules` leads, lies in one,
    /// or holds either (`plan::BoxPlan`), since a later unconfined tool that
    /// writes a file there by its name follows a symlink planted at it, and
    /// blocks reading a FIFO; a rename within one directory is kept.
    pub const WRITE_PLAIN: Self = Self {
        read: false,
        write: true,
        special: false,
        execute: false,
    };

    /// Executing files: Landlock's `Execute`.
    pub const EXECUTE: Self = Self {
        read: false,
        write: false,
        special: false,
        execute: true,
    };

    /// Whether these grant writing.
    pub const fn writes(self) -> bool {
        self.write
    }

    /// Whether these grant writing beyond plain entries: making a symlink
    /// or a FIFO, or moving an entry in from another directory.
    #[cfg(test)]
    pub(super) const fn writes_special(self) -> bool {
        self.write && self.special
    }

    /// These without writing: what a program's box keeps of a rule it only
    /// reads beneath (`program::program_rights`).
    pub(super) const fn without_write(self) -> Self {
        Self {
            write: false,
            special: false,
            ..self
        }
    }

    /// Whether these grant nothing.
    pub(super) const fn is_empty(self) -> bool {
        !(self.read || self.write || self.execute)
    }

    /// The Landlock rights these grant on a handle of `kind`, at
    /// [`LANDLOCK_ABI`]: on anything but a directory, only those Landlock
    /// applies to a file (`AccessFs::from_file`), since the kernel refuses
    /// a rule on a file that grants a directory's.
    pub(super) fn access(self, kind: Kind) -> BitFlags<AccessFs> {
        let mut access = BitFlags::empty();
        if self.read {
            access |= READ_ACCESS;
        }
        if self.write {
            access |= WRITE_ACCESS;
            if !self.special {
                access &= !SPECIAL_ACCESS;
            }
        }
        if self.execute {
            access |= AccessFs::Execute;
        }
        match kind {
            Kind::Directory => access,
            Kind::File => access & AccessFs::from_file(LANDLOCK_ABI),
        }
    }
}

impl BitOr for Rights {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            read: self.read || other.read,
            write: self.write || other.write,
            special: self.special || other.special,
            execute: self.execute || other.execute,
        }
    }
}

/// [`Rights::READ`]'s Landlock rights.
const READ_ACCESS: BitFlags<AccessFs> = make_bitflags!(AccessFs::{ReadFile | ReadDir});

/// [`Rights::WRITE`]'s Landlock rights.
const WRITE_ACCESS: BitFlags<AccessFs> = make_bitflags!(AccessFs::{
    WriteFile | Truncate | MakeReg | MakeDir | MakeFifo | MakeSym | RemoveFile | RemoveDir | Refer
});

/// The rights of [`Rights::WRITE`] that [`Rights::WRITE_PLAIN`] leaves out.
const SPECIAL_ACCESS: BitFlags<AccessFs> = make_bitflags!(AccessFs::{MakeSym | MakeFifo | Refer});

/// What a rule opened by its real path grants, which never writes: a rule
/// that writes is opened beneath a directory's descriptor, every symlink
/// refused (`BoxRules::beneath`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealRights {
    /// [`Rights::READ`].
    Read,
    /// [`Rights::READ`] and [`Rights::EXECUTE`].
    ReadExecute,
}

impl From<RealRights> for Rights {
    fn from(rights: RealRights) -> Self {
        match rights {
            RealRights::Read => Self::READ,
            RealRights::ReadExecute => Self::READ | Self::EXECUTE,
        }
    }
}

/// What a rule's handle names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A directory: the rule's rights hold for everything beneath it.
    Directory,
    /// Anything else (a file, a device, a FIFO): the rule's rights hold for
    /// it alone.
    File,
}

/// Whether a rule's absence is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Nothing there fails the box.
    Required,
    /// Nothing there is allowed, and recorded ([`BoxRules::absent`]);
    /// anything else that keeps it from opening fails the box.
    Optional,
}

/// One rule: a handle opened before the box goes up, what it names, and
/// what it grants beneath it.
#[derive(Debug)]
pub struct Rule {
    /// The handle, `O_PATH`: what Landlock's rule names.
    handle: OwnedFd,
    /// Its absolute path when it opened, for messages.
    path: PathBuf,
    /// What it names.
    kind: Kind,
    /// What it grants.
    rights: Rights,
    /// Why it's there.
    source: RuleSource,
}

impl Rule {
    /// Its absolute path when it opened.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What it names.
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// What it grants.
    pub const fn rights(&self) -> Rights {
        self.rights
    }

    /// Why it's there.
    pub const fn source(&self) -> RuleSource {
        self.source
    }

    /// The Landlock rights it grants (`Rights::access`).
    pub(super) fn access(&self) -> BitFlags<AccessFs> {
        self.rights.access(self.kind)
    }

    /// Its handle, which Landlock's rule names.
    pub(super) fn handle(&self) -> BorrowedFd<'_> {
        self.handle.as_fd()
    }
}

/// A rule as the box installed it: what it named and granted, without its
/// handle, which the box's install hands back beside it
/// (`BoxRules::into_installed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRule {
    /// Its absolute path when it opened, for messages.
    path: PathBuf,
    /// What it named.
    kind: Kind,
    /// What it grants.
    rights: Rights,
    /// Why it's there.
    source: RuleSource,
}

impl InstalledRule {
    /// Its absolute path when it opened.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What it named.
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// What it grants.
    pub const fn rights(&self) -> Rights {
        self.rights
    }

    /// Why it's there.
    pub const fn source(&self) -> RuleSource {
        self.source
    }
}

/// The box's rules, in the order they were added, and the optional ones
/// that had nothing to name.
#[derive(Debug, Default)]
pub struct BoxRules {
    /// The rules.
    rules: Vec<Rule>,
    /// The paths of the optional rules that had nothing to name.
    absent: Vec<PathBuf>,
}

impl BoxRules {
    /// No rules: the run's, which only its opener adds to, once the
    /// process is hardened (`run::RunHardened::prepare`).
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The read rules on directories already opened by their real paths
    /// ([`open_real_dir`]), each with its path, in order: the roots of
    /// eval's box, the project's first, then each linked package's.
    pub(super) fn read_dirs(dirs: impl IntoIterator<Item = (PathBuf, OwnedFd)>) -> Self {
        Self {
            rules: dirs
                .into_iter()
                .enumerate()
                .map(|(at, (path, handle))| Rule {
                    handle,
                    path,
                    kind: Kind::Directory,
                    rights: Rights::READ,
                    source: if at == 0 {
                        RuleSource::Project
                    } else {
                        RuleSource::Linked
                    },
                })
                .collect(),
            absent: Vec::new(),
        }
    }

    /// Adds a rule granting `rights` on what the absolute real path `path`
    /// names, for `source`, opened by that path: `O_PATH | O_NOFOLLOW`, kept only when it
    /// isn't a symlink and the kernel names the handle by `path`.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError::Missing`] when nothing is at `path` and the rule
    /// is [`Need::Required`]; [`RuleError::Symlink`] when `path` names a
    /// symlink; [`RuleError::NotReal`] when the kernel names the handle by
    /// another path (a symlink on `path`, or a directory on it swapped since
    /// it was read); [`RuleError::Open`] when `path` isn't absolute, which
    /// is refused before any call, or when the open or the reads of what it
    /// opened fail otherwise.
    pub(super) fn real(
        &mut self,
        path: &Path,
        rights: RealRights,
        need: Need,
        source: RuleSource,
    ) -> Result<(), RuleError> {
        let opened = if path.is_absolute() {
            open_real(path, libc::O_PATH | libc::O_NOFOLLOW)
                .map_err(Unopened::from)
                .and_then(kind_of)
        } else {
            Err(Unopened::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a rule's real path is absolute",
            )))
        };
        self.add(path.to_owned(), opened, rights.into(), need, source)
    }

    /// Adds a rule granting `rights` on what `path` names beneath the
    /// directory `dir`, whose absolute path is `dir_path`, for `source`, opened by one
    /// `openat2` beneath it with every symlink on the path refused, the last
    /// component's included.
    ///
    /// # Errors
    ///
    /// Returns [`RuleError::Missing`] when nothing is at `path` and the rule
    /// is [`Need::Required`]; [`RuleError::Symlink`] when a symlink is on the
    /// path; [`RuleError::Escapes`] when the path leaves `dir`'s tree (it's
    /// absolute, or a `..` climbs above it); [`RuleError::Open`] when `path`
    /// is empty, which is refused before any call, or when the open or the
    /// read of what it opened fail otherwise.
    pub(super) fn beneath(
        &mut self,
        dir: BorrowedFd<'_>,
        dir_path: &Path,
        path: &Path,
        rights: Rights,
        need: Need,
        source: RuleSource,
    ) -> Result<(), RuleError> {
        let opened = if path.as_os_str().is_empty() {
            Err(Unopened::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a rule's path beneath a directory names something: `.` for the directory",
            )))
        } else {
            beneath::open(dir, path, OpenAs::Path, Links::Refused)
                .map_err(Unopened::Io)
                .and_then(kind_of)
        };
        self.add(dir_path.join(path), opened, rights, need, source)
    }

    /// The rules, in the order they were added.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// The paths of the optional rules that had nothing to name, in the
    /// order they were added.
    pub fn absent(&self) -> &[PathBuf] {
        &self.absent
    }

    /// The rules as installed, each with its handle, in the order they were
    /// added, and the paths of the optional ones that had nothing to name:
    /// what a box that took them keeps, the handles closed by the caller
    /// but those it holds for a program's box (`program::ProgramRules::hold`).
    pub(super) fn into_installed(self) -> (Vec<(InstalledRule, OwnedFd)>, Vec<PathBuf>) {
        let rules = self
            .rules
            .into_iter()
            .map(|rule| {
                let installed = InstalledRule {
                    path: rule.path,
                    kind: rule.kind,
                    rights: rule.rights,
                    source: rule.source,
                };
                (installed, rule.handle)
            })
            .collect();
        (rules, self.absent)
    }

    /// Adds the rule at `path` that `opened` made, or records its absence
    /// or refuses it, as `need` says.
    fn add(
        &mut self,
        path: PathBuf,
        opened: Result<(OwnedFd, Kind), Unopened>,
        rights: Rights,
        need: Need,
        source: RuleSource,
    ) -> Result<(), RuleError> {
        match opened {
            Ok((handle, kind)) => {
                self.rules.push(Rule {
                    handle,
                    path,
                    kind,
                    rights,
                    source,
                });
                Ok(())
            }
            Err(Unopened::Io(error)) if error.raw_os_error() == Some(libc::ENOENT) => match need {
                Need::Optional => {
                    self.absent.push(path);
                    Ok(())
                }
                Need::Required => Err(RuleError::Missing { path }),
            },
            Err(Unopened::Symlink) => Err(RuleError::Symlink { path }),
            Err(Unopened::Renamed(opened)) => Err(RuleError::NotReal { path, opened }),
            Err(Unopened::Io(error)) if error.raw_os_error() == Some(libc::ELOOP) => {
                Err(RuleError::Symlink { path })
            }
            Err(Unopened::Io(error)) if beneath::is_escape(&error) => {
                Err(RuleError::Escapes { path })
            }
            Err(Unopened::Io(source)) => Err(RuleError::Open { path, source }),
        }
    }
}

/// Why a rule couldn't be added: each names the path it was for.
#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    /// A required rule has nothing to name.
    #[error("the box's rule for {} is refused: nothing is at its path", path.display())]
    Missing {
        /// Its path.
        path: PathBuf,
    },
    /// A symlink is on the rule's path, or is what it names.
    #[error(
        "the box's rule for {} is refused: a symlink is on its path, and a rule names what it \
         opened, never what a link leads to",
        path.display()
    )]
    Symlink {
        /// Its path.
        path: PathBuf,
    },
    /// The rule's path leaves the directory it's opened beneath.
    #[error(
        "the box's rule for {} is refused: its path leaves the directory it's opened beneath",
        path.display()
    )]
    Escapes {
        /// Its path.
        path: PathBuf,
    },
    /// The kernel names the rule's handle by another path than the one
    /// opened.
    #[error(
        "the box's rule for {} is refused: the kernel names what it opened {}, so the path \
         isn't real (a symlink is on it, or a directory on it was swapped since it was read)",
        path.display(),
        opened.display()
    )]
    NotReal {
        /// Its path.
        path: PathBuf,
        /// The path the kernel names its handle by.
        opened: PathBuf,
    },
    /// The rule opens beneath the run's private directory, and none was
    /// made.
    #[error(
        "the box's rule for {} is refused: it opens beneath the run's private directory, and none \
         was made",
        path.display()
    )]
    NoPrivateDir {
        /// Its path beneath the private directory.
        path: PathBuf,
    },
    /// The run's private directory's descriptor no longer names the
    /// directory made for the run.
    #[error(
        "the box's rule for {} is refused: the run's private directory's descriptor no longer \
         names the directory made for it",
        path.display()
    )]
    PrivateDirChanged {
        /// The private directory's path.
        path: PathBuf,
    },
    /// The rule's open, or a read of what it opened, failed otherwise.
    #[error("failed to open the box's rule for {}: {source}", path.display())]
    Open {
        /// Its path.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
}

/// Why a rule's handle didn't open, before [`BoxRules::add`] says what that
/// makes of the rule.
enum Unopened {
    /// The open, or a read of what it opened, failed.
    Io(io::Error),
    /// The kernel names the handle by this path, not the one opened.
    Renamed(PathBuf),
    /// It opened a symlink.
    Symlink,
}

impl From<RealUnopened> for Unopened {
    fn from(failure: RealUnopened) -> Self {
        match failure {
            RealUnopened::Io(error) => Self::Io(error),
            RealUnopened::Renamed(opened) => Self::Renamed(opened),
        }
    }
}

/// Why [`open_real`] kept no handle.
enum RealUnopened {
    /// The open, or the read of the handle's path, failed.
    Io(io::Error),
    /// The kernel names the handle by this path, not the one opened.
    Renamed(PathBuf),
}

/// Opens the directory at its absolute real path `dir`, as a root of
/// eval's box: `O_PATH | O_DIRECTORY | O_NOFOLLOW` (and std's
/// `O_CLOEXEC`), so only a directory opens and a symlink in its last
/// component doesn't, kept only when the kernel names the handle by `dir`,
/// so a directory on its path swapped since `dir` was read is refused.
///
/// # Errors
///
/// Returns the error of the open or of reading the handle's path, or one of
/// kind [`io::ErrorKind::Other`] when the handle names another path.
pub(super) fn open_real_dir(dir: &Path) -> io::Result<OwnedFd> {
    open_real(dir, libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW).map_err(|failure| {
        match failure {
            RealUnopened::Io(error) => error,
            RealUnopened::Renamed(opened) => io::Error::other(format!(
                "opened {} as {}, another directory",
                dir.display(),
                opened.display()
            )),
        }
    })
}

/// Opens `path` with `flags`, kept only when the kernel names the handle by
/// `path`, read from `/proc/self/fd`.
fn open_real(path: &Path, flags: libc::c_int) -> Result<OwnedFd, RealUnopened> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .map_err(RealUnopened::Io)?;
    let opened =
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|error| {
            RealUnopened::Io(io::Error::new(
                error.kind(),
                format!("failed to read the path of its handle from /proc/self/fd: {error}"),
            ))
        })?;
    if opened != path {
        return Err(RealUnopened::Renamed(opened));
    }
    Ok(file.into())
}

/// The kind of what `handle` names, read from the handle itself, a symlink
/// refused.
fn kind_of(handle: OwnedFd) -> Result<(OwnedFd, Kind), Unopened> {
    let file = std::fs::File::from(handle);
    let file_type = file.metadata().map_err(Unopened::Io)?.file_type();
    if file_type.is_symlink() {
        return Err(Unopened::Symlink);
    }
    let kind = if file_type.is_dir() {
        Kind::Directory
    } else {
        Kind::File
    };
    Ok((file.into(), kind))
}
