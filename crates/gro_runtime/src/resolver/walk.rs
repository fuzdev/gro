//! The upward walks over `package.json` files: the package scope and the project root.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::error::ResolveErrorKind;
use super::roots::{ConsultedFile, FileRole};
use super::tree::{ELOOP, Entry, MAX_PACKAGE_JSON_SIZE, SourceTree};

/// The files a resolution read, each `(path, role)` once.
#[derive(Debug, Default)]
pub(super) struct Consulted(pub(super) Vec<ConsultedFile>);

impl Consulted {
    pub(super) fn add(&mut self, path: &Path, role: FileRole) {
        if !self
            .0
            .iter()
            .any(|file| file.path == path && file.role == role)
        {
            self.0.push(ConsultedFile {
                path: path.to_owned(),
                role,
            });
        }
    }
}

/// A parsed `package.json`.
pub(super) struct PackageJson {
    /// The directory it was looked up in, which a symlinked `package.json`
    /// isn't in.
    pub(super) dir: PathBuf,
    /// Its real path.
    pub(super) path: PathBuf,
    /// Its fields, in the file's key order.
    fields: serde_json::Map<String, Value>,
}

impl PackageJson {
    /// The field `key`, unless absent or not a string.
    pub(super) fn string(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    /// Its `"exports"`, unless absent or `null`.
    pub(super) fn exports(&self) -> Option<&Value> {
        self.fields
            .get("exports")
            .filter(|exports| !exports.is_null())
    }

    /// Whether it only marks its directory's module type: every key is
    /// `"type"` or `"sideEffects"`, which the empty object satisfies.
    fn is_type_marker(&self) -> bool {
        self.fields
            .keys()
            .all(|key| matches!(key.as_str(), "type" | "sideEffects"))
    }
}

/// Looks up `path`, naming a symlink loop as one.
pub(super) fn lookup(
    tree: &impl SourceTree,
    path: &Path,
) -> Result<Option<Entry>, ResolveErrorKind> {
    tree.lookup(path).map_err(|error| {
        if ELOOP.is_some() && error.raw_os_error() == ELOOP {
            ResolveErrorKind::SymlinkLoop(path.to_owned())
        } else {
            ResolveErrorKind::Io {
                path: path.to_owned(),
                error: error.to_string(),
            }
        }
    })
}

/// Reads `dir`'s `package.json`, when it has one. A leading byte order mark
/// is skipped, as Node skips it. A `"name"` or `"type"` that isn't a string
/// makes it invalid, as it does in Node, and so does a `package.json` that
/// is a special file (a FIFO, a socket, or a device) when it is looked up,
/// which isn't opened; one swapped in after the lookup is opened without
/// blocking and refused by its identity ([`read_checked`](super::read_checked)).
pub(super) fn read_package_json(
    tree: &impl SourceTree,
    dir: &Path,
    role: FileRole,
    consulted: &mut Consulted,
) -> Result<Option<PackageJson>, ResolveErrorKind> {
    let path = dir.join("package.json");
    let (path, id) = match lookup(tree, &path)? {
        Some(Entry::File { path, id }) => (path, id),
        Some(Entry::Other) => {
            return Err(ResolveErrorKind::InvalidPackageJson {
                path,
                reason: "it is not a regular file (a FIFO, a socket, or a device)".to_owned(),
            });
        }
        Some(Entry::Dir(_)) | None => return Ok(None),
    };
    let invalid = |reason: String| ResolveErrorKind::InvalidPackageJson {
        path: path.clone(),
        reason,
    };
    let bytes = match tree.read(&path, id) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::FileTooLarge => {
            return Err(invalid(format!(
                "it is larger than {} MiB, the most a package.json may be",
                MAX_PACKAGE_JSON_SIZE / (1024 * 1024)
            )));
        }
        Err(error) => {
            return Err(ResolveErrorKind::Io {
                path: path.clone(),
                error: error.to_string(),
            });
        }
    };
    consulted.add(&path, role);
    let json = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let fields = match serde_json::from_slice(json) {
        Ok(Value::Object(fields)) => fields,
        Ok(_) => return Err(invalid("it is not a JSON object".to_owned())),
        Err(error) => return Err(invalid(error.to_string())),
    };
    for key in ["name", "type"] {
        if fields.get(key).is_some_and(|value| !value.is_string()) {
            return Err(invalid(format!("its \"{key}\" is not a string")));
        }
    }
    Ok(Some(PackageJson {
        dir: dir.to_owned(),
        path,
        fields,
    }))
}

/// How an upward walk from a directory ended.
pub(super) enum Walk<T> {
    /// It found what it walked for.
    Found(T),
    /// It reached the filesystem's root, or the end its rule sets, without
    /// finding it.
    Ended,
    /// It stopped at this world-writable directory without finding it,
    /// reading nothing in it.
    Stopped(PathBuf),
}

impl<T> Walk<T> {
    /// What it found, if anything.
    pub(super) const fn found(&self) -> Option<&T> {
        match self {
            Self::Found(found) => Some(found),
            Self::Ended | Self::Stopped(_) => None,
        }
    }
}

/// Whether `dir` is world-writable, which ends every upward walk before
/// anything in it is read.
pub(super) fn is_world_writable(
    tree: &impl SourceTree,
    dir: &Path,
) -> Result<bool, ResolveErrorKind> {
    tree.is_world_writable(dir)
        .map_err(|error| ResolveErrorKind::Io {
            path: dir.to_owned(),
            error: error.to_string(),
        })
}

/// The package scope of `dir`: the nearest `package.json` at or above it,
/// the search ending at a `node_modules` directory, as Node's is, and at a
/// world-writable directory, which Node's isn't.
pub(super) fn package_scope(
    tree: &impl SourceTree,
    dir: &Path,
    consulted: &mut Consulted,
) -> Result<Walk<PackageJson>, ResolveErrorKind> {
    for dir in dir.ancestors() {
        if dir.file_name().is_some_and(|name| name == "node_modules") {
            break;
        }
        if is_world_writable(tree, dir)? {
            return Ok(Walk::Stopped(dir.to_owned()));
        }
        if let Some(package_json) = read_package_json(tree, dir, FileRole::PackageScope, consulted)?
        {
            return Ok(Walk::Found(package_json));
        }
    }
    Ok(Walk::Ended)
}

/// The project root above `dir`, which bounds its `node_modules` search: the
/// first directory at or above it, outside `node_modules`, whose
/// `package.json` isn't a type marker. The walk stops at a world-writable
/// directory. `scope` is `dir`'s package scope, already read, so it isn't
/// read again. Returns the root's directory and its `package.json`.
pub(super) fn find_bound(
    tree: &impl SourceTree,
    dir: &Path,
    scope: Option<&PackageJson>,
    consulted: &mut Consulted,
) -> Result<Walk<(PathBuf, PathBuf)>, ResolveErrorKind> {
    for dir in dir.ancestors() {
        // checked inside node_modules too, since the search looks there
        if is_world_writable(tree, dir)? {
            return Ok(Walk::Stopped(dir.to_owned()));
        }
        if in_node_modules(dir) {
            continue;
        }
        // any package.json but a type marker ends the walk, the scope's read once
        let package_json = match scope {
            Some(scope) if scope.dir == dir => {
                consulted.add(&scope.path, FileRole::SearchBound);
                (!scope.is_type_marker()).then(|| scope.path.clone())
            }
            _ => read_package_json(tree, dir, FileRole::SearchBound, consulted)?
                .filter(|package_json| !package_json.is_type_marker())
                .map(|package_json| package_json.path),
        };
        if let Some(package_json) = package_json {
            return Ok(Walk::Found((dir.to_owned(), package_json)));
        }
    }
    Ok(Walk::Ended)
}

/// Whether `path` has a `node_modules` component.
pub(super) fn in_node_modules(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "node_modules")
}
