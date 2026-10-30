//! Resolving a bare specifier through the package it names.

use std::path::{Path, PathBuf};

use url::Url;

use super::error::{PackageSearchBound, ResolveErrorKind};
use super::exports::resolve_exports;
use super::roots::{FileRole, Origin, Root, Roots};
use super::tree::{Entry, SourceTree};
use super::walk::{
    Consulted, Walk, find_bound, in_node_modules, is_world_writable, lookup, package_scope,
    read_package_json,
};

/// Splits a bare specifier into its package name and its subpath (`.` or
/// `./…`), as Node does, refusing a malformed name. Node accepts empty, `.`,
/// and `..` segments in a scoped name; they're refused here, since they name
/// a directory other than the package's.
pub(super) fn parse_package_name(specifier: &str) -> Result<(&str, String), ResolveErrorKind> {
    let mut end = specifier.find('/');
    if specifier.starts_with('@') {
        let Some(scope_end) = end else {
            return Err(ResolveErrorKind::InvalidPackageName);
        };
        end = specifier[scope_end + 1..]
            .find('/')
            .map(|index| scope_end + 1 + index);
    }
    let (name, rest) = specifier.split_at(end.unwrap_or(specifier.len()));
    if name.starts_with('.')
        || name.contains(['%', '\\'])
        || name
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".."))
    {
        return Err(ResolveErrorKind::InvalidPackageName);
    }
    Ok((name, format!(".{rest}")))
}

/// What a bare specifier names: the URL its package's `"exports"` give, not
/// yet checked to exist, and the root it must stay inside.
pub(super) struct Package {
    /// The `"exports"` target's URL.
    pub(super) url: Url,
    /// The package's root, or for a self-reference the importer's.
    pub(super) root: Root,
    /// For a self-reference, its package's real directory, which the module
    /// must stay inside too.
    pub(super) scope_dir: Option<PathBuf>,
}

/// Resolves a bare specifier, imported by the module at `importer_path`
/// inside `importer_root`, to its package's `"exports"` target and root. A
/// package root already in `roots` keeps the origin it was admitted with.
pub(super) fn resolve_package(
    tree: &impl SourceTree,
    specifier: &str,
    importer_path: &Path,
    importer_root: &Root,
    roots: &Roots,
    consulted: &mut Consulted,
) -> Result<Package, ResolveErrorKind> {
    let (name, subpath) = parse_package_name(specifier)?;
    let importer_dir = importer_path.parent().unwrap_or(importer_path);

    // a self-reference: the importer's own package, by its name, resolved from
    // the directory the package.json was found in (the importer's real one),
    // not the file's real path, which a symlinked package.json moves
    let scope = package_scope(tree, importer_dir, consulted)?;
    let scope = scope.found();
    if let Some(scope) = scope
        && let Some(exports) = scope.exports()
        && scope.string("name") == Some(name)
    {
        consulted.add(&scope.path, FileRole::Exports);
        let url = resolve_exports(&scope.dir, &scope.path, exports, &subpath)?;
        return Ok(Package {
            url,
            root: importer_root.clone(),
            scope_dir: Some(scope.dir.clone()),
        });
    }

    // the bound first, so no node_modules is consulted outside the project
    let (bound_dir, bound_package_json) = match find_bound(tree, importer_dir, scope, consulted)? {
        Walk::Found(bound) => bound,
        Walk::Ended => {
            return Err(ResolveErrorKind::NoProjectRoot {
                dir: importer_dir.to_owned(),
                stopped_at: None,
            });
        }
        Walk::Stopped(stopped_at) => {
            return Err(ResolveErrorKind::NoProjectRoot {
                dir: importer_dir.to_owned(),
                stopped_at: Some(stopped_at),
            });
        }
    };
    let mut found = None;
    for dir in importer_dir.ancestors() {
        let node_modules = dir.join("node_modules");
        let package_dir = node_modules.join(name);
        if let Some(Entry::Dir(root)) = lookup(tree, &package_dir)? {
            found = Some((node_modules, package_dir, root));
            break;
        }
        if dir == bound_dir {
            break;
        }
    }
    let Some((node_modules, package_dir, root)) = found else {
        return Err(package_not_found(tree, name, bound_dir, bound_package_json));
    };
    // nothing in a world-writable directory is read: not the node_modules the
    // package is found in, its scope's directory, or its root, nor, for a
    // linked root, the real directories holding it (where pnpm's and Deno's
    // stores install it), outermost first
    let scope_dir = name
        .split_once('/')
        .map(|(scope, _)| node_modules.join(scope));
    // a linked root is held by every directory between it and the first one
    // already checked: the real node_modules it was found through (pnpm's and
    // Deno's stores there), or the importer's directories up to the project
    // root (a dependency's store entry beside its importer's included); one
    // reaching neither is held by as many directories as its name has segments
    let real_node_modules = match lookup(tree, &node_modules)? {
        Some(Entry::Dir(path)) => path,
        _ => node_modules.clone(),
    };
    // find_bound checked these
    let walked: Vec<&Path> = importer_dir
        .ancestors()
        .take_while(|dir| dir.starts_with(&bound_dir))
        .collect();
    let between = root
        .ancestors()
        .skip(1)
        .position(|dir| dir == real_node_modules || walked.contains(&dir));
    let mut holders: Vec<&Path> = root
        .ancestors()
        .skip(1)
        .take(between.unwrap_or_else(|| name.split('/').count()))
        .collect();
    holders.reverse();
    // a linked node_modules or scope directory is held by its real parent, as
    // a linked root is, unless that's already checked: the importer's
    // directories, or for a scope directory the real node_modules
    let real_scope_dir = match &scope_dir {
        Some(scope_dir) => match lookup(tree, scope_dir)? {
            Some(Entry::Dir(path)) => Some(path),
            _ => None,
        },
        None => None,
    };
    let linked_holders = [Some(&real_node_modules), real_scope_dir.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|dir| dir.parent())
        .filter(|dir| *dir != real_node_modules && !walked.contains(dir));
    let mut checked: Vec<&Path> = Vec::new();
    for dir in [Some(node_modules.as_path()), scope_dir.as_deref()]
        .into_iter()
        .flatten()
        .chain(linked_holders)
        .chain(holders)
        .chain([root.as_path()])
    {
        if checked.contains(&dir) {
            continue;
        }
        checked.push(dir);
        if is_world_writable(tree, dir)? {
            return Err(ResolveErrorKind::WorldWritablePackage {
                name: name.to_owned(),
                dir: dir.to_owned(),
            });
        }
    }
    let package_json = read_package_json(tree, &root, FileRole::Exports, consulted)?
        .ok_or(ResolveErrorKind::NoPackageJson(package_dir))?;
    let exports = package_json
        .exports()
        .ok_or_else(|| ResolveErrorKind::NoExports(package_json.path.clone()))?;
    let url = resolve_exports(&root, &package_json.path, exports, &subpath)?;
    // one directory under two names keeps its first admission's origin
    let origin = roots.get(&root).map_or_else(
        || Origin::Package {
            name: name.to_owned(),
            version: package_json.string("version").map(str::to_owned),
            linked: !in_node_modules(&root),
        },
        |admitted| admitted.origin.clone(),
    );
    Ok(Package {
        url,
        root: Root { dir: root, origin },
        scope_dir: None,
    })
}

/// The error for a package the search didn't find up to the project root at
/// `dir`, whose `package.json` is `package_json`. The nearest `package.json`
/// above that directory is named as unsearched, the walk to it stopping at a
/// world-writable directory as every walk does.
fn package_not_found(
    tree: &impl SourceTree,
    name: &str,
    dir: PathBuf,
    package_json: PathBuf,
) -> ResolveErrorKind {
    // only for the message, so a failure to look is no `package.json`, and a
    // failure to read a directory's mode stops the walk
    let above = dir
        .ancestors()
        .skip(1)
        .take_while(|dir| matches!(tree.is_world_writable(dir), Ok(false)))
        .find_map(|dir| match tree.lookup(&dir.join("package.json")) {
            Ok(Some(Entry::File { path, .. })) => Some(path),
            _ => None,
        });
    ResolveErrorKind::PackageNotFound {
        name: name.to_owned(),
        bound: Box::new(PackageSearchBound {
            dir,
            package_json,
            above,
        }),
    }
}
