//! The filesystem as resolution reads it: [`SourceTree`], its disk implementation, and
//! file identity.

use std::io;
use std::path::{Path, PathBuf};

/// The most bytes a `package.json` may have. Packages are untrusted input,
/// and no real one comes near it.
pub const MAX_PACKAGE_JSON_SIZE: u64 = 16 * 1024 * 1024;

/// `ELOOP`, the OS error a path through a symlink loop fails with: 40 on
/// Linux and Android outside the MIPS and SPARC architectures, which number
/// it differently, and 62 on macOS, iOS, and the BSDs. Elsewhere it isn't
/// known, and a symlink loop reports as the plain I/O error.
// TODO: `io::ErrorKind::FilesystemLoop` once it's stable, in place of the table
pub(super) const ELOOP: Option<i32> = if cfg!(all(
    any(target_os = "linux", target_os = "android"),
    not(any(
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "mips32r6",
        target_arch = "mips64r6",
        target_arch = "sparc",
        target_arch = "sparc64"
    ))
)) {
    Some(40)
} else if cfg!(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)) {
    Some(62)
} else {
    None
};

/// `O_NONBLOCK`, the open flag that makes opening a FIFO return at once
/// instead of waiting for a writer; it changes nothing about reading a
/// regular file. `None` where it isn't known, and files open blocking.
const O_NONBLOCK: Option<i32> = if cfg!(all(
    any(target_os = "linux", target_os = "android"),
    not(any(
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "mips32r6",
        target_arch = "mips64r6",
        target_arch = "sparc",
        target_arch = "sparc64"
    ))
)) {
    Some(0o4000)
} else if cfg!(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)) {
    Some(4)
} else {
    None
};

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
    /// error, and for [`DiskTree`], an error when the real path turns out to
    /// be a symlink (it changed while it was looked up), and off Unix, an
    /// error of kind [`io::ErrorKind::Unsupported`] for a file (no file
    /// identity to read), so resolution fails closed.
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>>;

    /// Reads the file at `path`, a real path [`SourceTree::lookup`] found a
    /// file at with the identity `id`, refusing when the file it opens has
    /// another. Only `package.json` files are read, so a file longer than
    /// [`MAX_PACKAGE_JSON_SIZE`] is refused without reading it all.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`io::ErrorKind::FileTooLarge`] for a file
    /// over [`MAX_PACKAGE_JSON_SIZE`], an error when the opened file isn't
    /// the one `id` names, and any failure to read the file.
    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>>;

    /// Whether the directory at `dir`, a real path, is world-writable: its
    /// mode has the other-write bit (`0o002`), whatever its sticky bit. Any
    /// user can plant files in one, so an upward walk ends below it, reading
    /// no `package.json` in it, and no package is read through one.
    ///
    /// # Errors
    ///
    /// Returns any failure to read the directory's metadata, and, for
    /// [`DiskTree`] off Unix, an error of kind [`io::ErrorKind::Unsupported`]
    /// (no mode bits to read), so resolution fails closed.
    fn is_world_writable(&self, dir: &Path) -> io::Result<bool>;
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
#[derive(Debug, Clone, Copy, Default)]
pub struct DiskTree;

impl SourceTree for DiskTree {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        let real_path = match std::fs::canonicalize(path) {
            Ok(real_path) => real_path,
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
        // not followed: a real path has no symlink in it, so a last component
        // swapped for one since `canonicalize` reads as one and is refused
        let metadata = std::fs::symlink_metadata(&real_path)?;
        Ok(Some(if metadata.is_file() {
            let id = FileId::of(&metadata)?;
            Entry::File {
                path: real_path,
                id,
            }
        } else if metadata.is_dir() {
            Entry::Dir(real_path)
        } else if metadata.is_symlink() {
            return Err(io::Error::other(format!(
                "{} is a symlink, though it is the real path of {}, so it changed while it was \
                 looked up",
                real_path.display(),
                path.display()
            )));
        } else {
            Entry::Other
        }))
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        read_checked(path, id, Some(MAX_PACKAGE_JSON_SIZE))?
            .ok_or_else(|| io::Error::other(FileId::MISMATCH))
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        let metadata = std::fs::metadata(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            Ok(metadata.permissions().mode() & 0o002 != 0)
        }
        // TODO: no mode bits to read; an ACL check if gro ever runs off Unix,
        // failing closed until then
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "telling a world-writable directory needs Unix mode bits",
            ))
        }
    }
}

/// A file's identity, its device and inode numbers and its birth time, taken
/// when the resolver looked it up.
///
/// The module loader opens the path and refuses the file when the one it
/// opened has another, so a file or a directory on its path swapped between
/// resolving and reading can't substitute another file, except a file remade
/// with no birth time to tell it apart (see [`FileId::born`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    /// The device holding the file.
    pub dev: u64,
    /// The file's inode number on that device.
    pub ino: u64,
    /// When the inode was made, where the filesystem records it: ext4 hands
    /// a file made after one is removed the freed inode number, so the
    /// number alone can't tell them apart. `None` where the filesystem
    /// records no birth time, and then a remade file that reuses the inode
    /// isn't caught, nor is one made within the same tick of the
    /// filesystem's clock as the original.
    pub born: Option<std::time::SystemTime>,
}

/// Opens the file at `path`, checks that it is the file `id` names, and
/// reads it from that open file, never from the path again, since the path
/// can name another file by then. Returns `None` when the opened file has
/// another identity: the file, or a directory on the way to it, was replaced
/// since it was looked up. The path is followed as any open follows it, so a
/// symlink put in its place opens its target, which has another identity.
///
/// The open doesn't block ([`O_NONBLOCK`]), so a FIFO put in the file's
/// place is refused by its identity rather than waiting for a writer.
///
/// # Errors
///
/// Returns any failure to open, inspect, or read the file, an error of kind
/// [`io::ErrorKind::FileTooLarge`] when it holds more than `max` bytes, one of
/// kind [`io::ErrorKind::OutOfMemory`] when its size can't be reserved, and
/// off Unix, an error of kind [`io::ErrorKind::Unsupported`] ([`FileId::of`]).
pub fn read_checked(path: &Path, id: FileId, max: Option<u64>) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    if let Some(flag) = O_NONBLOCK {
        options.custom_flags(flag);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if FileId::of(&metadata)? != id {
        return Ok(None);
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
        return Ok(Some(bytes));
    };
    // bounded again, since the file can grow after its size is read
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        return Err(too_large());
    }
    Ok(Some(bytes))
}

impl FileId {
    /// What a read says when the file it opened isn't the one looked up.
    pub const MISMATCH: &str = "the file at its path is not the one looked up, so it isn't read";

    /// The identity of the file `metadata` was read from.
    ///
    /// # Errors
    ///
    /// Off Unix, an error of kind [`io::ErrorKind::Unsupported`]: there is no
    /// identity to read, so resolution and loading fail closed.
    pub fn of(metadata: &std::fs::Metadata) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Ok(Self {
                dev: metadata.dev(),
                ino: metadata.ino(),
                born: metadata.created().ok(),
            })
        }
        // TODO: the volume serial and file index if gro ever runs off Unix,
        // failing closed until then
        #[cfg(not(unix))]
        {
            let _ = metadata;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "telling a file's identity needs Unix device and inode numbers",
            ))
        }
    }
}
