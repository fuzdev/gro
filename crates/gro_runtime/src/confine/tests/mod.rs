//! Tests for confinement: shared scratch directories here, then the link
//! closure, the seccomp tables and filters, and the Landlock floor by topic.

use std::path::{Path, PathBuf};

use std::os::unix::fs::PermissionsExt as _;

use super::{BoxRoots, ConfineError, Jit, Prepared};
use crate::resolver::test_tree::TestDir;

mod closure;
#[cfg(target_os = "linux")]
mod landlock;
#[cfg(target_os = "linux")]
mod seccomp;

/// A fresh directory under the system's temporary directory (see
/// [`TestDir`]). Every directory it makes has the mode `0o755`, whatever
/// the umask, as the [`TestDir`] itself has, so none is world-writable
/// unless a test makes it so.
struct Scratch(TestDir);

impl Scratch {
    fn new(name: &str) -> Self {
        Self(TestDir::in_system_temp(&format!("confine_{name}")))
    }

    /// The directory's canonical path.
    fn path(&self) -> &Path {
        self.0.path()
    }

    fn dir(&self, path: &str) -> PathBuf {
        let dir = self.path().join(path);
        std::fs::create_dir_all(&dir).unwrap();
        let mut made = self.path().to_owned();
        for component in Path::new(path).components() {
            made.push(component);
            set_mode(&made, 0o755);
        }
        dir
    }

    fn file(&self, path: &str) {
        std::fs::write(self.path().join(path), "{}").unwrap();
    }

    fn link(&self, target: &Path, at: &str) {
        std::os::unix::fs::symlink(target, self.path().join(at)).unwrap();
    }
}

fn set_mode(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn box_roots_hold_by_component() {
    let roots = BoxRoots(vec![PathBuf::from("/p"), PathBuf::from("/q/lib")]);
    assert!(roots.holds(Path::new("/p")));
    assert!(roots.holds(Path::new("/p/x.js")));
    assert!(roots.holds(Path::new("/q/lib/node_modules/y/z.js")));
    assert!(!roots.holds(Path::new("/pp/x.js")));
    assert!(!roots.holds(Path::new("/q/x.js")));
}

/// A test runs beside the harness's own threads, so confining from it is
/// refused with the count, before anything is installed.
#[cfg(target_os = "linux")]
#[test]
fn confining_with_another_thread_alive_is_refused() {
    let (stop, wait) = std::sync::mpsc::channel::<()>();
    let other = std::thread::spawn(move || wait.recv());
    let scratch = Scratch::new("threads");
    let prepared = Prepared::for_project(scratch.path().to_owned()).unwrap();
    match prepared.confine(Jit::Off) {
        Err(ConfineError::Threads { count }) => assert!(count >= 2, "{count}"),
        other => panic!("expected a refusal by the thread count, got {other:?}"),
    }
    stop.send(()).unwrap();
    other.join().unwrap().unwrap();
}
