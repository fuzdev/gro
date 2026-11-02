//! Tests for the link closure.

use std::path::{Path, PathBuf};

use super::{Scratch, set_mode};
use crate::confine::Prepared;

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
        &scratch.path().join("w/tool/package.json"),
        "w/linked_json/package.json",
    );
    for dir in [&shared, &open] {
        set_mode(dir, 0o777);
    }
    std::fs::write(scratch.path().join("w/file.txt"), "").unwrap();
    scratch.dir("w/project/node_modules/@fuzdev");
    scratch.dir("w/project/node_modules/installed");
    scratch.dir("w/project/node_modules/@scope/installed");
    scratch.dir("w/tool/node_modules");
    scratch.file("w/project/node_modules/installed/package.json");
    scratch.link(&tool, "w/project/node_modules/@fuzdev/tool");
    scratch.link(&scoped, "w/project/node_modules/@scope/linked");
    scratch.link(&hidden, "w/project/node_modules/.bin");
    scratch.link(&hidden, "w/project/node_modules/@fuzdev/.hidden");
    scratch.link(
        &scratch.path().join("absent"),
        "w/project/node_modules/dangling",
    );
    scratch.link(
        &scratch.path().join("w/file.txt"),
        "w/project/node_modules/file",
    );
    scratch.link(&scratch.path().join("w"), "w/project/node_modules/ancestor");
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

/// A package found through a `node_modules` or an `@scope` linked into a
/// world-writable directory adds nothing, even when it is a link to a
/// directory no world-writable one holds.
#[test]
fn a_linked_holder_in_a_world_writable_directory_holds_no_root() {
    let scratch = Scratch::new("open_holders");
    let project = scratch.dir("w/project");
    let plain = scratch.dir("w/plain");
    let open = scratch.dir("w/open");
    let store = scratch.dir("w/open/store");
    let scope = scratch.dir("w/open/scope");
    scratch.dir("w/plain/node_modules");
    for dir in ["w/a", "w/b", "w/c"] {
        scratch.dir(dir);
        scratch.file(&format!("{dir}/package.json"));
    }
    set_mode(&open, 0o777);
    scratch.link(&store, "w/project/node_modules");
    scratch.link(&scratch.path().join("w/a"), "w/open/store/a");
    scratch.link(&scope, "w/plain/node_modules/@s");
    scratch.link(&scratch.path().join("w/b"), "w/open/scope/b");
    scratch.link(&scratch.path().join("w/c"), "w/plain/node_modules/c");
    assert_eq!(closure(&project), [project]);
    assert_eq!(closure(&plain), [plain, scratch.path().join("w/c")]);
}

/// An entry that isn't a symlink is a root when the directory holding it
/// is real outside the roots: a project's `node_modules` linked out, an
/// `@scope` in it linked out, or an `@scope` in a linked package's
/// `node_modules` linked out holds packages installed there, and each is
/// one. A package installed under a root, the project's or a linked
/// package's, is not; a linked package is, and so is a link in its own
/// `node_modules`.
#[test]
fn entries_held_by_a_linked_directory_are_read_at_their_real_paths() {
    let scratch = Scratch::new("holders");
    // below the scratch directory, whose own parent is world-writable
    let project = scratch.dir("w/project");
    let outside = scratch.dir("w/store/outside");
    let scope = scratch.dir("w/store/scope");
    let lib = scratch.dir("w/lib");
    let util = scratch.dir("w/util");
    let lib_store = scratch.dir("w/lib_store");
    for dir in [
        "w/store/outside/alpha",
        "w/store/outside/beta",
        "w/store/scope/gamma",
        "w/lib/node_modules/installed",
        "w/lib_store/delta",
    ] {
        scratch.dir(dir);
        scratch.file(&format!("{dir}/package.json"));
    }
    for dir in ["w/lib", "w/util"] {
        scratch.file(&format!("{dir}/package.json"));
    }
    // the project's `node_modules` is a directory outside it, with
    // packages installed in it and an `@scope` linked further out
    scratch.link(&outside, "w/project/node_modules");
    scratch.link(&scope, "w/store/outside/@linked");
    // a linked package, its own installed package, a link in its
    // `node_modules`, and an `@scope` there holding a store linked out
    scratch.dir("w/store/outside/@fuzdev");
    scratch.link(&lib, "w/store/outside/@fuzdev/lib");
    scratch.link(&util, "w/lib/node_modules/util");
    scratch.link(&lib_store, "w/lib/node_modules/@store");
    assert_eq!(
        closure(&project),
        [
            project.clone(),
            lib.clone(),
            scope.join("gamma"),
            outside.join("alpha"),
            outside.join("beta"),
            lib_store.join("delta"),
            util.clone(),
        ]
    );

    // the same packages under a real `node_modules` in the project: the
    // installed ones are inside it, and only the links reach out
    let plain = scratch.dir("w/plain");
    scratch.dir("w/plain/node_modules/@fuzdev");
    for dir in [
        "w/plain/node_modules/installed",
        "w/plain/node_modules/@fuzdev/scoped",
    ] {
        scratch.dir(dir);
        scratch.file(&format!("{dir}/package.json"));
    }
    scratch.link(&lib, "w/plain/node_modules/@fuzdev/lib");
    scratch.link(&scope, "w/plain/node_modules/@linked");
    assert_eq!(
        closure(&plain),
        [
            plain,
            lib,
            scope.join("gamma"),
            lib_store.join("delta"),
            util,
        ]
    );
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
    scratch.link(&scratch.path().join("real"), "alias");
    assert!(Prepared::for_project(scratch.path().join("linked")).is_err());
    assert!(Prepared::for_project(scratch.path().join("alias/project")).is_err());
    assert!(Prepared::for_project(scratch.path().join("absent")).is_err());
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
    assert!(
        !prepared
            .roots()
            .holds(&scratch.path().join("projectile/b.ts"))
    );
}
