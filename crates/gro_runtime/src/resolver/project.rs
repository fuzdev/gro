//! A project's `package.json` and its installed packages, as a command reads
//! them before it starts: the packages the project declares, and the version
//! of one it installed, through the reads and the package search resolution
//! uses ([`read_package_json`], [`find_installed`]), so a project reads one
//! way whoever asks.
//!
//! Nothing here is recorded as a resolution's dependency
//! ([`Consulted`]): a command's reads happen once,
//! before any module is resolved.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::error::ResolveErrorKind;
use super::package::{find_installed, parse_package_name};
use super::tree::SourceTree;
use super::walk::{Consulted, read_package_json};

/// The fields of a `package.json` whose keys are the packages it depends on
/// directly, as Gro's tasks read them; `peerDependencies` declares what a
/// package works beside, not what it runs, so it isn't one.
const DIRECT_DEPENDENCY_FIELDS: [&str; 2] = ["dependencies", "devDependencies"];

/// A project's `package.json`, as a command reads it before it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPackage {
    /// Its real path.
    pub path: PathBuf,
    /// The packages it declares in `dependencies` or `devDependencies`, by
    /// name, each whose spec is set: a value JS reads as true, as Gro's
    /// `package_json_has_dependency` tests it, so an empty spec declares
    /// nothing.
    pub declared: BTreeSet<String>,
}

impl ProjectPackage {
    /// Whether it declares the package `name` in `dependencies` or
    /// `devDependencies`.
    pub fn declares(&self, name: &str) -> bool {
        self.declared.contains(name)
    }
}

/// Reads the `package.json` of the project whose root is `dir`, a real path,
/// `None` where there's none.
///
/// It's read as resolution reads it: a leading byte order mark skipped, a
/// special file or one over the size limit refused, and a `"name"` or
/// `"type"` that isn't a string making it invalid.
///
/// # Errors
///
/// Returns the failure to read or parse it, as resolution names it.
pub fn read_project_package(
    tree: &impl SourceTree,
    dir: &Path,
) -> Result<Option<ProjectPackage>, ResolveErrorKind> {
    let Some(package_json) = read_package_json(tree, dir)? else {
        return Ok(None);
    };
    let declared = DIRECT_DEPENDENCY_FIELDS
        .into_iter()
        .filter_map(|field| package_json.object(field))
        .flat_map(|deps| deps.iter())
        .filter(|(_, spec)| is_truthy(spec))
        .map(|(name, _)| name.clone())
        .collect();
    Ok(Some(ProjectPackage {
        path: package_json.path,
        declared,
    }))
}

/// A package a project installed, found in its root's `node_modules`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPackage {
    /// Its real root.
    pub root: PathBuf,
    /// Its `package.json`'s `"version"`, unless absent or not a string.
    pub version: Option<String>,
}

/// Reads the package `name` installed in the `node_modules` of the project
/// whose root is `project_dir`, `None` when it isn't installed.
///
/// `project_dir` is a real path holding a `package.json`. The package is
/// found and read as resolution finds and reads one the project imports:
/// nothing is read in a world-writable directory, a linked package's real
/// directories included.
///
/// # Errors
///
/// Returns [`ResolveErrorKind::InvalidPackageName`] for a `name` that isn't
/// a package's alone (a subpath after it, say), a world-writable directory
/// holding the package, a package with no `package.json`, and any failure to
/// read one.
pub fn read_installed_package(
    tree: &impl SourceTree,
    project_dir: &Path,
    name: &str,
) -> Result<Option<InstalledPackage>, ResolveErrorKind> {
    let (parsed, subpath) = parse_package_name(name)?;
    if parsed != name || subpath != "." {
        return Err(ResolveErrorKind::InvalidPackageName);
    }
    let Some((package_dir, root)) = find_installed(tree, name, project_dir)? else {
        return Ok(None);
    };
    let package_json =
        read_package_json(tree, &root)?.ok_or(ResolveErrorKind::NoPackageJson(package_dir))?;
    let version = package_json
        .version(&mut Consulted::default())
        .map(str::to_owned);
    Ok(Some(InstalledPackage { root, version }))
}

/// Whether JS reads `value` as true: anything but `false`, `null`, `0`, and
/// the empty string (`NaN` has no JSON form).
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}
