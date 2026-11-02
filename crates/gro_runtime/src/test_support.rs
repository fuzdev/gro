//! The directories the crate's disk tests write in.

use std::path::{Path, PathBuf};

/// The path of [`TestDir::on_disk`]'s directory for `name`, in the workspace's
/// `target/tmp`, found from this crate's manifest directory. It's on the
/// checkout's filesystem, not the system's temp directory, often a tmpfs, so
/// the file identity tests meet the filesystem the code runs on, such as ext4,
/// which can hand a remade file a freed inode number.
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
fn system_temp() -> PathBuf {
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
