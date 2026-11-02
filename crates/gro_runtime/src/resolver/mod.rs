//! The strict ESM resolver: turns an import specifier into the `file:` URL of
//! a module on disk.
//!
//! The rules and every departure from Node are `src/docs/resolver.md`'s;
//! this covers what a reader of the code needs beside them.
//!
//! A pure module: no V8 or `deno_core` types. It reads the filesystem only
//! through [`SourceTree`] ([`DiskTree`] is the disk) and returns a
//! [`Resolution`] record rather than a bare URL: the URL, the [`Root`] the
//! module is inside and its [`Origin`], the [`FileId`] it found, and the
//! `package.json` files read to get there. The module loader is a thin
//! adapter over it, so static imports, `import()`, `import.meta.resolve`, and
//! the entry module ([`resolve_entry`]) all resolve the same way. The caller
//! owns the graph's [`Roots`] and passes them into `resolve_from` with the
//! importer's root it recorded, which admits nothing.
//!
//! Details the published doc leaves to the code:
//!
//! - **A relative or absolute specifier's query and fragment survive
//!   resolution**, so `./a.js?x` is a separate module instance from
//!   `./a.js`.
//! - **`"exports"` is Node's `PACKAGE_EXPORTS_RESOLVE`** (`exports.rs`): exact
//!   subpaths, then single-`*` patterns by Node's ordering, conditions
//!   matched in the package's key order from [`CONDITIONS`], arrays as
//!   fallbacks, and every target checked to start with `./` and stay inside
//!   the package by its path.
//! - **The package search** tries the importer's own package first (a
//!   self-reference, when its `package.json` has `"exports"` and its name
//!   matches; its root is the importer's), then `node_modules/<name>` in each
//!   directory from the importer's real location up to its project root, the
//!   first that is a directory winning. Inside `node_modules` it continues
//!   past package roots, where npm hoists dependencies and where nested
//!   layouts (`node_modules/.deno/<pkg>@<version>/node_modules/`) put them.
//! - **A `.js` file's `"type"`** is the nearest `package.json` above it, the
//!   search ending at a `node_modules` directory, as Node reads it; the
//!   format is read from the real file's name ([`module_format`],
//!   [`ModuleFormat`]).
//! - **Also refused**, each with its reason: a bare Node built-in name
//!   (`fs`), looked up as a package and not found; a `%2F`, `%5C`, or `%00`
//!   in a resolved path; a symlink loop, named as one where the OS's `ELOOP`
//!   is known; a `package.json` over [`MAX_PACKAGE_JSON_SIZE`] or failing
//!   [`SourceTree::read`]'s identity check; and the empty specifier.
//!
//! Each departure from Node refuses with its own kind: containment with
//! [`ResolveErrorKind::OutsideRoot`], [`ResolveErrorKind::IntoNodeModules`],
//! and [`ResolveErrorKind::IntoPackageRoot`], the project root's bound with
//! [`ResolveErrorKind::NoProjectRoot`], and a world-writable directory with
//! [`ResolveErrorKind::WorldWritablePackage`].

mod entry;
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

pub use entry::{EntryBound, resolve_entry, resolve_entry_with_bound};
pub use error::{PackageSearchBound, ResolveError, ResolveErrorKind, TargetFault, UrlFault};
pub use format::{ModuleFormat, module_format};
pub use roots::{Origin, Root, Roots};
pub(crate) use tree::read_checked;
pub use tree::{
    DiskTree, Entry, FileId, MAX_MODULE_SIZE, MAX_PACKAGE_JSON_SIZE, ReadRefusal, SourceTree,
};

use std::path::{Path, PathBuf};
use url::Url;

use find::find_module;
use package::resolve_package;
use roots::{contain, written_into_node_modules};
use specifier::{Request, parse_specifier, url_to_path};
use walk::Consulted;
pub use walk::{ConsultedFile, FileRole};

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
    /// The root the module is inside, whose origin is the module's: its
    /// importer's for a relative or absolute specifier or a self-reference,
    /// the package's for any other bare one.
    pub root: Root,
    /// The `package.json` files read to resolve it, at their real paths, each
    /// once per role. A relative specifier to a `.mjs` or TypeScript file
    /// reads none.
    pub files_consulted: Vec<ConsultedFile>,
}

/// Resolves as `resolve_from` does, the importer's root found as the one
/// its path is inside ([`Roots::containing`]).
///
/// # Errors
///
/// As `resolve_from`, and [`ResolveErrorKind::ImporterOutsideRoots`] when
/// the importer is inside none of `roots`.
#[cfg(test)]
pub(crate) fn resolve(
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
    let importer_path = importer_path(importer).map_err(error)?;
    let importer_root = roots
        .containing(&importer_path)
        .ok_or_else(|| error(ResolveErrorKind::ImporterOutsideRoots))?;
    resolve_from(tree, specifier, importer, importer_root, roots)
}

/// Resolves `specifier`, imported by the module at `importer`, a real path's
/// URL inside `importer_root`, one of `roots`, the roots its graph has
/// admitted.
///
/// A relative or absolute specifier resolves only inside the importer's
/// root, never into a `node_modules` segment below it, by its real path or
/// as written, and never into another admitted root nested in it. A bare
/// one's real path stays inside its package's real root, and a
/// self-reference's inside its package's directory and the importer's root
/// both. The result names the root it is inside, which a caller admits for
/// a package; nothing is admitted here.
///
/// `importer_root` must be the root [`Roots::containing`] finds for the
/// importer's path, since it decides containment: a caller that recorded
/// the importer's root when it resolved it passes that, skipping the walk.
/// A debug build checks it.
///
/// # Errors
///
/// Returns a [`ResolveError`] when the specifier names no loadable module
/// inside its root; its kind says why.
pub(crate) fn resolve_from(
    tree: &impl SourceTree,
    specifier: &str,
    importer: &Url,
    importer_root: &Root,
    roots: &Roots,
) -> Result<Resolution, ResolveError> {
    let error = |kind| ResolveError {
        specifier: specifier.to_owned(),
        importer: importer.to_string(),
        kind,
    };
    debug_assert_eq!(
        importer
            .to_file_path()
            .ok()
            .and_then(|path| roots.containing(&path)),
        Some(importer_root),
        "the importer's root is the one its path is inside",
    );
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
            // only a bare specifier walks from the importer's path
            let importer_path = importer_path(importer).map_err(error)?;
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
                // the tree's failure, not the target's
                Err(kind) if !kind.is_refusal() => return Err(error(kind)),
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

/// The real path of the module at `importer`.
fn importer_path(importer: &Url) -> Result<PathBuf, ResolveErrorKind> {
    importer
        .to_file_path()
        .map_err(|()| ResolveErrorKind::InvalidUrl(UrlFault::ImporterNoLocalPath))
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
        .map_err(|()| ResolveErrorKind::InvalidUrl(UrlFault::NoLocalPath))?;
    resolved.set_query(url.query());
    resolved.set_fragment(url.fragment());
    Ok(Resolution {
        url: resolved,
        file_id,
        root,
        files_consulted: consulted.0,
    })
}
