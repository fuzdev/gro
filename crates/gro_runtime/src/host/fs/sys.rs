//! The calls each op makes, over owned arguments: a root's descriptor,
//! shared by count, and the paths as Rust text, so none borrows V8's heap
//! and any can move to another thread. A read's open does too, and the
//! [`Reading`] it returns can move; the read into the buffer V8 allocated,
//! which isn't `Send`, is made on the isolate's thread.

use std::io;

/// What `stat` reported of an entry.
pub struct Info {
    /// `file`, `directory`, `symlink`, or `other`.
    pub kind: &'static str,
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
    pub name: std::ffi::OsString,
    /// `file`, `directory`, `symlink`, or `other`, a symlink never
    /// followed.
    pub kind: &'static str,
}

/// An op's failure: opening the handle's directory beneath its root, or
/// resolving the target beneath that.
pub enum OpError {
    /// The handle's directory's open failed.
    Handle(io::Error),
    /// The target's resolution failed.
    Target(io::Error),
}

/// What [`open_in_root`] opens a path as.
#[derive(Clone, Copy)]
pub enum Opened {
    /// A directory, anything else failing with `ENOTDIR`.
    Directory,
    /// An entry of any kind.
    Entry,
}

#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "linux")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "each op owns what it's given, so it can move to another thread as it is"
)]
mod linux {
    use std::ffi::OsString;
    use std::io;
    use std::os::fd::{AsFd as _, OwnedFd};
    use std::path::Path;
    use std::sync::Arc;

    pub use gro_sys::beneath::Reading;
    use gro_sys::beneath::{self, FileKind, FinalLink, Links, OpenAs};

    use super::{Info, Listed, OpError, Opened};

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

    /// The handle's directory at `beneath` its root, opened, or none
    /// for the root's own: its path is real, so a symlink on it, one
    /// swapped in since, is refused.
    fn handle_dir(root: &OwnedFd, beneath: &str) -> Result<Option<OwnedFd>, OpError> {
        if beneath.is_empty() {
            return Ok(None);
        }
        beneath::open(
            root.as_fd(),
            Path::new(beneath),
            OpenAs::Directory,
            Links::Refused,
        )
        .map(Some)
        .map_err(OpError::Handle)
    }

    /// The entry at `rel` beneath the handle's directory, at `beneath`
    /// the root, its final symlink followed when `follow` says.
    pub fn stat(
        root: Arc<OwnedFd>,
        beneath: String,
        rel: String,
        follow: bool,
    ) -> Result<Info, OpError> {
        let held = handle_dir(&root, &beneath)?;
        let dir = held
            .as_ref()
            .map_or_else(|| root.as_fd(), |held| held.as_fd());
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
            kind: kind_name(stat.kind),
            size,
            mtime_ms: stat.mtime_ms(),
            mode: stat.mode,
        })
    }

    /// The name of an entry's kind.
    const fn kind_name(kind: FileKind) -> &'static str {
        match kind {
            FileKind::File => "file",
            FileKind::Directory => "directory",
            FileKind::Symlink => "symlink",
            FileKind::Other => "other",
        }
    }

    /// The regular file at `rel` beneath the handle's directory, at
    /// `beneath` the root, opened to read to its end; anything else,
    /// and a file that reports more than the most a read takes, is
    /// refused before a byte is read.
    pub fn open_read(root: Arc<OwnedFd>, beneath: String, rel: String) -> Result<Reading, OpError> {
        let held = handle_dir(&root, &beneath)?;
        let dir = held
            .as_ref()
            .map_or_else(|| root.as_fd(), |held| held.as_fd());
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
        let held = handle_dir(&root, &beneath)?;
        let dir = held
            .as_ref()
            .map_or_else(|| root.as_fd(), |held| held.as_fd());
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
        let held = handle_dir(&root, &beneath)?;
        let dir = held
            .as_ref()
            .map_or_else(|| root.as_fd(), |held| held.as_fd());
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
                kind: kind_name(kind),
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
        let held = handle_dir(&root, &beneath)?;
        let dir = held
            .as_ref()
            .map_or_else(|| root.as_fd(), |held| held.as_fd());
        let opened = beneath::open(dir, Path::new(&rel), OpenAs::Directory, Links::Beneath)
            .map_err(OpError::Target)?;
        beneath::path_beneath(root.as_fd(), Path::new(&root_path), opened.as_fd())
            .map_err(OpError::Target)
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
}

#[cfg(not(target_os = "linux"))]
pub use other::*;

/// Every call fails where `gro_sys` isn't built: fs on fgro takes
/// Linux.
#[cfg(not(target_os = "linux"))]
mod other {
    use std::ffi::OsString;
    use std::io;
    use std::os::fd::OwnedFd;
    use std::path::Path;
    use std::sync::Arc;

    use super::{Info, Listed, OpError, Opened};

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "fs on fgro takes Linux")
    }

    pub fn check() -> io::Result<()> {
        Err(unsupported())
    }

    pub fn open_root(_dir: &Path) -> io::Result<OwnedFd> {
        Err(unsupported())
    }

    pub const fn is_escape(_error: &io::Error) -> bool {
        false
    }

    pub const fn is_symlink_refused(_error: &io::Error) -> bool {
        false
    }

    pub fn dir_metadata(_root: &OwnedFd, _rel: &Path) -> io::Result<std::fs::Metadata> {
        Err(unsupported())
    }

    pub fn stat(
        _root: Arc<OwnedFd>,
        _beneath: String,
        _rel: String,
        _follow: bool,
    ) -> Result<Info, OpError> {
        Err(OpError::Target(unsupported()))
    }

    pub fn open_dir(
        _root: Arc<OwnedFd>,
        _root_path: String,
        _beneath: String,
        _rel: String,
    ) -> Result<Option<OsString>, OpError> {
        Err(OpError::Target(unsupported()))
    }

    /// A file opened to read, which nothing makes off Linux.
    pub enum Reading {}

    impl Reading {
        pub const fn size(&self) -> usize {
            match *self {}
        }

        pub fn fill(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            match *self {}
        }

        pub fn rest(&mut self) -> io::Result<Vec<u8>> {
            match *self {}
        }

        pub fn to_end(self) -> io::Result<Vec<u8>> {
            match self {}
        }
    }

    pub fn open_read(
        _root: Arc<OwnedFd>,
        _beneath: String,
        _rel: String,
    ) -> Result<Reading, OpError> {
        Err(OpError::Target(unsupported()))
    }

    pub fn read_link(
        _root: Arc<OwnedFd>,
        _beneath: String,
        _rel: String,
        _follow: bool,
    ) -> Result<OsString, OpError> {
        Err(OpError::Target(unsupported()))
    }

    pub fn list(
        _root: Arc<OwnedFd>,
        _beneath: String,
        _rel: String,
    ) -> Result<Vec<Listed>, OpError> {
        Err(OpError::Target(unsupported()))
    }

    pub fn open_in_root(
        _root: Arc<OwnedFd>,
        _root_path: String,
        _rel: String,
        _opened: Opened,
        _whole: bool,
    ) -> io::Result<Option<OsString>> {
        Err(unsupported())
    }
}
