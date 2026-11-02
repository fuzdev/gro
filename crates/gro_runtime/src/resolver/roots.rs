//! The roots a module graph has admitted, the origin a module takes from its
//! root, and the containment check that keeps a module inside its root.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Bound;
use std::path::{Path, PathBuf};

use super::error::ResolveErrorKind;
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
    /// a real path inside the root at `dir` with no `node_modules` segment
    /// below `dir`, which `contain` checks first.
    pub(super) fn inner(&self, dir: &Path, path: &Path) -> Option<&Root> {
        // the packages under `dir` are one range in component order; those
        // under `dir/node_modules` can't hold `path`, which `contain` refuses
        // first when it has a `node_modules` segment below `dir`, so when the
        // range starts there it's probed again past them
        let first = self
            .packages
            .range::<Path, _>((Bound::Excluded(dir), Bound::Unbounded))
            .next()
            .map(|(package_dir, _)| package_dir.as_path());
        let nested_package = first
            .and_then(|first| first.strip_prefix(dir).ok())
            .is_some_and(|below| {
                below
                    .components()
                    .next()
                    .is_none_or(|component| component.as_os_str() != "node_modules")
                    || {
                        // the least component past `node_modules`, so every path under it sorts before
                        let past_node_modules = dir.join("node_modules\0");
                        self.packages
                            .range::<Path, _>((
                                Bound::Excluded(past_node_modules.as_path()),
                                Bound::Unbounded,
                            ))
                            .next()
                            .is_some_and(|(package_dir, _)| package_dir.starts_with(dir))
                    }
            });
        let nested_project = || self.project.dir != dir && self.project.dir.starts_with(dir);
        if !nested_package && !nested_project() {
            return None;
        }
        // the ancestors strictly below `dir`, one per component below it
        let depth = path.strip_prefix(dir).ok()?.components().count();
        path.ancestors()
            .take(depth)
            .find_map(|ancestor| self.get(ancestor))
    }

    /// Admits `root`, unless its directory is admitted already.
    pub(crate) fn admit(&mut self, root: Root) {
        if self.get(&root.dir).is_none() {
            self.packages.insert(root.dir.clone(), root);
        }
    }
}

/// Whether `path`, as written, has a `node_modules` segment past what it
/// shares with the root at `root`, its importer's.
pub(super) fn written_into_node_modules(path: &Path, root: &Path) -> bool {
    let mut root = root.components();
    path.components()
        .skip_while(|component| root.next() == Some(*component))
        .any(|component| component.as_os_str() == "node_modules")
}

/// Checks that the real path `path` is inside the root at `dir`: under it,
/// with no `node_modules` segment below it, and in no admitted root nested
/// in it. `package` names the root's package when it is an installed one.
pub(super) fn contain(
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
