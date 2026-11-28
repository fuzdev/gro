//! Tests for an entry on disk resolved before its run
//! (`resolve_file_entry`): its project root is the one its own resolution
//! found, never taken from the working directory it was resolved from, and it carries
//! how that root was found.

use std::path::Path;

use crate::resolve_file_entry;
use crate::resolver::{EntryBound, ResolveErrorKind};
use crate::test_support::TestDir;

/// Writes each of `files`, relative to `dir`, making its directories.
fn write(dir: &Path, files: &[(&str, &str)]) {
    for (file, content) in files {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

/// A project with a `package.json`, a module in it and one in a package it
/// installed, a directory beside it with a type marker and no project's
/// `package.json`, and an empty one, each in the system's temporary
/// directory, which ends every walk up.
fn tree(name: &str) -> TestDir {
    let dir = TestDir::in_system_temp(&format!("file_entry_{name}"));
    write(
        dir.path(),
        &[
            ("p/package.json", r#"{"name": "p"}"#),
            ("p/src/main.mjs", ""),
            ("p/node_modules/q/package.json", r#"{"name": "q"}"#),
            ("p/node_modules/q/index.mjs", ""),
            ("loose/package.json", r#"{"type": "module"}"#),
            ("loose/sub/x.mjs", ""),
        ],
    );
    std::fs::create_dir(dir.path().join("elsewhere")).unwrap();
    dir
}

/// The root is the project's the entry's walk found, by a relative path
/// or an absolute one, from a working directory outside it or inside it
/// below the root.
#[test]
fn the_root_is_the_entry_s_project_not_the_working_directory() {
    let dir = tree("found");
    let project = dir.path().join("p");
    for (path, cwd) in [
        ("../p/src/main.mjs", dir.path().join("elsewhere")),
        ("main.mjs", project.join("src")),
        (
            project.join("src/main.mjs").to_str().unwrap(),
            dir.path().join("loose"),
        ),
    ] {
        let entry = resolve_file_entry(Path::new(path), &cwd).unwrap();
        assert_eq!(entry.root(), project, "{path} from {}", cwd.display());
        assert_eq!(entry.bound(), &EntryBound::Found);
    }
}

/// An entry whose walk finds no project's `package.json`, a type marker
/// above it, its root its own directory, though the working directory is a
/// project's: the bound says the walk ended at the world-writable
/// directory.
#[test]
fn an_unbounded_entry_s_root_is_its_own_directory() {
    let dir = tree("unbounded");
    let entry = resolve_file_entry(Path::new("../loose/sub/x.mjs"), &dir.path().join("p")).unwrap();
    assert_eq!(entry.root(), dir.path().join("loose/sub"));
    assert_eq!(
        entry.bound(),
        &EntryBound::Unbounded {
            stopped_at: dir.path().parent().map(Path::to_owned),
        }
    );
}

/// An entry inside `node_modules` runs from the project holding it, its
/// package's directory no root of the project's.
#[test]
fn an_entry_in_node_modules_has_the_project_holding_it() {
    let dir = tree("in_node_modules");
    let entry =
        resolve_file_entry(Path::new("node_modules/q/index.mjs"), &dir.path().join("p")).unwrap();
    assert_eq!(entry.root(), dir.path().join("p"));
    assert_eq!(entry.bound(), &EntryBound::InNodeModules);
}

/// A path naming no module is the resolver's refusal, as the run's.
#[test]
fn a_missing_entry_is_refused() {
    let dir = tree("missing");
    let error = resolve_file_entry(Path::new("nope.mjs"), &dir.path().join("p")).unwrap_err();
    assert!(
        matches!(error.kind, ResolveErrorKind::NotFound { .. }),
        "{error:?}"
    );
    assert_eq!(error.specifier, "nope.mjs");
}

/// An entry resolved on one thread runs on another, as a boxed run's is
/// resolved before its sized thread starts.
#[test]
fn a_file_entry_moves_to_another_thread() {
    let dir = tree("sent");
    let entry = resolve_file_entry(Path::new("src/main.mjs"), &dir.path().join("p")).unwrap();
    let root = dir.path().join("p");
    let moved = std::thread::spawn(move || entry).join().unwrap();
    assert_eq!(moved.root(), root);
}

/// An entry with no project's `package.json` at or above it, in a confined
/// run, is prepared with its import graph read, the `package.json` files
/// its walk read above its root among what's read; an unconfined run's,
/// and any other entry, read nothing.
#[test]
fn only_a_confined_unbounded_entry_reads_its_graph() {
    let dir = tree("markers");
    let real = dir.path().canonicalize().unwrap();
    let confined = crate::Authority::Confined(crate::Grants::default());
    let conditions = crate::resolver::Conditions::default();
    let loose = resolve_file_entry(Path::new("loose/sub/x.mjs"), dir.path()).unwrap();
    assert!(loose.is_unbounded());
    let prepared = tsv_ts::run_with_stack(move || loose.prepare(&confined, conditions)).unwrap();
    let graph = prepared.graph().unwrap();
    assert!(
        graph
            .files()
            .any(|file| file == real.join("loose/package.json")),
        "{graph:?}"
    );
    assert!(
        graph
            .modules()
            .any(|module| module == real.join("loose/sub/x.mjs")),
        "{graph:?}"
    );
    let loose = resolve_file_entry(Path::new("loose/sub/x.mjs"), dir.path()).unwrap();
    assert!(
        loose
            .prepare(&crate::Authority::Unconfined, conditions)
            .graph()
            .is_none()
    );
    for path in ["p/src/main.mjs", "p/node_modules/q/index.mjs"] {
        let entry = resolve_file_entry(Path::new(path), dir.path()).unwrap();
        assert!(!entry.is_unbounded(), "{path}");
        let confined = crate::Authority::Confined(crate::Grants::default());
        assert!(
            entry.prepare(&confined, conditions).graph().is_none(),
            "{path}"
        );
    }
}
