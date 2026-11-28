//! The filesystem as resolution reads it: [`SourceTree`] and its disk
//! implementation.

use std::io;
use std::path::{Path, PathBuf};

use crate::file_id::FileId;

/// The most bytes a `package.json` may have. Packages are untrusted input,
/// and no real one comes near it.
pub const MAX_PACKAGE_JSON_SIZE: u64 = 16 * 1024 * 1024;

/// The most bytes a module's source may have, the same as
/// [`MAX_PACKAGE_JSON_SIZE`].
///
/// The loader refuses a larger module as it reads it, before any of it is
/// parsed or stripped, so a module that would take the process's memory
/// past any config's fails to load instead.
pub const MAX_MODULE_SIZE: u64 = MAX_PACKAGE_JSON_SIZE;
// gro_cli's tests state it as a literal, which keeps V8 out of their binaries
const _: () = assert!(MAX_MODULE_SIZE == 16 << 20);

/// A read a [`SourceTree`] refused on purpose, rather than failed to make.
///
/// A tree returns it as the source of an `io::Error` (by `From`), so a
/// resolution names it by its variant ([`ReadRefusal::of`]), never by its
/// message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadRefusal {
    /// The file a lookup finds at the path isn't the one an earlier read in
    /// the run found there, so it isn't read again: a tree that serves a
    /// run's later reads from its first refuses it before opening anything.
    #[error(
        "the file at its path is not the one an earlier resolution in this run read, so it \
         isn't read again"
    )]
    ReplacedSinceFirstRead,
    /// The file opened at the path isn't the one its lookup found: it, or a
    /// directory on its path, was replaced in between, so it isn't read
    /// (`read_checked`). Its load-side form is `LoadFailure::ReplacedSinceLookup`.
    #[error("the file at its path is not the one looked up, so it isn't read")]
    ReplacedSinceLookup,
    /// The path lies outside the box the reads are confined to
    /// ([`SourceTree::check_read`]), so it isn't read.
    #[error("{} lies outside the box's roots, so it isn't read", .0.display())]
    OutsideBox(PathBuf),
    /// The path is no file the run read before it started for an entry in
    /// no project, and no directory the run's loader reads from disk holds
    /// it, so it isn't read (`loader::prepass`).
    #[error(
        "{} isn't among the files the run read before it started, so it isn't read",
        .0.display()
    )]
    OutsideGraph(PathBuf),
}

impl ReadRefusal {
    /// The refusal `error` carries, if it is one.
    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref()?.downcast_ref()
    }
}

impl From<ReadRefusal> for io::Error {
    /// The error a tree returns for `refusal`, whose message is the
    /// refusal's.
    fn from(refusal: ReadRefusal) -> Self {
        let kind = match &refusal {
            ReadRefusal::ReplacedSinceFirstRead | ReadRefusal::ReplacedSinceLookup => {
                io::ErrorKind::Other
            }
            ReadRefusal::OutsideBox(_) | ReadRefusal::OutsideGraph(_) => {
                io::ErrorKind::PermissionDenied
            }
        };
        Self::new(kind, refusal)
    }
}

/// A read-only view of the files modules resolve against.
pub trait SourceTree {
    /// Looks up `path`, an absolute path. Returns `Ok(None)` when nothing
    /// exists there, following symlinks. A file comes back with its
    /// [`FileId`], which [`SourceTree::read`] and the module loader check
    /// the file they open against.
    ///
    /// # Errors
    ///
    /// Returns any other failure to read the tree, such as a permission
    /// error.
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>>;

    /// Reads the file at `path`, a real path [`SourceTree::lookup`] found a
    /// file at with the identity `id`, refusing when the file it opens has
    /// another. Only `package.json` files are read, so a file longer than
    /// [`MAX_PACKAGE_JSON_SIZE`] is refused without reading it all.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`io::ErrorKind::FileTooLarge`] for a file
    /// over [`MAX_PACKAGE_JSON_SIZE`], [`ReadRefusal::ReplacedSinceLookup`]
    /// (by `From`) when the opened file isn't the one `id` names, and any
    /// failure to read the file.
    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>>;

    /// Whether the directory at `dir`, a real path, is world-writable: its
    /// mode has the other-write bit (`0o002`), whatever its sticky bit. Any
    /// user can plant files in one, so an upward walk ends below it, reading
    /// no `package.json` in it, and no package is read through one.
    ///
    /// # Errors
    ///
    /// Returns any failure to read the directory's metadata.
    fn is_world_writable(&self, dir: &Path) -> io::Result<bool>;

    /// Told of each path a resolution looked up and went on past, finding
    /// nothing of the kind it looked for there: no `package.json` file in a
    /// directory an upward walk passed, and no package directory in a
    /// `node_modules` the package search passed. A resolution that succeeds
    /// depends on these staying absent as much as on the files it read, so
    /// a tree that records what a graph depends on keeps them. The default
    /// does nothing, so a plain resolution does no work for it.
    fn note_absent(&self, path: &Path) {
        let _ = path;
    }

    /// Checks that the file at the real path `path` may be read, before
    /// any read of it is served. A tree that applies a rule runs it at the
    /// start of its `read`, and a tree that serves a read from bytes it
    /// kept, without reading its inner tree again, runs its inner tree's
    /// check first, so the rule holds however a read is served. The default
    /// admits every path.
    ///
    /// # Errors
    ///
    /// Returns an error when `path` may not be read.
    fn check_read(&self, path: &Path) -> io::Result<()> {
        let _ = path;
        Ok(())
    }
}

/// What [`SourceTree::lookup`] found at a path, at its real path (every
/// symlink resolved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A regular file.
    File {
        /// Its real path.
        path: PathBuf,
        /// Its identity when it was looked up.
        id: FileId,
    },
    /// A directory.
    Dir(PathBuf),
    /// Any other entry, such as a socket or a device.
    Other,
}

/// The process's filesystem.
///
/// A lookup opens the path as a handle that reads nothing (`O_PATH`), then
/// takes the real path the kernel names the handle by (`/proc/self/fd`) and
/// the identity from the handle, so both are one file's whatever is renamed
/// or swapped around it. The path is walked as an open walks it: through a
/// directory the process can't search it fails with `EACCES`, even where a
/// `..` leaves that directory again.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiskTree;

impl SourceTree for DiskTree {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        use std::os::fd::AsRawFd as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        // one handle, so its real path and its identity are one file's
        // whatever is renamed or replaced around it; `O_PATH` reads nothing,
        // so it needs no permission on the file, and a FIFO doesn't block
        let handle = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH)
            .open(path)
        {
            Ok(handle) => handle,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let real_path = std::fs::read_link(format!("/proc/self/fd/{}", handle.as_raw_fd()))
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("its real path can't be read from /proc/self/fd: {error}"),
                )
            })?;
        found(real_path, &handle.metadata()?)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        read_checked(path, id, Some(MAX_PACKAGE_JSON_SIZE))
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        use std::os::unix::fs::PermissionsExt as _;
        let metadata = std::fs::metadata(dir)?;
        Ok(metadata.permissions().mode() & 0o002 != 0)
    }
}

/// What a lookup found: the entry `metadata` describes, read from an open
/// handle after `real_path`, the kernel's name for that handle. `None`
/// when the entry was removed since it was opened, since it has no real
/// path then:
///
/// - its link count is 0, read after the name, so a name read before the
///   removal is never kept;
/// - or the kernel named it by its last path and " (deleted)", as it names
///   an entry unlinked while other links to its file remain, and no entry
///   of its identity is at that name (one whose own name ends so is).
///
/// # Errors
///
/// Returns any failure to read the metadata of what is at `real_path`.
pub(super) fn found(real_path: PathBuf, metadata: &std::fs::Metadata) -> io::Result<Option<Entry>> {
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::MetadataExt as _;
    if metadata.nlink() == 0 {
        return Ok(None);
    }
    if real_path.as_os_str().as_bytes().ends_with(b" (deleted)") {
        match std::fs::symlink_metadata(&real_path) {
            Ok(named) if FileId::of(&named) == FileId::of(metadata) => {}
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
    }
    Ok(Some(if metadata.is_file() {
        Entry::File {
            path: real_path,
            id: FileId::of(metadata),
        }
    } else if metadata.is_dir() {
        Entry::Dir(real_path)
    } else {
        Entry::Other
    }))
}

/// Opens the file at `path`, checks that it is the file `id` names, and
/// reads it from that open file, never from the path again, since the path
/// can name another file by then. The path is followed as any open follows
/// it, so a symlink put in its place opens its target, which has another
/// identity.
///
/// The open doesn't block (`O_NONBLOCK`), so a FIFO put in the
/// file's place is refused by its identity rather than waiting for a writer;
/// the flag changes nothing about reading a regular file.
///
/// # Errors
///
/// Returns [`ReadRefusal::ReplacedSinceLookup`] (by `From`) when the opened
/// file has another identity: the file, or a directory on the way to it, was
/// replaced since it was looked up. Otherwise returns any failure to open,
/// inspect, or read the file, an error of kind
/// [`io::ErrorKind::FileTooLarge`] when it holds more than `max` bytes, and one
/// of kind [`io::ErrorKind::OutOfMemory`] when its size can't be reserved.
pub fn read_checked(path: &Path, id: FileId, max: Option<u64>) -> io::Result<Vec<u8>> {
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    options.custom_flags(libc::O_NONBLOCK);
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if FileId::of(&metadata) != id {
        return Err(ReadRefusal::ReplacedSinceLookup.into());
    }
    let size = metadata.len();
    let too_large = || io::Error::from(io::ErrorKind::FileTooLarge);
    if max.is_some_and(|max| size > max) {
        return Err(too_large());
    }
    // reserved only once the size is known to fit, and fallibly, so a
    // sparse file claiming a huge size errors rather than aborting
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(usize::try_from(size).unwrap_or(usize::MAX))?;
    let Some(max) = max else {
        (&file).read_to_end(&mut bytes)?;
        return Ok(bytes);
    };
    // bounded again, since the file can grow after its size is read
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        return Err(too_large());
    }
    Ok(bytes)
}
