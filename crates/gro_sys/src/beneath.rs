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
//!   to resolve beneath ([`OpenAs::Directory`]), a file to read, or a file
//!   to write, created or replaced ([`OpenAs::Write`]) or only created
//!   ([`OpenAs::CreateNew`]); [`Links::Refused`] refuses every symlink on
//!   the way.
//! - [`stat`] reports an entry's kind, size, permission bits, and
//!   modification time ([`Stat`]), its final symlink followed or not
//!   ([`FinalLink`]).
//! - [`read_dir`] lists a directory's entries ([`DirEntry`]), names as the
//!   kernel's bytes, `.` and `..` left out, in the kernel's order.
//! - [`mkdir`], [`remove_file`], [`remove_dir`], and [`rename`] act on a
//!   path's last component: its parent is opened beneath the descriptor by
//!   `openat2`, then the call (`mkdirat`, `unlinkat`, `renameat`) acts on
//!   that one name in that one directory, a final symlink never followed.
//!
//! Every open of a file's contents is non-blocking (`O_NONBLOCK`), so no
//! open waits: not for a FIFO's other end, a device, or a lease's break.
//! Linux ignores the flag for a regular file's reads and writes; anything
//! else keeps it, so a read with nothing to read fails with `EAGAIN` rather
//! than waiting, and a FIFO with no writer reads as empty. A caller that
//! reads or writes contents tells a regular file by the descriptor's
//! metadata and refuses anything else.
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
//!   calls make returns it raw: a [`rename`] across mounts fails with
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
use std::io;
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

/// What [`open`] opens a path as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAs {
    /// A directory to hold and resolve beneath, readable by nothing
    /// (`O_PATH | O_DIRECTORY`): a handle's own directory, a root's, or the
    /// parent the calls on a last component act in. Anything else fails
    /// with `ENOTDIR`.
    Directory,
    /// A file to read (`O_RDONLY | O_NOCTTY | O_NONBLOCK`). A directory
    /// opens too, and a read of it fails with `EISDIR`; a FIFO opens at once,
    /// with or without a writer.
    Read,
    /// A file to write, created with `mode` (less the process's umask) when
    /// missing and emptied when it exists (`O_WRONLY | O_CREAT | O_TRUNC |
    /// O_NOCTTY | O_NONBLOCK`). A final symlink is followed, beneath; a
    /// directory there fails with `EISDIR`, and a FIFO with no reader with
    /// `ENXIO`.
    Write {
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

/// Opens `path` beneath `dir` as `how` says, `O_CLOEXEC`, resolving it by
/// one `openat2` with `RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS`, and with
/// `RESOLVE_NO_SYMLINKS` for [`Links::Refused`].
///
/// `path` is relative to `dir`; `.` is `dir` itself, and an empty path
/// names nothing (`ENOENT`).
///
/// # Errors
///
/// Returns the kernel's error as the [module doc](self) lists them:
/// `EXDEV` for a path that leaves `dir`'s tree, `ELOOP` for a magic link or
/// a symlink [`Links::Refused`] refuses, `EINVAL` for a mode above
/// `0o7777`.
pub fn open(dir: BorrowedFd<'_>, path: &Path, how: OpenAs, links: Links) -> io::Result<OwnedFd> {
    let (flags, mode) = match how {
        OpenAs::Directory => (libc::O_PATH | libc::O_DIRECTORY, 0),
        OpenAs::Read => (libc::O_RDONLY | NO_WAIT, 0),
        OpenAs::Write { mode } => (
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | NO_WAIT,
            mode,
        ),
        OpenAs::CreateNew { mode } => (
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | NO_WAIT,
            mode,
        ),
    };
    let resolve = match links {
        Links::Beneath => BENEATH,
        Links::Refused => BENEATH | libc::RESOLVE_NO_SYMLINKS,
    };
    openat2(dir, &to_c_path(path.as_os_str())?, flags, mode, resolve)
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
    let mut buffer = vec![0u8; DIR_BUFFER];
    let mut entries = Vec::new();
    loop {
        let filled = getdents64(fd.as_fd(), &mut buffer)?;
        if filled == 0 {
            return Ok(entries);
        }
        parse_dirents(&buffer[..filled], &mut entries)?;
    }
}

/// Makes the directory `path` names beneath `dir`, with `mode` (less the
/// process's umask), by `mkdirat` in its parent.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidInput`] for a last
/// component the [module doc](self) refuses, else the kernel's: `EEXIST`
/// when any entry is there, a symlink included, `ENOENT` for a missing
/// parent.
#[expect(
    unsafe_code,
    reason = "std makes a directory only by a path from the working directory, and a libc call is `unsafe`"
)]
pub fn mkdir(dir: BorrowedFd<'_>, path: &Path, mode: u32) -> io::Result<()> {
    with_last(dir, path, |parent, name| {
        // SAFETY: `mkdirat` reads only the name, a live NUL-terminated
        // string, and takes the descriptor, borrowed for the call
        cvt(unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), mode) })
    })
}

/// Removes the entry `path` names beneath `dir`, which isn't a directory, by
/// `unlinkat` in its parent; a symlink is removed, never its target.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidInput`] for a last
/// component the [module doc](self) refuses, else the kernel's: `EISDIR`
/// for a directory, `ENOENT` when nothing is there.
pub fn remove_file(dir: BorrowedFd<'_>, path: &Path) -> io::Result<()> {
    with_last(dir, path, |parent, name| unlinkat(parent, name, 0))
}

/// Removes the empty directory `path` names beneath `dir`, by `unlinkat`
/// with `AT_REMOVEDIR` in its parent.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidInput`] for a last
/// component the [module doc](self) refuses, else the kernel's:
/// `ENOTEMPTY` for a directory with entries, `ENOTDIR` for anything but a
/// directory, a symlink to one included.
pub fn remove_dir(dir: BorrowedFd<'_>, path: &Path) -> io::Result<()> {
    with_last(dir, path, |parent, name| {
        unlinkat(parent, name, libc::AT_REMOVEDIR)
    })
}

/// Renames the entry `from` names beneath `from_dir` to `to` beneath
/// `to_dir`, by `renameat` from the one parent to the other, replacing an
/// entry at `to` as POSIX's `rename` does; a symlink is moved, never its
/// target.
///
/// Each side's parent is opened beneath its own descriptor, so a rename
/// between two trees leaves neither.
///
/// # Errors
///
/// Returns an error of kind [`io::ErrorKind::InvalidInput`] for a last
/// component the [module doc](self) refuses on either side; one of kind
/// [`io::ErrorKind::CrossesDevices`], with no OS code, when the two are on
/// different mounts, two filesystems or two mounts of one, so a raw `EXDEV`
/// means only a path leaving its tree; else the kernel's: `ENOENT` when
/// nothing is at `from`, `EISDIR` or `ENOTDIR` when `to` is a directory and
/// `from` isn't or the reverse, `ENOTEMPTY` for a directory with entries at
/// `to`.
#[expect(
    unsafe_code,
    reason = "std renames only by paths from the working directory, and a libc call is `unsafe`"
)]
pub fn rename(
    from_dir: BorrowedFd<'_>,
    from: &Path,
    to_dir: BorrowedFd<'_>,
    to: &Path,
) -> io::Result<()> {
    let (from_parent, from_name) = split_last(from.as_os_str())?;
    let (to_parent, to_name) = split_last(to.as_os_str())?;
    let from_parent = open_parent(from_dir, from_parent)?;
    let to_parent = open_parent(to_dir, to_parent)?;
    // SAFETY: `renameat` reads only the two names, live NUL-terminated
    // strings, and takes the two descriptors, each borrowed or owned here
    // for the call
    let result = cvt(unsafe {
        libc::renameat(
            from_parent.borrow(from_dir).as_raw_fd(),
            from_name.as_ptr(),
            to_parent.borrow(to_dir).as_raw_fd(),
            to_name.as_ptr(),
        )
    });
    match result {
        Err(error) if error.raw_os_error() == Some(libc::EXDEV) => Err(context(
            "a rename can't move an entry to another filesystem",
            &error,
        )),
        result => result,
    }
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
/// tree: a raw `EXDEV`, which no call here returns for anything else (a
/// [`rename`] across mounts has the same kind and no OS code).
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

/// Runs `call` with the parent of `path`'s last component, opened beneath
/// `dir` (or `dir` itself for a path without a `/`), and that component.
fn with_last(
    dir: BorrowedFd<'_>,
    path: &Path,
    call: impl FnOnce(BorrowedFd<'_>, &CStr) -> io::Result<()>,
) -> io::Result<()> {
    let (parent, name) = split_last(path.as_os_str())?;
    let parent = open_parent(dir, parent)?;
    call(parent.borrow(dir), &name)
}

/// The parent a last component is acted on in: `dir`, or a directory
/// opened beneath it.
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
