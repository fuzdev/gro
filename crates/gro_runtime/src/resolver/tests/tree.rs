//! The in-memory tree against the disk: the same layout looked up in both
//! finds the same real paths, symlinks on any component included; and the
//! disk's own answers where the kernel's differ from a path's text.

use std::io;
use std::path::{Path, PathBuf};

use crate::resolver::test_tree::MemoryTree;
use crate::resolver::tree;
use crate::resolver::{DiskTree, Entry, SourceTree};
use crate::test_support::TestDir;

/// What a lookup found, with no file identity, which differs by tree.
#[derive(Debug, PartialEq, Eq)]
enum Found {
    File(PathBuf),
    Dir(PathBuf),
    Nothing,
    Error(Option<i32>),
}

fn found(result: io::Result<Option<Entry>>, base: &Path) -> Found {
    let below = |path: PathBuf| Path::new("/").join(path.strip_prefix(base).unwrap_or(&path));
    match result {
        Ok(Some(Entry::File { path, .. })) => Found::File(below(path)),
        Ok(Some(Entry::Dir(path))) => Found::Dir(below(path)),
        Ok(Some(Entry::Other) | None) => Found::Nothing,
        Err(error) => Found::Error(error.raw_os_error()),
    }
}

/// Lookups through symlinks on a last component and on one before it,
/// relative targets, `..` after a symlink (taken on the real path), a
/// dangling symlink, a symlink loop, and a file with a component after it,
/// in memory and on disk alike.
#[test]
fn the_memory_tree_finds_what_the_disk_does() {
    let files = [
        "/t/p/package.json",
        "/t/p/a.js",
        "/t/linked/foo/package.json",
        "/t/linked/foo/dist/i.mjs",
    ];
    // every target relative, so the layout is the same under any base
    let links = [
        ("/t/p/node_modules/foo", "../../linked/foo"),
        ("/t/linked/foo/lib", "dist"),
        ("/t/p/up", ".."),
        ("/t/p/file_link.js", "a.js"),
        ("/t/p/dangling", "missing"),
        ("/t/p/loop", "loop"),
    ];
    let memory = MemoryTree {
        files: files.into_iter().map(|file| (file, "")).collect(),
        links: links.into_iter().collect(),
        ..MemoryTree::default()
    };
    let test_dir = TestDir::on_disk("tree_faithful");
    let dir = test_dir.path();
    for file in files {
        let path = dir.join(file.trim_start_matches('/'));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    for (link, target) in links {
        let path = dir.join(link.trim_start_matches('/'));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(target, path).unwrap();
    }
    let probes = [
        "/t/p/node_modules/foo",
        "/t/p/node_modules/foo/package.json",
        "/t/p/node_modules/foo/lib/i.mjs",
        "/t/p/node_modules/foo/../foo/package.json",
        "/t/p/node_modules/foo/..",
        "/t/p/node_modules/foo/lib/../package.json",
        "/t/p/node_modules",
        "/t/p/up/p/a.js",
        "/t/p/up/linked/foo/lib/i.mjs",
        "/t/p/file_link.js",
        "/t/p/dangling",
        "/t/p/loop",
        "/t/p/loop/a.js",
        "/t/p/a.js/x",
        "/t/p/a.js/..",
        "/t/p/a.js/.",
        "/t/p/a.js/",
        "/t/p/node_modules/foo/",
        "/t/p/node_modules/foo/.",
        "/t/p/missing/../a.js",
        "/t/p//a.js",
        "/t/p/./a.js",
        "/t/p/../p/a.js",
        "/t/p/missing.js",
        "/t",
    ];
    let results: Vec<_> = probes
        .iter()
        .map(|probe| {
            let on_disk = found(
                DiskTree.lookup(&dir.join(probe.trim_start_matches('/'))),
                dir,
            );
            let in_memory = found(memory.lookup(Path::new(probe)), Path::new("/"));
            (*probe, on_disk, in_memory)
        })
        .collect();
    for (probe, on_disk, in_memory) in results {
        assert_eq!(in_memory, on_disk, "{probe}");
    }
    // and the probes found something of each kind
    let memory_found = |probe| found(memory.lookup(Path::new(probe)), Path::new("/"));
    assert_eq!(
        memory_found("/t/p/node_modules/foo/lib/i.mjs"),
        Found::File(PathBuf::from("/t/linked/foo/dist/i.mjs"))
    );
    assert_eq!(
        memory_found("/t/p/node_modules/foo/.."),
        Found::Dir(PathBuf::from("/t/linked"))
    );
    assert!(matches!(memory_found("/t/p/loop"), Found::Error(Some(_))));
}

/// Restores a directory's mode on drop, so a test that took its search
/// permission away leaves it removable whether or not the test passes.
struct ModeRestored<'a>(&'a Path);

impl Drop for ModeRestored<'_> {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// A symlink whose target enters a directory the process can't search and
/// leaves it by `..` fails with the kernel's `EACCES`, as an open of the
/// path does, rather than resolving by its text.
#[test]
fn a_link_through_an_unsearchable_directory_fails_as_an_open_does() {
    use std::os::unix::fs::PermissionsExt as _;
    let test_dir = TestDir::on_disk("tree_unsearchable");
    let dir = test_dir.path();
    std::fs::create_dir(dir.join("denied")).unwrap();
    std::fs::write(dir.join("a.js"), "").unwrap();
    std::os::unix::fs::symlink("denied/../a.js", dir.join("link.js")).unwrap();
    let denied = dir.join("denied");
    std::fs::set_permissions(&denied, std::fs::Permissions::from_mode(0o000)).unwrap();
    let _restored = ModeRestored(&denied);
    // a process the mode doesn't hold (one with CAP_DAC_OVERRIDE) proves nothing here
    if std::fs::metadata(denied.join("..")).is_ok() {
        return;
    }
    let error = DiskTree.lookup(&dir.join("link.js")).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EACCES));
    assert_eq!(
        std::fs::File::open(dir.join("link.js"))
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EACCES)
    );
    // the directory itself is found: only its parent is searched
    assert_eq!(
        found(DiskTree.lookup(&denied), dir),
        Found::Dir(PathBuf::from("/denied"))
    );
}

/// What a `/proc` link names that is no path, a pipe here, is found as
/// another kind of entry, as its handle's metadata says.
#[test]
fn a_proc_link_to_a_pipe_is_another_kind_of_entry() {
    use std::os::fd::AsRawFd as _;
    let (reader, _writer) = io::pipe().unwrap();
    let path = PathBuf::from(format!("/proc/self/fd/{}", reader.as_raw_fd()));
    assert_eq!(DiskTree.lookup(&path).unwrap(), Some(Entry::Other));
}

/// An entry removed after it was opened has no real path, so it is found
/// absent: one whose link count fell to 0 after the kernel named it, one
/// the kernel named " (deleted)" while another link to its file remains,
/// and not one whose own name ends so.
#[test]
fn an_entry_removed_since_it_was_opened_is_absent() {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let test_dir = TestDir::on_disk("tree_removed");
    let dir = test_dir.path();
    let open = |path: &Path| {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH)
            .open(path)
            .unwrap()
    };
    let name_of = |handle: &std::fs::File| {
        std::fs::read_link(format!("/proc/self/fd/{}", handle.as_raw_fd())).unwrap()
    };
    // named before its removal, counted after
    std::fs::write(dir.join("solo.js"), "").unwrap();
    let handle = open(&dir.join("solo.js"));
    let name = name_of(&handle);
    assert_eq!(name, dir.join("solo.js"));
    std::fs::remove_file(dir.join("solo.js")).unwrap();
    assert_eq!(
        tree::found(name, &handle.metadata().unwrap()).unwrap(),
        None
    );
    // unlinked while another link remains
    std::fs::write(dir.join("a.js"), "").unwrap();
    std::fs::hard_link(dir.join("a.js"), dir.join("b.js")).unwrap();
    let handle = open(&dir.join("a.js"));
    std::fs::remove_file(dir.join("a.js")).unwrap();
    let name = name_of(&handle);
    assert_eq!(name, dir.join("a.js (deleted)"));
    assert_eq!(
        tree::found(name, &handle.metadata().unwrap()).unwrap(),
        None
    );
    // another file now at the name the kernel gave
    std::fs::write(dir.join("a.js (deleted)"), "").unwrap();
    let name = name_of(&handle);
    assert_eq!(
        tree::found(name, &handle.metadata().unwrap()).unwrap(),
        None
    );
    // a file whose own name ends so is found
    std::fs::write(dir.join("c.js (deleted)"), "").unwrap();
    assert_eq!(
        found(DiskTree.lookup(&dir.join("c.js (deleted)")), dir),
        Found::File(PathBuf::from("/c.js (deleted)"))
    );
}

/// A lookup through a `/proc` link takes the identity from the handle it
/// opened, never from the path the kernel names that handle by: a file
/// unlinked at one name while another link to it remains, with another file
/// now at the name the kernel gives it (the old name and " (deleted)"), is
/// found absent, not as that other file.
#[test]
fn a_proc_link_to_an_unlinked_name_is_not_the_file_now_at_that_name() {
    use std::os::fd::AsRawFd as _;
    let test_dir = TestDir::on_disk("tree_unlinked_name");
    let dir = test_dir.path();
    std::fs::write(dir.join("a.js"), "").unwrap();
    std::fs::hard_link(dir.join("a.js"), dir.join("b.js")).unwrap();
    let held = std::fs::File::open(dir.join("a.js")).unwrap();
    std::fs::remove_file(dir.join("a.js")).unwrap();
    std::fs::write(dir.join("a.js (deleted)"), "planted").unwrap();
    let path = PathBuf::from(format!("/proc/self/fd/{}", held.as_raw_fd()));
    assert_eq!(DiskTree.lookup(&path).unwrap(), None);
}
