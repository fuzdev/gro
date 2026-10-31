//! The roots a module graph has admitted, and the origin a module takes from its root.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use super::walk::in_node_modules;

/// Where a resolved module comes from, the input a grant decision reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The project's own files: the entry, and what its graph reaches by
    /// relative and absolute specifiers and by self-reference.
    Project,
    /// An installed package, reached by its name, and what it reaches by
    /// relative specifiers and by self-reference.
    Package {
        /// The name it was imported by, its directory under `node_modules`.
        name: String,
        /// Its `package.json` `"version"`, when that is a string.
        version: Option<String>,
        /// Whether it is linked: its real root isn't inside `node_modules`,
        /// as with a symlinked path dependency.
        linked: bool,
    },
    // TODO: `Builtin` joins with the builtin modules
}

/// A directory whose modules share an origin: the project's root, or a
/// package's real root. A relative or absolute specifier resolves only
/// inside its importer's root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// Its real path.
    pub dir: PathBuf,
    /// The origin of the modules inside it.
    pub origin: Origin,
}

impl Root {
    /// Whether the real path `path` is inside this root: under its
    /// directory, with no `node_modules` segment below it. The segments
    /// above it don't count, since where the root itself lives is the
    /// installer's or the user's choice.
    pub fn contains(&self, path: &Path) -> bool {
        path.strip_prefix(&self.dir)
            .is_ok_and(|below| !in_node_modules(below))
    }

    /// Its package's name, when it is an installed package's root, not
    /// linked.
    pub(super) fn installed_name(&self) -> Option<&str> {
        match &self.origin {
            Origin::Package {
                name,
                linked: false,
                ..
            } => Some(name),
            Origin::Package { linked: true, .. } | Origin::Project => None,
        }
    }
}

impl fmt::Display for Root {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.origin {
            Origin::Project => write!(f, "the project at {}", self.dir.display()),
            Origin::Package { name, .. } => {
                write!(f, "package '{name}' at {}", self.dir.display())
            }
        }
    }
}

/// The roots a module graph has admitted: the project's, and the root of
/// each package a bare specifier resolved into.
///
/// They're keyed by real directory, so one directory reached under two
/// names keeps its first admission's origin. A module's root is the
/// innermost one its real path is inside. Lookups walk a path's ancestors,
/// each found in an ordered map, so their cost grows with the path's depth,
/// and only logarithmically with the number of roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    /// The project's root, the entry's project root.
    project: Root,
    /// The packages' roots, by real directory, which `Path`'s ordering
    /// compares by component. None is at the project's directory.
    packages: BTreeMap<PathBuf, Root>,
}

impl Roots {
    /// The roots of a graph that has admitted only the project at
    /// `project_dir`, a real path.
    pub(crate) const fn new(project_dir: PathBuf) -> Self {
        Self {
            project: Root {
                dir: project_dir,
                origin: Origin::Project,
            },
            packages: BTreeMap::new(),
        }
    }

    /// The project's root.
    pub const fn project(&self) -> &Root {
        &self.project
    }

    /// Every admitted root, the project's first, then the packages' by
    /// directory.
    pub fn iter(&self) -> impl Iterator<Item = &Root> {
        std::iter::once(&self.project).chain(self.packages.values())
    }

    /// The root admitted at the real directory `dir`, if any.
    pub fn get(&self, dir: &Path) -> Option<&Root> {
        if self.project.dir == dir {
            Some(&self.project)
        } else {
            self.packages.get(dir)
        }
    }

    /// The root the real path `path` is inside: the innermost admitted root
    /// above it, by path component, unless a `node_modules` segment is below
    /// that root, when it is in none.
    pub fn containing(&self, path: &Path) -> Option<&Root> {
        path.ancestors()
            .find_map(|dir| self.get(dir))
            .filter(|root| root.contains(path))
    }

    /// The innermost admitted root strictly inside `dir` that holds `path`,
    /// a real path inside the root at `dir`.
    pub(super) fn inner(&self, dir: &Path, path: &Path) -> Option<&Root> {
        path.ancestors()
            .take_while(|ancestor| *ancestor != dir && ancestor.starts_with(dir))
            .find_map(|ancestor| self.get(ancestor))
    }

    /// Admits `root`, unless its directory is admitted already.
    pub(crate) fn admit(&mut self, root: Root) {
        if self.get(&root.dir).is_none() {
            self.packages.insert(root.dir.clone(), root);
        }
    }
}

/// A metadata file resolution read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsultedFile {
    /// Its real path.
    pub path: PathBuf,
    /// What it was read for.
    pub role: FileRole,
}

/// What a consulted file was read for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileRole {
    /// A package's `package.json`, for its `"exports"`.
    Exports,
    /// The nearest `package.json` above a module, for its `"type"`, or above
    /// an importer, for a self-reference's `"name"` and `"exports"`.
    PackageScope,
    /// A `package.json` on the walk for the project root that bounds the
    /// `node_modules` search, or for the entry's project root, read to tell
    /// a type marker, which the walk passes, from the root it ends at, which
    /// is always read.
    SearchBound,
    /// The `package.json` of the package an entry inside `node_modules` is
    /// in, for its `"version"`, its origin's.
    Version,
}
