//! Tests for the identity of the file loaded: a swapped, remade, or
//! replaced file is refused.

use super::*;

/// The control: a file left alone between resolving and loading loads,
/// the entry's and an import's alike.
#[test]
fn an_unchanged_file_loads() {
    let fixtures = DiskFixtures::new("unchanged", &[("main.ts", ""), ("a.js", SOURCE)]);
    let loader = fixtures.loader("main.ts");
    let main = fixtures.url("main.ts");
    let url = loader
        .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
        .unwrap();
    assert_eq!(load_disk(&loader, &url).unwrap(), SOURCE);
    assert_eq!(load_disk(&loader, &main).unwrap(), "");
    // resolving it again finds the same file
    loader
        .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
        .unwrap();
    // and a URL never resolved here isn't loaded
    let never = fixtures.url("package.json");
    load_disk(&loader, &never).unwrap_err();
    assert_failed(
        &graph(&loader),
        never.as_str(),
        &FailureReason::Load(LoadFailure::NeverResolved),
    );
}

/// A file replaced by a rename after it was resolved isn't loaded, for an
/// import and the entry, and resolving its URL again keeps the first
/// record, so the load is still refused.
#[test]
fn a_file_renamed_over_is_refused() {
    let fixtures = DiskFixtures::new("renamed", &[("main.ts", ""), ("a.js", SOURCE)]);
    let loader = fixtures.loader("main.ts");
    let main = fixtures.url("main.ts");
    let url = loader
        .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
        .unwrap();
    fixtures.replace("a.js", "export const a = 2;\n");
    assert_replaced(&loader, &url);
    // resolving it again keeps the first file, which the load still checks
    loader
        .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
        .unwrap();
    assert_replaced(&loader, &url);
    fixtures.replace("main.ts", "");
    assert_replaced(&loader, &main);
}

/// A directory on the path swapped for a symlink to a twin directory
/// holding a file of the same name and content: the path now opens
/// another file, refused.
#[test]
fn a_directory_swapped_for_a_link_is_refused() {
    let fixtures = DiskFixtures::new(
        "component",
        &[("main.ts", ""), ("sub/a.js", SOURCE), ("twin/a.js", SOURCE)],
    );
    let loader = fixtures.loader("main.ts");
    let url = loader
        .resolve_reached(
            &DiskTree,
            "./sub/a.js",
            fixtures.url("main.ts").as_str(),
            STATIC,
        )
        .unwrap();
    std::fs::rename(fixtures.path("sub"), fixtures.path("old_sub")).unwrap();
    fixtures.link("sub", "twin");
    assert_replaced(&loader, &url);
}

/// The file swapped for a symlink to an identical file: the open follows
/// it to a file with another identity, refused.
#[test]
fn a_file_swapped_for_a_link_is_refused() {
    let fixtures = DiskFixtures::new(
        "last_component",
        &[("main.ts", ""), ("a.js", SOURCE), ("copy.js", SOURCE)],
    );
    let loader = fixtures.loader("main.ts");
    let url = loader
        .resolve_reached(
            &DiskTree,
            "./a.js",
            fixtures.url("main.ts").as_str(),
            STATIC,
        )
        .unwrap();
    fixtures.link("a.js", "copy.js");
    assert_replaced(&loader, &url);
}

/// A file removed and made again after it was resolved isn't loaded,
/// though ext4 hands the new file the old one's freed inode number.
#[test]
fn a_file_removed_and_remade_is_refused() {
    let fixtures = DiskFixtures::new("remade", &[("main.ts", ""), ("a.js", SOURCE)]);
    let loader = fixtures.loader("main.ts");
    let url = loader
        .resolve_reached(
            &DiskTree,
            "./a.js",
            fixtures.url("main.ts").as_str(),
            STATIC,
        )
        .unwrap();
    // past the filesystem clock's tick, so the birth times differ
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::remove_file(fixtures.path("a.js")).unwrap();
    std::fs::write(fixtures.path("a.js"), "export const a = 2;\n").unwrap();
    assert_replaced(&loader, &url);
}

/// A FIFO put in the file's place after it was resolved is refused
/// without blocking on the open. The load runs on its own thread, so a
/// blocking open fails the test by its timeout rather than hanging it.
#[test]
fn a_fifo_swapped_in_is_refused_without_blocking() {
    let fixtures = DiskFixtures::new("fifo", &[("main.ts", ""), ("a.js", SOURCE)]);
    let loader = fixtures.loader("main.ts");
    let url = loader
        .resolve_reached(
            &DiskTree,
            "./a.js",
            fixtures.url("main.ts").as_str(),
            STATIC,
        )
        .unwrap();
    let fifo = fixtures.path("a.js.fifo");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    std::fs::rename(fifo, fixtures.path("a.js")).unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let load_url = url.clone();
    std::thread::spawn(move || {
        let _ = send.send((load_disk(&loader, &load_url), graph(&loader)));
    });
    let Ok((result, graph)) = receive.recv_timeout(std::time::Duration::from_secs(5)) else {
        // a writer releases the blocked open, so the thread ends
        drop(
            std::fs::OpenOptions::new()
                .write(true)
                .open(fixtures.path("a.js")),
        );
        panic!("opening a FIFO in the file's place blocked");
    };
    result.unwrap_err();
    assert_failed(
        &graph,
        url.as_str(),
        &FailureReason::Load(LoadFailure::ReplacedSinceLookup),
    );
}
