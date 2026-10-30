//! The strict ESM resolver: turns an import specifier into the `file:` URL of
//! a module on disk.
//!
//! A pure module: no V8 or `deno_core` types. It reads the filesystem only
//! through [`SourceTree`] ([`DiskTree`] is the disk) and returns a
//! [`Resolution`] record rather than a bare URL: the URL, the [`Root`] the
//! module is inside and its [`Origin`], and the `package.json` files read to
//! get there. The module loader is a thin adapter over it, so static
//! imports, `import()`, `import.meta.resolve`, and the entry module all
//! resolve the same way.
//!
//! A module's origin is its root's, a function of its real path over the
//! [`Roots`] its graph has admitted: the project root, which
//! [`resolve_entry`] finds from the entry, and the real root of each
//! package a bare specifier resolved into, the innermost one by path
//! component winning, and none when a `node_modules` segment is below it.
//! The caller owns the roots and passes them into [`resolve`], which admits
//! nothing; a linked package's real path has no `node_modules` in it, so
//! only the admission says it's a package.
//!
//! What resolves today:
//!
//! - **Relative and absolute specifiers** (`./`, `../`, `/`, `file:`) parse as
//!   URLs against the importer, the web's algorithm. The extension is
//!   mandatory: nothing is probed, no extension is added, and a directory
//!   never resolves to an index file. The query and fragment survive
//!   resolution, so `./a.js?x` is a separate module instance from `./a.js`.
//!   They resolve only inside the importer's root, with its origin.
//! - **Bare specifiers** name a package (`svelte`, `@fuzdev/fuz_util/string.ts`;
//!   a scoped name takes two segments) and resolve through its
//!   `package.json` `"exports"`, Node's algorithm: exact subpaths, then
//!   single-`*` patterns by Node's ordering, conditions matched in the
//!   package's key order from the set [`CONDITIONS`], arrays as fallbacks,
//!   and every target checked to start with `./` and stay inside the package
//!   by its path.
//!   The importer's own package comes first (a self-reference, when its
//!   `package.json` has `"exports"` and its name matches), then
//!   `node_modules/<name>` in each directory from the importer's real
//!   location up to its project root, the first one that is a directory
//!   winning. A package with no `"exports"` is refused: `main` and `module`
//!   are never read. The package's real root is the result's, admitted by
//!   the caller; a self-reference's is the importer's.
//! - **The file must exist**, and resolves to its real path (symlinks
//!   followed), so a module's own imports resolve from where it really lives.
//! - **`.js`, `.mjs`, `.ts`, and `.mts` load**, as ES modules, the TypeScript
//!   stripped of its types in process ([`ModuleFormat`], read from the real
//!   file's name by [`module_format`]), inside `node_modules` too. A `.js`
//!   file must be under `"type": "module"`: the nearest `package.json` above
//!   it, the search ending at a `node_modules` directory, sets it, as Node
//!   reads it. The TypeScript that can't load that way is refused with its
//!   own reason: `.cts` (not an ES module), `.tsx` (JSX), declaration files
//!   (`.d.ts`, `.d.mts`, `.d.cts`), and Svelte modules (`.svelte.ts`,
//!   `.svelte.js`, and names like `.svelte.test.ts`). Every other extension
//!   is refused by name.
//!
//! Where it departs from Node, deliberately:
//!
//! - **Every module stays inside its root** (origin containment), checked
//!   on its real path after every symlink, before anything about it is
//!   read. A relative or absolute specifier resolves only inside the
//!   importer's root ([`ResolveErrorKind::OutsideRoot`]), never into a
//!   `node_modules` segment below it, by its real path or as written, since
//!   an in-tree checkout linked into `node_modules` has none in its real
//!   path ([`ResolveErrorKind::IntoNodeModules`]), and never into another
//!   admitted root nested in it ([`ResolveErrorKind::IntoPackageRoot`]): the
//!   project reaches a package, and a package another, only by name, through
//!   its `"exports"`. A package's `"exports"` target stays inside its real
//!   root by the same rules, so a symlinked target can't leave it, and a
//!   self-reference's stays inside both its package's directory, the one its
//!   `package.json` was found in rather than that file's real path, and the
//!   importer's root. An entry inside `node_modules` runs as the package it
//!   is in. Node allows every crossing, each of which bypasses `"exports"`.
//! - **A resolution names the file it found**: the [`FileId`] the lookup
//!   took, its device and inode numbers and birth time, read on the real
//!   path without following a symlink there, so a last component swapped
//!   for a symlink after the path was canonicalized reads as one and is
//!   refused. The module loader opens the path without blocking and loads
//!   only a file with that identity, read from the handle it checked, and
//!   every `package.json` read is checked against its own lookup the same
//!   way ([`SourceTree::read`]). A file remade where the filesystem records
//!   no birth time, or within the same tick of its clock as the original's
//!   birth, has only its device and inode to tell it apart, and ext4 reuses
//!   a removed file's inode. A directory on the path swapped between
//!   canonicalizing and reading the identity isn't caught: the identity is
//!   then the new file's. Node reads whatever is at the path.
//! - **The `node_modules` search is bounded by the project root, found
//!   first**: the first directory at or above the importer, outside
//!   `node_modules`, with a `package.json` that isn't a type marker. The walk
//!   for it runs before any `node_modules` is looked up, and the search then
//!   runs from the importer up to it, its own `node_modules` included, so a
//!   `node_modules` above the project is never consulted. A module with no
//!   project root gets no bare imports except a self-reference, which
//!   resolves through its own package's `"exports"` and stays inside its
//!   package's directory: any other bare specifier is
//!   refused ([`ResolveErrorKind::NoProjectRoot`]), where Node searches every
//!   ancestor. A marker is a JSON object whose keys
//!   are all `"type"` or `"sideEffects"`, such as a `{"type": "module"}` in
//!   `dist/esm/` (the empty object is one too); every other `package.json`
//!   ends the walk, named or not, so an unrecognized one fails closed. A
//!   project whose packages aren't installed fails, rather than resolving a
//!   parent directory's, and so does a package of an npm or yarn workspace
//!   that is installed only at the workspace's root. Inside `node_modules`
//!   the search continues past package roots, where npm hoists dependencies
//!   and where nested layouts (`node_modules/.deno/<pkg>@<version>/node_modules/`)
//!   put them.
//! - **A world-writable directory ends every walk up**, and no `package.json`
//!   or `node_modules` in it is read: not its `package.json` (as a project
//!   root, a `"type"` scope, or a self-reference's scope), and not its
//!   `node_modules`. A package the search finds through one, the
//!   `node_modules` it is installed in, its scope's directory, its own root,
//!   for a linked package the real directories holding it, or for a linked
//!   `node_modules` or scope directory its real parent, is refused
//!   ([`ResolveErrorKind::WorldWritablePackage`]), since any user can plant
//!   or replace a package there. Module files are outside the rule: one in a
//!   world-writable directory still loads by a relative specifier or as a
//!   package's `"exports"` target, though a walk up from it still stops
//!   there. World-writable is the other-write bit (`0o002`), whatever the
//!   sticky bit; a group-writable directory doesn't stop a walk. The
//!   importer's own directory counts, so a module directly in `/tmp` runs
//!   with relative imports only, and a `.js` file there has no `"type"`
//!   scope. Node reads them all, so a `package.json` or a `node_modules` any
//!   user can plant in `/tmp` would resolve for every module below it.
//! - **Every `package.json` the walk passes outside `node_modules` is read**,
//!   to tell a marker from a bound, so one that isn't valid (not JSON, a
//!   `"name"` or `"type"` that isn't a string, or a special file) fails the
//!   import, where Node never reads it.
//! - **A duplicated key keeps its last value**, as `JSON.parse` does, so
//!   `{"type": 5, "type": "module"}` reads as a module scope, where Node
//!   checks every occurrence and refuses it.
//! - **A `.js` file with no `package.json` above it is refused**, where Node
//!   guesses the format from the code; `.mjs` needs no `package.json`.
//! - **`.ts` inside `node_modules` is stripped** like any other, where Node
//!   refuses to strip there.
//! - **The conditions are only [`CONDITIONS`]**: `node`, `require`,
//!   `browser`, `svelte`, and the rest never match.
//! - **A pattern's match can't hide a `..` behind a tab or a line break**
//!   (`.\t.`), which URL parsing drops, where Node substitutes it unchecked
//!   and loads what it names outside the package.
//!
//! What fails, loudly, with the reason and a fix: `node:` (no Node compat),
//! `npm:` and `jsr:` (registry specifiers), `http:`, `https:`, and `data:`
//! (remote and inline; only files on disk load), any other non-`file:`
//! scheme, `#` package imports, a bare specifier other than a self-reference
//! with no project root above the importer, a bare Node built-in name (`fs`,
//! which is looked up as a package and not found), a module outside its
//! root, in `node_modules` below it, or in another root nested in it, a
//! `%2F`, `%5C`, or `%00`
//! in a resolved path, a `.js` file outside `"type": "module"`, a symlink loop
//! (named as one where the OS's `ELOOP` is known), a `package.json` over
//! [`MAX_PACKAGE_JSON_SIZE`], one whose `"name"` or `"type"` isn't a string,
//! or one that is a special file, and the empty specifier.

mod error;
mod exports;
mod find;
mod format;
mod package;
mod roots;
mod specifier;
#[cfg(test)]
pub(crate) mod test_tree;
#[cfg(test)]
mod tests;
mod tree;
mod walk;

pub use error::{PackageSearchBound, ResolveError, ResolveErrorKind, TargetFault};
pub use format::{ModuleFormat, module_format};
pub use roots::{ConsultedFile, FileRole, Origin, Root, Roots};
pub(crate) use tree::read_checked;
pub use tree::{DiskTree, Entry, FileId, MAX_PACKAGE_JSON_SIZE, SourceTree};

use std::path::{Component, Path, PathBuf};
use url::Url;

use find::find_module;
use package::{parse_package_name, resolve_package};
use specifier::{Request, parse_specifier, url_to_path};
use walk::{Consulted, Walk, find_bound, in_node_modules, read_package_json};

/// The conditions a package's `"exports"` match, in no order of their own:
/// the first key in the package's order that is one of these wins.
pub const CONDITIONS: [&str; 3] = ["gro", "import", "default"];

/// A resolved module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The module's `file:` URL: its real path, with the specifier's query and
    /// fragment re-attached.
    pub url: Url,
    /// The module file's identity when it was resolved, which the loader
    /// checks the file it opens against.
    pub file_id: FileId,
    /// The real directory of the root the module is inside: its importer's
    /// for a relative or absolute specifier or a self-reference, the
    /// package's for any other bare one.
    pub root: PathBuf,
    /// Where the module comes from, its root's origin.
    pub origin: Origin,
    /// The `package.json` files read to resolve it, at their real paths, each
    /// once per role. A relative specifier to a `.mjs` or TypeScript file
    /// reads none.
    pub files_consulted: Vec<ConsultedFile>,
}

/// Resolves `specifier`, imported by the module at `importer`, a real path's
/// URL inside one of `roots`, the roots its graph has admitted. The
/// importer's root is the one its path is inside ([`Roots::containing`]).
///
/// A relative or absolute specifier resolves only inside the importer's
/// root, never into a `node_modules` segment below it, by its real path or
/// as written, and never into another admitted root nested in it. A bare
/// one's real path stays inside its package's real root, and a
/// self-reference's inside its package's directory and the importer's root
/// both. The result names the root it is inside, which a caller admits for
/// a package; nothing is admitted here.
///
/// # Errors
///
/// Returns a [`ResolveError`] when the specifier names no loadable module
/// inside its root; its kind says why.
pub fn resolve(
    tree: &impl SourceTree,
    specifier: &str,
    importer: &Url,
    roots: &Roots,
) -> Result<Resolution, ResolveError> {
    let error = |kind| ResolveError {
        specifier: specifier.to_owned(),
        importer: importer.to_string(),
        kind,
    };
    let importer_path = importer.to_file_path().map_err(|()| {
        error(ResolveErrorKind::InvalidUrl(
            "the importer names no local path".to_owned(),
        ))
    })?;
    let importer_root = roots.containing(&importer_path).ok_or_else(|| {
        error(ResolveErrorKind::InvalidUrl(
            "the importer is inside no admitted root".to_owned(),
        ))
    })?;
    let mut consulted = Consulted::default();
    match parse_specifier(specifier, importer).map_err(error)? {
        Request::Url(url) => {
            let path = url_to_path(&url).map_err(error)?;
            // as written too: an in-tree checkout linked into node_modules has none in its real path
            if written_into_node_modules(&path, &importer_root.dir) {
                // a segment past where it leaves the root is no segment below the root
                if !path.starts_with(&importer_root.dir) {
                    return Err(error(ResolveErrorKind::OutsideRoot {
                        path,
                        root: importer_root.dir.clone(),
                        package: importer_root.installed_name().map(Box::from),
                    }));
                }
                return Err(error(ResolveErrorKind::IntoNodeModules {
                    path,
                    root: importer_root.dir.clone(),
                }));
            }
            let found = find_module(tree, &path, Some(specifier), &mut consulted, |real| {
                contain(
                    real,
                    &importer_root.dir,
                    importer_root.installed_name(),
                    roots,
                )
            })
            .map_err(error)?;
            to_resolution(found, &url, importer_root.clone(), consulted).map_err(error)
        }
        Request::Package(specifier) => {
            let package = resolve_package(
                tree,
                specifier,
                &importer_path,
                importer_root,
                roots,
                &mut consulted,
            )
            .map_err(error)?;
            let path = url_to_path(&package.url).map_err(error)?;
            let name = package.root.installed_name();
            let check = |real: &Path| {
                if let Some(scope_dir) = &package.scope_dir {
                    // the name is the importer root's, so only when the scope is that root
                    contain(
                        real,
                        scope_dir,
                        name.filter(|_| *scope_dir == package.root.dir),
                        roots,
                    )?;
                }
                contain(real, &package.root.dir, name, roots)
            };
            let found = match find_module(tree, &path, None, &mut consulted, check) {
                Ok(found) => found,
                Err(kind @ (ResolveErrorKind::Io { .. } | ResolveErrorKind::SymlinkLoop(_))) => {
                    return Err(error(kind));
                }
                // the error names the file, not the specifier
                Err(kind) => {
                    return Err(error(ResolveErrorKind::Target {
                        path,
                        error: Box::new(kind),
                    }));
                }
            };
            to_resolution(found, &package.url, package.root, consulted).map_err(error)
        }
    }
}

/// Resolves the entry module at `path`, relative to `cwd`, returning it and
/// the roots its graph starts with.
///
/// `cwd` is an absolute path, and an absolute `path` ignores it. The same
/// rules apply as to an import: the extension is mandatory and the file must
/// exist. It is a path, not a URL, so `%`, `?`, and `#` are file-name
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
    let specifier = path.display().to_string();
    let error = |kind| ResolveError {
        specifier: specifier.clone(),
        importer: cwd.display().to_string(),
        kind,
    };
    // `join` keeps an absolute `path` as is
    let path = normalize(&cwd.join(path));
    if !path.is_absolute() {
        return Err(error(ResolveErrorKind::InvalidUrl(
            "not an absolute path".to_owned(),
        )));
    }
    let mut consulted = Consulted::default();
    let (real_path, file_id) =
        find_module(tree, &path, Some(&specifier), &mut consulted, |_| Ok(())).map_err(error)?;
    let (roots, root) = entry_roots(tree, &real_path, &mut consulted).map_err(error)?;
    let url = Url::from_file_path(&real_path).map_err(|()| {
        error(ResolveErrorKind::InvalidUrl(
            "names no local path".to_owned(),
        ))
    })?;
    let resolution = Resolution {
        url,
        file_id,
        root: root.dir,
        origin: root.origin,
        files_consulted: consulted.0,
    };
    Ok((resolution, roots))
}

/// The roots a graph whose entry is the module at `real_path` starts with,
/// and the entry's own root among them (see [`resolve_entry`]).
fn entry_roots(
    tree: &impl SourceTree,
    real_path: &Path,
    consulted: &mut Consulted,
) -> Result<(Roots, Root), ResolveErrorKind> {
    let dir = real_path.parent().unwrap_or(real_path);
    let segments: Vec<usize> = real_path
        .components()
        .enumerate()
        .filter(|(_, component)| component.as_os_str() == "node_modules")
        .map(|(index, _)| index)
        .collect();
    let (Some(&first), Some(&last)) = (segments.first(), segments.last()) else {
        let project_dir = match find_bound(tree, dir, None, consulted)? {
            Walk::Found((bound_dir, _)) => bound_dir,
            Walk::Ended | Walk::Stopped(_) => dir.to_owned(),
        };
        let roots = Roots::new(project_dir);
        let root = roots.project().clone();
        return Ok((roots, root));
    };
    let project_dir: PathBuf = real_path.components().take(first).collect();
    let mut roots = Roots::new(project_dir);
    // the package's name: one segment after node_modules, or a scope and one
    let names: Vec<&str> = real_path
        .components()
        .skip(last + 1)
        .map(|component| component.as_os_str().to_str().unwrap_or("%"))
        .collect();
    let length = if names.first().is_some_and(|name| name.starts_with('@')) {
        2
    } else {
        1
    };
    let name = names.get(..length).map(|names| names.join("/"));
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
    Ok((roots, root))
}

/// Whether `path`, as written, has a `node_modules` segment past what it
/// shares with the root at `root`, its importer's.
fn written_into_node_modules(path: &Path, root: &Path) -> bool {
    let shared = path
        .components()
        .zip(root.components())
        .take_while(|(a, b)| a == b)
        .count();
    in_node_modules(&path.components().skip(shared).collect::<PathBuf>())
}

/// Checks that the real path `path` is inside the root at `dir`: under it,
/// with no `node_modules` segment below it, and in no admitted root nested
/// in it. `package` names the root's package when it is an installed one.
fn contain(
    path: &Path,
    dir: &Path,
    package: Option<&str>,
    roots: &Roots,
) -> Result<(), ResolveErrorKind> {
    let Ok(below) = path.strip_prefix(dir) else {
        return Err(ResolveErrorKind::OutsideRoot {
            path: path.to_owned(),
            root: dir.to_owned(),
            package: package.map(Box::from),
        });
    };
    if in_node_modules(below) {
        return Err(ResolveErrorKind::IntoNodeModules {
            path: path.to_owned(),
            root: dir.to_owned(),
        });
    }
    if let Some(inner) = roots.inner(dir, path) {
        return Err(ResolveErrorKind::IntoPackageRoot {
            path: path.to_owned(),
            root: Box::new(inner.clone()),
        });
    }
    Ok(())
}

/// The resolution of the module `find_module` found, its real path and its
/// identity, inside `root`, with `url`'s query and fragment re-attached.
fn to_resolution(
    (real_path, file_id): (PathBuf, FileId),
    url: &Url,
    root: Root,
    consulted: Consulted,
) -> Result<Resolution, ResolveErrorKind> {
    let mut resolved = Url::from_file_path(&real_path)
        .map_err(|()| ResolveErrorKind::InvalidUrl("names no local path".to_owned()))?;
    resolved.set_query(url.query());
    resolved.set_fragment(url.fragment());
    Ok(Resolution {
        url: resolved,
        file_id,
        root: root.dir,
        origin: root.origin,
        files_consulted: consulted.0,
    })
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
