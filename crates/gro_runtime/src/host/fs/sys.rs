//! The calls each op makes, over owned arguments: a root's descriptor,
//! shared by count, the paths as Rust text, a write's bytes copied out of
//! V8's heap, and the roots a rename or a remove checks its ends against
//! ([`Held`]), so none borrows V8's heap and any can move to another
//! thread. A read's open does too, and the [`Reading`] it returns can move;
//! the read into the buffer V8 allocated, which isn't `Send`, is made on
//! the isolate's thread.

#![expect(
    clippy::needless_pass_by_value,
    reason = "each op owns what it's given, so it can move to another thread as it is"
)]

use std::ffi::OsString;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;

pub use gro_sys::beneath::Reading;
use gro_sys::beneath::{self, FileKind, FinalLink, Last, Links, OpenAs};

use super::roots::{Held, Refusal};

/// What `stat` reported of an entry.
pub struct Info {
    /// What it is.
    pub kind: FileKind,
    /// Its size in bytes.
    pub size: f64,
    /// Its modification time in milliseconds since the epoch, as Node's
    /// `mtimeMs`.
    pub mtime_ms: f64,
    /// Its permission bits, without the file type's.
    pub mode: u32,
}

/// An entry of a listed directory.
pub struct Listed {
    /// Its name, the kernel's bytes.
    pub name: OsString,
    /// What it is, a symlink never followed.
    pub kind: FileKind,
}

/// An op's failure: opening the handle's directory beneath its root,
/// resolving the target beneath that or acting on it, or the rule for a
/// root's directory refusing it.
pub enum OpError {
    /// The handle's directory's open failed.
    Handle(io::Error),
    /// The target's resolution, or the call on it, failed.
    Target(io::Error),
    /// The target is a root's directory or holds one ([`Held`]).
    Refused(Refusal),
}

/// A rename's failure, by the end it's of.
pub enum RenameError {
    /// The handle's directory's open failed.
    Handle(io::Error),
    /// An end's parent's open, its real path's read, or the rename failed,
    /// of the destination when `to` says, as the Node module names it.
    At { to: bool, error: io::Error },
    /// The rule for a root's directory refused it ([`Held::movable`]), of
    /// the destination when `to` says.
    Refused { to: bool, why: Refusal },
}

/// What [`open_in_root`] opens a path as.
#[derive(Clone, Copy)]
pub enum Opened {
    /// A directory, anything else failing with `ENOTDIR`.
    Directory,
    /// An entry of any kind.
    Entry,
}

/// That the kernel serves `openat2`.
pub fn check() -> io::Result<()> {
    beneath::check_openat2()
}

/// A root's descriptor, opened by its real path.
pub fn open_root(dir: &Path) -> io::Result<OwnedFd> {
    beneath::open_root(dir)
}

/// Whether `error` is a path leaving the directory it resolved
/// beneath.
pub fn is_escape(error: &io::Error) -> bool {
    beneath::is_escape(error)
}

/// Whether `error` is a symlink refused on a path resolved with
/// none allowed.
pub fn is_symlink_refused(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

/// The metadata of the directory at `rel` beneath `root`, a
/// symlink anywhere on the way refused.
pub fn dir_metadata(root: &OwnedFd, rel: &Path) -> io::Result<std::fs::Metadata> {
    let fd = beneath::open(root.as_fd(), rel, OpenAs::Directory, Links::Refused)?;
    std::fs::File::from(fd).metadata()
}

/// The directory an op on a handle acts in: its root's, or the handle's
/// directory opened beneath it.
struct HandleDir<'r> {
    /// The root's descriptor.
    root: BorrowedFd<'r>,
    /// The handle's directory, none for the root's own.
    opened: Option<OwnedFd>,
}

impl<'r> HandleDir<'r> {
    /// The handle's directory at `beneath` the root `root`, opened, or
    /// the root's own for none: its path is real, so a symlink on it,
    /// one swapped in since, is refused.
    fn open(root: &'r OwnedFd, beneath: &str) -> io::Result<Self> {
        let root = root.as_fd();
        let opened = if beneath.is_empty() {
            None
        } else {
            Some(beneath::open(
                root,
                Path::new(beneath),
                OpenAs::Directory,
                Links::Refused,
            )?)
        };
        Ok(Self { root, opened })
    }

    /// Its descriptor, borrowed.
    fn fd(&self) -> BorrowedFd<'_> {
        self.opened.as_ref().map_or(self.root, AsFd::as_fd)
    }
}

/// The entry at `rel` beneath the handle's directory, at `beneath`
/// the root, its final symlink followed when `follow` says.
pub fn stat(
    root: Arc<OwnedFd>,
    beneath: String,
    rel: String,
    follow: bool,
) -> Result<Info, OpError> {
    let handle = HandleDir::open(&root, &beneath).map_err(OpError::Handle)?;
    let dir = handle.fd();
    let final_link = if follow {
        FinalLink::Follow
    } else {
        FinalLink::Keep
    };
    let stat = beneath::stat(dir, Path::new(&rel), final_link).map_err(OpError::Target)?;
    #[expect(
        clippy::cast_precision_loss,
        reason = "Node's `size` is the same double"
    )]
    let size = stat.size as f64;
    Ok(Info {
        kind: stat.kind,
        size,
        mtime_ms: stat.mtime_ms(),
        mode: stat.mode,
    })
}

/// The regular file at `rel` beneath the handle's directory, at
/// `beneath` the root, opened to read to its end; anything else,
/// and a file that reports more than the most a read takes, is
/// refused before a byte is read.
pub fn open_read(root: Arc<OwnedFd>, beneath: String, rel: String) -> Result<Reading, OpError> {
    let handle = HandleDir::open(&root, &beneath).map_err(OpError::Handle)?;
    let dir = handle.fd();
    beneath::open_read(dir, Path::new(&rel)).map_err(OpError::Target)
}

/// The target of the symlink at `rel` beneath the handle's
/// directory, at `beneath` the root, as stored; a final symlink
/// followed when `follow` says, which leaves no symlink to read.
pub fn read_link(
    root: Arc<OwnedFd>,
    beneath: String,
    rel: String,
    follow: bool,
) -> Result<OsString, OpError> {
    let handle = HandleDir::open(&root, &beneath).map_err(OpError::Handle)?;
    let dir = handle.fd();
    let final_link = if follow {
        FinalLink::Follow
    } else {
        FinalLink::Keep
    };
    beneath::read_link(dir, Path::new(&rel), final_link).map_err(OpError::Target)
}

/// The entries of the directory at `rel` beneath the handle's
/// directory, at `beneath` the root, a final symlink followed, in
/// the kernel's order: each one's kind as the directory records it,
/// or, where it records none, as its own metadata says, a symlink
/// not followed. The directory's descriptor is closed before this
/// returns, whatever it returns.
pub fn list(root: Arc<OwnedFd>, beneath: String, rel: String) -> Result<Vec<Listed>, OpError> {
    let handle = HandleDir::open(&root, &beneath).map_err(OpError::Handle)?;
    let dir = handle.fd();
    let entries = beneath::read_dir(dir, Path::new(&rel)).map_err(OpError::Target)?;
    let mut listed = Vec::with_capacity(entries.len());
    for entry in entries {
        let kind = if let Some(kind) = entry.kind {
            kind
        } else {
            let named = Path::new(&rel).join(&entry.name);
            beneath::stat(dir, &named, FinalLink::Keep)
                .map_err(OpError::Target)?
                .kind
        };
        listed.push(Listed {
            name: entry.name,
            kind,
        });
    }
    Ok(listed)
}

/// The real path beneath the root, whose real path is
/// `root_path`, of the directory at `rel` beneath the handle's
/// directory, at `beneath` the root; none when the kernel's path
/// isn't beneath the root's ([`beneath::path_beneath`]).
pub fn open_dir(
    root: Arc<OwnedFd>,
    root_path: String,
    beneath: String,
    rel: String,
) -> Result<Option<OsString>, OpError> {
    let handle = HandleDir::open(&root, &beneath).map_err(OpError::Handle)?;
    let dir = handle.fd();
    let opened = beneath::open(dir, Path::new(&rel), OpenAs::Directory, Links::Beneath)
        .map_err(OpError::Target)?;
    beneath::path_beneath(root.as_fd(), Path::new(&root_path), opened.as_fd())
        .map_err(OpError::Target)
}

/// The permission bits a directory is made with, less the
/// process's umask, as Node's `mkdir` makes one.
const DIR_MODE: u32 = 0o777;

/// Writes `bytes` to the regular file at `rel` beneath the handle's
/// directory, at `beneath` the root, a final symlink followed
/// beneath: replacing what it held, or made with `mode` (less the
/// process's umask) when missing; with `create_new`, only made, any
/// entry there failing with `EEXIST`. Anything but a regular file is
/// refused before a byte is written ([`OpenAs::Write`]), though a
/// file's contents are emptied as it opens, before the write, as
/// Node's `writeFile` empties them.
pub fn write(
    root: Arc<OwnedFd>,
    beneath: String,
    rel: String,
    bytes: Vec<u8>,
    create_new: bool,
    mode: u32,
) -> Result<(), OpError> {
    use std::io::Write as _;

    let handle = HandleDir::open(&root, &beneath).map_err(OpError::Handle)?;
    let dir = handle.fd();
    let how = if create_new {
        OpenAs::CreateNew { mode }
    } else {
        OpenAs::Write { mode }
    };
    let fd = beneath::open(dir, Path::new(&rel), how, Links::Beneath).map_err(OpError::Target)?;
    std::fs::File::from(fd)
        .write_all(&bytes)
        .map_err(OpError::Target)
}

/// Makes the directory at `rel` beneath the handle's directory, at
/// `beneath` the root, `rel` as the lexical path spells it (`.` for the
/// handle's own, which is there, and a trailing `/` kept): with
/// `recursive`, each directory missing on the way too. An entry already
/// there is judged as through the Node module: a final symlink is
/// followed beneath the handle, as the Node module's confinement follows
/// it, so a `stat` of the entry that fails for anything but nothing there
/// is the failure (one that leaves the handle as leaving it, a loop, a
/// target through a file); with `recursive`, as Node's recursive `mkdir`
/// judges it, a directory passes, and a `stat` that finds nothing is the
/// failure too (`ENOENT` for a dangling symlink); with `exists_ok`, as
/// the Node module's check judges it, a `stat` of the path as given, its
/// trailing `/` kept, passes on a directory, and is the failure when it
/// fails for anything but nothing there (`ENOTDIR` for a file named with
/// a `/`); anything else fails with `EEXIST`.
pub fn mkdir(
    root: Arc<OwnedFd>,
    beneath: String,
    rel: String,
    recursive: bool,
    exists_ok: bool,
) -> Result<(), OpError> {
    let handle = HandleDir::open(&root, &beneath).map_err(OpError::Handle)?;
    let dir = handle.fd();
    let name = rel.trim_end_matches('/');
    let made = if name == "." {
        Err(io::Error::from_raw_os_error(libc::EEXIST))
    } else if recursive {
        mkdir_all(dir, name)
    } else {
        Last::open(dir, Path::new(name)).and_then(|last| last.mkdir(DIR_MODE))
    };
    let error = match made {
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => error,
        made => return made.map_err(OpError::Target),
    };
    match beneath::stat(dir, Path::new(name), FinalLink::Follow) {
        Err(found) if recursive || found.kind() != io::ErrorKind::NotFound => {
            return Err(OpError::Target(found));
        }
        Ok(stat) if recursive && stat.kind == FileKind::Directory => return Ok(()),
        _ => {}
    }
    if exists_ok {
        match beneath::stat(dir, Path::new(&rel), FinalLink::Follow) {
            Ok(stat) if stat.kind == FileKind::Directory => return Ok(()),
            Err(found) if found.kind() != io::ErrorKind::NotFound => {
                return Err(OpError::Target(found));
            }
            _ => {}
        }
    }
    Err(OpError::Target(error))
}

/// Makes the directory at `rel`, names joined by `/`, beneath `dir`,
/// and each directory missing on the way, as Node's recursive
/// `mkdir` does: the whole path first, then, while a parent is
/// missing, the path a name shorter, until one is made or found,
/// then each after it. A directory found on the way, a final symlink
/// followed beneath, is passed; anything else there is `ENOTDIR`,
/// and at the end, `EEXIST`.
fn mkdir_all(dir: BorrowedFd<'_>, rel: &str) -> io::Result<()> {
    let names: Vec<&str> = rel.split('/').collect();
    let make = |count: usize| {
        let path = names[..count].join("/");
        Last::open(dir, Path::new(&path)).and_then(|last| last.mkdir(DIR_MODE))
    };
    // what's there at a prefix the path runs through
    let through = |count: usize, error: io::Error| {
        let path = names[..count].join("/");
        match beneath::stat(dir, Path::new(&path), FinalLink::Follow) {
            Ok(stat) if stat.kind == FileKind::Directory => Ok(()),
            Err(found) if beneath::is_escape(&found) => Err(found),
            _ if count == names.len() => Err(error),
            _ => Err(io::Error::from_raw_os_error(libc::ENOTDIR)),
        }
    };
    let mut made = names.len();
    loop {
        match make(made) {
            Ok(()) => break,
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) && made > 1 => {
                made -= 1;
            }
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) && made < names.len() => {
                through(made, error)?;
                break;
            }
            Err(error) => return Err(error),
        }
    }
    for count in made + 1..=names.len() {
        match make(count) {
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => {
                through(count, error)?;
            }
            made => made?,
        }
    }
    Ok(())
}

/// The real path of `last`'s entry, as text: its parent's real path,
/// the handle's directory's, `handle_path`, for one in it, else read
/// beneath the root at `root_path` ([`beneath::path_beneath`]), joined
/// with its name, slashes after it dropped.
fn real_end(
    root: &OwnedFd,
    root_path: &str,
    handle_path: &str,
    last: &Last<'_>,
) -> io::Result<String> {
    let parent = match last.parent() {
        None => handle_path.to_owned(),
        Some(parent) => {
            let found = beneath::path_beneath(root.as_fd(), Path::new(root_path), parent)?;
            let beneath = super::beneath_of(root_path, found).map_err(io::Error::other)?;
            super::join(root_path, &beneath)
        }
    };
    let name = last.name().to_string_lossy();
    Ok(super::join(&parent, name.trim_end_matches('/')))
}

/// Removes the entry at `rel` beneath the handle's directory, at
/// `beneath` the root, whose real path is `root_path`, the handle's
/// directory's `handle_path`: a final symlink removed, never its
/// target; a directory when it's empty, or with `recursive`, with
/// everything in it ([`Last::remove_tree`]). When `held` names roots
/// beneath the handle's directory, the entry's real path is read
/// through the directory the remove acts in, and one that is or
/// holds a root's directory is refused, before anything is removed.
pub fn remove(
    root: Arc<OwnedFd>,
    root_path: String,
    beneath: String,
    handle_path: String,
    rel: String,
    recursive: bool,
    held: Option<Held>,
) -> Result<(), OpError> {
    let handle = HandleDir::open(&root, &beneath).map_err(OpError::Handle)?;
    let dir = handle.fd();
    let last = Last::open(dir, Path::new(&rel)).map_err(OpError::Target)?;
    if let Some(held) = held {
        let real = real_end(&root, &root_path, &handle_path, &last).map_err(OpError::Target)?;
        held.removable(&real).map_err(OpError::Refused)?;
    }
    let removed = if recursive {
        last.remove_tree()
    } else {
        match last.remove_file() {
            // Linux's `unlink` refuses a directory with `EISDIR`
            Err(error) if error.raw_os_error() == Some(libc::EISDIR) => last.remove_dir(),
            removed => removed,
        }
    };
    removed.map_err(OpError::Target)
}

/// Renames the entry at `from` beneath the handle's directory, at
/// `beneath` the root, whose real path is `root_path`, the handle's
/// directory's `handle_path`, to `to` beneath it, replacing what's
/// there as the kernel does; a symlink is moved, never its target.
/// When `held` names roots beneath the handle's directory, each
/// end's real path is read through the directory the rename acts in,
/// and [`Held::movable`] decides, before the rename.
///
/// A failure names an end as the Node module's does: a missing
/// entry or a non-directory on the way, which could be either end's,
/// the destination when the source is there (`lstat`, a slash after
/// it kept), else the source; any other failure of the destination's
/// parent's open, the destination, and any other, the source.
pub fn rename(
    root: Arc<OwnedFd>,
    root_path: String,
    beneath: String,
    handle_path: String,
    from: String,
    to: String,
    held: Option<Held>,
) -> Result<(), RenameError> {
    let handle = HandleDir::open(&root, &beneath).map_err(RenameError::Handle)?;
    let dir = handle.fd();
    let at = |to: bool| move |error: io::Error| RenameError::At { to, error };
    let source = Last::open(dir, Path::new(&from)).map_err(at(false))?;
    // which end a missing entry or a non-directory is of
    let named = |error: io::Error| {
        let either = matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR));
        RenameError::At {
            to: either && source.stat().is_ok(),
            error,
        }
    };
    // a missing parent, or one that's no directory, could be either
    // end's, as the Node module confines the destination's no further;
    // anything else is the destination's
    let destination = Last::open(dir, Path::new(&to)).map_err(|error| {
        if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) {
            named(error)
        } else {
            RenameError::At { to: true, error }
        }
    })?;
    if let Some(held) = held {
        let from = real_end(&root, &root_path, &handle_path, &source).map_err(at(false))?;
        let to = real_end(&root, &root_path, &handle_path, &destination).map_err(at(true))?;
        held.movable(&from, &to)
            .map_err(|(to, why)| RenameError::Refused { to, why })?;
    }
    source.rename_to(&destination).map_err(named)
}

/// The real path beneath `root`, whose real path is `root_path`,
/// of what `rel` names beneath it, opened as `opened` says: as the
/// kernel resolves a path, from `root`, when `whole` says the root
/// is `/`, else beneath it; none when the kernel's path isn't
/// beneath the root's ([`beneath::path_beneath`]).
pub fn open_in_root(
    root: Arc<OwnedFd>,
    root_path: String,
    rel: String,
    opened: Opened,
    whole: bool,
) -> io::Result<Option<OsString>> {
    let how = match opened {
        Opened::Directory => OpenAs::Directory,
        Opened::Entry => OpenAs::Path,
    };
    let links = if whole { Links::InRoot } else { Links::Beneath };
    let fd = beneath::open(root.as_fd(), Path::new(&rel), how, links)?;
    beneath::path_beneath(root.as_fd(), Path::new(&root_path), fd.as_fd())
}
