//! A source tree in memory, for the resolver's and the loader's tests.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::resolver::tree::ELOOP;
use crate::resolver::{Entry, FileId, MAX_PACKAGE_JSON_SIZE, SourceTree};

/// An in-memory tree: files by path with their contents, symlinks as a
/// map to their target, symlink loops, which fail with `ELOOP`, and the
/// world-writable directories. A directory is any path a file or a
/// listed directory is under. A file's identity is its place in `files`:
/// device 1, inode its index plus one.
#[derive(Default)]
pub struct MemoryTree {
    pub files: BTreeMap<&'static str, &'static str>,
    pub dirs: Vec<&'static str>,
    pub links: BTreeMap<&'static str, &'static str>,
    pub loops: Vec<&'static str>,
    pub world_writable: Vec<&'static str>,
}

impl SourceTree for MemoryTree {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        // as `canonicalize` does, `a//b` is `a/b`
        let path: PathBuf = path.components().collect();
        if self.loops.iter().any(|link| path.starts_with(link)) {
            return Err(io::Error::from_raw_os_error(
                ELOOP.expect("the loop tests need this platform's ELOOP"),
            ));
        }
        let path = path.to_str().unwrap();
        let path = self.links.get(path).copied().unwrap_or(path);
        let is_under = |entry: &str| {
            entry
                .strip_prefix(path)
                .is_some_and(|rest| rest.starts_with('/'))
        };
        if let Some(id) = self.id(path) {
            return Ok(Some(Entry::File {
                path: PathBuf::from(path),
                id,
            }));
        }
        Ok(
            if self.dirs.contains(&path)
                || self.files.keys().copied().any(is_under)
                || self.dirs.iter().copied().any(is_under)
            {
                Some(Entry::Dir(PathBuf::from(path)))
            } else {
                None
            },
        )
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        let path = path.to_str().unwrap();
        let content = self
            .files
            .get(path)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        if self.id(path) != Some(id) {
            return Err(io::Error::other(FileId::MISMATCH));
        }
        if content.len() as u64 > MAX_PACKAGE_JSON_SIZE {
            return Err(io::Error::from(io::ErrorKind::FileTooLarge));
        }
        Ok(content.as_bytes().to_vec())
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        Ok(self.world_writable.contains(&dir.to_str().unwrap()))
    }
}

impl MemoryTree {
    /// The identity of the file at `path`, a real path, if one is there.
    pub fn id(&self, path: &str) -> Option<FileId> {
        let index = self.files.keys().position(|file| *file == path)?;
        Some(FileId {
            dev: 1,
            ino: u64::try_from(index).unwrap() + 1,
            born: None,
        })
    }
}
