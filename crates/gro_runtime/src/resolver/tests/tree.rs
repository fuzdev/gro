//! The in-memory tree against the disk: the same layout looked up in both
//! finds the same real paths, symlinks on any component included.

use std::io;
use std::path::{Path, PathBuf};

use crate::resolver::test_tree::{MemoryTree, disk_test_dir};
use crate::resolver::{DiskTree, Entry, SourceTree};

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
    let dir = disk_test_dir("tree_faithful");
    // a run that crashed with this pid would leave its links behind
    let _ = std::fs::remove_dir_all(&dir);
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
    let base = dir.canonicalize().unwrap();
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
                DiskTree.lookup(&base.join(probe.trim_start_matches('/'))),
                &base,
            );
            let in_memory = found(memory.lookup(Path::new(probe)), Path::new("/"));
            (*probe, on_disk, in_memory)
        })
        .collect();
    // removed before any assertion, so a failure leaves nothing behind
    std::fs::remove_dir_all(&dir).unwrap();
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
