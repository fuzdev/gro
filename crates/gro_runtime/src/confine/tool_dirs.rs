//! The tool directories a run's box writes, made before it goes up:
//! [`RunHardened::make_tool_dirs`].
//!
//! A rule names what it opened, so a tool directory that isn't there when
//! the box goes up can never be written: each is made first, as a real
//! directory, beneath its anchor (the project's root, or `/` for one in
//! where a `node_modules` symlink leads), every directory on the way
//! opened by its own name with every symlink refused
//! (`gro_sys::beneath::Links::Refused`), and the last made by `mkdirat` in
//! the directory so held. One there already passes only as a directory: a
//! symlink (`node_modules/.vite` linked to the home directory, `.svelte-kit`
//! to `/tmp`) or a file is refused, as is a file on the way to one (`.fuz`
//! for `.fuz/cache`), and nothing is made where it leads. A
//! missing directory on the way is made only beneath the project's root
//! (`.fuz` for `.fuz/cache`); beneath `/`, one missing fails.
//!
//! Kind: part of the box's boundary, the opening without following a
//! symlink, against a symlink planted before the box by code that ran
//! earlier in the tree; its asset is everything outside the tool
//! directory's path, the home directory and `.git` among them.

use std::ffi::OsStr;
use std::io;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::path::{Component, Path, PathBuf};

use gro_sys::beneath::{self, FileKind, Last, Links, OpenAs};

use super::plan::{Anchor, BoxPlan, Opening};
use super::run::RunHardened;

/// The mode a tool directory is made with, less the process's umask, as a
/// tool's own `mkdir` makes it.
const TOOL_DIR_MODE: u32 = 0o777;

/// Why a tool directory couldn't be made.
#[derive(Debug, thiserror::Error)]
pub enum ToolDirError {
    /// Something other than a directory is at its path or on the way to it
    /// (a file at `.fuz` for `.fuz/cache`).
    #[error(
        "the box's tool directory {} is refused: something other than a directory is there or on \
         the way to it, and a tool directory is made only as a real one",
        path.display()
    )]
    NotADirectory {
        /// Its absolute path.
        path: PathBuf,
    },
    /// A symlink is on its path.
    #[error(
        "the box's tool directory {} is refused: a symlink is on its path, and a tool directory \
         is made only where its own path leads",
        path.display()
    )]
    Symlink {
        /// Its absolute path.
        path: PathBuf,
    },
    /// A call failed otherwise.
    #[error("failed to make the box's tool directory {}", path.display())]
    Make {
        /// Its absolute path.
        path: PathBuf,
        /// The call's error.
        #[source]
        source: io::Error,
    },
}

impl RunHardened {
    /// Makes each tool directory `plan` names, as the [module
    /// doc](self) says, once the process is hardened.
    ///
    /// # Errors
    ///
    /// Returns the first that couldn't be made ([`ToolDirError`]); those
    /// before it stay made.
    pub fn make_tool_dirs(&self, plan: &BoxPlan) -> Result<(), ToolDirError> {
        let mut anchors: [Option<OwnedFd>; 2] = [None, None];
        for (_, rule) in plan.tool_dirs() {
            let Opening::Beneath { anchor, path, .. } = &rule.opening else {
                continue;
            };
            let (slot, anchor_path, parents) = match anchor {
                Anchor::Project => (0, plan.root(), Parents::Made),
                Anchor::Slash => (1, Path::new("/"), Parents::Found),
                Anchor::PrivateTemp => continue,
            };
            let full = anchor_path.join(path);
            let fail = |source: io::Error| refused(full.clone(), source);
            let anchor_fd = match &mut anchors[slot] {
                Some(fd) => fd,
                none => none.insert(beneath::open_root(anchor_path).map_err(fail)?),
            };
            make_beneath(anchor_fd.as_fd(), path, parents).map_err(|error| match error {
                Made::NotADirectory => ToolDirError::NotADirectory { path: full.clone() },
                Made::Io(source) => fail(source),
            })?;
        }
        Ok(())
    }
}

/// The error for the tool directory at `path` whose call failed with
/// `source`: a symlink's refusal named as one, and a non-directory on the
/// way to it (`ENOTDIR`) as what's planted there.
fn refused(path: PathBuf, source: io::Error) -> ToolDirError {
    match source.raw_os_error() {
        Some(libc::ELOOP) => ToolDirError::Symlink { path },
        Some(libc::ENOTDIR) => ToolDirError::NotADirectory { path },
        _ => ToolDirError::Make { path, source },
    }
}

/// Whether a missing directory on a tool directory's way is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Parents {
    /// Made, beneath the project's root.
    Made,
    /// Not made: it must be found.
    Found,
}

/// Why [`make_beneath`] made nothing.
enum Made {
    /// Something other than a directory is at the path.
    NotADirectory,
    /// A call failed.
    Io(io::Error),
}

impl From<io::Error> for Made {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Makes the directory at `path` beneath `anchor`, each directory on the
/// way opened by its own name with every symlink refused, a missing one
/// made as `parents` says.
fn make_beneath(anchor: BorrowedFd<'_>, path: &Path, parents: Parents) -> Result<(), Made> {
    let names = path
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a tool directory's path is plain names",
            )),
        })
        .collect::<io::Result<Vec<&OsStr>>>()?;
    let Some((last, on_the_way)) = names.split_last() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a tool directory's path is empty",
        )
        .into());
    };
    let mut dir = beneath::open(anchor, Path::new("."), OpenAs::Directory, Links::Refused)?;
    for name in on_the_way {
        let name = Path::new(name);
        dir = match beneath::open(dir.as_fd(), name, OpenAs::Directory, Links::Refused) {
            Err(error)
                if error.raw_os_error() == Some(libc::ENOENT) && parents == Parents::Made =>
            {
                make_one(dir.as_fd(), name)?;
                beneath::open(dir.as_fd(), name, OpenAs::Directory, Links::Refused)?
            }
            opened => opened?,
        };
    }
    make_one(dir.as_fd(), Path::new(last))
}

/// Makes the directory `name`, one plain name, in `dir`, by `mkdirat`; one
/// there already passes only as a directory, never followed.
fn make_one(dir: BorrowedFd<'_>, name: &Path) -> Result<(), Made> {
    // a name without a `/`, acted on in `dir` itself
    let last = Last::open(dir, name)?;
    match last.mkdir(TOOL_DIR_MODE) {
        Err(error) if error.raw_os_error() == Some(libc::EEXIST) => match last.stat()?.kind {
            FileKind::Directory => Ok(()),
            FileKind::Symlink => Err(io::Error::from_raw_os_error(libc::ELOOP).into()),
            FileKind::File | FileKind::Other => Err(Made::NotADirectory),
        },
        made => Ok(made?),
    }
}
