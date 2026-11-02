//! Resolving the entry module, and the roots its graph starts with.

use std::path::{Component, Path, PathBuf};

use url::Url;

use super::Resolution;
use super::error::{ResolveError, ResolveErrorKind, UrlFault};
use super::find::find_module;
use super::package::parse_package_name;
use super::roots::{Origin, Root, Roots};
use super::tree::SourceTree;
use super::walk::{Consulted, FileRole, Walk, find_bound, read_package_json};

/// Resolves the entry module at `path`, relative to `cwd`, returning it and
/// the roots its graph starts with.
///
/// `cwd` is an absolute path, and an absolute `path` ignores it. The same
/// rules apply as to an import: the extension is mandatory, a path ending in
/// `/`, or in a `.` or `..` component, which leaves one as a URL, names a
/// directory and is refused, and the file must exist. It is a path, not a URL, so `%`, `?`, and `#` are file-name
/// characters. Errors name the path as given, imported from `cwd`.
///
/// The entry's origin comes from its real path, as every module's does. The
/// project root is the entry's project root, the directory the walk for one
/// finds, or with none (no `package.json` but type markers above it, or a
/// world-writable directory first) the entry's own directory. An entry
/// inside `node_modules` runs as the package it is in, the one after the
/// last `node_modules` segment, whose `package.json` gives its version, and
/// its project root is the directory holding the first segment; one in no
/// package is refused.
///
/// # Errors
///
/// Returns a [`ResolveError`] when the path names no loadable module.
pub fn resolve_entry(
    tree: &impl SourceTree,
    path: &Path,
    cwd: &Path,
) -> Result<(Resolution, Roots), ResolveError> {
    resolve_entry_with_bound(tree, path, cwd).map(|(resolution, roots, _)| (resolution, roots))
}

/// How the entry's project root was found, which [`resolve_entry_with_bound`]
/// reports beside the resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryBound {
    /// The walk for the project root found one: the project root is the
    /// directory of the first `package.json` at or above the entry, outside
    /// `node_modules`, that isn't a type marker.
    Found,
    /// The walk ended with none, at the filesystem's root or, when
    /// `stopped_at` is set, at that world-writable directory, so the project
    /// root is the entry's own directory.
    Unbounded {
        /// The world-writable directory the walk stopped at, reading nothing
        /// in it; unset when it reached the filesystem's root.
        stopped_at: Option<PathBuf>,
    },
    /// The entry is inside `node_modules`: its project root is the directory
    /// holding the first `node_modules` segment, found by the path, not by a
    /// walk.
    InNodeModules,
}

/// Resolves the entry as [`resolve_entry`] does, and also returns how its
/// project root was found.
///
/// # Errors
///
/// Returns a [`ResolveError`] when the path names no loadable module.
pub fn resolve_entry_with_bound(
    tree: &impl SourceTree,
    path: &Path,
    cwd: &Path,
) -> Result<(Resolution, Roots, EntryBound), ResolveError> {
    let specifier = path.display().to_string();
    let error = |kind| ResolveError {
        specifier: specifier.clone(),
        importer: cwd.display().to_string(),
        kind,
    };
    // checked as written, since normalizing drops a trailing `/`, `.`, or `..`,
    // where an import's URL keeps a trailing `/`, which names a directory
    if names_directory(path) {
        return Err(error(ResolveErrorKind::NoExtension { suggestion: None }));
    }
    // `join` keeps an absolute `path` as is
    let path = normalize(&cwd.join(path));
    if !path.is_absolute() {
        return Err(error(ResolveErrorKind::InvalidUrl(UrlFault::NotAbsolute)));
    }
    let mut consulted = Consulted::default();
    let (real_path, file_id) =
        find_module(tree, &path, Some(&specifier), &mut consulted, |_| Ok(())).map_err(error)?;
    let (roots, root, bound) = entry_roots(tree, &real_path, &mut consulted).map_err(error)?;
    let url = Url::from_file_path(&real_path)
        .map_err(|()| error(ResolveErrorKind::InvalidUrl(UrlFault::NoLocalPath)))?;
    let resolution = Resolution {
        url,
        file_id,
        root,
        files_consulted: consulted.0,
    };
    Ok((resolution, roots, bound))
}

/// The roots a graph whose entry is the module at `real_path` starts with,
/// the entry's own root among them, and how the project root was found (see
/// [`resolve_entry`]).
pub(super) fn entry_roots(
    tree: &impl SourceTree,
    real_path: &Path,
    consulted: &mut Consulted,
) -> Result<(Roots, Root, EntryBound), ResolveErrorKind> {
    let dir = real_path.parent().unwrap_or(real_path);
    let segments: Vec<usize> = real_path
        .components()
        .enumerate()
        .filter(|(_, component)| component.as_os_str() == "node_modules")
        .map(|(index, _)| index)
        .collect();
    let (Some(&first), Some(&last)) = (segments.first(), segments.last()) else {
        let (project_dir, bound) = match find_bound(tree, dir, None, consulted)? {
            Walk::Found((bound_dir, _)) => (bound_dir, EntryBound::Found),
            Walk::Ended => (dir.to_owned(), EntryBound::Unbounded { stopped_at: None }),
            Walk::Stopped(stopped_at) => (
                dir.to_owned(),
                EntryBound::Unbounded {
                    stopped_at: Some(stopped_at),
                },
            ),
        };
        let roots = Roots::new(project_dir);
        let root = roots.project().clone();
        return Ok((roots, root, bound));
    };
    let project_dir: PathBuf = real_path.components().take(first).collect();
    let mut roots = Roots::new(project_dir);
    // the package's name: one segment after node_modules, or a scope and one
    let names: Vec<Option<&str>> = real_path
        .components()
        .skip(last + 1)
        .map(|component| component.as_os_str().to_str())
        .collect();
    let length = if names
        .first()
        .copied()
        .flatten()
        .is_some_and(|name| name.starts_with('@'))
    {
        2
    } else {
        1
    };
    // a segment that isn't UTF-8 names no package
    let name = names
        .get(..length)
        .and_then(|names| names.iter().copied().collect::<Option<Vec<_>>>())
        .map(|names| names.join("/"));
    // the file itself is no package, and neither is `.bin` or `.deno`
    let Some(name) = name.filter(|name| {
        names.len() > length && parse_package_name(name).is_ok_and(|(_, subpath)| subpath == ".")
    }) else {
        return Err(ResolveErrorKind::InNoPackage(real_path.to_owned()));
    };
    let package_dir: PathBuf = real_path.components().take(last + 1 + length).collect();
    let package_json = read_package_json(tree, &package_dir, FileRole::Version, consulted)?
        .ok_or_else(|| ResolveErrorKind::NoPackageJson(package_dir.clone()))?;
    let root = Root {
        origin: Origin::Package {
            name,
            version: package_json.string("version").map(str::to_owned),
            linked: false,
        },
        dir: package_dir,
    };
    roots.admit(root.clone());
    Ok((roots, root, EntryBound::InNodeModules))
}

/// Removes `.` and resolves `..` lexically, as URL parsing does for a
/// specifier, so an entry path names the same file its URL does.
fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component),
        }
    }
    normalized
}

/// Whether `path`, as written, names a directory by its form: it ends in `/`,
/// or its last component is `.` or `..` (or it is empty).
fn names_directory(path: &Path) -> bool {
    let bytes = path.as_os_str().as_encoded_bytes();
    let last = bytes
        .rsplit(|byte| *byte == b'/')
        .next()
        .unwrap_or_default();
    matches!(last, b"" | b"." | b"..")
}
