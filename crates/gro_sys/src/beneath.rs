//! Filesystem calls confined beneath a directory descriptor, each decided by
//! the kernel in the call that resolves the path, never by a check before
//! it.
//!
//! Every path resolves by `openat2` beneath the descriptor it's given, with
//! `RESOLVE_BENEATH` and `RESOLVE_NO_MAGICLINKS`: a path, or a symlink it
//! passes through, that would leave the descriptor's tree fails, as does
//! any magic link (`/proc/self/fd/N` and its kin). A whole path opens in one
//! call, so a symlink at its end is followed only as far as the descriptor's
//! tree, never out.
//!
//! The tree is of names, not of files: a mount inside it is inside, and so
//! is a hard link, which is the file it links whatever other names it has,
//! outside the tree among them.
//!
//! - [`open`] opens a path beneath a descriptor, as a directory to hold and
//!   to resolve beneath ([`OpenAs::Directory`]), an entry of any kind to
//!   name ([`OpenAs::Path`]), a regular file to read, or a regular file to
//!   write, created or replaced ([`OpenAs::Write`]), created or kept to
//!   write over ([`OpenAs::Overwrite`]), or only created
//!   ([`OpenAs::CreateNew`]); [`Links::Refused`] refuses every symlink on
//!   the way.
//! - [`open_read`] opens a regular file to read to its end ([`Reading`]),
//!   into a buffer the caller allocates by the size it reports, the rest,
//!   if it lied, into one of the read's own.
//! - [`read_link`] reads a symlink's target as stored, never following it.
//! - [`open_root`] opens a directory by its absolute real path, the one
//!   descriptor the others resolve beneath, refusing any symlink on it.
//! - [`path_beneath`] reads the path the kernel holds for what a
//!   descriptor names, beneath a root's, held to name that entry by a
//!   lookup beneath the root's descriptor.
//! - [`stat`] reports an entry's kind, size, permission bits, and
//!   modification time ([`Stat`]), its final symlink followed or not
//!   ([`FinalLink`]).
//! - [`read_dir`] lists a directory's entries ([`DirEntry`]), names as the
//!   kernel's bytes, `.` and `..` left out, in the kernel's order.
//! - [`set_mode`] sets an entry's permission bits and [`set_times`] its
//!   access and modification times, each on the entry a final symlink
//!   leads to beneath the descriptor, opened as a path only and acted on
//!   through `/proc/self/fd`, never a path the kernel resolves again; or,
//!   for [`set_times`] with [`FinalLink::Keep`], on the entry the last
//!   component names, a symlink itself ([`Last`]).
//! - [`Last`] holds a path's last component: its parent is opened beneath
//!   the descriptor by `openat2`, then each call on it (`mkdirat`,
//!   `symlinkat`, `unlinkat`, `renameat`, `utimensat`) acts on that one
//!   name in that one directory, a final symlink never followed; [`Last::remove_tree`] empties a
//!   directory first, entry by entry, each beneath its own directory's
//!   descriptor.
//!
//! Every open of a file's contents is non-blocking (`O_NONBLOCK`), so no
//! open waits: not for a FIFO's other end, a device, or a lease's break.
//! Then the opened descriptor's metadata (`fstat`) refuses anything but a
//! regular file, before anything is read or written: a directory with
//! `EISDIR`, and a FIFO or a device with an error of no OS code
//! ([`is_not_regular`]), so a FIFO with no writer never reads as an empty
//! file, one with a writer never fails with a spurious `EAGAIN`, and an
//! endless device (`/dev/zero`) is never read. A unix socket fails the
//! open itself (`ENXIO`). Linux ignores the flag for a regular file's
//! reads and writes, which are all that's left.
//!
//! # The last component
//!
//! The calls that act on a path's last component split the path at its last
//! `/` and refuse, before any call, a component that is empty (an empty
//! path, or one of slashes only), `.`, or `..`, or that holds a NUL byte,
//! with an error of kind [`io::ErrorKind::InvalidInput`] and no OS code.
//! Slashes after the last component stay on it and mean what they mean to
//! the kernel: the entry must be a directory, and a symlink named with one
//! is refused (`ENOTDIR`), never acted on through. A path without a `/`
//! names an entry of the descriptor's own directory, acted on with no
//! `openat2`.
//!
//! Between the parent's open and the call, the parent is held by its
//! descriptor, not its path: a rename of a directory on the path in that
//! moment can't make the call act anywhere but in the directory that was
//! beneath the descriptor when it opened, and the call never follows the
//! name. A directory moved out of the tree after its open is acted in where
//! it now is, as a handle's own descriptor keeps a directory moved after it
//! opened; moving it takes write access to both places already.
//!
//! # Errors
//!
//! Each call's error is an [`io::Error`] carrying the OS code, so a caller
//! tells the cases apart by [`io::Error::raw_os_error`]:
//!
//! - `EXDEV`: the path leaves the descriptor's tree, by being absolute, by a
//!   `..` above it, or by a symlink whose target is absolute or climbs above
//!   it, even one that lands back inside (`../root/file`). Nothing else these
//!   calls make returns it raw: a rename the kernel refuses between two
//!   directories ([`Last::rename_to`]) fails with
//!   [`io::ErrorKind::CrossesDevices`] and no OS code. A raw `EXDEV`'s kind
//!   is [`io::ErrorKind::CrossesDevices`] too, so [`is_escape`] is the test
//!   for an escape, never the kind. Whether a path leaving its descriptor
//!   is outside a handle or outside every grant is the caller's to say, by
//!   which descriptor it resolved beneath.
//! - `ELOOP`: a magic link on the path, any symlink under
//!   [`Links::Refused`], or more symlinks than the kernel follows (40).
//! - `ENOENT`, `EEXIST`, `EACCES` or `EPERM`, `ENOTDIR`, `EISDIR`,
//!   `ENOTEMPTY`: the kernel's, for the entry or a directory on its path.
//! - No OS code, [`io::ErrorKind::InvalidInput`]: a path holding a NUL byte,
//!   or a last component refused before any call.
//! - No OS code, told by [`is_not_regular`]: an open of a file's contents
//!   that reached a FIFO or a device; one that reached a directory is
//!   `EISDIR`, as the kernel answers a read of one.
//! - No OS code, [`io::ErrorKind::FileTooLarge`]: an [`open_read`] of a
//!   file that reports more than [`READ_LIMIT`] bytes, and a [`Reading`]
//!   that reads more.
//! - `EINVAL`: a [`read_link`] of an entry that isn't a symlink.
//! - No OS code, [`io::ErrorKind::Unsupported`]: `openat2` is missing, on a
//!   kernel before Linux 5.6 (`ENOSYS`) or under a seccomp filter that
//!   refuses it (`ENOSYS`, or an `EPERM` that [`check_openat2`]'s probe gets
//!   too). Nothing falls back to an unconfined open. A call on a path
//!   without a `/` makes no `openat2` and works there all the same: it acts
//!   on one name in the descriptor's own directory, never followed, so it
//!   can't leave the tree, but the floor isn't met by the first call that
//!   succeeds. A caller meets it at its first use by [`check_openat2`].
//!
//! An `EAGAIN` from `openat2`, which the kernel returns when a rename or a
//! mount during the walk leaves it unable to tell whether a `..` stayed
//! beneath, or when another process's lease on the file would make the open
//! wait, is retried, at most [`RACE_RETRIES`] times, then returned.
//!
//! Each call is synchronous. Its arguments are borrowed for the call only
//! and descriptors are `Send` and `Sync`, so a caller can make it on any
//! thread.

use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs::File;
use std::io::{self, Read as _};
use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, FromRawFd as _, OwnedFd};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::context;

/// The most times [`open`] retries an `openat2` that failed with `EAGAIN`,
/// a rename or a mount racing a `..` of the walk, before returning it.
pub const RACE_RETRIES: u32 = 32;

/// The bytes [`read_dir`] asks `getdents64` for at a time.
const DIR_BUFFER: usize = 32 * 1024;

/// The most bytes a [`Reading`] reads of a file, `2 ** 31 - 1`, the most Node's
/// `readFile` reads: a longer file fails, whatever size it reported.
pub const READ_LIMIT: u64 = (1 << 31) - 1;

/// What [`open`] opens a path as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAs {
    /// A directory to hold and resolve beneath, readable by nothing
    /// (`O_PATH | O_DIRECTORY`): a handle's own directory, a root's, or the
    /// parent the calls on a last component act in. Anything else fails
    /// with `ENOTDIR`.
    Directory,
    /// An entry of any kind to name, readable by nothing (`O_PATH`): its
    /// path beneath a root ([`path_beneath`]) or its metadata. Nothing is opened for its
    /// contents, so a FIFO or a device opens at once, and no permission but
    /// the walk's is needed.
    Path,
    /// A regular file to read (`O_RDONLY | O_NOCTTY | O_NONBLOCK`). The
    /// open never waits, and the descriptor's metadata then refuses
    /// anything else: a directory with `EISDIR`, a FIFO, with or without a
    /// writer, or a device with an error of no OS code
    /// ([`is_not_regular`]); a unix socket fails the open (`ENXIO`).
    Read,
    /// A regular file to write, created with `mode` (less the process's
    /// umask) when missing and emptied when it exists (`O_WRONLY | O_CREAT |
    /// O_TRUNC | O_NOCTTY | O_NONBLOCK`). A final symlink is followed,
    /// beneath; a directory there fails with `EISDIR`, a FIFO with no
    /// reader with `ENXIO`, and one with a reader, or a device, with an
    /// error of no OS code ([`is_not_regular`]), as [`OpenAs::Read`]
    /// refuses them.
    Write {
        /// The permission bits a created file gets, at most `0o7777`.
        mode: u32,
    },
    /// A regular file to write over, created with `mode` (less the
    /// process's umask) when missing and kept as it is when it exists
    /// (`O_WRONLY | O_CREAT | O_NOCTTY | O_NONBLOCK`), so its caller can
    /// tell what it opened before changing a byte, as a copy tells its
    /// source from its destination; refused as [`OpenAs::Write`] refuses
    /// anything but a regular file.
    Overwrite {
        /// The permission bits a created file gets, at most `0o7777`.
        mode: u32,
    },
    /// A file to write that the call creates, with `mode` (less the
    /// process's umask) (`O_WRONLY | O_CREAT | O_EXCL | O_NOCTTY |
    /// O_NONBLOCK`). Any entry already there fails with `EEXIST`: a file, a
    /// directory, or a symlink, dangling or not, which is never followed.
    CreateNew {
        /// The permission bits the file gets, at most `0o7777`.
        mode: u32,
    },
}

/// How [`open`] treats the symlinks on a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Links {
    /// Followed while the walk stays beneath the descriptor, the final one
    /// included (but for [`OpenAs::CreateNew`]).
    Beneath,
    /// Refused wherever they are on the path, the final one included
    /// (`RESOLVE_NO_SYMLINKS`, `ELOOP`): a root or a handle's directory
    /// reached by its own name only.
    Refused,
    /// Followed as though the descriptor were the filesystem's root
    /// (`RESOLVE_IN_ROOT`): an absolute symlink resolves from the
    /// descriptor, and a `..` at it stays there. Beneath the process's own
    /// `/`, that's the kernel's resolution of an absolute path, magic links
    /// still refused; beneath any other directory it would read a symlink's
    /// target as another path than it names, so only a root at `/` takes it.
    InRoot,
}

/// Whether [`stat`] follows a symlink that is a path's last component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalLink {
    /// Followed, beneath, so the entry reported is the link's target.
    Follow,
    /// Not followed, so the entry reported is the link itself.
    Keep,
}

/// The kind of a filesystem entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symlink.
    Symlink,
    /// Anything else: a FIFO, a socket, a device.
    Other,
}

/// What [`stat`] reports of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    /// The entry's kind.
    pub kind: FileKind,
    /// Its size in bytes.
    pub size: u64,
    /// Its permission bits, the setuid, setgid, and sticky bits among them
    /// (`st_mode & 0o7777`), without the file type's.
    pub mode: u32,
    /// Its modification time's whole seconds since the epoch, rounded down,
    /// so a time before the epoch has a negative count and a positive
    /// [`Stat::mtime_nsec`].
    pub mtime_sec: i64,
    /// Its modification time's nanoseconds past [`Stat::mtime_sec`], below a
    /// billion.
    pub mtime_nsec: u32,
}

impl Stat {
    /// The modification time in milliseconds since the epoch, the double
    /// Node's `mtimeMs` is: the seconds times 1000 plus the nanoseconds over
    /// a million, each a double first.
    #[expect(
        clippy::cast_precision_loss,
        reason = "Node's `mtimeMs` is the same double, made by the same conversion"
    )]
    #[expect(
        clippy::suboptimal_flops,
        reason = "a fused multiply-add rounds once where Node rounds twice, so it can differ"
    )]
    pub fn mtime_ms(&self) -> f64 {
        (self.mtime_sec as f64) * 1000.0 + f64::from(self.mtime_nsec) / 1_000_000.0
    }
}

/// One entry of a directory [`read_dir`] lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Its name, the kernel's bytes, which needn't be UTF-8.
    pub name: OsString,
    /// Its kind as the directory records it, a symlink never followed, or
    /// `None` where the filesystem doesn't record it (`DT_UNKNOWN`), when
    /// [`stat`] with [`FinalLink::Keep`] on the name tells.
    pub kind: Option<FileKind>,
}

/// Opens `path` beneath `dir` as `how` says, `O_CLOEXEC`, by one `openat2`.
///
/// It resolves with `RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS`, with
/// `RESOLVE_NO_SYMLINKS` too for [`Links::Refused`], or with
/// `RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS` for [`Links::InRoot`].
///
/// An open of a file's contents ([`OpenAs::Read`], [`OpenAs::Write`],
/// [`OpenAs::Overwrite`], [`OpenAs::CreateNew`]) keeps a regular file
/// only, told by the descriptor's metadata.
///
/// `path` is relative to `dir`; `.` is `dir` itself, and an empty path
/// names nothing (`ENOENT`).
///
/// # Errors
///
/// Returns the kernel's error as the [module doc](self) lists them:
/// `EXDEV` for a path that leaves `dir`'s tree, `ELOOP` for a magic link or
/// a symlink [`Links::Refused`] refuses, `EINVAL` for a mode above
/// `0o7777`; and for an open of contents, `EISDIR` for a directory and an
/// error of no OS code for anything else that isn't a regular file
/// ([`is_not_regular`]).
pub fn open(dir: BorrowedFd<'_>, path: &Path, how: OpenAs, links: Links) -> io::Result<OwnedFd> {
    let fd = open_unchecked(dir, path, how, links)?;
    match how {
        OpenAs::Directory | OpenAs::Path => Ok(fd),
        OpenAs::Read
        | OpenAs::Write { .. }
        | OpenAs::Overwrite { .. }
        | OpenAs::CreateNew { .. } => only_regular(fd).map(|(fd, _)| fd),
    }
}

/// [`open`] without the regular-file check.
fn open_unchecked(
    dir: BorrowedFd<'_>,
    path: &Path,
    how: OpenAs,
    links: Links,
) -> io::Result<OwnedFd> {
    let (flags, mode) = match how {
        OpenAs::Directory => (libc::O_PATH | libc::O_DIRECTORY, 0),
        OpenAs::Path => (libc::O_PATH, 0),
        OpenAs::Read => (libc::O_RDONLY | NO_WAIT, 0),
        OpenAs::Write { mode } => (
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | NO_WAIT,
            mode,
        ),
        OpenAs::Overwrite { mode } => (libc::O_WRONLY | libc::O_CREAT | NO_WAIT, mode),
        OpenAs::CreateNew { mode } => (
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | NO_WAIT,
            mode,
        ),
    };
    let resolve = match links {
        Links::Beneath => BENEATH,
        Links::Refused => BENEATH | libc::RESOLVE_NO_SYMLINKS,
        Links::InRoot => libc::RESOLVE_IN_ROOT | libc::RESOLVE_NO_MAGICLINKS,
    };
    openat2(dir, &to_c_path(path.as_os_str())?, flags, mode, resolve)
}

/// Opens the regular file at `path` beneath `dir` to read to its end, a
/// final symlink followed beneath.
///
/// It's opened as [`OpenAs::Read`], so anything but a regular file is
/// refused before a byte is read, and so is one that reports more than
/// [`READ_LIMIT`] bytes, as Node's `readFile` refuses by the size before
/// it reads. The size it reports is where a read starts, never where it
/// ends ([`Reading`]): a `/proc` file reports 0.
///
/// # Errors
///
/// Returns [`open`]'s errors for [`OpenAs::Read`], and an error of kind
/// [`io::ErrorKind::FileTooLarge`], with no OS code, for a file that
/// reports more than [`READ_LIMIT`] bytes.
pub fn open_read(dir: BorrowedFd<'_>, path: &Path) -> io::Result<Reading> {
    let fd = open_unchecked(dir, path, OpenAs::Read, Links::Beneath)?;
    let (fd, size) = only_regular(fd)?;
    Reading::new(File::from(fd), size, READ_LIMIT)
}

/// A regular file [`open_read`] opened, to read from its start to its end.
///
/// It's read in at most two parts: [`fill`](Self::fill) reads into a buffer the caller
/// allocates of the [`size`](Self::size) the file reported, then
/// [`rest`](Self::rest) reads whatever is past it into a buffer of its
/// own, nothing when the size was right, which takes one more read to
/// tell. Or [`to_end`](Self::to_end) reads it all into one buffer of its
/// own. Past its limit, [`READ_LIMIT`], each fails with an error of kind
/// [`io::ErrorKind::FileTooLarge`] and no OS code.
///
/// A file whose size is right is read straight into the caller's buffer,
/// and nothing more is allocated. One that grew since it reported its
/// size, or reported none, as a `/proc` file reports 0, has its rest read
/// into a buffer of the read's own; one that shrank fills less than the
/// caller's buffer, and its rest is empty.
///
/// It holds the open file and owns everything it's given, so it can move
/// to another thread and be read there.
#[derive(Debug)]
pub struct Reading {
    /// The file, opened.
    file: File,
    /// The size it reported when opened, at most `limit`.
    size: usize,
    /// The bytes read of it so far.
    read: u64,
    /// The most bytes it reads.
    limit: u64,
}

impl Reading {
    /// `file`, opened at its start, which reported `size` bytes, read to at
    /// most `limit` bytes; refused at once when `size` is past `limit`.
    fn new(file: File, size: u64, limit: u64) -> io::Result<Self> {
        if size > limit {
            return Err(too_large(limit));
        }
        let size = usize::try_from(size).map_err(|_| too_large(limit))?;
        Ok(Self {
            file,
            size,
            read: 0,
            limit,
        })
    }

    /// The size the file reported when opened, at most [`READ_LIMIT`]: the
    /// bytes to allocate for [`fill`](Self::fill).
    pub const fn size(&self) -> usize {
        self.size
    }

    /// Reads into `buffer` until it's full or the file ends, and returns
    /// how many bytes it read: fewer than `buffer` holds only at the
    /// file's end.
    ///
    /// # Errors
    ///
    /// Returns the kernel's error for a read, and an error of kind
    /// [`io::ErrorKind::FileTooLarge`] once more than the limit is read.
    pub fn fill(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut filled = 0;
        while filled < buffer.len() {
            match (&self.file).read(&mut buffer[filled..]) {
                Ok(0) => break,
                Ok(read) => filled += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        self.count(filled)?;
        Ok(filled)
    }

    /// Reads the rest of the file, from where it is to its end, into a
    /// buffer of its own: empty when the file ended where it was, which
    /// one read tells without allocating.
    ///
    /// # Errors
    ///
    /// Returns the kernel's error for a read, and an error of kind
    /// [`io::ErrorKind::FileTooLarge`] once more than the limit is read,
    /// counting what was read before.
    pub fn rest(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.read_rest(&mut bytes)?;
        Ok(bytes)
    }

    /// Reads the whole file, from where it is to its end, into one buffer
    /// of its own, its capacity the size the file reported.
    ///
    /// # Errors
    ///
    /// As [`rest`](Self::rest).
    pub fn to_end(mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(self.size);
        self.read_rest(&mut bytes)?;
        Ok(bytes)
    }

    /// Appends the rest of the file to `bytes`, reading at most one byte
    /// past the limit, so a file longer than it fails without being read
    /// whole.
    fn read_rest(&mut self, bytes: &mut Vec<u8>) -> io::Result<()> {
        let left = self.limit.saturating_add(1).saturating_sub(self.read);
        let read = (&self.file).take(left).read_to_end(bytes)?;
        self.count(read)
    }

    /// Counts `read` more bytes read, failing past the limit.
    fn count(&mut self, read: usize) -> io::Result<()> {
        self.read = self
            .read
            .saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if self.read > self.limit {
            return Err(too_large(self.limit));
        }
        Ok(())
    }
}

/// The error of a read past `limit` bytes.
fn too_large(limit: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::FileTooLarge,
        format!("the file is longer than {limit} bytes, the most a read takes"),
    )
}

/// Reads the target of the symlink at `path` beneath `dir`, as stored, the
/// kernel's bytes, never following it.
///
/// With [`FinalLink::Follow`], a final symlink is followed beneath first,
/// so what's reached is no symlink.
/// The entry is opened as a path only (`O_PATH`, with `O_NOFOLLOW` for
/// [`FinalLink::Keep`]), then its target read from the descriptor
/// (`readlinkat` with an empty path), so the link read is the one the
/// open resolved.
///
/// # Errors
///
/// Returns the kernel's error as the [module doc](self) lists them,
/// `ENOENT` when nothing is there, and `EINVAL` for an entry that isn't a
/// symlink, as `readlink` does.
pub fn read_link(dir: BorrowedFd<'_>, path: &Path, final_link: FinalLink) -> io::Result<OsString> {
    let flags = match final_link {
        FinalLink::Follow => libc::O_PATH,
        FinalLink::Keep => libc::O_PATH | libc::O_NOFOLLOW,
    };
    let fd = openat2(dir, &to_c_path(path.as_os_str())?, flags, 0, BENEATH)?;
    let file = File::from(fd);
    if !file.metadata()?.file_type().is_symlink() {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    readlink_empty(file.as_fd())
}

/// The target of the symlink `fd`, opened `O_PATH | O_NOFOLLOW`, names, by
/// `readlinkat` with an empty path, its buffer grown until the target fits.
#[expect(
    unsafe_code,
    reason = "std reads a link only by a path from the working directory, and a libc call is `unsafe`"
)]
fn readlink_empty(fd: BorrowedFd<'_>) -> io::Result<OsString> {
    let mut buffer = vec![0u8; PATH_BUFFER];
    loop {
        // SAFETY: `readlinkat` writes at most `buffer.len()` bytes into the
        // buffer, a live mutable slice, reads only the empty path, a
        // NUL-terminated literal, and takes the descriptor, borrowed for
        // the call
        let result = unsafe {
            libc::readlinkat(
                fd.as_raw_fd(),
                c"".as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        let read = usize::try_from(result).map_err(|_| io::Error::last_os_error())?;
        // a target that fills the buffer may have been cut short
        if read < buffer.len() {
            buffer.truncate(read);
            return Ok(OsString::from_vec(buffer));
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

/// The bytes [`read_link`] first reads a target into, Linux's `PATH_MAX`.
const PATH_BUFFER: usize = 4096;

/// `fd`, an open of a file's contents, and the size it reports, when it's
/// a regular file; `EISDIR` for a directory, as the kernel answers a read
/// or a write of one, and an error of no OS code for anything else
/// ([`is_not_regular`]), which isn't read from or written to.
fn only_regular(fd: OwnedFd) -> io::Result<(OwnedFd, u64)> {
    let file = File::from(fd);
    let metadata = file.metadata()?;
    let file_type = metadata.file_type();
    if file_type.is_file() {
        return Ok((file.into(), metadata.size()));
    }
    if file_type.is_dir() {
        return Err(io::Error::from_raw_os_error(libc::EISDIR));
    }
    Err(io::Error::other(NotRegular))
}

/// The error of an open of contents that reached something other than a
/// regular file or a directory: a FIFO or a device.
#[derive(Debug)]
struct NotRegular;

impl std::fmt::Display for NotRegular {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "not a regular file (a FIFO or a device), whose contents are neither read nor \
             written, since a read or a write of one could wait or never end",
        )
    }
}

impl std::error::Error for NotRegular {}

/// Whether `error`, from an open of a file's contents, is one that reached
/// something other than a regular file or a directory: a FIFO or a device,
/// refused by its metadata with no OS code.
pub fn is_not_regular(error: &io::Error) -> bool {
    error.raw_os_error().is_none()
        && error
            .get_ref()
            .is_some_and(<dyn std::error::Error + Send + Sync>::is::<NotRegular>)
}

/// Opens the directory at `path`, an absolute real path, to resolve beneath.
///
/// It's opened as a path only (`O_PATH | O_DIRECTORY`, `O_CLOEXEC`), by one
/// `openat2` that refuses any symlink on the path (`RESOLVE_NO_SYMLINKS`)
/// and any magic link: a root's own descriptor, opened by the path its
/// holder read as real, so a
/// component swapped for a symlink since then fails rather than opening
/// where the link leads.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidInput`], with no OS
/// code, for a relative path, else the kernel's: `ELOOP` for a symlink on
/// the path, `ENOTDIR` for anything but a directory, `ENOENT` when nothing
/// is there.
#[expect(
    unsafe_code,
    reason = "`openat2` takes the working directory's marker as its descriptor, which only a raw borrow names"
)]
pub fn open_root(path: &Path) -> io::Result<OwnedFd> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a root is opened by its absolute path",
        ));
    }
    // SAFETY: `AT_FDCWD` is a marker the kernel reads as the working
    // directory, not a descriptor anything here closes or reads, and isn't
    // `-1`, the one value a `BorrowedFd` can't hold; `openat2` alone takes
    // it, and ignores it for an absolute path
    let cwd = unsafe { BorrowedFd::borrow_raw(libc::AT_FDCWD) };
    openat2(
        cwd,
        &to_c_path(path.as_os_str())?,
        libc::O_PATH | libc::O_DIRECTORY,
        0,
        libc::RESOLVE_NO_SYMLINKS | libc::RESOLVE_NO_MAGICLINKS,
    )
}

/// The path beneath the root `root` of what `fd` names, as the kernel
/// holds it.
///
/// `root_path` is the root's absolute real path. The path is empty for the
/// root's own directory, and `None` when the kernel's path isn't beneath
/// `root_path`, which only an entry or a root moved since it was opened
/// makes.
///
/// The kernel's path is read from `/proc/self/fd` and checked to be
/// beneath `root_path` by its text before anything is looked up, then held
/// to be `fd`'s entry by opening it beneath `root`'s descriptor, no
/// symlink allowed, and comparing device and inode, so no lookup leaves the
/// root, and a name the kernel keeps for an entry removed since it opened,
/// or one a rename made stale, is never returned.
///
/// # Errors
///
/// Returns `ENOENT` when the path no longer names `fd`'s entry: it was
/// removed, or moved where its name can't be read; an error of kind
/// [`io::ErrorKind::Other`], with no OS code, when `/proc` can't be read
/// (not mounted, or refused).
pub fn path_beneath(
    root: BorrowedFd<'_>,
    root_path: &Path,
    fd: BorrowedFd<'_>,
) -> io::Result<Option<OsString>> {
    let link = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
        .map_err(|error| io::Error::other(format!("/proc/self/fd can't be read: {error}")))?;
    let Some(beneath) = strip_root(root_path.as_os_str(), link.as_os_str()) else {
        return Ok(None);
    };
    let held = File::from(fd.try_clone_to_owned()?).metadata()?;
    let gone = || io::Error::from_raw_os_error(libc::ENOENT);
    let named = if beneath.is_empty() {
        File::from(root.try_clone_to_owned()?).metadata()?
    } else {
        let opened = openat2(
            root,
            &to_c_path(beneath)?,
            libc::O_PATH | libc::O_NOFOLLOW,
            0,
            BENEATH | libc::RESOLVE_NO_SYMLINKS,
        )
        .map_err(|_| gone())?;
        File::from(opened).metadata()?
    };
    if (named.dev(), named.ino()) != (held.dev(), held.ino()) {
        return Err(gone());
    }
    Ok(Some(beneath.to_owned()))
}

/// `path` beneath `root`, both absolute, by their text: empty for `root`
/// itself, else what follows `root` and a `/`; `None` when `path` isn't
/// `root` or beneath it.
pub fn strip_root<'p>(root: &OsStr, path: &'p OsStr) -> Option<&'p OsStr> {
    let (root, path) = (root.as_bytes(), path.as_bytes());
    let rest = path.strip_prefix(root)?;
    let beneath = if rest.is_empty() || root.ends_with(b"/") {
        rest
    } else {
        rest.strip_prefix(b"/")?
    };
    Some(OsStr::from_bytes(beneath))
}

/// Reports the entry at `path` beneath `dir`.
///
/// The entry is opened by `openat2` as a path only (`O_PATH`, with
/// `O_NOFOLLOW` for [`FinalLink::Keep`]), which any entry allows, then read
/// by `statx` on the descriptor. A magic link kept is reported as the link;
/// followed, it's refused (`ELOOP`).
///
/// # Errors
///
/// Returns the kernel's error as the [module doc](self) lists them,
/// `ENOENT` when nothing is there.
pub fn stat(dir: BorrowedFd<'_>, path: &Path, final_link: FinalLink) -> io::Result<Stat> {
    let flags = match final_link {
        FinalLink::Follow => libc::O_PATH,
        FinalLink::Keep => libc::O_PATH | libc::O_NOFOLLOW,
    };
    let fd = openat2(dir, &to_c_path(path.as_os_str())?, flags, 0, BENEATH)?;
    // std reads a descriptor's metadata by `statx` with `AT_EMPTY_PATH`,
    // which reports what an `O_PATH` descriptor names, a symlink included
    let metadata = File::from(fd).metadata()?;
    let file_type = metadata.file_type();
    let kind = if file_type.is_file() {
        FileKind::File
    } else if file_type.is_dir() {
        FileKind::Directory
    } else if file_type.is_symlink() {
        FileKind::Symlink
    } else {
        FileKind::Other
    };
    Ok(Stat {
        kind,
        size: metadata.size(),
        mode: metadata.mode() & 0o7777,
        mtime_sec: metadata.mtime(),
        // the kernel's are below a billion
        mtime_nsec: u32::try_from(metadata.mtime_nsec()).unwrap_or(0),
    })
}

/// Lists the directory at `path` beneath `dir`: every entry but `.` and
/// `..`, unsorted.
///
/// The directory is opened by `openat2` (`O_RDONLY | O_DIRECTORY`, a final
/// symlink followed beneath) and read by `getdents64`, the entries in the
/// order the kernel gives.
///
/// # Errors
///
/// Returns the kernel's error as the [module doc](self) lists them,
/// `ENOTDIR` for anything but a directory; an error of kind
/// [`io::ErrorKind::InvalidData`] when the kernel's records don't parse.
pub fn read_dir(dir: BorrowedFd<'_>, path: &Path) -> io::Result<Vec<DirEntry>> {
    let fd = openat2(
        dir,
        &to_c_path(path.as_os_str())?,
        libc::O_RDONLY | libc::O_DIRECTORY,
        0,
        BENEATH,
    )?;
    entries(fd.as_fd())
}

/// Sets the permission bits of the entry at `path` beneath `dir`, a final
/// symlink followed beneath, to `mode`, at most `0o7777`.
///
/// The entry is opened as a path only ([`OpenAs::Path`]), which any entry
/// allows, then changed by `chmod` of its descriptor's `/proc/self/fd`
/// link, which names the opened entry and nothing a path resolves again;
/// Linux before 6.6 changes no mode through an `O_PATH` descriptor any
/// other way.
///
/// # Errors
///
/// Returns [`open`]'s errors for the entry, else the kernel's for the
/// change (`EPERM` for an entry the process doesn't own), or an error of
/// kind [`io::ErrorKind::Other`], with no OS code, when `/proc` can't be
/// reached.
pub fn set_mode(dir: BorrowedFd<'_>, path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let fd = open(dir, path, OpenAs::Path, Links::Beneath)?;
    through_proc(fd.as_fd(), |link| {
        std::fs::set_permissions(link, std::fs::Permissions::from_mode(mode))
    })
}

/// Sets the access and modification times of the entry at `path` beneath
/// `dir`, each a time since the epoch, to the nanosecond.
///
/// With [`FinalLink::Follow`], it sets those of the entry a final symlink leads to beneath,
/// opened as a path only and changed by `utimensat` of its descriptor's
/// `/proc/self/fd` link, as [`set_mode`] changes one; with
/// [`FinalLink::Keep`], of the entry the last component names, a symlink
/// itself, by `utimensat` in its parent ([`Last`]), which refuses a last
/// component of `.` or `..` as it does.
///
/// # Errors
///
/// Returns [`open`]'s errors for the entry, or [`Last::open`]'s, else the
/// kernel's for the change, or an error of kind [`io::ErrorKind::Other`],
/// with no OS code, when `/proc` can't be reached.
pub fn set_times(
    dir: BorrowedFd<'_>,
    path: &Path,
    final_link: FinalLink,
    atime: std::time::Duration,
    mtime: std::time::Duration,
) -> io::Result<()> {
    let times = [to_timespec(atime)?, to_timespec(mtime)?];
    match final_link {
        FinalLink::Follow => {
            let fd = open(dir, path, OpenAs::Path, Links::Beneath)?;
            through_proc(fd.as_fd(), |link| {
                utimensat(None, &to_c_path(link.as_os_str())?, &times, 0)
            })
        }
        FinalLink::Keep => {
            let last = Last::open(dir, path)?;
            utimensat(
                Some(last.held()),
                &last.name,
                &times,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        }
    }
}

/// `time` as the `timespec` a call takes.
fn to_timespec(time: std::time::Duration) -> io::Result<libc::timespec> {
    Ok(libc::timespec {
        tv_sec: libc::time_t::try_from(time.as_secs())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a time past a time_t"))?,
        tv_nsec: libc::c_long::from(time.subsec_nanos()),
    })
}

/// `call` on the `/proc/self/fd` link of `fd`, which names what `fd` holds
/// open; a missing link is `/proc` that can't be reached, never a missing
/// entry, since `fd` holds the entry.
pub(crate) fn through_proc(
    fd: BorrowedFd<'_>,
    call: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let link = format!("/proc/self/fd/{}", fd.as_raw_fd());
    call(Path::new(&link)).map_err(|error| {
        if error.raw_os_error() == Some(libc::ENOENT) {
            io::Error::other(format!("/proc/self/fd can't be reached: {error}"))
        } else {
            error
        }
    })
}

/// `utimensat(dir, name, times, flags)`, from the working directory when
/// `dir` is none.
#[expect(
    unsafe_code,
    reason = "std sets times only through an open file or a path from the working directory, and a libc call is `unsafe`"
)]
fn utimensat(
    dir: Option<BorrowedFd<'_>>,
    name: &CStr,
    times: &[libc::timespec; 2],
    flags: libc::c_int,
) -> io::Result<()> {
    let dir = dir.map_or(libc::AT_FDCWD, |dir| dir.as_raw_fd());
    // SAFETY: `utimensat` reads only the name, a live NUL-terminated
    // string, and the two times, a live array of two; the descriptor is
    // borrowed for the call, or the working directory's marker
    cvt(unsafe { libc::utimensat(dir, name.as_ptr(), times.as_ptr(), flags) })
}

/// The entries of the directory `fd` holds open to read, but `.` and `..`,
/// in the kernel's order.
fn entries(fd: BorrowedFd<'_>) -> io::Result<Vec<DirEntry>> {
    let mut buffer = vec![0u8; DIR_BUFFER];
    let mut entries = Vec::new();
    loop {
        let filled = getdents64(fd, &mut buffer)?;
        if filled == 0 {
            return Ok(entries);
        }
        parse_dirents(&buffer[..filled], &mut entries)?;
    }
}

/// A path's last component, held in the directory it names an entry of.
///
/// The directory is the path's parent, opened beneath a descriptor by
/// `openat2`, symlinks followed beneath, or the descriptor's own directory
/// for a path without a `/`; the component keeps any slashes after it.
///
/// Each call on it acts on that one name in that one directory by its
/// descriptor (`mkdirat`, `unlinkat`, `renameat`), a final symlink never
/// followed, so what a caller learns of the parent between the open and
/// the call (its real path, by [`path_beneath`]) is of the directory the
/// call acts in.
#[derive(Debug)]
pub struct Last<'d> {
    /// The descriptor the path resolved beneath.
    dir: BorrowedFd<'d>,
    /// The parent, `dir` or a directory opened beneath it.
    parent: Parent,
    /// The component, any slashes after it kept.
    name: CString,
}

impl<'d> Last<'d> {
    /// The last component of `path` beneath `dir`, its parent opened.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`io::ErrorKind::InvalidInput`] for a last
    /// component the [module doc](self) refuses, else the parent's open's:
    /// `EXDEV` for a parent that leaves `dir`'s tree, `ENOENT` for a missing
    /// one, `ENOTDIR` for one that isn't a directory.
    pub fn open(dir: BorrowedFd<'d>, path: &Path) -> io::Result<Self> {
        let (parent, name) = split_last(path.as_os_str())?;
        Ok(Self {
            dir,
            parent: open_parent(dir, parent)?,
            name,
        })
    }

    /// The parent opened beneath the descriptor, or none when it's the
    /// descriptor's own directory.
    pub fn parent(&self) -> Option<BorrowedFd<'_>> {
        match &self.parent {
            Parent::Dir => None,
            Parent::Opened(fd) => Some(fd.as_fd()),
        }
    }

    /// The component, any slashes after it kept.
    pub fn name(&self) -> &OsStr {
        OsStr::from_bytes(self.name.as_bytes())
    }

    /// The directory the component is acted on in.
    fn held(&self) -> BorrowedFd<'_> {
        self.parent.borrow(self.dir)
    }

    /// Reports the entry, as [`stat`] with [`FinalLink::Keep`] does, as
    /// `lstat` does: a symlink not followed, but for a component a slash
    /// follows, which the kernel reads as a directory to follow to.
    ///
    /// # Errors
    ///
    /// Returns [`stat`]'s errors, `ENOENT` when nothing is there.
    pub fn stat(&self) -> io::Result<Stat> {
        stat(self.held(), Path::new(self.name()), FinalLink::Keep)
    }

    /// Makes a directory of the entry, with `mode` (less the process's
    /// umask), by `mkdirat`.
    ///
    /// # Errors
    ///
    /// Returns the kernel's error: `EEXIST` when any entry is there, a
    /// symlink included.
    #[expect(
        unsafe_code,
        reason = "std makes a directory only by a path from the working directory, and a libc call is `unsafe`"
    )]
    pub fn mkdir(&self, mode: u32) -> io::Result<()> {
        // SAFETY: `mkdirat` reads only the name, a live NUL-terminated
        // string, and takes the descriptor, borrowed for the call
        cvt(unsafe { libc::mkdirat(self.held().as_raw_fd(), self.name.as_ptr(), mode) })
    }

    /// Makes a symlink of the entry, pointing at `target`, stored as it is,
    /// by `symlinkat`.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`io::ErrorKind::InvalidInput`] for a
    /// target holding a NUL byte, else the kernel's: `EEXIST` when any
    /// entry is there, a dangling symlink included, which is left as it
    /// is.
    #[expect(
        unsafe_code,
        reason = "std makes a symlink only by a path from the working directory, and a libc call is `unsafe`"
    )]
    pub fn symlink(&self, target: &OsStr) -> io::Result<()> {
        let target = to_c_path(target)?;
        // SAFETY: `symlinkat` reads only the target and the name, live
        // NUL-terminated strings, and takes the descriptor, borrowed for the
        // call
        cvt(unsafe {
            libc::symlinkat(target.as_ptr(), self.held().as_raw_fd(), self.name.as_ptr())
        })
    }

    /// Removes the entry, which isn't a directory, by `unlinkat`; a symlink
    /// is removed, never its target.
    ///
    /// # Errors
    ///
    /// Returns the kernel's error: `EISDIR` for a directory, `ENOENT` when
    /// nothing is there, `ENOTDIR` for a component a slash follows that
    /// isn't a directory, a symlink to one included.
    pub fn remove_file(&self) -> io::Result<()> {
        unlinkat(self.held(), &self.name, 0)
    }

    /// Removes the entry, an empty directory, by `unlinkat` with
    /// `AT_REMOVEDIR`.
    ///
    /// # Errors
    ///
    /// Returns the kernel's error: `ENOTEMPTY` for a directory with
    /// entries, `ENOTDIR` for anything but a directory, a symlink to one
    /// included.
    pub fn remove_dir(&self) -> io::Result<()> {
        unlinkat(self.held(), &self.name, libc::AT_REMOVEDIR)
    }

    /// Removes the entry and, when it's a directory, everything in it
    /// first, no symlink followed: a symlink is removed, never its target.
    ///
    /// A directory is emptied by an explicit stack, never recursion, each
    /// entry acted on by its name in its own directory's descriptor, opened
    /// beneath the one holding it with no symlink allowed
    /// (`RESOLVE_NO_SYMLINKS`), so nothing outside the entry's tree is
    /// reached by a symlink, and a directory swapped for one after it's
    /// listed fails rather than leads anywhere. A mount inside the tree is
    /// inside it, as the [module doc](self) says, and is emptied as any
    /// directory is. Each directory on the way down holds a descriptor
    /// until it's emptied, so a tree deeper than the process may open
    /// descriptors fails (`EMFILE`). An entry gone when it's reached, as
    /// another process makes, is passed over.
    ///
    /// # Errors
    ///
    /// Returns the first call's error, the kernel's, naming no entry:
    /// `ENOENT` when nothing is at the entry, `ENOTDIR` for a component a
    /// slash follows that isn't a directory, `EACCES` for a directory inside
    /// it the process may not change, `ELOOP` for one swapped for a symlink.
    pub fn remove_tree(&self) -> io::Result<()> {
        match unlinkat(self.held(), &self.name, 0) {
            Err(error) if error.raw_os_error() == Some(libc::EISDIR) => {}
            result => return result,
        }
        let top = open_listed(self.held(), &self.name)?;
        // the directories being emptied, the entry's own first, each with
        // its name in the one before it and the entries it has left
        let mut stack = vec![top];
        while let Some(last) = stack.last_mut() {
            let Some(entry) = last.left.pop() else {
                let emptied = stack.pop().map(|listed| listed.name);
                let held = stack
                    .last()
                    .map_or_else(|| self.held(), |listed| listed.fd.as_fd());
                if let Some(name) = emptied {
                    unlinkat(held, &name, libc::AT_REMOVEDIR)?;
                }
                continue;
            };
            let name = to_c_bytes(entry.name.as_bytes())?;
            if entry.kind != Some(FileKind::Directory) {
                match unlinkat(last.fd.as_fd(), &name, 0) {
                    // a directory the listing didn't say was one
                    Err(error) if error.raw_os_error() == Some(libc::EISDIR) => {}
                    Err(error) if error.raw_os_error() == Some(libc::ENOENT) => continue,
                    result => {
                        result?;
                        continue;
                    }
                }
            }
            match open_listed(last.fd.as_fd(), &name) {
                Ok(listed) => stack.push(listed),
                Err(error) if error.raw_os_error() == Some(libc::ENOENT) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Renames the entry to `to`'s, by `renameat` from the one parent to
    /// the other, replacing an entry at `to` as POSIX's `rename` does; a
    /// symlink is moved, never its target. Each side's parent was opened
    /// beneath its own descriptor, so a rename between two trees leaves
    /// neither.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`io::ErrorKind::CrossesDevices`], with no
    /// OS code, when the kernel refuses to move the entry between the two
    /// directories (`EXDEV`): two mounts make it, and so does a Landlock
    /// domain the process inherited, so the error says no more than the
    /// kernel did, and a raw `EXDEV` still means only a path leaving its
    /// tree. Else the kernel's: `ENOENT` when nothing is at the entry,
    /// `EISDIR` or `ENOTDIR` when `to` is a directory and the entry isn't or
    /// the reverse, `ENOTEMPTY` for a directory with entries at `to`,
    /// `EINVAL` for a directory moved into itself.
    #[expect(
        unsafe_code,
        reason = "std renames only by paths from the working directory, and a libc call is `unsafe`"
    )]
    pub fn rename_to(&self, to: &Last<'_>) -> io::Result<()> {
        // SAFETY: `renameat` reads only the two names, live NUL-terminated
        // strings, and takes the two descriptors, each borrowed for the call
        let result = cvt(unsafe {
            libc::renameat(
                self.held().as_raw_fd(),
                self.name.as_ptr(),
                to.held().as_raw_fd(),
                to.name.as_ptr(),
            )
        });
        match result {
            Err(error) if error.raw_os_error() == Some(libc::EXDEV) => Err(context(
                "the kernel refused to move the entry between the two directories",
                &error,
            )),
            result => result,
        }
    }
}

/// A directory [`Last::remove_tree`] is emptying: its descriptor, its name
/// in the directory holding it, and the entries it has left.
struct Listed {
    fd: OwnedFd,
    name: CString,
    left: Vec<DirEntry>,
}

/// The directory `name` names in `parent`, opened to list (`O_RDONLY |
/// O_DIRECTORY`), no symlink allowed, with its entries.
fn open_listed(parent: BorrowedFd<'_>, name: &CStr) -> io::Result<Listed> {
    let fd = openat2(
        parent,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        0,
        BENEATH | libc::RESOLVE_NO_SYMLINKS,
    )?;
    let left = entries(fd.as_fd())?;
    Ok(Listed {
        fd,
        name: name.to_owned(),
        left,
    })
}

/// The flags of every open of a file's contents, so the open never waits:
/// not for a FIFO's other end, a device, or a lease's break, and never makes
/// a terminal the process's. The descriptor stays non-blocking, which a
/// regular file's reads and writes ignore; a caller that reads anything else
/// tells it by the descriptor's metadata first.
const NO_WAIT: libc::c_int = libc::O_NOCTTY | libc::O_NONBLOCK;

/// The resolve flags of every `openat2` here.
const BENEATH: u64 = libc::RESOLVE_BENEATH | libc::RESOLVE_NO_MAGICLINKS;

/// `openat2(dir, path, flags | O_CLOEXEC, mode, resolve)`, retrying an
/// interrupted call, and an `EAGAIN` [`RACE_RETRIES`] times; an `ENOSYS`
/// becomes an error saying what's missing.
#[expect(
    unsafe_code,
    reason = "std has no `openat2`, and a raw system call and adopting a descriptor are `unsafe`"
)]
fn openat2(
    dir: BorrowedFd<'_>,
    path: &CStr,
    flags: libc::c_int,
    mode: u32,
    resolve: u64,
) -> io::Result<OwnedFd> {
    // SAFETY: an all-zero `open_how` is a valid value of the struct of
    // integers, and any field a later `libc` adds stays zero, which the
    // kernel reads as unset
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = u64::from((flags | libc::O_CLOEXEC).cast_unsigned());
    how.mode = u64::from(mode);
    how.resolve = resolve;
    retry_races(|| {
        // SAFETY: `openat2` reads only the path, a live NUL-terminated
        // string, and `how`, a live local whose size is passed with it, and
        // takes the directory's descriptor, borrowed for the call; it writes
        // none of this process's memory
        let result = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                dir.as_raw_fd(),
                path.as_ptr(),
                &raw const how,
                size_of::<libc::open_how>(),
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = libc::c_int::try_from(result)
            .map_err(|_| io::Error::other("openat2 returned a descriptor past an int"))?;
        // SAFETY: the kernel just returned `fd`, a new descriptor that
        // nothing else in the process holds, so the `OwnedFd` is its one
        // owner
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    })
    .map_err(|error| to_openat2_error(error, check_openat2))
}

/// Makes `call` again while it fails with `EINTR`, and at most
/// [`RACE_RETRIES`] times more while it fails with `EAGAIN`, then returns
/// what it last returned.
fn retry_races<T>(mut call: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut races = 0;
    loop {
        match call() {
            Err(error) if error.raw_os_error() == Some(libc::EINTR) => {}
            Err(error) if error.raw_os_error() == Some(libc::EAGAIN) && races < RACE_RETRIES => {
                races += 1;
            }
            result => return result,
        }
    }
}

/// Whether `error`, from any call here, is a path leaving the descriptor's
/// tree: a raw `EXDEV`, which no call here returns for anything else.
///
/// A rename the kernel refuses between two directories
/// ([`Last::rename_to`]) has the same kind and no OS code.
pub fn is_escape(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EXDEV)
}

/// Checks that the kernel serves `openat2` to this process, the call every
/// path here resolves by.
///
/// A caller makes it at its first use, so the Linux 5.6 floor is met there
/// rather than at the first call that needs `openat2` (a call on a single
/// name doesn't).
///
/// It makes one `openat2` the kernel refuses as malformed (`EINVAL`) before
/// reading any argument, so it opens and touches nothing; a seccomp filter
/// refusing `openat2` answers first, with a code of its own. Once it finds
/// `openat2` unserved, it says so again without a call, since a filter can
/// be added to a process but never removed.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::Unsupported`], with no OS
/// code, when `openat2` isn't served.
pub fn check_openat2() -> io::Result<()> {
    check_once(&UNSERVED, probe_openat2)
}

/// Set once [`check_openat2`] finds `openat2` unserved, which stays so.
static UNSERVED: AtomicBool = AtomicBool::new(false);

/// [`check_openat2`] over its record, `unserved`, and its call, `probe`:
/// the probe is made until it once fails, and never after.
fn check_once(unserved: &AtomicBool, probe: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    if unserved.load(Ordering::Relaxed) {
        return Err(unavailable(&"an earlier check found it unserved"));
    }
    probe().map_err(|error| {
        unserved.store(true, Ordering::Relaxed);
        unavailable(&error)
    })
}

/// `openat2`'s error as [`open`] returns it: an `ENOSYS`, or an `EPERM`
/// when [`check_openat2`] finds `openat2` unserved (a seccomp filter
/// refusing it with `EPERM`), says what's missing and keeps no OS code, so
/// no caller reads it as any other unsupported call or as a refused
/// permission; any other is as the kernel gave it.
fn to_openat2_error(error: io::Error, check: impl FnOnce() -> io::Result<()>) -> io::Error {
    match error.raw_os_error() {
        Some(libc::ENOSYS) => unavailable(&error),
        Some(libc::EPERM) if check().is_err() => unavailable(&error),
        _ => error,
    }
}

/// The error saying `openat2` is missing, `cause` the call's error or why
/// no call was made.
fn unavailable(cause: &dyn std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "openat2 is unavailable, which takes Linux 5.6 or later and no seccomp filter \
             refusing it, and no path opens without it: {cause}"
        ),
    )
}

/// One `openat2` with a size below any `open_how`'s: `Ok` when the kernel
/// answers it, with `EINVAL`, else the answer.
#[expect(
    unsafe_code,
    reason = "std has no `openat2`, and a raw system call is `unsafe`"
)]
fn probe_openat2() -> io::Result<()> {
    // SAFETY: with a size of 0 the kernel returns `EINVAL` before reading
    // the path or `how`, both null, and opens nothing; a filter's answer
    // reads nothing either
    let result = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            libc::AT_FDCWD,
            std::ptr::null::<libc::c_char>(),
            std::ptr::null::<libc::open_how>(),
            0usize,
        )
    };
    to_probe_answer(result, io::Error::last_os_error())
}

/// What a probe's `result` and the error it left say: `Ok` for the kernel's
/// `EINVAL`, else the answer as an error.
fn to_probe_answer(result: libc::c_long, error: io::Error) -> io::Result<()> {
    if result != -1 {
        Err(io::Error::other(format!(
            "openat2 answered {result} to a call the kernel refuses"
        )))
    } else if error.raw_os_error() == Some(libc::EINVAL) {
        Ok(())
    } else {
        Err(error)
    }
}

/// `getdents64(fd, buffer)`: the bytes of records the kernel wrote, 0 at the
/// directory's end.
#[expect(
    unsafe_code,
    reason = "std reads a directory only by its path, and a raw system call is `unsafe`"
)]
fn getdents64(fd: BorrowedFd<'_>, buffer: &mut [u8]) -> io::Result<usize> {
    // SAFETY: `getdents64` writes at most `buffer.len()` bytes into the
    // buffer, a live mutable slice, and takes the descriptor, borrowed for
    // the call
    let result = unsafe {
        libc::syscall(
            libc::SYS_getdents64,
            fd.as_raw_fd(),
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    usize::try_from(result).map_err(|_| io::Error::other("getdents64 returned past a usize"))
}

/// Where a `linux_dirent64` record's fields lie: `d_ino` (8 bytes), `d_off`
/// (8), `d_reclen` (2), `d_type` (1), then the NUL-terminated name.
const DIRENT_RECLEN: usize = 16;
const DIRENT_TYPE: usize = 18;
const DIRENT_NAME: usize = 19;

/// Appends each entry of `records`, `getdents64`'s bytes, to `entries`, but
/// `.` and `..`.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidData`] for a record
/// whose length runs past the bytes or whose name has no NUL.
fn parse_dirents(records: &[u8], entries: &mut Vec<DirEntry>) -> io::Result<()> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "getdents64 wrote a record that doesn't parse",
        )
    };
    let mut rest = records;
    while !rest.is_empty() {
        let reclen = rest
            .get(DIRENT_RECLEN..DIRENT_RECLEN + 2)
            .map(|bytes| usize::from(u16::from_ne_bytes([bytes[0], bytes[1]])))
            .ok_or_else(invalid)?;
        let record = rest
            .get(..reclen)
            .filter(|record| record.len() > DIRENT_NAME)
            .ok_or_else(invalid)?;
        let name = &record[DIRENT_NAME..];
        let name = &name[..name
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(invalid)?];
        if name != b"." && name != b".." {
            entries.push(DirEntry {
                name: OsString::from_vec(name.to_vec()),
                kind: match record[DIRENT_TYPE] {
                    libc::DT_REG => Some(FileKind::File),
                    libc::DT_DIR => Some(FileKind::Directory),
                    libc::DT_LNK => Some(FileKind::Symlink),
                    libc::DT_UNKNOWN => None,
                    _ => Some(FileKind::Other),
                },
            });
        }
        rest = &rest[reclen..];
    }
    Ok(())
}

/// `unlinkat(parent, name, flags)`.
#[expect(
    unsafe_code,
    reason = "std removes only by a path from the working directory, and a libc call is `unsafe`"
)]
fn unlinkat(parent: BorrowedFd<'_>, name: &CStr, flags: libc::c_int) -> io::Result<()> {
    // SAFETY: `unlinkat` reads only the name, a live NUL-terminated string,
    // and takes the descriptor, borrowed for the call
    cvt(unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) })
}

/// The parent a last component is acted on in: `dir`, or a directory
/// opened beneath it.
#[derive(Debug)]
enum Parent {
    Dir,
    Opened(OwnedFd),
}

impl Parent {
    fn borrow<'a>(&'a self, dir: BorrowedFd<'a>) -> BorrowedFd<'a> {
        match self {
            Self::Dir => dir,
            Self::Opened(fd) => fd.as_fd(),
        }
    }
}

/// Opens `parent` beneath `dir` as a directory, symlinks followed beneath,
/// or names `dir` itself when there's none.
fn open_parent(dir: BorrowedFd<'_>, parent: Option<CString>) -> io::Result<Parent> {
    parent.map_or(Ok(Parent::Dir), |parent| {
        openat2(dir, &parent, libc::O_PATH | libc::O_DIRECTORY, 0, BENEATH).map(Parent::Opened)
    })
}

/// Splits `path` at the `/` before its last component: the parent, `None`
/// for a path without one (`/` for one whose only `/` leads it), and the
/// component with any slashes after it.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidInput`], before any
/// call, for a path holding a NUL byte or a last component that is empty,
/// `.`, or `..`.
fn split_last(path: &OsStr) -> io::Result<(Option<CString>, CString)> {
    let bytes = path.as_bytes();
    let trimmed = &bytes[..bytes
        .iter()
        .rposition(|&byte| byte != b'/')
        .map_or(0, |last| last + 1)];
    let (parent, name, component) = match trimmed.iter().rposition(|&byte| byte == b'/') {
        None => (None, bytes, trimmed),
        Some(0) => (Some(&b"/"[..]), &bytes[1..], &trimmed[1..]),
        Some(slash) => (
            Some(&trimmed[..slash]),
            &bytes[slash + 1..],
            &trimmed[slash + 1..],
        ),
    };
    if matches!(component, b"" | b"." | b"..") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} names no entry by its last component, which is empty, `.`, or `..`",
                path.display()
            ),
        ));
    }
    let parent = parent.map(to_c_bytes).transpose()?;
    Ok((parent, to_c_bytes(name)?))
}

/// `path` as the NUL-terminated string a call takes.
fn to_c_path(path: &OsStr) -> io::Result<CString> {
    to_c_bytes(path.as_bytes())
}

/// `bytes` as a NUL-terminated string, or an error of kind
/// [`io::ErrorKind::InvalidInput`] when they hold a NUL.
fn to_c_bytes(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a path can't hold a NUL byte"))
}

/// A libc call's result, 0 or -1 with `errno` set, as a result.
fn cvt(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::io;
    use std::os::unix::ffi::OsStrExt as _;

    /// A missing `openat2` is an error that says so, with no OS code a
    /// caller could read as another call's; an `EPERM` is one only when the
    /// check finds `openat2` unserved, and only an `EPERM` makes the check;
    /// any other error is the kernel's.
    #[test]
    fn a_missing_openat2_says_so() {
        let checks = std::cell::Cell::new(0);
        let to_error = |code, is_served: bool| {
            super::to_openat2_error(io::Error::from_raw_os_error(code), || {
                checks.set(checks.get() + 1);
                if is_served {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(libc::EPERM))
                }
            })
        };
        for (code, is_served, checked) in [
            (libc::ENOSYS, true, 0),
            (libc::ENOSYS, false, 0),
            (libc::EPERM, false, 1),
        ] {
            checks.set(0);
            let error = to_error(code, is_served);
            assert_eq!(
                (error.kind(), error.raw_os_error(), checks.get()),
                (io::ErrorKind::Unsupported, None, checked),
                "{code} {is_served}"
            );
            let message = error.to_string();
            assert!(
                message.starts_with("openat2 is unavailable, which takes Linux 5.6 or later"),
                "{message}"
            );
        }
        for (code, is_served, checked) in [
            (libc::EPERM, true, 1),
            (libc::EACCES, false, 0),
            (libc::EXDEV, false, 0),
        ] {
            checks.set(0);
            let error = to_error(code, is_served);
            assert_eq!(
                (error.raw_os_error(), checks.get()),
                (Some(code), checked),
                "{code} {is_served}"
            );
        }
    }

    /// The kernel the tests run on serves `openat2`, as every other test here
    /// needs, and the check says so, by a probe that opens nothing.
    #[test]
    fn openat2_is_served_here() {
        super::probe_openat2().unwrap();
        super::check_openat2().unwrap();
    }

    /// A served answer is never recorded, so each check probes again; an
    /// unserved one is, so no later check probes, and every one fails.
    #[test]
    fn only_an_unserved_openat2_is_recorded() {
        let unserved = std::sync::atomic::AtomicBool::new(false);
        let probes = std::cell::Cell::new(0);
        let check = |served: bool| {
            super::check_once(&unserved, || {
                probes.set(probes.get() + 1);
                if served {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(libc::EPERM))
                }
            })
        };
        for _ in 0..2 {
            check(true).unwrap();
        }
        assert_eq!(probes.get(), 2);
        for (served, cause) in [
            (false, "Operation not permitted (os error 1)"),
            (false, "an earlier check found it unserved"),
            (true, "an earlier check found it unserved"),
        ] {
            let error = check(served).unwrap_err();
            assert_eq!(
                (error.kind(), error.raw_os_error()),
                (io::ErrorKind::Unsupported, None)
            );
            let message = error.to_string();
            assert!(message.ends_with(cause), "{message}");
        }
        assert_eq!(probes.get(), 3);
    }

    /// Only the kernel's `EINVAL` says the probe was served: a filter's
    /// code, or a result a refused call never has, says it wasn't.
    #[test]
    fn only_einval_answers_the_probe() {
        let answer =
            |result, code| super::to_probe_answer(result, io::Error::from_raw_os_error(code));
        assert!(answer(-1, libc::EINVAL).is_ok());
        for (result, code) in [
            (-1, libc::ENOSYS),
            (-1, libc::EPERM),
            (-1, libc::EACCES),
            (0, libc::EINVAL),
            (3, 0),
        ] {
            assert!(answer(result, code).is_err(), "{result} {code}");
        }
    }

    /// Only a raw `EXDEV` is an escape: not a rename across mounts, which
    /// has its kind, nor any other error.
    #[test]
    fn an_escape_is_told_by_its_code_alone() {
        assert!(super::is_escape(&io::Error::from_raw_os_error(libc::EXDEV)));
        let across = crate::context("a rename", &io::Error::from_raw_os_error(libc::EXDEV));
        assert_eq!(across.kind(), io::ErrorKind::CrossesDevices);
        assert!(!super::is_escape(&across));
        assert!(!super::is_escape(&io::Error::from_raw_os_error(
            libc::ELOOP
        )));
    }

    /// An `EAGAIN` is retried `RACE_RETRIES` times and then returned, an
    /// `EINTR` between them counting for none, and any other result is
    /// returned at once.
    #[test]
    fn an_eagain_is_retried_a_bounded_number_of_times() {
        fn error<T>(code: i32) -> io::Result<T> {
            Err(io::Error::from_raw_os_error(code))
        }
        // a retry past the bound would see `ENOENT`, not `EAGAIN`
        let mut calls = 0;
        let result: io::Result<()> = super::retry_races(|| {
            calls += 1;
            error(if calls > 1000 {
                libc::ENOENT
            } else {
                libc::EAGAIN
            })
        });
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EAGAIN));
        assert_eq!(calls, super::RACE_RETRIES + 1);

        let mut calls = 0;
        let result = super::retry_races(|| {
            calls += 1;
            match calls {
                n if n % 2 == 0 => error(libc::EINTR),
                n if n < 2 * super::RACE_RETRIES + 1 => error(libc::EAGAIN),
                n => Ok(n),
            }
        });
        assert_eq!(result.unwrap(), 2 * super::RACE_RETRIES + 1);

        let mut calls = 0;
        let result: io::Result<()> = super::retry_races(|| {
            calls += 1;
            error(libc::EXDEV)
        });
        assert_eq!(
            (result.unwrap_err().raw_os_error(), calls),
            (Some(libc::EXDEV), 1)
        );
    }

    /// The parent and the last component of each path, slashes after the
    /// component kept on it, and a parent of `/` for a path whose only `/`
    /// leads it.
    #[test]
    fn a_path_splits_at_its_last_component() {
        for (path, parent, name) in [
            ("f", None, "f"),
            ("f/", None, "f/"),
            ("a/f", Some("a"), "f"),
            ("a/b/f//", Some("a/b"), "f//"),
            ("a//f", Some("a/"), "f"),
            ("/f", Some("/"), "f"),
            ("//f", Some("/"), "f"),
            ("../f", Some(".."), "f"),
            (".f", None, ".f"),
            ("a/...", Some("a"), "..."),
        ] {
            let (got_parent, got_name) = super::split_last(OsStr::new(path)).unwrap();
            assert_eq!(
                got_parent.as_ref().map(|parent| parent.to_bytes()),
                parent.map(str::as_bytes),
                "{path}"
            );
            assert_eq!(got_name.to_bytes(), name.as_bytes(), "{path}");
        }
    }

    /// A read takes a file to its end, whatever size within its limit it
    /// reported, more or less than it holds or none, whole or filled to
    /// its size then the rest, and fails past its limit, never at it, at
    /// once for a size reported past it, before a byte is read.
    #[test]
    fn a_read_ends_at_the_file_s_end_within_its_limit() {
        use std::io::{Seek as _, Write as _};
        let path = std::env::temp_dir().join(format!("gro_sys_read_within_{}", std::process::id()));
        let mut file = std::fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        file.write_all(b"12345").unwrap();
        let too_large = Err((io::ErrorKind::FileTooLarge, None));
        for (size, limit, expected, filled) in [
            (0, 5, Ok(&b"12345"[..]), 0),
            (2, 5, Ok(b"12345"), 2),
            (5, 5, Ok(b"12345"), 5),
            // a size past what the file holds fills less
            (7, 9, Ok(b"12345"), 5),
            // a size past the limit fails before a byte is read
            (6, 5, too_large, 0),
            (5, 4, too_large, 0),
            // a size within the limit, and more bytes than it
            (2, 4, too_large, 2),
            (0, 0, too_large, 0),
        ] {
            let shown = |read: io::Result<Vec<u8>>| {
                read.map_err(|error| (error.kind(), error.raw_os_error()))
            };
            let expected = expected.map(<[u8]>::to_vec);
            let mut at_start = || {
                file.rewind().unwrap();
                super::Reading::new(file.try_clone().unwrap(), size, limit)
            };
            let whole = at_start().and_then(super::Reading::to_end);
            assert_eq!(shown(whole), expected, "{size} {limit} whole");
            let parts = at_start().and_then(|mut reading| {
                let mut bytes = vec![0; reading.size()];
                let read = reading.fill(&mut bytes)?;
                assert_eq!(read, filled, "{size} {limit} filled");
                bytes.truncate(read);
                bytes.extend(reading.rest()?);
                Ok(bytes)
            });
            assert_eq!(shown(parts), expected, "{size} {limit} in parts");
            if size > limit {
                assert_eq!(file.stream_position().unwrap(), 0, "{size} {limit} read");
            }
        }
    }

    /// A fill reads until its buffer is full, however few bytes each read
    /// gives: a datagram socket gives one datagram a read, as a regular
    /// file gives at most `0x7ffff000` bytes.
    #[test]
    fn a_fill_reads_until_its_buffer_is_full() {
        use std::os::unix::net::UnixDatagram;
        let (reader, writer) = UnixDatagram::pair().unwrap();
        writer.send(b"12").unwrap();
        writer.send(b"345").unwrap();
        let file = std::fs::File::from(std::os::fd::OwnedFd::from(reader));
        let mut reading = super::Reading::new(file, 5, 5).unwrap();
        let mut bytes = [0; 5];
        assert_eq!(reading.fill(&mut bytes).unwrap(), 5);
        assert_eq!(&bytes, b"12345");
    }

    /// A read past its limit reads at most one byte past it, counting what
    /// its fill read, so a file longer than the limit is never read whole.
    #[test]
    fn a_read_past_its_limit_reads_one_byte_past_it() {
        use std::io::{Read as _, Write as _};
        use std::os::unix::net::UnixStream;
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"1234567890").unwrap();
        drop(writer);
        let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(reader));
        let mut reading = super::Reading::new(file.try_clone().unwrap(), 2, 4).unwrap();
        let mut bytes = [0; 2];
        assert_eq!(reading.fill(&mut bytes).unwrap(), 2);
        let error = reading.rest().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
        let mut left = Vec::new();
        file.read_to_end(&mut left).unwrap();
        assert_eq!(left, b"67890");
    }

    /// Records that run past the bytes, are too short for a name, or hold a
    /// name with no NUL fail to parse; `.` and `..` are left out.
    #[test]
    fn dirent_records_parse_or_fail() {
        let record = |name: &[u8], kind: u8, reclen: u16| {
            let mut bytes = vec![0u8; 16];
            bytes.extend_from_slice(&reclen.to_ne_bytes());
            bytes.push(kind);
            bytes.extend_from_slice(name);
            bytes
        };
        let mut good = record(b".\0", libc::DT_DIR, 24);
        good.resize(24, 0);
        good.extend(record(b"f\xff\0", libc::DT_REG, 24));
        good.resize(48, 0);
        let mut entries = Vec::new();
        super::parse_dirents(&good, &mut entries).unwrap();
        assert_eq!(
            entries,
            [super::DirEntry {
                name: OsStr::from_bytes(b"f\xff").to_owned(),
                kind: Some(super::FileKind::File),
            }]
        );
        for bad in [
            record(b"f\0", libc::DT_REG, 64),
            record(b"", libc::DT_REG, 19),
            record(b"f", libc::DT_REG, 20),
            vec![0u8; 10],
        ] {
            let error = super::parse_dirents(&bad, &mut Vec::new()).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{bad:?}");
        }
    }
}
