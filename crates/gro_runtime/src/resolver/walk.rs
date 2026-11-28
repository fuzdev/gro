//! The upward walks over `package.json` files: the package scope and the project root.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::{Map, Value};

use super::error::ResolveErrorKind;
use super::tree::{Entry, MAX_PACKAGE_JSON_SIZE, ReadRefusal, SourceTree};

/// A `package.json` resolution read, and the fields it read of it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConsultedFile {
    /// Its real path.
    pub path: PathBuf,
    /// The fields its resolutions read, each once.
    pub fields: PackageFields,
}

/// A field of a `package.json` that resolution reads, or for
/// [`PackageField::Keys`], its set of keys. Ordered by name, as a dump
/// writes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PackageField {
    /// `"exports"`: a package's, or a self-reference's.
    Exports,
    /// `"imports"`: a `#` specifier's.
    Imports,
    /// Its keys, which say whether it is a type marker the walk for the
    /// project root passes or the root it ends at: any key but `"type"` and
    /// `"sideEffects"` makes it the root.
    Keys,
    /// `"name"`: a self-reference's.
    Name,
    /// `"type"`: a `.js` module's.
    Type,
    /// `"version"`: a package's origin's.
    Version,
}

impl PackageField {
    /// Every field, in order.
    pub const ALL: [Self; 6] = [
        Self::Exports,
        Self::Imports,
        Self::Keys,
        Self::Name,
        Self::Type,
        Self::Version,
    ];

    /// Its bit in a [`PackageFields`].
    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of [`PackageField`]s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageFields(u8);

impl PackageFields {
    /// The set of `fields`.
    pub fn of(fields: impl IntoIterator<Item = PackageField>) -> Self {
        fields
            .into_iter()
            .fold(Self::default(), |set, field| Self(set.0 | field.bit()))
    }

    /// Whether `field` is in it.
    pub const fn contains(self, field: PackageField) -> bool {
        self.0 & field.bit() != 0
    }

    /// The fields of both.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Its fields, in order.
    pub fn iter(self) -> impl Iterator<Item = PackageField> {
        PackageField::ALL
            .into_iter()
            .filter(move |field| self.contains(*field))
    }
}

/// The `package.json` files a resolution read, each once, with the fields
/// it read of each.
#[derive(Debug, Default)]
pub(super) struct Consulted(pub(super) Vec<ConsultedFile>);

impl Consulted {
    /// Records that `field` of the `package.json` at `path` was read.
    fn add(&mut self, path: &Path, field: PackageField) {
        let fields = PackageFields::of([field]);
        if let Some(file) = self.0.iter_mut().find(|file| file.path == path) {
            file.fields = file.fields.union(fields);
        } else {
            self.0.push(ConsultedFile {
                path: path.to_owned(),
                fields,
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
    /// Its fields, in the file's key order, shared with the run's other
    /// reads of the same file ([`super::ParsedPackages`]).
    fields: Arc<Map<String, Value>>,
}

// each field a resolution reads is read through the method that records it,
// so the record is what decided the resolution
impl PackageJson {
    /// Its `"name"`, unless absent.
    pub(super) fn name(&self, consulted: &mut Consulted) -> Option<&str> {
        self.string(PackageField::Name, "name", consulted)
    }

    /// Whether its `"type"` is `"module"`.
    pub(super) fn is_module_type(&self, consulted: &mut Consulted) -> bool {
        self.string(PackageField::Type, "type", consulted) == Some("module")
    }

    /// Its `"version"`, unless absent or not a string.
    pub(super) fn version(&self, consulted: &mut Consulted) -> Option<&str> {
        self.string(PackageField::Version, "version", consulted)
    }

    /// Its `"exports"`, unless absent or `null`.
    pub(super) fn exports(&self, consulted: &mut Consulted) -> Option<&Value> {
        consulted.add(&self.path, PackageField::Exports);
        self.fields
            .get("exports")
            .filter(|exports| !exports.is_null())
    }

    /// Its `"imports"`, unless absent or not an object, which Node reads as
    /// none.
    pub(super) fn imports(&self, consulted: &mut Consulted) -> Option<&Map<String, Value>> {
        consulted.add(&self.path, PackageField::Imports);
        self.fields.get("imports").and_then(Value::as_object)
    }

    /// The field `key`, unless absent or not an object. No resolution reads
    /// one, so it records nothing: a command's reads before it starts call it
    /// ([`super::project`]).
    pub(super) fn object(&self, key: &str) -> Option<&Map<String, Value>> {
        self.fields.get(key).and_then(Value::as_object)
    }

    /// Whether it only marks its directory's module type: every key is
    /// `"type"` or `"sideEffects"`, which the empty object satisfies.
    pub(super) fn is_type_marker(&self, consulted: &mut Consulted) -> bool {
        consulted.add(&self.path, PackageField::Keys);
        self.fields
            .keys()
            .all(|key| matches!(key.as_str(), "type" | "sideEffects"))
    }

    /// The field `key`, `field`, unless absent or not a string.
    fn string(&self, field: PackageField, key: &str, consulted: &mut Consulted) -> Option<&str> {
        consulted.add(&self.path, field);
        self.fields.get(key).and_then(Value::as_str)
    }
}

/// Looks up `path`, naming a symlink loop as one.
pub(super) fn lookup(
    tree: &impl SourceTree,
    path: &Path,
) -> Result<Option<Entry>, ResolveErrorKind> {
    tree.lookup(path).map_err(|error| {
        // TODO: io::ErrorKind::FilesystemLoop once it's stable
        if error.raw_os_error() == Some(libc::ELOOP) {
            ResolveErrorKind::SymlinkLoop(path.to_owned())
        } else {
            read_error(path, &error)
        }
    })
}

/// The failure of a read at `path`: a refusal the tree made on purpose by
/// its variant, any other by its kind and message.
fn read_error(path: &Path, error: &io::Error) -> ResolveErrorKind {
    match ReadRefusal::of(error) {
        Some(ReadRefusal::ReplacedSinceFirstRead) => {
            ResolveErrorKind::ReadReplacedSinceFirstRead(path.to_owned())
        }
        Some(ReadRefusal::ReplacedSinceLookup) => {
            ResolveErrorKind::ReadReplacedSinceLookup(path.to_owned())
        }
        Some(ReadRefusal::OutsideBox(_)) => ResolveErrorKind::ReadOutsideBox(path.to_owned()),
        Some(ReadRefusal::OutsideGraph(_)) => ResolveErrorKind::ReadOutsideGraph(path.to_owned()),
        None => ResolveErrorKind::Io {
            path: path.to_owned(),
            kind: error.kind(),
            message: error.to_string(),
        },
    }
}

/// Reads `dir`'s `package.json`, when it has one, recording none of its
/// fields: the caller records each it reads ([`Consulted`]). A leading byte
/// order mark is skipped, as Node skips it. A `"name"` or `"type"` that isn't a string
/// makes it invalid, as it does in Node, and so does a `package.json` that
/// is a special file (a FIFO, a socket, or a device) when it is looked up,
/// which isn't opened; one swapped in after the lookup is opened without
/// blocking and refused by its identity ([`read_checked`](super::read_checked)).
pub(super) fn read_package_json(
    tree: &impl SourceTree,
    dir: &Path,
) -> Result<Option<PackageJson>, ResolveErrorKind> {
    let path = dir.join("package.json");
    let (path, id, stamp) = match lookup(tree, &path)? {
        Some(Entry::File { path, id, stamp }) => (path, id, stamp),
        Some(Entry::Other) => {
            return Err(ResolveErrorKind::InvalidPackageJson {
                path,
                reason: "it is not a regular file (a FIFO, a socket, or a device)".to_owned(),
            });
        }
        Some(Entry::Dir(_)) | None => {
            tree.note_absent(&path);
            return Ok(None);
        }
    };
    let parsed = tree.parsed();
    // parsed before as the file it is now: checked as a read is, then served
    if let Some(fields) = parsed.and_then(|parsed| parsed.get(&path, id, stamp)) {
        tree.check_read(&path)
            .map_err(|error| read_error(&path, &error))?;
        return Ok(Some(PackageJson {
            dir: dir.to_owned(),
            path,
            fields,
        }));
    }
    let invalid = |reason: String| ResolveErrorKind::InvalidPackageJson {
        path: path.clone(),
        reason,
    };
    // taken before the read, so a rewrite during it can't share the stamp
    let read_at = SystemTime::now();
    let bytes = match tree.read(&path, id) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::FileTooLarge => {
            return Err(invalid(format!(
                "it is larger than {} MiB, the most a package.json may be",
                MAX_PACKAGE_JSON_SIZE / (1024 * 1024)
            )));
        }
        Err(error) => return Err(read_error(&path, &error)),
    };
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
    let fields = Arc::new(fields);
    if let Some(parsed) = parsed {
        parsed.insert(&path, id, stamp, read_at, Arc::clone(&fields));
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
    /// It ended without finding it.
    NotFound {
        /// The world-writable directory it stopped at, reading nothing in
        /// it; unset when it reached the filesystem's root, or the end its
        /// rule sets.
        stopped_at: Option<PathBuf>,
    },
}

impl<T> Walk<T> {
    /// What it found, if anything.
    pub(super) const fn found(&self) -> Option<&T> {
        match self {
            Self::Found(found) => Some(found),
            Self::NotFound { .. } => None,
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
        .map_err(|error| read_error(dir, &error))
}

/// The package scope of `dir`: the nearest `package.json` at or above it,
/// the search ending at a `node_modules` directory, as Node's is, and at a
/// world-writable directory, which Node's isn't.
pub(super) fn package_scope(
    tree: &impl SourceTree,
    dir: &Path,
) -> Result<Walk<PackageJson>, ResolveErrorKind> {
    for dir in dir.ancestors() {
        if dir.file_name().is_some_and(|name| name == "node_modules") {
            break;
        }
        if is_world_writable(tree, dir)? {
            return Ok(Walk::NotFound {
                stopped_at: Some(dir.to_owned()),
            });
        }
        if let Some(package_json) = read_package_json(tree, dir)? {
            return Ok(Walk::Found(package_json));
        }
    }
    Ok(Walk::NotFound { stopped_at: None })
}

/// The project root above `dir`, which bounds its `node_modules` search: the
/// first directory at or above it, outside `node_modules`, whose
/// `package.json` isn't a type marker; or, for a `dir` inside a
/// `node_modules`, the directory holding its outermost `node_modules`,
/// whatever is there, its `package.json` never read, so a module of a
/// package resolves its own imports within the `node_modules` it was
/// installed into, whether that is the project's or another checkout's a
/// symlink leads to. The walk stops at a world-writable directory. `scope`
/// is `dir`'s package scope, already read, so it isn't read again. Returns
/// the root's directory and the `package.json` that bounds it, none for a
/// `node_modules`'s holder.
pub(super) fn find_bound(
    tree: &impl SourceTree,
    dir: &Path,
    scope: Option<&PackageJson>,
    consulted: &mut Consulted,
) -> Result<Walk<(PathBuf, Option<PathBuf>)>, ResolveErrorKind> {
    let inside = in_node_modules(dir);
    for dir in dir.ancestors() {
        // checked inside node_modules too, since the search looks there
        if is_world_writable(tree, dir)? {
            return Ok(Walk::NotFound {
                stopped_at: Some(dir.to_owned()),
            });
        }
        if in_node_modules(dir) {
            continue;
        }
        // the first directory outside node_modules holds the outermost one
        if inside {
            return Ok(Walk::Found((dir.to_owned(), None)));
        }
        // any package.json but a type marker ends the walk, the scope's read once
        let package_json = match scope {
            Some(scope) if scope.dir == dir => {
                (!scope.is_type_marker(consulted)).then(|| scope.path.clone())
            }
            _ => read_package_json(tree, dir)?
                .filter(|package_json| !package_json.is_type_marker(consulted))
                .map(|package_json| package_json.path),
        };
        if let Some(package_json) = package_json {
            return Ok(Walk::Found((dir.to_owned(), Some(package_json))));
        }
    }
    Ok(Walk::NotFound { stopped_at: None })
}

/// Whether `path` has a `node_modules` component.
pub(super) fn in_node_modules(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "node_modules")
}
