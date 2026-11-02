//! A source tree in memory, for the resolver's and the loader's tests, and the
//! directories the crate's disk tests write in.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::resolver::tree::ELOOP;
use crate::resolver::{Entry, FileId, MAX_PACKAGE_JSON_SIZE, SourceTree};

/// An in-memory tree: files by path with their contents, symlinks as a
/// map to their target, symlink loops, which fail with `ELOOP`, and the
/// world-writable directories. A directory is any path a file, a listed
/// directory, a symlink, or a loop is under. A file's identity is its place
/// in `files`: device 1, inode its index plus one.
///
/// A lookup finds the real path as `canonicalize` does, a component at a
/// time: a symlink at any component, the last or one before it, is replaced
/// by its target, read from the symlink's directory when relative, and `..`
/// steps up from the real path found so far, not from the path as written.
/// So a symlink's key is a real path: one under a linked directory is never
/// reached, as on disk, where it would be inside the link's target.
#[derive(Default)]
pub struct MemoryTree {
    pub files: BTreeMap<&'static str, &'static str>,
    pub dirs: Vec<&'static str>,
    pub links: BTreeMap<&'static str, &'static str>,
    pub loops: Vec<&'static str>,
    pub world_writable: Vec<&'static str>,
}

/// The most symlinks one lookup follows before it fails with `ELOOP`, Linux's
/// `MAXSYMLINKS`.
const MAX_SYMLINKS: usize = 40;

impl SourceTree for MemoryTree {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        let Some(real_path) = self.real_path(path)? else {
            return Ok(None);
        };
        let path = real_path.to_str().unwrap();
        if let Some(id) = self.id(path) {
            return Ok(Some(Entry::File {
                path: real_path,
                id,
            }));
        }
        Ok(self.is_dir(path).then_some(Entry::Dir(real_path)))
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

    /// Whether a directory is at `path`, a real path.
    fn is_dir(&self, path: &str) -> bool {
        let is_under = |entry: &&str| {
            entry
                .strip_prefix(path)
                .is_some_and(|rest| rest.starts_with('/'))
                || (path == "/" && entry.starts_with('/'))
        };
        self.dirs.contains(&path)
            || self.files.keys().any(is_under)
            || self.dirs.iter().any(is_under)
            || self.links.keys().any(is_under)
            || self.loops.iter().any(is_under)
    }

    /// The real path of `path`, an absolute path, or `None` when nothing is
    /// there: when a component is missing, or a file has a component, a
    /// trailing `/`, or a trailing `/.` after it. Fails with `ELOOP` on a
    /// listed loop or past [`MAX_SYMLINKS`].
    fn real_path(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        let eloop = || {
            io::Error::from_raw_os_error(ELOOP.expect("the loop tests need this platform's ELOOP"))
        };
        let mut real = PathBuf::from("/");
        // the components still to walk, the next last
        let mut rest: Vec<Component<'_>> = path.components().rev().collect();
        let mut followed = 0;
        while let Some(component) = rest.pop() {
            match component {
                Component::RootDir => real = PathBuf::from("/"),
                Component::CurDir => {}
                Component::ParentDir => {
                    real.pop();
                }
                Component::Normal(name) => {
                    let next = real.join(name);
                    let next_str = next.to_str().unwrap();
                    if self.loops.contains(&next_str) {
                        return Err(eloop());
                    }
                    if let Some(target) = self.links.get(next_str) {
                        followed += 1;
                        if followed > MAX_SYMLINKS {
                            return Err(eloop());
                        }
                        // read from the symlink's directory, `real` as it is
                        rest.extend(Path::new(target).components().rev());
                    } else if self.id(next_str).is_some() {
                        // a file has no components below it, not even `.`
                        if !rest.is_empty() {
                            return Ok(None);
                        }
                        real = next;
                    } else if self.is_dir(next_str) {
                        real = next;
                    } else {
                        return Ok(None);
                    }
                }
                Component::Prefix(_) => return Ok(None),
            }
        }
        // `components` drops a trailing `/` and `/.`, which still need a directory
        let raw = path.as_os_str().as_encoded_bytes();
        if (raw.ends_with(b"/") || raw.ends_with(b"/."))
            && self.id(real.to_str().unwrap()).is_some()
        {
            return Ok(None);
        }
        Ok(Some(real))
    }
}

/// The path of [`TestDir::on_disk`]'s directory for `name`, in the
/// workspace's `target/tmp`, found from this crate's manifest directory. It's on the
/// checkout's filesystem, not the system's temp directory, often a tmpfs,
/// so the file identity tests meet the filesystem the code runs on, such as
/// ext4, which can hand a remade file a freed inode number.
fn disk_test_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/tmp")
        .join(format!("gro_runtime_{}_{name}", std::process::id()))
}

/// The system's temporary directory, canonical, checked to be
/// world-writable with no `package.json` at or above it.
///
/// # Panics
///
/// Panics, naming `TMPDIR`, when it isn't.
pub fn system_temp() -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let temp = std::env::temp_dir().canonicalize().unwrap();
    let mode = std::fs::metadata(&temp).unwrap().permissions().mode();
    assert!(
        mode & 0o002 != 0,
        "the temp dir {} (TMPDIR) isn't world-writable (mode {mode:o}), and the disk tests \
         expect every walk up from their files to stop there. Set TMPDIR to one that is, like /tmp",
        temp.display()
    );
    let package_json = temp
        .ancestors()
        .map(|dir| dir.join("package.json"))
        .find(|package_json| package_json.exists());
    if let Some(package_json) = package_json {
        panic!(
            "the temp dir {} (TMPDIR) is under a project: {} is at or above it, and the disk \
             tests expect none. Set TMPDIR outside one",
            temp.display(),
            package_json.display()
        );
    }
    temp
}

/// A fresh directory for one test's files, removed on drop, so a test that
/// panics, on its own thread or one it joins, leaves nothing behind. One of
/// the same name a killed run left behind, under a pid since reused, is
/// removed first.
#[derive(Debug)]
pub struct TestDir(PathBuf);

impl TestDir {
    /// The directory [`disk_test_dir`] names, on the checkout's filesystem.
    pub fn on_disk(name: &str) -> Self {
        Self::at(&disk_test_dir(name))
    }

    /// A directory named for `name` under the system's temporary directory,
    /// which is world-writable, so every walk up from it stops there.
    ///
    /// # Panics
    ///
    /// Panics, naming `TMPDIR`, when the temp dir isn't world-writable or a
    /// `package.json` is at or above it, which the tests that expect every
    /// walk to stop there, finding none, would fail on misleadingly.
    pub fn in_system_temp(name: &str) -> Self {
        Self::at(&system_temp().join(format!("gro_runtime_{}_{name}", std::process::id())))
    }

    /// A fresh directory at `dir` with the mode `0o755`, whatever the umask,
    /// so it isn't world-writable, which would end every walk up at it.
    fn at(dir: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self(dir.canonicalize().unwrap())
    }

    /// The directory's canonical path.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
