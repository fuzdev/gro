//! What a graph's resolutions read from the tree, recorded in full: each
//! metadata file's single read, which every later resolution in the run is
//! served from, with the BLAKE3 hash of its bytes, and each path they went
//! on past finding absent ([`SourceTree::note_absent`]).
//!
//! Without it, a resolution reads a `package.json` again each time it
//! passes one, and nothing says which bytes it parsed. With it, a file is
//! read once per run: its first read's identity, bytes, and hash stand, a
//! later read finding the same identity gets the same bytes, and one
//! finding another identity at the path is refused, as a module's load is
//! ([`FileId`]). So the hash describes the bytes every resolution in the
//! run parsed, even when the file is rewritten in place meanwhile. A read
//! served from the record passes the inner tree's check first
//! ([`SourceTree::check_read`]), as one that reaches the tree does, so a
//! record seeded before a rule existed serves nothing the rule refuses.
//!
//! Only a graph recording in full uses it: off, a resolution reads the tree
//! directly, hashing and keeping nothing.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::resolver::{Entry, FileId, SourceTree};

/// The metadata files a graph's resolutions read, each once, by real path,
/// and the paths they found absent.
#[derive(Debug, Clone, Default)]
pub struct Reads {
    /// Each file read, by its real path: its first read.
    files: BTreeMap<PathBuf, FileRead>,
    /// Each path a resolution went on past, finding nothing of the kind it
    /// looked for, as it was looked up.
    absent: BTreeSet<PathBuf>,
}

/// A file's first read in a run.
#[derive(Clone)]
struct FileRead {
    /// The identity the read checked.
    id: FileId,
    /// The bytes read, which a later read of the same identity is served.
    bytes: Vec<u8>,
    /// The BLAKE3 hash of `bytes`.
    hash: blake3::Hash,
}

impl fmt::Debug for FileRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRead")
            .field("id", &self.id)
            .field("len", &self.bytes.len())
            .field("hash", &self.hash)
            .finish()
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the graph dump, its reader, lands next")
)]
impl Reads {
    /// What a read says when the file at a path isn't the one an earlier
    /// read in the run found there.
    pub const CHANGED: &str = "the file at its path is not the one an earlier resolution in this \
                               run read, so it isn't read again";

    /// The BLAKE3 hash of the bytes read from the file at the real path
    /// `path`, if it was read.
    pub fn hash(&self, path: &Path) -> Option<blake3::Hash> {
        self.files.get(path).map(|read| read.hash)
    }

    /// The paths found absent, as they were looked up.
    pub const fn absent(&self) -> &BTreeSet<PathBuf> {
        &self.absent
    }
}

/// A tree that records into [`Reads`]: it serves each file's later reads
/// from its first, and keeps the paths found absent.
pub struct RecordingTree<'a, T> {
    /// The tree read.
    tree: &'a T,
    /// The record, borrowed for the resolution.
    reads: RefCell<&'a mut Reads>,
}

impl<T> fmt::Debug for RecordingTree<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordingTree")
            .field("reads", &self.reads)
            .finish_non_exhaustive()
    }
}

impl<'a, T: SourceTree> RecordingTree<'a, T> {
    /// `tree`, recording into `reads`.
    pub const fn new(tree: &'a T, reads: &'a mut Reads) -> Self {
        Self {
            tree,
            reads: RefCell::new(reads),
        }
    }
}

impl<T: SourceTree> SourceTree for RecordingTree<'_, T> {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        self.tree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        // a read served from the record passes the inner tree's check, as
        // one that reaches it does, however the record was seeded
        self.tree.check_read(path)?;
        let mut reads = self.reads.borrow_mut();
        if let Some(read) = reads.files.get(path) {
            if read.id != id {
                return Err(io::Error::other(Reads::CHANGED));
            }
            return Ok(read.bytes.clone());
        }
        let bytes = self.tree.read(path, id)?;
        let read = FileRead {
            id,
            hash: blake3::hash(&bytes),
            bytes: bytes.clone(),
        };
        reads.files.insert(path.to_owned(), read);
        Ok(bytes)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        self.tree.is_world_writable(dir)
    }

    fn note_absent(&self, path: &Path) {
        let mut reads = self.reads.borrow_mut();
        if !reads.absent.contains(path) {
            reads.absent.insert(path.to_owned());
        }
    }

    fn check_read(&self, path: &Path) -> io::Result<()> {
        self.tree.check_read(path)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;

    use super::*;
    use crate::resolver::test_tree::MemoryTree;

    /// A tree that counts its reads.
    struct Counting {
        tree: MemoryTree,
        reads: Cell<usize>,
    }

    impl SourceTree for Counting {
        fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
            self.tree.lookup(path)
        }

        fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
            self.reads.set(self.reads.get() + 1);
            self.tree.read(path, id)
        }

        fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
            self.tree.is_world_writable(dir)
        }
    }

    /// A file is read once: a later read of the same identity is served the
    /// first's bytes, the hash is theirs, one of another identity is
    /// refused, and a failed read records nothing.
    #[test]
    fn a_file_is_read_once_and_hashed() {
        let counting = Counting {
            tree: MemoryTree {
                files: BTreeMap::from([("/p/package.json", "{}"), ("/p/x.json", "[]")]),
                ..MemoryTree::default()
            },
            reads: Cell::new(0),
        };
        let path = Path::new("/p/package.json");
        let id = counting.tree.id("/p/package.json").unwrap();
        let mut reads = Reads::default();
        {
            let tree = RecordingTree::new(&counting, &mut reads);
            assert_eq!(tree.read(path, id).unwrap(), b"{}");
            assert_eq!(tree.read(path, id).unwrap(), b"{}");
            assert_eq!(counting.reads.get(), 1);
            let other = counting.tree.id("/p/x.json").unwrap();
            let error = tree.read(path, other).unwrap_err();
            assert_eq!(error.to_string(), Reads::CHANGED);
            // the inner tree refuses a mismatched identity, and nothing is kept
            assert!(tree.read(Path::new("/p/x.json"), id).is_err());
            assert_eq!(counting.reads.get(), 2);
            tree.note_absent(Path::new("/p/a/package.json"));
            tree.note_absent(Path::new("/p/a/package.json"));
        }
        assert_eq!(reads.hash(path), Some(blake3::hash(b"{}")));
        assert_eq!(reads.hash(Path::new("/p/x.json")), None);
        assert_eq!(
            reads.absent().iter().collect::<Vec<_>>(),
            [Path::new("/p/a/package.json")]
        );
    }

    /// A tree that refuses every read once `refusing` is set.
    struct Gated {
        tree: MemoryTree,
        refusing: Cell<bool>,
    }

    impl SourceTree for Gated {
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

        fn check_read(&self, path: &Path) -> io::Result<()> {
            if self.refusing.get() {
                return Err(io::Error::other(format!("{} is refused", path.display())));
            }
            Ok(())
        }
    }

    /// A read served from the record passes the inner tree's check as one
    /// that reaches it does: a file read before the check refused it isn't
    /// served after.
    #[test]
    fn a_read_served_from_the_record_is_checked() {
        let gated = Gated {
            tree: MemoryTree {
                files: BTreeMap::from([("/p/package.json", "{}")]),
                ..MemoryTree::default()
            },
            refusing: Cell::new(false),
        };
        let path = Path::new("/p/package.json");
        let id = gated.tree.id("/p/package.json").unwrap();
        let mut reads = Reads::default();
        let tree = RecordingTree::new(&gated, &mut reads);
        assert_eq!(tree.read(path, id).unwrap(), b"{}");
        gated.refusing.set(true);
        let error = tree.read(path, id).unwrap_err();
        assert_eq!(error.to_string(), "/p/package.json is refused");
        assert!(tree.check_read(path).is_err());
    }
}
