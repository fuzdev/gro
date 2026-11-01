//! Confinement: the box a run's process is held in, and the type state that
//! gates the engine on it.
//!
//! A run is either confined or explicitly unconfined before the engine starts:
//! [`engine::init_engine`](crate::engine::init_engine) takes a [`Confinement`],
//! and its token ([`Engine`](crate::engine::Engine)) names which. The states:
//!
//! - [`Prepared`]: the box's roots, computed before anything is confined
//!   ([`Prepared::for_project`]): the project's real directory, and the link
//!   closure below it, each opened as it is found, so the box goes up on the
//!   directories found, not on whatever their paths name later.
//! - [`Confined`]: the process passed through [`Prepared::confine`], made only
//!   there, on the process's one thread.
//! - [`Confinement::Unconfined`]: named by the caller, for a run that holds
//!   no box (`fgro run`, the snapshot builder, and tests).
//!
//! The box must hold every thread the process has, so it is installed while
//! the process has one: what resolves the run's roots runs on a thread joined
//! before [`Prepared::confine`], which checks the count first and refuses
//! otherwise, and every thread the engine starts comes after it, spawned from
//! the thread that installed it.
//!
//! The kernel's walls aren't built yet: [`Prepared::confine`] checks the
//! thread count and nothing more, and the box's roots bound only the
//! loader's reads (the loader's `Rules::box_roots`).
//!
//! The link closure is the box's directory structure, read with no parsing:
//! starting from the project, every entry of a root's `node_modules` (each
//! `<name>`, and each `<name>` under an `@<scope>` directory, the dot-named
//! entries skipped) whose real path is a directory lying outside every root so
//! far, holding none, and holding a `package.json` that is a file (not a
//! symlink), becomes a root, unless a world-writable directory holds it, and
//! its own `node_modules` is read the same way, to a fixed point. An installed
//! package sits under the project's `node_modules`, already inside it; a
//! linked one (a symlinked path dependency, a workspace checkout) is where the
//! closure reaches outside. A link to a file, to an ancestor of the project,
//! to `/`, or to a directory that isn't a package (`/etc`, `~/.ssh`) adds
//! nothing.
//!
//! On Linux each root is opened as it is found (`O_PATH`, with
//! `O_DIRECTORY` and `O_NOFOLLOW`), and kept only when the kernel names the
//! handle by the root's real path, so a directory swapped in after the
//! closure read it isn't what the box admits.

use std::io;
use std::path::{Path, PathBuf};

use crate::resolver::{DiskTree, SourceTree};

/// A root, opened before anything is confined: on Linux, an `O_PATH` handle
/// to the directory ([`pin`]).
#[cfg(target_os = "linux")]
type Pin = std::os::fd::OwnedFd;

/// A root, opened before anything is confined: off Linux, where there is no
/// box, nothing.
#[cfg(not(target_os = "linux"))]
type Pin = ();

/// The directories a confined run may read: the project's real directory
/// first, then each link closure root, every one a real path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoxRoots(Vec<PathBuf>);

impl BoxRoots {
    /// Roots as given, for a test of what reads them.
    #[cfg(test)]
    pub(crate) const fn for_test(roots: Vec<PathBuf>) -> Self {
        Self(roots)
    }

    /// Whether the real path `path` is one of the roots or lies under one,
    /// component by component.
    pub fn holds(&self, path: &Path) -> bool {
        self.0.iter().any(|root| path.starts_with(root))
    }

    /// The roots, the project's first.
    pub fn as_slice(&self) -> &[PathBuf] {
        &self.0
    }
}

/// The box, computed: its roots, before anything is confined, each opened.
#[derive(Debug)]
pub struct Prepared {
    /// What the box admits.
    roots: BoxRoots,
    /// Each root, opened, in the roots' order.
    #[cfg_attr(
        not(all(test, target_os = "linux")),
        expect(
            dead_code,
            reason = "the kernel's walls, not built yet, are installed on them"
        )
    )]
    pins: Vec<Pin>,
}

impl Prepared {
    /// The box for the project whose real directory is `project`: it and the
    /// link closure below it, read from the disk now, each root opened as it
    /// is found. A `node_modules` or scope directory that can't be listed,
    /// an entry whose real path can't be read (a link to nothing), and a
    /// root that can't be opened as the directory found add nothing.
    ///
    /// # Errors
    ///
    /// Returns the error when `project` can't be opened as a directory
    /// without following a symlink, or the handle the kernel opened isn't
    /// `project` (a directory on its path swapped since it was read).
    pub fn for_project(project: PathBuf) -> io::Result<Self> {
        let pinned = pin(&project)?;
        let (roots, pins) = link_closure(project, pinned);
        Ok(Self {
            roots: BoxRoots(roots),
            pins,
        })
    }

    /// The roots the box admits.
    pub const fn roots(&self) -> &BoxRoots {
        &self.roots
    }

    /// Confines the process to the box, from its one thread.
    ///
    /// # Errors
    ///
    /// Returns [`ConfineError::Threads`] when the process has another thread,
    /// [`ConfineError::ThreadsUnread`] when its thread count can't be read,
    /// and [`ConfineError::Unsupported`] off Linux.
    pub fn confine(&self) -> Result<Confined, ConfineError> {
        alone()?;
        Ok(Confined {
            roots: self.roots.clone(),
        })
    }
}

/// The process confined to its box, from the thread that installed it.
/// Made only by [`Prepared::confine`].
#[derive(Debug, PartialEq, Eq)]
pub struct Confined {
    /// What the box admits.
    roots: BoxRoots,
}

impl Confined {
    /// The roots the box admits.
    pub const fn roots(&self) -> &BoxRoots {
        &self.roots
    }
}

/// Whether a run's process is confined, which the engine takes to start.
#[derive(Debug, PartialEq, Eq)]
pub enum Confinement {
    /// Held in its box.
    Confined(Confined),
    /// Held in no box, by the caller's choice.
    Unconfined,
}

/// Why [`Prepared::confine`] refused.
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    /// The process has more than one thread, so a box installed now would
    /// leave the others outside it.
    #[error(
        "the process has {count} threads, so a box installed now would leave all but one \
         outside it: the box is installed while the process has one"
    )]
    Threads {
        /// How many threads it has.
        count: usize,
    },
    /// The process's thread count couldn't be read.
    #[error("failed to read the process's thread count from /proc/self/status: {0}")]
    ThreadsUnread(#[source] io::Error),
    /// The platform has no box: confinement is Linux's.
    #[error("confinement is built for Linux only, and this platform has no box")]
    Unsupported,
}

/// The link closure of the project at `project`, opened as `pinned`: it
/// first, then each `node_modules` entry's real path that is a package
/// directory ([`is_package`]), neither under a root so far nor holding one,
/// held by no world-writable directory ([`held_by_world_writable`]), and
/// opened as found ([`pin`]), read in turn to a fixed point. Returns the
/// roots and their handles, in the same order.
fn link_closure(project: PathBuf, pinned: Pin) -> (Vec<PathBuf>, Vec<Pin>) {
    let mut roots = vec![project];
    let mut pins = vec![pinned];
    let mut next = 0;
    while let Some(root) = roots.get(next) {
        let node_modules = root.join("node_modules");
        let entries = package_entries(&node_modules);
        next += 1;
        for entry in entries {
            // a package is a directory with a package.json, and none holds a
            // root: a link to a file, to an ancestor of the project, to `/`,
            // or to a directory that isn't a package widens nothing
            if let Ok(target) = std::fs::canonicalize(&entry)
                && is_package(&target)
                && !roots
                    .iter()
                    .any(|root| target.starts_with(root) || root.starts_with(&target))
                && !held_by_world_writable(&node_modules, &entry, &target)
                && let Ok(pinned) = pin(&target)
            {
                roots.push(target);
                pins.push(pinned);
            }
        }
    }
    (roots, pins)
}

/// Whether the real path `target` is a package's directory: a directory
/// holding a `package.json` that is a regular file, not a symlink, which the
/// resolver would read at its own real path, possibly outside the box.
fn is_package(target: &Path) -> bool {
    target.is_dir()
        && std::fs::symlink_metadata(target.join("package.json"))
            .is_ok_and(|metadata| metadata.file_type().is_file())
}

/// Opens the real directory `dir` as a root's handle: `O_PATH`, with
/// `O_DIRECTORY` and `O_NOFOLLOW` (and std's `O_CLOEXEC`), so only a
/// directory opens and a symlink in its last component doesn't, then checks
/// that the kernel names the handle by `dir`, so a directory on its path
/// swapped since `dir` was read is refused.
///
/// # Errors
///
/// Returns the error of the open or of reading the handle's path, or one of
/// kind [`io::ErrorKind::Other`] when the handle names another path.
#[cfg(target_os = "linux")]
fn pin(dir: &Path) -> io::Result<Pin> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)?;
    let opened =
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to read the path of its handle from /proc/self/fd: {error}"),
            )
        })?;
    if opened != dir {
        return Err(io::Error::other(format!(
            "opened {} as {}, another directory",
            dir.display(),
            opened.display()
        )));
    }
    Ok(file.into())
}

#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature as Linux's, which can fail"
)]
const fn pin(_dir: &Path) -> io::Result<Pin> {
    Ok(())
}

/// Whether the package linked at `entry` in `node_modules`, whose real path
/// is `target`, is held by a world-writable directory, which any user can
/// plant files in: `node_modules` and the entry's `@scope` directory, the real
/// parent of each, `target`, or the directories holding `target`, one per
/// segment of the package's name. The resolver refuses a
/// package found through any of them, by the same predicate
/// ([`SourceTree::is_world_writable`]), so none is worth a root. A directory
/// whose mode can't be read counts as one.
fn held_by_world_writable(node_modules: &Path, entry: &Path, target: &Path) -> bool {
    let scope = entry.parent().filter(|parent| *parent != node_modules);
    let segments = if scope.is_some() { 2 } else { 1 };
    let real_parents = [Some(node_modules), scope]
        .into_iter()
        .flatten()
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .filter_map(|real| real.parent().map(Path::to_path_buf));
    let mut dirs: Vec<PathBuf> = [Some(node_modules), scope]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf)
        .collect();
    dirs.extend(real_parents);
    dirs.extend(target.ancestors().take(segments + 1).map(Path::to_path_buf));
    dirs.iter()
        .any(|dir| !matches!(DiskTree.is_world_writable(dir), Ok(false)))
}

/// The package entries of the `node_modules` at `node_modules`, sorted: each
/// `<name>` and each `@<scope>/<name>`, the dot-named ones (`.bin`, a
/// `.package-lock.json`, a `.deno` store) skipped.
fn package_entries(node_modules: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    for entry in list(node_modules) {
        let Some(name) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if name.starts_with('@') {
            entries.extend(list(&entry).into_iter().filter(|scoped| {
                !scoped
                    .file_name()
                    .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
            }));
        } else {
            entries.push(entry);
        }
    }
    entries.sort();
    entries
}

/// The paths of the entries of the directory `dir`, or none when it can't be
/// listed.
fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |entries| {
            entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect()
        },
    )
}

/// How many times [`alone`] reads the thread count before it refuses: a
/// thread whose join has returned can stay counted for a moment longer, while
/// the kernel finishes its exit (the joiner wakes when the thread's id is
/// cleared, before the thread is removed from its group).
#[cfg(target_os = "linux")]
const THREAD_COUNT_READS: usize = 1000;

/// Checks that the process has one thread, the caller.
#[cfg(target_os = "linux")]
fn alone() -> Result<(), ConfineError> {
    let mut count = 0;
    for _ in 0..THREAD_COUNT_READS {
        count = thread_count().map_err(ConfineError::ThreadsUnread)?;
        if count == 1 {
            return Ok(());
        }
        std::thread::yield_now();
    }
    Err(ConfineError::Threads { count })
}

#[cfg(not(target_os = "linux"))]
const fn alone() -> Result<(), ConfineError> {
    Err(ConfineError::Unsupported)
}

/// The process's thread count, its `Threads:` line in `/proc/self/status`.
#[cfg(target_os = "linux")]
fn thread_count() -> io::Result<usize> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .and_then(|count| count.trim().parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no Threads: line"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use std::os::unix::fs::PermissionsExt as _;

    use super::{BoxRoots, ConfineError, Prepared};

    /// A fresh directory under the system's temporary directory, removed on
    /// drop, its path canonical. Every directory it makes has the mode
    /// `0o755`, whatever the umask, so none is world-writable unless a test
    /// makes it so.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("gro_runtime_confine_{}_{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            set_mode(&dir, 0o755);
            Self(dir.canonicalize().unwrap())
        }

        fn dir(&self, path: &str) -> PathBuf {
            let dir = self.0.join(path);
            std::fs::create_dir_all(&dir).unwrap();
            let mut made = self.0.clone();
            for component in Path::new(path).components() {
                made.push(component);
                set_mode(&made, 0o755);
            }
            dir
        }

        fn file(&self, path: &str) {
            std::fs::write(self.0.join(path), "{}").unwrap();
        }

        fn link(&self, target: &Path, at: &str) {
            std::os::unix::fs::symlink(target, self.0.join(at)).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// The roots of the box for `project`.
    fn closure(project: &Path) -> Vec<PathBuf> {
        Prepared::for_project(project.to_owned())
            .unwrap()
            .roots()
            .as_slice()
            .to_vec()
    }

    /// Links out of the project are followed to a fixed point through each
    /// new root's own `node_modules`, scoped names included; a link back into
    /// a root, an installed package, a dot-named entry, a link to nothing, a
    /// link to a file, to an ancestor of the project, or to `/`, a link to a
    /// directory with no `package.json`, or whose `package.json` is a symlink
    /// or a directory, and a link to a directory a world-writable one holds
    /// or that is one add nothing.
    #[test]
    fn the_link_closure_follows_links_out_to_a_fixed_point() {
        let scratch = Scratch::new("closure");
        // below the scratch directory, whose own parent is world-writable
        let project = scratch.dir("w/project");
        let tool = scratch.dir("w/tool");
        let util = scratch.dir("w/util");
        let hidden = scratch.dir("w/hidden");
        let scoped = scratch.dir("w/scoped");
        let shared = scratch.dir("w/shared");
        let planted = scratch.dir("w/shared/planted");
        let open = scratch.dir("w/open");
        // not packages: no package.json (`/etc`, `~/.ssh`), one symlinked to
        // a file, and one that is a directory
        let bare = scratch.dir("w/bare");
        let linked_json = scratch.dir("w/linked_json");
        let dir_json = scratch.dir("w/dir_json");
        scratch.dir("w/dir_json/package.json");
        for dir in [
            "w",
            "w/tool",
            "w/util",
            "w/hidden",
            "w/scoped",
            "w/shared/planted",
            "w/open",
        ] {
            scratch.file(&format!("{dir}/package.json"));
        }
        scratch.link(
            &scratch.0.join("w/tool/package.json"),
            "w/linked_json/package.json",
        );
        for dir in [&shared, &open] {
            set_mode(dir, 0o777);
        }
        std::fs::write(scratch.0.join("w/file.txt"), "").unwrap();
        scratch.dir("w/project/node_modules/@fuzdev");
        scratch.dir("w/project/node_modules/installed");
        scratch.dir("w/project/node_modules/@scope/installed");
        scratch.dir("w/tool/node_modules");
        scratch.file("w/project/node_modules/installed/package.json");
        scratch.link(&tool, "w/project/node_modules/@fuzdev/tool");
        scratch.link(&scoped, "w/project/node_modules/@scope/linked");
        scratch.link(&hidden, "w/project/node_modules/.bin");
        scratch.link(&hidden, "w/project/node_modules/@fuzdev/.hidden");
        scratch.link(&scratch.0.join("absent"), "w/project/node_modules/dangling");
        scratch.link(&scratch.0.join("w/file.txt"), "w/project/node_modules/file");
        scratch.link(&scratch.0.join("w"), "w/project/node_modules/ancestor");
        scratch.link(Path::new("/"), "w/project/node_modules/root");
        scratch.link(&planted, "w/project/node_modules/planted");
        scratch.link(&open, "w/project/node_modules/open");
        scratch.link(&bare, "w/project/node_modules/bare");
        scratch.link(&linked_json, "w/project/node_modules/linked_json");
        scratch.link(&dir_json, "w/project/node_modules/dir_json");
        scratch.link(&util, "w/tool/node_modules/util");
        scratch.link(&project, "w/tool/node_modules/project");
        scratch.link(
            &project.join("node_modules/installed"),
            "w/tool/node_modules/back",
        );
        assert_eq!(closure(&project), [project, tool, scoped, util]);
    }

    /// Each root is opened as the directory found: the kernel names every
    /// handle by its root's real path.
    #[cfg(target_os = "linux")]
    #[test]
    fn each_root_is_opened_as_the_directory_found() {
        use std::os::fd::AsRawFd as _;

        let scratch = Scratch::new("pinned");
        let project = scratch.dir("project");
        let lib = scratch.dir("lib");
        scratch.file("lib/package.json");
        scratch.dir("project/node_modules");
        scratch.link(&lib, "project/node_modules/lib");
        let prepared = Prepared::for_project(project.clone()).unwrap();
        assert_eq!(prepared.roots().as_slice(), [project, lib]);
        let named: Vec<PathBuf> = prepared
            .pins
            .iter()
            .map(|pin| std::fs::read_link(format!("/proc/self/fd/{}", pin.as_raw_fd())).unwrap())
            .collect();
        assert_eq!(named, prepared.roots().as_slice());
    }

    /// A project path that isn't its real directory isn't opened: a
    /// symlink, a path through a symlinked directory, which the kernel names
    /// by another path once opened, or no directory.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_project_that_is_not_its_real_directory_is_refused() {
        let scratch = Scratch::new("unpinned");
        let project = scratch.dir("real/project");
        scratch.link(&project, "linked");
        scratch.link(&scratch.0.join("real"), "alias");
        assert!(Prepared::for_project(scratch.0.join("linked")).is_err());
        assert!(Prepared::for_project(scratch.0.join("alias/project")).is_err());
        assert!(Prepared::for_project(scratch.0.join("absent")).is_err());
        assert!(Prepared::for_project(project).is_ok());
    }

    /// A project with no `node_modules` is its own box.
    #[test]
    fn a_project_with_no_node_modules_is_its_own_box() {
        let scratch = Scratch::new("bare");
        let project = scratch.dir("project");
        let prepared = Prepared::for_project(project.clone()).unwrap();
        assert_eq!(prepared.roots().as_slice(), std::slice::from_ref(&project));
        assert!(prepared.roots().holds(&project.join("a/b.ts")));
        assert!(!prepared.roots().holds(&scratch.0.join("projectile/b.ts")));
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
        let prepared = Prepared::for_project(scratch.0.clone()).unwrap();
        match prepared.confine() {
            Err(ConfineError::Threads { count }) => assert!(count >= 2, "{count}"),
            other => panic!("expected a refusal by the thread count, got {other:?}"),
        }
        stop.send(()).unwrap();
        other.join().unwrap().unwrap();
    }
}
