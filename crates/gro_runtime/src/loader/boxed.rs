//! The box-checking wrapper over a `SourceTree`.

use std::io;
use std::path::Path;

use super::Loader;
use crate::resolver::{Entry, FileId, SourceTree};

/// A tree whose reads are checked against the loader's box first
/// ([`Loader::check_read`]).
pub(super) struct Boxed<'a, T> {
    /// The loader whose box checks the reads.
    pub(super) loader: &'a Loader,
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
        self.loader.check_read(path)?;
        self.tree.check_read(path)
    }
}
