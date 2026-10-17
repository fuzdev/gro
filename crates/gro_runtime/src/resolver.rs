//! The strict ESM resolver: turns an import specifier into the `file:` URL of
//! a module on disk.
//!
//! A pure module: no V8 or `deno_core` types. It reads the filesystem only
//! through [`SourceTree`] ([`DiskTree`] is the disk) and returns a
//! [`Resolution`] record rather than a bare URL. The module loader is a thin
//! adapter over it, so static imports, `import()`, and the entry module all
//! resolve the same way.
//!
//! What resolves today:
//!
//! - **Relative and absolute specifiers** (`./`, `../`, `/`, `file:`) parse as
//!   URLs against the importer, the web's algorithm. The extension is
//!   mandatory: nothing is probed, no extension is added, and a directory
//!   never resolves to an index file. The query and fragment survive
//!   resolution, so `./a.js?x` is a separate module instance from `./a.js`.
//! - **The file must exist**, and resolves to its real path (symlinks
//!   followed), so a module's own relative imports resolve from where it
//!   really lives.
//! - **Only `.js` and `.mjs` load**, as ES modules. TypeScript and every other
//!   extension are refused by name.
//!
//! What fails, loudly, with the reason and a fix: `node:` (no Node compat),
//! `npm:`, `jsr:`, `http:`, `https:`, and `data:` (registry and remote
//! specifiers; only files on disk load), any other non-`file:` scheme, `#`
//! package imports, and bare specifiers, since package resolution isn't
//! supported yet.

use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use url::Url;

/// A read-only view of the files modules resolve against.
pub trait SourceTree {
    /// Looks up `path`, an absolute path. Returns `Ok(None)` when nothing
    /// exists there, following symlinks.
    ///
    /// # Errors
    ///
    /// Returns any other failure to read the tree, such as a permission error.
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>>;
}

/// What [`SourceTree::lookup`] found at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A file, at its real path (every symlink resolved).
    File(PathBuf),
    /// A directory or any other entry that isn't a regular file.
    NotAFile,
}

/// The process's filesystem.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiskTree;

impl SourceTree for DiskTree {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        let real_path = match std::fs::canonicalize(path) {
            Ok(real_path) => real_path,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        Ok(Some(if std::fs::metadata(&real_path)?.is_file() {
            Entry::File(real_path)
        } else {
            Entry::NotAFile
        }))
    }
}

/// A resolved module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The module's `file:` URL: its real path, with the specifier's query and
    /// fragment re-attached.
    pub url: Url,
    /// Where the module comes from.
    pub origin: Origin,
    /// The metadata files read to resolve it, such as a `package.json`. Empty
    /// until package resolution exists, since a relative or absolute specifier
    /// reads none.
    pub files_consulted: Vec<PathBuf>,
}

/// Where a resolved module comes from, the input a grant decision reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The project's own files, reached by a relative or absolute specifier.
    // TODO: `Package` (name and version) and `Builtin` join with package
    // resolution and the builtin modules
    Project,
}

/// A specifier that didn't resolve.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot resolve '{specifier}' from {importer}: {kind}")]
pub struct ResolveError {
    /// The specifier as written.
    pub specifier: String,
    /// The importing module's URL, or the working directory for an entry.
    pub importer: String,
    /// Why it failed.
    pub kind: ResolveErrorKind,
}

/// Why a specifier didn't resolve. Each message names the specifier, so it
/// reads on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveErrorKind {
    /// A `node:` specifier.
    NodeBuiltin,
    /// A registry specifier: `npm:`, `jsr:`.
    Registry,
    /// A remote or inline specifier: `http:`, `https:`, `data:`.
    Remote,
    /// Any other scheme that isn't `file:`.
    UnsupportedScheme(String),
    /// A `#` package import.
    PackageImport,
    /// A bare specifier, which names a package.
    Bare,
    /// The empty string.
    Empty,
    /// A relative or absolute specifier that isn't a valid URL, or a `file:`
    /// URL that names no local path.
    InvalidUrl(String),
    /// The path has no extension. `suggestion` names the first sibling that
    /// exists with one, for the message only (resolution never probes).
    NoExtension {
        /// The specifier with the extension of an existing sibling.
        suggestion: Option<String>,
    },
    /// Nothing exists at the path. `ts_sibling` is set when a `.js`
    /// specifier's `.ts` sibling exists.
    NotFound {
        /// The specifier with `.ts` in place of `.js`.
        ts_sibling: Option<String>,
    },
    /// The path exists but isn't a regular file: a directory or a special
    /// file.
    NotAFile,
    /// A TypeScript file, which can't run yet.
    TypeScript,
    /// An extension that isn't loaded as a module.
    UnsupportedExtension(String),
    /// Reading the tree failed.
    Io(String),
}

impl ResolveErrorKind {
    /// Whether the specifier must change for the import to resolve, the
    /// common case, as opposed to a failure reading the files.
    pub const fn is_specifier_error(&self) -> bool {
        !matches!(self, Self::Io(_))
    }
}

/// The extensions that load as ES modules.
const MODULE_EXTENSIONS: [&str; 2] = ["js", "mjs"];

/// The extensions a missing-extension message suggests, in order.
const SUGGESTED_EXTENSIONS: [&str; 3] = ["ts", "js", "mjs"];

impl fmt::Display for ResolveErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // the specifier is quoted by `ResolveError`, so these restate it only where the spec's wording does
        match self {
            Self::NodeBuiltin => f.write_str(
                "a node: specifier names a Node built-in, and there is no Node compat. \
                 Fix: import the fuz_host equivalent, or run this under Node",
            ),
            Self::Registry => f.write_str(
                "it is a registry specifier; only files on disk load. \
                 Fix: install the package into node_modules and import it by name",
            ),
            Self::Remote => f.write_str(
                "it is a remote or inline specifier; only files on disk load. \
                 Fix: save the module into the project and import it by a relative path",
            ),
            Self::UnsupportedScheme(scheme) => write!(
                f,
                "the scheme '{scheme}:' is not supported; only files on disk (file: URLs) load"
            ),
            Self::PackageImport => f.write_str(
                "it is a # package import; package.json \"imports\" are not supported yet. \
                 Fix: import the file by a relative path",
            ),
            Self::Bare => f.write_str(
                "a bare specifier names a package, and package resolution is not supported yet. \
                 Fix: import the file by a relative path",
            ),
            Self::Empty => f.write_str("the specifier is empty"),
            Self::InvalidUrl(reason) => write!(f, "not a valid file URL: {reason}"),
            Self::NoExtension { suggestion } => {
                f.write_str(
                    "the path has no extension; extensions and directory indexes are never probed",
                )?;
                if let Some(suggestion) = suggestion {
                    write!(f, ". Did you mean '{suggestion}'?")?;
                }
                Ok(())
            }
            Self::NotFound { ts_sibling } => {
                f.write_str("no such file")?;
                if let Some(ts_sibling) = ts_sibling {
                    write!(f, ", but '{ts_sibling}' exists; write the real extension")?;
                }
                Ok(())
            }
            Self::NotAFile => f.write_str(
                "it is not a regular file (a directory or a special file); name a module file",
            ),
            Self::TypeScript => {
                f.write_str("TypeScript is not supported yet; .js and .mjs modules load")
            }
            Self::UnsupportedExtension(extension) => write!(
                f,
                "the extension '.{extension}' does not load as a module; .js and .mjs modules load"
            ),
            Self::Io(error) => write!(f, "failed to read the file: {error}"),
        }
    }
}

/// Resolves `specifier`, imported by the module at `importer`.
///
/// # Errors
///
/// Returns a [`ResolveError`] when the specifier names no loadable module;
/// its kind says why.
pub fn resolve(
    tree: &impl SourceTree,
    specifier: &str,
    importer: &Url,
) -> Result<Resolution, ResolveError> {
    let error = |kind| ResolveError {
        specifier: specifier.to_owned(),
        importer: importer.to_string(),
        kind,
    };
    let url = parse_specifier(specifier, importer).map_err(error)?;
    finalize(tree, specifier, &url).map_err(error)
}

/// Resolves the entry module at `path`, relative to `cwd`.
///
/// `cwd` is an absolute path, and an absolute `path` ignores it. The same
/// rules apply as to an import: the extension is mandatory and the file must
/// exist. Errors name the path as given, imported from `cwd`.
///
/// # Errors
///
/// Returns a [`ResolveError`] when the path names no loadable module.
pub fn resolve_entry(
    tree: &impl SourceTree,
    path: &Path,
    cwd: &Path,
) -> Result<Resolution, ResolveError> {
    let specifier = path.display().to_string();
    let error = |kind| ResolveError {
        specifier: specifier.clone(),
        importer: cwd.display().to_string(),
        kind,
    };
    // `join` keeps an absolute `path` as is
    let url = Url::from_file_path(normalize(&cwd.join(path))).map_err(|()| {
        error(ResolveErrorKind::InvalidUrl(
            "not an absolute path".to_owned(),
        ))
    })?;
    finalize(tree, &specifier, &url).map_err(error)
}

/// Turns a specifier into a URL, refusing every form but relative and
/// absolute `file:` ones.
fn parse_specifier(specifier: &str, importer: &Url) -> Result<Url, ResolveErrorKind> {
    if specifier.starts_with("./") || specifier.starts_with("../") || specifier.starts_with('/') {
        return importer
            .join(specifier)
            .map_err(|error| ResolveErrorKind::InvalidUrl(error.to_string()));
    }
    if let Ok(url) = Url::parse(specifier) {
        return match url.scheme() {
            "file" => Ok(url),
            "node" => Err(ResolveErrorKind::NodeBuiltin),
            "npm" | "jsr" => Err(ResolveErrorKind::Registry),
            "http" | "https" | "data" => Err(ResolveErrorKind::Remote),
            scheme => Err(ResolveErrorKind::UnsupportedScheme(scheme.to_owned())),
        };
    }
    if specifier.is_empty() {
        return Err(ResolveErrorKind::Empty);
    }
    if specifier.starts_with('#') {
        return Err(ResolveErrorKind::PackageImport);
    }
    Err(ResolveErrorKind::Bare)
}

/// Checks the extension, finds the real file, and re-attaches the query and
/// fragment.
fn finalize(
    tree: &impl SourceTree,
    specifier: &str,
    url: &Url,
) -> Result<Resolution, ResolveErrorKind> {
    let path = url
        .to_file_path()
        .map_err(|()| ResolveErrorKind::InvalidUrl("names no local path".to_owned()))?;
    let io_error = |error: io::Error| ResolveErrorKind::Io(error.to_string());

    if path.extension().is_none() || url.path().ends_with('/') {
        let mut suggestion = None;
        if !url.path().ends_with('/') {
            for extension in SUGGESTED_EXTENSIONS {
                if let Some(Entry::File(_)) = tree
                    .lookup(&path.with_extension(extension))
                    .map_err(io_error)?
                {
                    suggestion = Some(with_extension(specifier, extension));
                    break;
                }
            }
        }
        return Err(ResolveErrorKind::NoExtension { suggestion });
    }

    let real_path = match tree.lookup(&path).map_err(io_error)? {
        Some(Entry::File(real_path)) => real_path,
        Some(Entry::NotAFile) => return Err(ResolveErrorKind::NotAFile),
        None => {
            let ts_sibling = if path.extension().is_some_and(|extension| extension == "js")
                && let Some(Entry::File(_)) =
                    tree.lookup(&path.with_extension("ts")).map_err(io_error)?
            {
                Some(replace_extension(specifier, "ts"))
            } else {
                None
            };
            return Err(ResolveErrorKind::NotFound { ts_sibling });
        }
    };

    // format by the real file's extension, which a symlink can change
    let extension = real_path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !MODULE_EXTENSIONS.contains(&extension.as_str()) {
        return Err(match extension.as_str() {
            "ts" | "mts" | "cts" | "tsx" => ResolveErrorKind::TypeScript,
            _ => ResolveErrorKind::UnsupportedExtension(extension),
        });
    }

    let mut resolved = Url::from_file_path(&real_path)
        .map_err(|()| ResolveErrorKind::InvalidUrl("names no local path".to_owned()))?;
    resolved.set_query(url.query());
    resolved.set_fragment(url.fragment());
    Ok(Resolution {
        url: resolved,
        origin: Origin::Project,
        files_consulted: Vec::new(),
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

/// Splits a specifier into its path and its query-and-fragment suffix.
fn split_suffix(specifier: &str) -> (&str, &str) {
    specifier
        .find(['?', '#'])
        .map_or((specifier, ""), |index| specifier.split_at(index))
}

/// The specifier with `.{extension}` appended to its path.
fn with_extension(specifier: &str, extension: &str) -> String {
    let (path, suffix) = split_suffix(specifier);
    format!("{path}.{extension}{suffix}")
}

/// The specifier with its path's last extension replaced by `extension`.
fn replace_extension(specifier: &str, extension: &str) -> String {
    let (path, suffix) = split_suffix(specifier);
    let stem = path.rsplit_once('.').map_or(path, |(stem, _)| stem);
    format!("{stem}.{extension}{suffix}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// An in-memory tree: files by path, symlinks as a map to their target.
    #[derive(Default)]
    struct MemoryTree {
        files: Vec<&'static str>,
        dirs: Vec<&'static str>,
        links: BTreeMap<&'static str, &'static str>,
    }

    impl SourceTree for MemoryTree {
        fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
            let path = path.to_str().unwrap();
            let path = self.links.get(path).copied().unwrap_or(path);
            Ok(if self.files.contains(&path) {
                Some(Entry::File(PathBuf::from(path)))
            } else if self.dirs.contains(&path) {
                Some(Entry::NotAFile)
            } else {
                None
            })
        }
    }

    fn tree() -> MemoryTree {
        MemoryTree {
            files: vec![
                "/p/main.js",
                "/p/a.js",
                "/p/b.mjs",
                "/p/lib/c.js",
                "/p/typed.ts",
                "/p/only_ts.ts",
                "/p/data.json",
                "/p/schema.ts",
                "/real/linked.js",
                "/abs.js",
            ],
            dirs: vec!["/p/lib", "/p/dir.js"],
            links: BTreeMap::from([("/p/link.js", "/real/linked.js")]),
        }
    }

    fn importer() -> Url {
        Url::parse("file:///p/main.js").unwrap()
    }

    fn ok(specifier: &str) -> String {
        resolve(&tree(), specifier, &importer())
            .unwrap_or_else(|error| panic!("{error}"))
            .url
            .to_string()
    }

    fn kind(specifier: &str) -> ResolveErrorKind {
        resolve(&tree(), specifier, &importer()).unwrap_err().kind
    }

    #[test]
    fn relative() {
        assert_eq!(ok("./a.js"), "file:///p/a.js");
        assert_eq!(ok("./b.mjs"), "file:///p/b.mjs");
        assert_eq!(ok("./lib/c.js"), "file:///p/lib/c.js");
        assert_eq!(ok("./lib/../a.js"), "file:///p/a.js");
    }

    #[test]
    fn parent() {
        let importer = Url::parse("file:///p/lib/c.js").unwrap();
        let resolution = resolve(&tree(), "../a.js", &importer).unwrap();
        assert_eq!(resolution.url.as_str(), "file:///p/a.js");
        // `..` past the root stays at the root, as URL parsing does
        let resolution = resolve(&tree(), "../../../abs.js", &importer).unwrap();
        assert_eq!(resolution.url.as_str(), "file:///abs.js");
    }

    #[test]
    fn absolute() {
        assert_eq!(ok("/abs.js"), "file:///abs.js");
        assert_eq!(ok("file:///abs.js"), "file:///abs.js");
    }

    #[test]
    fn record() {
        let resolution = resolve(&tree(), "./a.js", &importer()).unwrap();
        assert_eq!(resolution.origin, Origin::Project);
        assert!(resolution.files_consulted.is_empty());
    }

    #[test]
    fn query_and_fragment_are_kept() {
        assert_eq!(ok("./a.js?v=1"), "file:///p/a.js?v=1");
        assert_eq!(ok("./a.js#x"), "file:///p/a.js#x");
        assert_eq!(ok("./a.js?v=1#x"), "file:///p/a.js?v=1#x");
    }

    #[test]
    fn symlinks_resolve_to_the_real_path() {
        assert_eq!(ok("./link.js"), "file:///real/linked.js");
        assert_eq!(ok("./link.js?q"), "file:///real/linked.js?q");
    }

    #[test]
    fn missing_extension_is_refused() {
        assert_eq!(
            kind("./a"),
            ResolveErrorKind::NoExtension {
                suggestion: Some("./a.js".to_owned())
            }
        );
        // the suggestion prefers `.ts` and keeps the query
        assert_eq!(
            kind("./schema?x"),
            ResolveErrorKind::NoExtension {
                suggestion: Some("./schema.ts?x".to_owned())
            }
        );
        assert_eq!(
            kind("./nothing"),
            ResolveErrorKind::NoExtension { suggestion: None }
        );
        // a directory never resolves to an index
        assert_eq!(
            kind("./lib"),
            ResolveErrorKind::NoExtension { suggestion: None }
        );
        assert_eq!(
            kind("./lib/"),
            ResolveErrorKind::NoExtension { suggestion: None }
        );
    }

    #[test]
    fn missing_file() {
        assert_eq!(
            kind("./missing.js"),
            ResolveErrorKind::NotFound { ts_sibling: None }
        );
        assert_eq!(
            kind("./only_ts.js"),
            ResolveErrorKind::NotFound {
                ts_sibling: Some("./only_ts.ts".to_owned())
            }
        );
        assert_eq!(kind("./dir.js"), ResolveErrorKind::NotAFile);
    }

    #[test]
    fn extensions() {
        assert_eq!(kind("./typed.ts"), ResolveErrorKind::TypeScript);
        assert_eq!(
            kind("./data.json"),
            ResolveErrorKind::UnsupportedExtension("json".to_owned())
        );
    }

    #[test]
    fn schemes() {
        assert_eq!(kind("node:fs"), ResolveErrorKind::NodeBuiltin);
        for specifier in ["npm:svelte", "jsr:@std/toml@^1"] {
            assert_eq!(kind(specifier), ResolveErrorKind::Registry, "{specifier}");
        }
        for specifier in [
            "http://example.com/a.js",
            "https://example.com/a.js",
            "data:text/javascript,export default 1",
        ] {
            assert_eq!(kind(specifier), ResolveErrorKind::Remote, "{specifier}");
        }
        assert_eq!(
            kind("ext:deno_web/00_url.js"),
            ResolveErrorKind::UnsupportedScheme("ext".to_owned())
        );
        assert_eq!(
            kind("blob:abc"),
            ResolveErrorKind::UnsupportedScheme("blob".to_owned())
        );
    }

    #[test]
    fn bare_and_package_imports() {
        assert_eq!(kind("svelte"), ResolveErrorKind::Bare);
        assert_eq!(kind("@fuzdev/gro/a.js"), ResolveErrorKind::Bare);
        assert_eq!(kind("a.js"), ResolveErrorKind::Bare);
        assert_eq!(kind("#lib/a.js"), ResolveErrorKind::PackageImport);
        assert_eq!(kind(""), ResolveErrorKind::Empty);
    }

    #[test]
    fn entry() {
        let cwd = Path::new("/p");
        let entry = |path| resolve_entry(&tree(), Path::new(path), cwd);
        assert_eq!(entry("main.js").unwrap().url.as_str(), "file:///p/main.js");
        assert_eq!(
            entry("lib/c.js").unwrap().url.as_str(),
            "file:///p/lib/c.js"
        );
        assert_eq!(entry("../abs.js").unwrap().url.as_str(), "file:///abs.js");
        assert_eq!(
            entry("./lib/../a.js").unwrap().url.as_str(),
            "file:///p/a.js"
        );
        assert_eq!(entry("/abs.js").unwrap().url.as_str(), "file:///abs.js");
        assert_eq!(
            entry("typed.ts").unwrap_err().kind,
            ResolveErrorKind::TypeScript
        );
        let error = entry("missing.js").unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::NotFound { ts_sibling: None });
        assert_eq!(
            error.to_string(),
            "cannot resolve 'missing.js' from /p: no such file"
        );
        // a path, not a URL: `?` and `#` are file name characters
        assert_eq!(
            entry("a.js?x").unwrap_err().kind,
            ResolveErrorKind::NotFound { ts_sibling: None }
        );
    }

    #[test]
    fn messages() {
        let message = |specifier| {
            resolve(&tree(), specifier, &importer())
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            message("node:fs"),
            "cannot resolve 'node:fs' from file:///p/main.js: a node: specifier names a Node \
             built-in, and there is no Node compat. Fix: import the fuz_host equivalent, or run \
             this under Node"
        );
        assert!(message("https://example.com/a.js").contains("only files on disk load"));
        assert!(!message("https://example.com/a.js").contains("node_modules"));
        assert_eq!(
            message(""),
            "cannot resolve '' from file:///p/main.js: the specifier is empty"
        );
        assert!(message("./dir.js").contains("not a regular file"));
        assert_eq!(
            message("./only_ts.js"),
            "cannot resolve './only_ts.js' from file:///p/main.js: no such file, but \
             './only_ts.ts' exists; write the real extension"
        );
        assert!(message("svelte").contains("package resolution is not supported yet"));
        assert!(message("./typed.ts").contains("TypeScript is not supported yet"));
    }
}
