//! The `package.json` files a run parsed ([`ParsedPackages`]), and the tree
//! wrapper that hands them to its resolutions ([`Parsing`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde_json::{Map, Value};

use crate::file_id::FileId;

use super::tree::{Entry, SourceTree, Stamp};

/// The `package.json` files a run parsed, each by its real path with the
/// identity and stamp the lookup before its read found.
///
/// A resolution still looks every file up, so what it finds at a path is
/// never older than the lookup: only the parse and its read are kept, and
/// they are kept for the file the lookup finds, unchanged. A file replaced
/// or changed since has another identity or stamp and is read and parsed
/// again. A rewrite in place to the same size within the same tick of the
/// filesystem's clock keeps both ([`Stamp`]), so a file is kept only once
/// its last change is two seconds before its read began: a rewrite after
/// that moves its stamp. A file served passes its tree's check first
/// ([`SourceTree::check_read`]), as a read does. A read that failed, or a
/// file that didn't parse, isn't kept.
///
/// It is a cache, not a check: what it serves is what the same file's own
/// read gave earlier in the run, and every path a resolution reaches is
/// still found by its own fresh lookup.
#[derive(Debug, Clone, Default)]
pub struct ParsedPackages(RefCell<HashMap<PathBuf, Parsed>>);

/// How long before a read a file must have last changed for its parse to be
/// kept: longer than a Linux filesystem's clock tick (a few milliseconds)
/// and as long as FAT's timestamp step.
pub(super) const SETTLED: Duration = Duration::from_secs(2);

impl Stamp {
    /// Whether the change it records was at least [`SETTLED`] before `at`.
    fn settled(self, at: SystemTime) -> bool {
        let Ok(secs) = u64::try_from(self.changed_secs) else {
            return false;
        };
        let nanos = u32::try_from(self.changed_nanos).unwrap_or(0);
        let changed = SystemTime::UNIX_EPOCH + Duration::new(secs, nanos);
        at.duration_since(changed).is_ok_and(|age| age >= SETTLED)
    }
}

/// A `package.json` a run parsed.
#[derive(Debug, Clone)]
struct Parsed {
    /// The identity its lookup found.
    id: FileId,
    /// The stamp its lookup found.
    stamp: Stamp,
    /// Its fields.
    fields: Arc<Map<String, Value>>,
}

impl ParsedPackages {
    /// The fields of the `package.json` at the real path `path`, parsed
    /// before at the identity `id` and the stamp `stamp`, if it was.
    pub(super) fn get(
        &self,
        path: &Path,
        id: FileId,
        stamp: Stamp,
    ) -> Option<Arc<Map<String, Value>>> {
        self.0
            .borrow()
            .get(path)
            .filter(|parsed| parsed.id == id && parsed.stamp == stamp)
            .map(|parsed| Arc::clone(&parsed.fields))
    }

    /// Keeps `fields`, parsed from the `package.json` at the real path
    /// `path`, read at the identity `id` and the stamp `stamp`, its read
    /// begun at `read_at`.
    ///
    /// A file changed less than [`SETTLED`] before its read began isn't
    /// kept, since a rewrite in the same tick of the filesystem's clock,
    /// during or after the read, would keep its stamp: a later read of it is
    /// parsed again, until it settles.
    pub(super) fn insert(
        &self,
        path: &Path,
        id: FileId,
        stamp: Stamp,
        read_at: SystemTime,
        fields: Arc<Map<String, Value>>,
    ) {
        if !stamp.settled(read_at) {
            return;
        }
        let parsed = Parsed { id, stamp, fields };
        let mut files = self.0.borrow_mut();
        if let Some(kept) = files.get_mut(path) {
            *kept = parsed;
        } else {
            files.insert(path.to_owned(), parsed);
        }
    }
}

/// A tree whose resolutions share `parsed`, reading `tree`.
pub struct Parsing<'a, T> {
    /// The tree read.
    tree: &'a T,
    /// The files parsed.
    parsed: &'a ParsedPackages,
}

impl<T> std::fmt::Debug for Parsing<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Parsing")
            .field("parsed", &self.parsed)
            .finish_non_exhaustive()
    }
}

impl<'a, T: SourceTree> Parsing<'a, T> {
    /// `tree`, its resolutions sharing `parsed`.
    pub const fn new(tree: &'a T, parsed: &'a ParsedPackages) -> Self {
        Self { tree, parsed }
    }
}

impl<T: SourceTree> SourceTree for Parsing<'_, T> {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        self.tree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        self.tree.read(path, id)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        self.tree.is_world_writable(dir)
    }

    fn note_absent(&self, path: &Path) {
        self.tree.note_absent(path);
    }

    fn check_read(&self, path: &Path) -> io::Result<()> {
        self.tree.check_read(path)
    }

    fn parsed(&self) -> Option<&ParsedPackages> {
        Some(self.parsed)
    }
}
