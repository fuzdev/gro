//! The link closure: the box's roots, read from the disk before anything is
//! confined, and the handle opened on each.

use std::io;
use std::path::{Path, PathBuf};

use super::Pin;
use crate::resolver::{DiskTree, SourceTree};

/// The link closure of the project at `project`, opened as `pinned`: it
/// first, then each `node_modules` entry's real path that is neither under a
/// root so far nor holding one, is a package directory ([`is_package`]), is
/// held by no world-writable directory ([`held_by_world_writable`]), and
/// opens as found ([`pin`]), read in turn to a fixed point. Returns the
/// roots and their handles, in the same order.
///
/// An entry the listing says isn't a symlink has its holder's real path,
/// read once per holder, joined with its name as its own
/// ([`PackageEntry::real`]), and the containment check, which reads nothing
/// from the disk, comes first: every package installed under a root is
/// skipped with no path resolved, and only a link, or an entry whose holder
/// is real outside the roots, is read further.
pub(super) fn link_closure(project: PathBuf, pinned: Pin) -> (Vec<PathBuf>, Vec<Pin>) {
    let mut roots = vec![project];
    let mut pins = vec![pinned];
    let mut next = 0;
    while let Some(root) = roots.get(next) {
        let listing = package_entries(root.join("node_modules"));
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
pub(super) fn pin(dir: &Path) -> io::Result<Pin> {
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
pub(super) const fn pin(_dir: &Path) -> io::Result<Pin> {
    Ok(())
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
struct Listing {
    /// The `node_modules` directory.
    node_modules: Holder,
    /// Its `@scope` directories that were listed.
    scopes: Vec<Holder>,
    /// The entries, sorted by path.
    entries: Vec<PackageEntry>,
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
struct PackageEntry {
    /// Its path, as listed.
    path: PathBuf,
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
/// a `.package-lock.json`, a `.deno` store) skipped. Its real path is read
/// once, after it's listed, and each `@scope`'s is derived from it the same
/// way an entry's is ([`PackageEntry::real`]).
fn package_entries(node_modules: PathBuf) -> Listing {
    let listed = list(&node_modules);
    let real = if listed.is_empty() {
        None
    } else {
        std::fs::canonicalize(&node_modules).ok()
    };
    let mut listing = Listing {
        node_modules: Holder {
            path: node_modules,
            real,
        },
        scopes: Vec::new(),
        entries: Vec::new(),
    };
    for (path, linked) in listed {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let real = real_by_name(listing.node_modules.real.as_deref(), &path, linked);
        if name.starts_with('@') {
            let scoped = list(&path);
            if scoped.is_empty() {
                continue;
            }
            let scope = Holder {
                real: real.or_else(|| std::fs::canonicalize(&path).ok()),
                path,
            };
            listing.entries.extend(
                scoped
                    .into_iter()
                    .filter(|(scoped, _)| {
                        !scoped
                            .file_name()
                            .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
                    })
                    .map(|(scoped, linked)| PackageEntry {
                        real: real_by_name(scope.real.as_deref(), &scoped, linked),
                        path: scoped,
                        scope: Some(listing.scopes.len()),
                    }),
            );
            listing.scopes.push(scope);
        } else {
            listing.entries.push(PackageEntry {
                path,
                scope: None,
                real,
            });
        }
    }
    listing.entries.sort_by(|a, b| a.path.cmp(&b.path));
    listing
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

/// The entries of the directory `dir`, or none when it can't be listed: each
/// path, and whether it may be a symlink, by its directory entry's type,
/// which std reads with `lstat` where the filesystem records none (and an
/// entry whose type can't be read may be one).
fn list(dir: &Path) -> Vec<(PathBuf, bool)> {
    std::fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |entries| {
            entries
                .filter_map(|entry| {
                    entry.ok().map(|entry| {
                        let linked = !entry.file_type().is_ok_and(|kind| !kind.is_symlink());
                        (entry.path(), linked)
                    })
                })
                .collect()
        },
    )
}
