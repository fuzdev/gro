//! The box a loader's reads are checked against, and the wrapper that
//! checks a `SourceTree`'s reads against it.

use std::cell::OnceCell;
use std::io;
use std::path::{Path, PathBuf};

use crate::confine::BoxRoots;
use crate::resolver::{Entry, FileId, ReadRefusal, SourceTree};

/// The roots of the box a loader's reads are confined to, if any, and the
/// first read it refused. Apart from the loader's graph, so a check made
/// while the graph is borrowed touches nothing of it.
#[derive(Debug, Default)]
pub(super) struct BoxCheck {
    /// The box's roots: a module or a `package.json` whose real path lies
    /// under none of them isn't read. Unset, every read is admitted.
    roots: Option<BoxRoots>,
    /// The first read refused for lying outside `roots`.
    breach: OnceCell<PathBuf>,
}

impl BoxCheck {
    /// A check against `roots`, admitting every read when unset.
    pub(super) const fn new(roots: Option<BoxRoots>) -> Self {
        Self {
            roots,
            breach: OnceCell::new(),
        }
    }

    /// The first path a read was refused at, if any.
    pub(super) fn breach(&self) -> Option<&Path> {
        self.breach.get().map(PathBuf::as_path)
    }

    /// Checks a read of the real path `path` against the box, recording the
    /// first refusal.
    pub(super) fn check_read(&self, path: &Path) -> io::Result<()> {
        match &self.roots {
            Some(roots) if !roots.holds(path) => {
                self.breach.get_or_init(|| path.to_owned());
                Err(ReadRefusal::OutsideBox(path.to_owned()).into())
            }
            _ => Ok(()),
        }
    }
}

/// A tree whose reads are checked against a [`BoxCheck`] first.
pub(super) struct Boxed<'a, T> {
    /// The box that checks the reads.
    pub(super) check: &'a BoxCheck,
    /// The tree read.
    pub(super) tree: &'a T,
}

impl<T: SourceTree> SourceTree for Boxed<'_, T> {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        self.tree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        self.check_read(path)?;
        self.tree.read(path, id)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        self.tree.is_world_writable(dir)
    }

    fn note_absent(&self, path: &Path) {
        self.tree.note_absent(path);
    }

    fn check_read(&self, path: &Path) -> io::Result<()> {
        self.check.check_read(path)?;
        self.tree.check_read(path)
    }
}
