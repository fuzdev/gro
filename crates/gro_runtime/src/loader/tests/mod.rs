//! Tests for the loader: shared fixtures here, then the roots, the box, the
//! identity of the file loaded, and the graph's record by topic.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};

use super::graph::Record;
use super::reads::RecordingTree;
use super::*;
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{self, Entry, FileId, ModuleFormat};
use crate::test_support::TestDir;

mod boxed;
mod builtins;
mod identity;
mod recording;
mod roots;

/// A project at `/p` with `bar` installed and `lnk` linked to a checkout
/// inside the project.
fn tree() -> MemoryTree {
    MemoryTree {
        files: BTreeMap::from([
            ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
            ("/p/main.ts", ""),
            (
                "/p/packages/lnk/package.json",
                r#"{"name": "lnk", "type": "module", "exports": {"./x.ts": "./x.ts"}}"#,
            ),
            ("/p/packages/lnk/x.ts", ""),
            (
                "/p/node_modules/bar/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/p/node_modules/bar/i.mjs", ""),
        ]),
        links: BTreeMap::from([("/p/node_modules/lnk", "/p/packages/lnk")]),
        ..MemoryTree::default()
    }
}

const MAIN: &str = "file:///p/main.ts";

/// A static import, the reach most tests resolve by.
const STATIC: Reached = Reached::Import(Reach::Static);

/// A loader for the graph of `/p/main.ts` in `tree`, recording in full.
fn new_loader(tree: &MemoryTree) -> Loader {
    let (entry, roots) =
        resolver::resolve_entry(tree, Path::new("/p/main.ts"), Path::new("/")).unwrap();
    Loader::new(&entry, roots, Recording::Full)
}

fn state(loader: &Loader) -> (Roots, HashMap<Url, Record>) {
    let graph = loader.graph.borrow();
    (graph.roots().clone(), graph.records().clone())
}

/// Files on disk in a fresh directory (see [`TestDir`]), under a named
/// `package.json`, so the project root is the directory itself. Every
/// directory is `0o755`, the [`TestDir`] by its own making and each file's
/// parent here, so a umask of 0 can't make one world-writable.
struct DiskFixtures(TestDir);

impl DiskFixtures {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TestDir::on_disk(&format!("loader_{name}"));
        let package_json = [("package.json", r#"{"name": "fixture", "type": "module"}"#)];
        for (file, content) in files.iter().chain(&package_json) {
            let path = dir.path().join(file);
            let parent = path.parent().unwrap();
            std::fs::create_dir_all(parent).unwrap();
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::write(path, content).unwrap();
        }
        Self(dir)
    }

    /// The directory's canonical path.
    fn dir(&self) -> &Path {
        self.0.path()
    }

    fn path(&self, file: &str) -> PathBuf {
        self.dir().join(file)
    }

    fn url(&self, file: &str) -> Url {
        Url::from_file_path(self.path(file)).unwrap()
    }

    /// A loader for the graph of `entry`, recording in full.
    fn loader(&self, entry: &str) -> Loader {
        self.loader_recording(entry, Recording::Full)
    }

    /// A loader for the graph of `entry`, recording as `recording` says.
    fn loader_recording(&self, entry: &str, recording: Recording) -> Loader {
        let (entry, roots) =
            resolver::resolve_entry(&DiskTree, Path::new(entry), self.dir()).unwrap();
        Loader::new(&entry, roots, recording)
    }

    /// Replaces the file at `file` with a new one holding `content`,
    /// written beside it first and renamed over it, so the old file's
    /// inode is still in use when the new one is made and can't be
    /// reused for it.
    fn replace(&self, file: &str, content: &str) {
        let replacement = self.path(&format!("{file}.replacement"));
        std::fs::write(&replacement, content).unwrap();
        std::fs::rename(replacement, self.path(file)).unwrap();
    }

    /// Puts a symlink to `target` at `file`, made beside it and renamed
    /// over whatever is there.
    fn link(&self, file: &str, target: &str) {
        let link = self.path(&format!("{file}.link"));
        std::os::unix::fs::symlink(target, &link).unwrap();
        std::fs::rename(link, self.path(file)).unwrap();
    }
}

const SOURCE: &str = "export const a = 1;\n";

fn load_disk(loader: &Loader, url: &Url) -> Result<String, String> {
    loader
        .load_reached(url, &RequestedModuleType::None, Reach::Static, None)
        .map(|source| String::from_utf8(source.code.as_bytes().to_vec()).unwrap())
        .map_err(|error| error.to_string())
}

fn graph(loader: &Loader) -> Graph {
    loader.graph_clone()
}

/// Asserts `graph` recorded `reason` at a site of `specifier`: the
/// specifier as written for a resolution, the URL for a load.
fn assert_failed(graph: &Graph, specifier: &str, reason: &FailureReason) {
    let failures = graph.failures();
    assert!(
        failures
            .iter()
            .filter(|(site, _)| site.specifier == specifier)
            .flat_map(|(_, failures)| failures)
            .any(|failure| failure.reason == *reason),
        "no {reason:?} at {specifier}: {failures:?}"
    );
}

/// Asserts loading `url` fails because the file at its path is not the one
/// resolved, and the graph records exactly that one reason at its site.
fn assert_replaced(loader: &Loader, url: &Url) {
    load_disk(loader, url).unwrap_err();
    let graph = graph(loader);
    let reasons: Vec<_> = graph
        .failures()
        .iter()
        .filter(|(site, _)| site.specifier == url.as_str())
        .flat_map(|(_, failures)| failures)
        .map(|failure| &failure.reason)
        .collect();
    assert_eq!(
        reasons,
        [&FailureReason::Load(LoadFailure::ReplacedSinceLookup)]
    );
}
