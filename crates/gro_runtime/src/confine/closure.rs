//! The link closure: the box's roots, read from the disk before anything is
//! confined, and the handle opened on each.

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use super::rules::open_real_dir;
use crate::resolver::{DiskTree, SourceTree};

/// The link closure from `seeds`, each a real directory and its handle,
/// the project's first: the seeds, then each `node_modules` entry's real
/// path that is neither under a root so far nor holding one, is a package
/// directory ([`is_package`]), is held by no world-writable directory
/// ([`held_by_world_writable`]), and opens as found ([`open_real_dir`]),
/// read in turn to a fixed point. Returns the roots and their handles, in
/// the same order.
///
/// A seed beside the project is where a `node_modules` that is a symlink
/// leads, so the packages installed there are inside a root, not each a
/// root of its own.
///
/// An entry the listing says isn't a symlink has its holder's real path,
/// read once per holder, joined with its name as its own
/// ([`PackageEntry::real`]), so one whose holder lies in a root lies there
/// too and is left out of the listing as it's read, before a path is made
/// or sorted for it ([`package_entries`]): every package installed under a
/// root costs its directory entry alone, and only a link, or an entry whose
/// holder is real outside the roots, is read further. The roots only grow,
/// so an entry left out would have been skipped by the containment check.
pub(super) fn link_closure(seeds: Vec<(PathBuf, OwnedFd)>) -> (Vec<PathBuf>, Vec<OwnedFd>) {
    let (mut roots, mut pins): (Vec<PathBuf>, Vec<OwnedFd>) = seeds.into_iter().unzip();
    let mut next = 0;
    while let Some(root) = roots.get(next) {
        let listing = package_entries(root.join("node_modules"), &roots);
        next += 1;
        for entry in &listing.entries {
            // a package is a directory with a package.json, and none holds a
            // root: a link to a file, to an ancestor of the project, to `/`,
            // or to a directory that isn't a package widens nothing
            let Some(target) = entry
                .real
                .clone()
                .or_else(|| std::fs::canonicalize(&entry.path).ok())
            else {
                continue;
            };
            if !roots
                .iter()
                .any(|root| target.starts_with(root) || root.starts_with(&target))
                && is_package(&target)
                && !held_by_world_writable(&listing, entry, &target)
                && let Ok(pinned) = open_real_dir(&target)
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

/// Whether the package linked at `entry` in `listing`'s `node_modules`,
/// whose real path is `target`, is held by a world-writable directory, which
/// any user can plant files in: `node_modules` and the entry's `@scope`
/// directory, the real parent of each, `target`, or the directories holding
/// `target`, one per segment of the package's name. The resolver refuses a
/// package found through any of them, by the same predicate
/// ([`SourceTree::is_world_writable`]), so none is worth a root. A directory
/// whose mode can't be read counts as one.
fn held_by_world_writable(listing: &Listing, entry: &PackageEntry, target: &Path) -> bool {
    let scope = entry.scope.map(|scope| &listing.scopes[scope]);
    let segments = if scope.is_some() { 2 } else { 1 };
    let holders = [Some(&listing.node_modules), scope].into_iter().flatten();
    let mut dirs: Vec<&Path> = holders
        .clone()
        .map(|holder| holder.path.as_path())
        .collect();
    dirs.extend(
        holders
            .filter_map(|holder| holder.real.as_deref())
            .filter_map(Path::parent),
    );
    dirs.extend(target.ancestors().take(segments + 1));
    dirs.iter()
        .any(|dir| !matches!(DiskTree.is_world_writable(dir), Ok(false)))
}

/// A `node_modules` directory's package entries, with the directories
/// holding them.
pub(super) struct Listing {
    /// The `node_modules` directory.
    node_modules: Holder,
    /// Its `@scope` directories that hold an entry listed.
    scopes: Vec<Holder>,
    /// The entries, sorted by path.
    pub(super) entries: Vec<PackageEntry>,
}

/// A directory holding package entries: a `node_modules` or an `@scope` in
/// one.
struct Holder {
    /// Its path, as listed.
    path: PathBuf,
    /// Its real path, or none when it can't be read.
    real: Option<PathBuf>,
}

/// A package entry: a `<name>` in `node_modules` or a `<name>` in one of its
/// `@scope` directories.
pub(super) struct PackageEntry {
    /// Its path, as listed.
    pub(super) path: PathBuf,
    /// The index in [`Listing::scopes`] of the `@scope` directory holding
    /// it, or none when `node_modules` does.
    scope: Option<usize>,
    /// Its real path when the listing gives it with no path resolved: the
    /// entry isn't a symlink (by its directory entry's type, or an `lstat`
    /// where the filesystem records none) and its holder's real path is
    /// known, so the real path is the holder's joined with the entry's
    /// name; otherwise none, and it's resolved in full.
    real: Option<PathBuf>,
}

/// The package entries of the `node_modules` at `node_modules`, sorted by
/// path: each `<name>` and each `@<scope>/<name>`, the dot-named ones (`.bin`,
/// a `.package-lock.json`, a `.deno` store) skipped, and so is each that
/// isn't a symlink in a directory whose real path lies in one of `roots`,
/// which lies there too. Its real path is read once, when it lists
/// anything, and each `@scope`'s is derived from it the same way an
/// entry's is ([`PackageEntry::real`]).
pub(super) fn package_entries(node_modules: PathBuf, roots: &[PathBuf]) -> Listing {
    let mut listed = list(&node_modules).peekable();
    let real = listed
        .peek()
        .and_then(|_| std::fs::canonicalize(&node_modules).ok());
    let installed = in_roots(real.as_deref(), roots);
    let mut listing = Listing {
        node_modules: Holder {
            path: node_modules,
            real,
        },
        scopes: Vec::new(),
        entries: Vec::new(),
    };
    for entry in listed {
        let linked = may_be_link(&entry);
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if name.starts_with('@') {
            let path = entry.path();
            let real = real_by_name(listing.node_modules.real.as_deref(), &path, linked);
            let installed = in_roots(real.as_deref(), roots);
            let scope = listing.scopes.len();
            let scoped: Vec<(PathBuf, bool)> = list(&path)
                .filter_map(|scoped| {
                    let linked = may_be_link(&scoped);
                    (linked || !installed)
                        .then(|| (scoped.path(), linked))
                        .filter(|(path, _)| {
                            !path
                                .file_name()
                                .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
                        })
                })
                .collect();
            if scoped.is_empty() {
                continue;
            }
            let holder = Holder {
                real: real.or_else(|| std::fs::canonicalize(&path).ok()),
                path,
            };
            listing
                .entries
                .extend(scoped.into_iter().map(|(scoped, linked)| PackageEntry {
                    real: real_by_name(holder.real.as_deref(), &scoped, linked),
                    path: scoped,
                    scope: Some(scope),
                }));
            listing.scopes.push(holder);
        } else if linked || !installed {
            let path = entry.path();
            listing.entries.push(PackageEntry {
                real: real_by_name(listing.node_modules.real.as_deref(), &path, linked),
                path,
                scope: None,
            });
        }
    }
    listing.entries.sort_by(|a, b| a.path.cmp(&b.path));
    listing
}

/// Whether `holder`, a real path, lies in one of `roots`: an entry it holds
/// that isn't a symlink lies there too. None, a holder whose real path
/// can't be read, lies in none.
fn in_roots(holder: Option<&Path>, roots: &[PathBuf]) -> bool {
    holder.is_some_and(|holder| roots.iter().any(|root| holder.starts_with(root)))
}

/// The real path of the entry at `path`, listed in a directory whose real
/// path is `holder`, when it isn't `linked`: `holder` joined with its name,
/// since only a symlink in its last component could resolve elsewhere.
fn real_by_name(holder: Option<&Path>, path: &Path, linked: bool) -> Option<PathBuf> {
    if linked {
        return None;
    }
    Some(holder?.join(path.file_name()?))
}

/// The entries of the directory `dir`, or none when it can't be listed.
fn list(dir: &Path) -> impl Iterator<Item = std::fs::DirEntry> + use<> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
}

/// Whether `entry` may be a symlink, by its directory entry's type, which
/// std reads with `lstat` where the filesystem records none (and an entry
/// whose type can't be read may be one).
fn may_be_link(entry: &std::fs::DirEntry) -> bool {
    !entry.file_type().is_ok_and(|kind| !kind.is_symlink())
}
