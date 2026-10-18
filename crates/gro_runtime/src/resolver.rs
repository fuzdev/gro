//! The strict ESM resolver: turns an import specifier into the `file:` URL of
//! a module on disk.
//!
//! A pure module: no V8 or `deno_core` types. It reads the filesystem only
//! through [`SourceTree`] ([`DiskTree`] is the disk) and returns a
//! [`Resolution`] record rather than a bare URL: the URL, the module's
//! [`Origin`], and the `package.json` files read to get there. The module
//! loader is a thin adapter over it, so static imports, `import()`, and the
//! entry module all resolve the same way. The importer's origin is an input,
//! not read from its path, since a linked package's real path has no
//! `node_modules` in it.
//!
//! What resolves today:
//!
//! - **Relative and absolute specifiers** (`./`, `../`, `/`, `file:`) parse as
//!   URLs against the importer, the web's algorithm. The extension is
//!   mandatory: nothing is probed, no extension is added, and a directory
//!   never resolves to an index file. The query and fragment survive
//!   resolution, so `./a.js?x` is a separate module instance from `./a.js`.
//!   They resolve with the importer's origin.
//! - **Bare specifiers** name a package (`svelte`, `@fuzdev/fuz_util/string.ts`;
//!   a scoped name takes two segments) and resolve through its
//!   `package.json` `"exports"`, Node's algorithm: exact subpaths, then
//!   single-`*` patterns by Node's ordering, conditions matched in the
//!   package's key order from the set [`CONDITIONS`], arrays as fallbacks,
//!   and every target checked to start with `./` and stay inside the package.
//!   The importer's own package comes first (a self-reference, when its
//!   `package.json` has `"exports"` and its name matches), then
//!   `node_modules/<name>` in each directory up from the importer's real
//!   location, the first one that is a directory winning. A package with no
//!   `"exports"` is refused: `main` and `module` are never read.
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
//! - **The `node_modules` search ends at the project root**: the first
//!   directory outside `node_modules` with a `package.json` that isn't a
//!   type marker, after checking that directory's own `node_modules`. A
//!   marker is a JSON object whose keys are all `"type"` or `"sideEffects"`,
//!   such as a `{"type": "module"}` in `dist/esm/` (the empty object is one
//!   too); every other `package.json` ends the search, named or not, so an
//!   unrecognized one fails closed. A project whose packages aren't installed
//!   fails, rather than resolving a parent directory's, and so does a
//!   package of an npm or yarn workspace that is installed only at the
//!   workspace's root. Inside `node_modules` the search continues past
//!   package roots, where npm hoists dependencies and where nested layouts
//!   (`node_modules/.deno/<pkg>@<version>/node_modules/`) put them.
//! - **Every `package.json` the search passes outside `node_modules` is
//!   read**, to tell a marker from a bound, so one that isn't valid (not
//!   JSON, a `"name"` or `"type"` that isn't a string, or a special file)
//!   fails the import, where Node never reads it.
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
//! scheme, `#` package imports, a bare Node built-in name (`fs`, which is
//! looked up as a package and not found), a `%2F`, `%5C`, or `%00` in a
//! resolved path, a `.js` file outside `"type": "module"`, a symlink loop
//! (named as one where the OS's `ELOOP` is known), a `package.json` over
//! [`MAX_PACKAGE_JSON_SIZE`], one whose `"name"` or `"type"` isn't a string,
//! or one that is a special file, and the empty specifier.

use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use url::Url;

/// The conditions a package's `"exports"` match, in no order of their own:
/// the first key in the package's order that is one of these wins.
pub const CONDITIONS: [&str; 3] = ["gro", "import", "default"];

/// The most bytes a `package.json` may have. Packages are untrusted input,
/// and no real one comes near it.
pub const MAX_PACKAGE_JSON_SIZE: u64 = 16 * 1024 * 1024;

/// `ELOOP`, the OS error a path through a symlink loop fails with: 40 on
/// Linux and Android outside the MIPS and SPARC architectures, which number
/// it differently, and 62 on macOS, iOS, and the BSDs. Elsewhere it isn't
/// known, and a symlink loop reports as the plain I/O error.
// TODO: `io::ErrorKind::FilesystemLoop` once it's stable, in place of the table
const ELOOP: Option<i32> = if cfg!(all(
    any(target_os = "linux", target_os = "android"),
    not(any(
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "mips32r6",
        target_arch = "mips64r6",
        target_arch = "sparc",
        target_arch = "sparc64"
    ))
)) {
    Some(40)
} else if cfg!(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
)) {
    Some(62)
} else {
    None
};

/// A read-only view of the files modules resolve against.
pub trait SourceTree {
    /// Looks up `path`, an absolute path. Returns `Ok(None)` when nothing
    /// exists there, following symlinks.
    ///
    /// # Errors
    ///
    /// Returns any other failure to read the tree, such as a permission error.
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>>;

    /// Reads the file at `path`, a real path [`SourceTree::lookup`] found a
    /// file at. Only `package.json` files are read, so a file longer than
    /// [`MAX_PACKAGE_JSON_SIZE`] is refused without reading it all.
    ///
    /// # Errors
    ///
    /// Returns an error of kind [`io::ErrorKind::FileTooLarge`] for a file
    /// over [`MAX_PACKAGE_JSON_SIZE`], and any failure to read the file.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
}

/// What [`SourceTree::lookup`] found at a path, at its real path (every
/// symlink resolved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A regular file.
    File(PathBuf),
    /// A directory.
    Dir(PathBuf),
    /// Any other entry, such as a socket or a device.
    Other,
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
        let metadata = std::fs::metadata(&real_path)?;
        Ok(Some(if metadata.is_file() {
            Entry::File(real_path)
        } else if metadata.is_dir() {
            Entry::Dir(real_path)
        } else {
            Entry::Other
        }))
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        use std::io::Read as _;
        let too_large = || io::Error::from(io::ErrorKind::FileTooLarge);
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        if size > MAX_PACKAGE_JSON_SIZE {
            return Err(too_large());
        }
        // bounded again, since the file can grow after its size is read
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or_default());
        file.take(MAX_PACKAGE_JSON_SIZE + 1)
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PACKAGE_JSON_SIZE {
            return Err(too_large());
        }
        Ok(bytes)
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
    /// The `package.json` files read to resolve it, at their real paths, each
    /// once per role. A relative specifier to a `.mjs` or TypeScript file
    /// reads none.
    pub files_consulted: Vec<ConsultedFile>,
}

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

/// A metadata file resolution read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsultedFile {
    /// Its real path.
    pub path: PathBuf,
    /// What it was read for.
    pub role: FileRole,
}

/// What a consulted file was read for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRole {
    /// A package's `package.json`, for its `"exports"`.
    Exports,
    /// The nearest `package.json` above a module, for its `"type"`, or above
    /// an importer, for a self-reference's `"name"` and `"exports"`.
    PackageScope,
    /// A `package.json` beside the `node_modules` search, read to tell a
    /// type marker, which the search passes, from the project root it ends
    /// at.
    SearchBound,
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
    /// The empty string.
    Empty,
    /// A relative or absolute specifier that isn't a valid URL, a `file:` URL
    /// that names no local path, or a resolved path with an encoded `/`, `\`,
    /// or NUL in it.
    InvalidUrl(String),
    /// A bare specifier whose package name is malformed: it starts with `.`,
    /// holds `%` or `\`, has an empty, `.`, or `..` segment, or is a scope
    /// with no name after it.
    InvalidPackageName,
    /// No `node_modules/<name>` directory was found, from the importer up to
    /// the `package.json` that ended the search, or the filesystem's root.
    PackageNotFound {
        /// The package name.
        name: String,
        /// What ended the search, unset when it reached the filesystem's
        /// root.
        bound: Option<Box<PackageSearchBound>>,
    },
    /// A `node_modules/<name>` directory without a `package.json`.
    NoPackageJson(PathBuf),
    /// A `package.json` that isn't a regular file or a JSON object, whose
    /// `"name"` or `"type"` isn't a string, or whose `"exports"` are
    /// malformed (subpath keys mixed with condition keys, numeric keys).
    InvalidPackageJson {
        /// Its real path.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// A package whose `package.json` has no `"exports"`, or `null`.
    NoExports(PathBuf),
    /// The subpath matches no `"exports"` key, or its target is `null`.
    NotExported {
        /// The subpath, `.` or `./…`.
        subpath: String,
        /// The package's `package.json`.
        package_json: PathBuf,
        /// The package's `"exports"` keys.
        exported: Vec<String>,
    },
    /// The subpath's target is conditional, and none of its conditions is in
    /// [`CONDITIONS`], or each one that is holds only unmatched conditions.
    NoMatchingCondition {
        /// The subpath, `.` or `./…`.
        subpath: String,
        /// The conditions that didn't match, in the package's order, a
        /// nested one after its matched parent and a dot (`gro.browser`).
        unmatched: Vec<String>,
    },
    /// The `"exports"` target the subpath selected is invalid.
    InvalidTarget {
        /// The `"exports"` key that matched.
        key: String,
        /// The target, as JSON.
        target: String,
        /// Why it's invalid.
        reason: TargetFault,
    },
    /// The part of the subpath a pattern's `*` matched has a `.`, `..`, or
    /// `node_modules` segment, or substituted into the target, it leaves
    /// the package's directory.
    InvalidSubpath {
        /// The `"exports"` pattern that matched.
        key: String,
        /// The part its `*` matched.
        matched: String,
    },
    /// A package exports the subpath as `path`, and loading it fails with
    /// `error`.
    Target {
        /// The file the package's `"exports"` name.
        path: PathBuf,
        /// Why it doesn't load.
        error: Box<Self>,
    },
    /// The path has no extension. `suggestion` names the first sibling that
    /// exists with one, for the message only (resolution never probes).
    NoExtension {
        /// The specifier with the extension of an existing sibling.
        suggestion: Option<String>,
    },
    /// Nothing exists at the path. `ts_sibling` is set when a `.js`
    /// specifier's `.ts` sibling exists, or a `.mjs` specifier's `.mts` one.
    NotFound {
        /// The specifier with `.ts` in place of `.js`, or `.mts` in place of
        /// `.mjs`; for a package's target, the target's path.
        ts_sibling: Option<String>,
    },
    /// The path exists but isn't a regular file: a directory or a special
    /// file.
    NotAFile,
    /// A `.cts` file, TypeScript that compiles to `require` and
    /// `module.exports` rather than an ES module.
    CommonJs,
    /// A `.js` file that isn't under `"type": "module"`, so it isn't an ES
    /// module. `package_json` is the nearest `package.json`, unset when none
    /// is above the file.
    NotModuleType {
        /// The `package.json` that doesn't set `"type": "module"`.
        package_json: Option<PathBuf>,
    },
    /// A `.tsx` file: JSX isn't supported.
    Jsx,
    /// A declaration file (`.d.ts`, `.d.mts`, `.d.cts`), which holds only
    /// types.
    Declaration,
    /// A Svelte module, whose runes need the Svelte compiler: a name with
    /// `.svelte.` in it that ends in `.js` or `.ts`, the rule
    /// vite-plugin-svelte compiles by default (`.svelte.ts`,
    /// `.svelte.test.ts`). `.svelte.mts` is outside it, so it loads as
    /// TypeScript.
    SvelteModule,
    /// An extension that isn't loaded as a module.
    UnsupportedExtension(String),
    /// The path leads into a symlink loop, so it names no file or directory.
    SymlinkLoop(PathBuf),
    /// Reading the tree failed.
    Io(String),
}

/// The `package.json` that ended a `node_modules` search, one that isn't a
/// type marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSearchBound {
    /// The directory the search found it in, whose `node_modules` was the
    /// last searched.
    pub dir: PathBuf,
    /// Its real path.
    pub package_json: PathBuf,
    /// The nearest `package.json` above it, whose `node_modules` aren't
    /// searched, as an npm or yarn workspace's root's aren't.
    pub above: Option<PathBuf>,
}

/// Why an `"exports"` target is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFault {
    /// It doesn't start with `./`: a bare name, an absolute path, `../`, or a
    /// URL.
    NotRelative,
    /// It has a `.`, `..`, or `node_modules` segment, percent-encoded or not.
    Segment,
    /// It resolves outside the package's directory.
    Outside,
    /// It isn't a string, an array, an object, or `null`.
    NotATarget,
}

impl ResolveErrorKind {
    /// Whether the specifier must change for the import to resolve, the
    /// common case, as opposed to a failure reading the files or a broken
    /// symlink among them.
    pub const fn is_specifier_error(&self) -> bool {
        !matches!(self, Self::Io(_) | Self::SymlinkLoop(_))
    }
}

/// How a module's source becomes the JS V8 evaluates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleFormat {
    /// `.js` and `.mjs`: evaluated as written.
    JavaScript,
    /// `.ts` and `.mts`: stripped of their types first, every surviving
    /// character kept at its line and column.
    TypeScript,
}

/// The format of the module file at `path`, by its name: the one table both
/// resolution and loading read.
///
/// The compound names (`.d.ts`, `.svelte.ts`, `.svelte.test.ts`) are checked
/// before the last extension. A `.js` file's `"type"` is checked at
/// resolution, not here.
///
/// # Errors
///
/// Returns the reason the file doesn't load as a module.
pub fn module_format(path: &Path) -> Result<ModuleFormat, ResolveErrorKind> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    if [".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        return Err(ResolveErrorKind::Declaration);
    }
    if is_svelte_module(&name) {
        return Err(ResolveErrorKind::SvelteModule);
    }
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy())
        .unwrap_or_default();
    match &*extension {
        "js" | "mjs" => Ok(ModuleFormat::JavaScript),
        "ts" | "mts" => Ok(ModuleFormat::TypeScript),
        "cts" => Err(ResolveErrorKind::CommonJs),
        "tsx" => Err(ResolveErrorKind::Jsx),
        _ => Err(ResolveErrorKind::UnsupportedExtension(
            extension.into_owned(),
        )),
    }
}

/// Whether a file named `name` is a Svelte module by vite-plugin-svelte's
/// default rule, applied to the file name (the query already stripped):
/// `.svelte.`, then any non-empty segments, then `js` or `ts`. vite's regex
/// also stops at `?`, `#`, and `\`, which are query and separator characters
/// in an id but plain file-name characters here, so such names are refused.
fn is_svelte_module(name: &str) -> bool {
    const INFIX: &str = ".svelte.";
    name.match_indices(INFIX).any(|(index, _)| {
        let mut segments = name[index + INFIX.len()..].split('.');
        matches!(segments.next_back(), Some("js" | "ts"))
            && segments.all(|segment| !segment.is_empty())
    })
}

/// The extensions a missing-extension message suggests, in order.
const SUGGESTED_EXTENSIONS: [&str; 4] = ["ts", "js", "mjs", "mts"];

/// What every extension message lists as loading.
const LOADING_EXTENSIONS: &str = ".js, .mjs, .ts, and .mts modules load";

/// The fix for a package that can't load as an ES module.
const PACKAGE_FIX: &str = "Fix: use a version with an ESM build, vendor an ESM bundle of it into \
                           the project, or run it under Node";

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
                 Fix: install the package into node_modules and import it by name \
                 (a jsr: package installs from npm as @jsr/<scope>__<name>, aliased to its own name)",
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
            Self::Empty => f.write_str("the specifier is empty"),
            Self::InvalidUrl(reason) => write!(f, "not a valid file URL: {reason}"),
            Self::InvalidPackageName => f.write_str(
                "it is not a valid package name: a name can't start with '.' or hold '%' or '\\', \
                 and a scoped one is '@scope/name'",
            ),
            Self::PackageNotFound {
                name,
                bound: Some(bound),
            } => {
                write!(
                    f,
                    "no package '{name}' is installed: no node_modules/{name} directory from the \
                     importer up to {}, which ends the search, as every package.json outside \
                     node_modules does except one holding nothing but \"type\" and \"sideEffects\"",
                    bound.package_json.display()
                )?;
                if let Some(above) = &bound.above {
                    write!(
                        f,
                        ", and the node_modules beside {} is not searched (npm and yarn \
                         workspaces, which install at their root, aren't supported)",
                        above.display()
                    )?;
                }
                write!(
                    f,
                    ". Fix: install it into {}",
                    bound.dir.join("node_modules").display()
                )
            }
            Self::PackageNotFound { name, bound: None } => write!(
                f,
                "no package '{name}' is installed: no node_modules/{name} directory in any \
                 directory above the importer. Fix: install it into node_modules"
            ),
            Self::NoPackageJson(dir) => write!(
                f,
                "{} has no package.json, so it is not an installed package. \
                 Fix: reinstall the package",
                dir.display()
            ),
            Self::InvalidPackageJson { path, reason } => write!(
                f,
                "{} is not a valid package.json: {reason}. Fix: correct it, or reinstall the package",
                path.display()
            ),
            Self::NoExports(package_json) => write!(
                f,
                "{} has no \"exports\" field, and only \"exports\" is read (not \"main\" or \
                 \"module\"). {PACKAGE_FIX}",
                package_json.display()
            ),
            Self::NotExported {
                subpath,
                package_json,
                exported,
            } => {
                write!(
                    f,
                    "subpath '{subpath}' is not exported by {} ",
                    package_json.display()
                )?;
                if exported.is_empty() {
                    f.write_str("(its \"exports\" are empty)")?;
                } else {
                    write!(f, "(exports: {})", exported.join(", "))?;
                }
                f.write_str(
                    ". Fix: import an exported subpath, or add one to the package's \"exports\"",
                )
            }
            Self::NoMatchingCondition { subpath, unmatched } => write!(
                f,
                "no condition matches for subpath '{subpath}': the package's conditions [{}] \
                 don't, and only {} are matched. Fix: add a \"gro\" or \"import\" condition \
                 before the others",
                unmatched.join(", "),
                CONDITIONS.join(", ")
            ),
            Self::InvalidTarget {
                key,
                target,
                reason,
            } => {
                write!(f, "the \"exports\" target {target} for '{key}' is invalid: ")?;
                f.write_str(match reason {
                    TargetFault::NotRelative => "a target must start with './'",
                    TargetFault::Segment => {
                        "a target can't have a '.', '..', or 'node_modules' segment, \
                         percent-encoded or not"
                    }
                    TargetFault::Outside => "it leaves the package's directory",
                    TargetFault::NotATarget => {
                        "a target is a string, an array, an object of conditions, or null"
                    }
                })?;
                f.write_str(". Fix: use a version of the package whose \"exports\" are valid")
            }
            Self::InvalidSubpath { key, matched } => write!(
                f,
                "the part of the subpath the pattern '{key}' matches, '{matched}', has a '.', \
                 '..', or 'node_modules' segment, percent-encoded or not, or takes the target \
                 out of the package's directory, and a pattern never substitutes those. Fix: \
                 import the module by its plain subpath"
            ),
            Self::Target { path, error } => {
                let path = path.display();
                match &**error {
                    Self::NotFound { ts_sibling } => {
                        write!(f, "the package exports it as {path}, which does not exist")?;
                        if let Some(ts_sibling) = ts_sibling {
                            write!(f, " ('{ts_sibling}' does)")?;
                        }
                        f.write_str(". Fix: reinstall the package")
                    }
                    Self::NotModuleType { .. } => write!(
                        f,
                        "the package exports it as {path}, which is CommonJS (no \"type\": \
                         \"module\", not .mjs), and only ES modules load. {PACKAGE_FIX}"
                    ),
                    error => write!(f, "the package exports it as {path}: {error}"),
                }
            }
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
            Self::CommonJs => f.write_str(
                "a .cts file is CommonJS, and only ES modules load. \
                 Fix: rename it to .ts or .mts and use import and export",
            ),
            Self::NotModuleType {
                package_json: Some(package_json),
            } => write!(
                f,
                "a .js file is an ES module only under \"type\": \"module\", which {} does not \
                 set, and CommonJS is not supported. Fix: add \"type\": \"module\" to it, or \
                 rename the file to .mjs",
                package_json.display()
            ),
            Self::NotModuleType { package_json: None } => f.write_str(
                "a .js file is an ES module only under a package.json with \"type\": \"module\", \
                 and none is above it (the format is never guessed from the code). \
                 Fix: rename the file to .mjs, or add a package.json with \"type\": \"module\"",
            ),
            Self::Jsx => write!(
                f,
                "JSX is not supported, so .tsx does not load; {LOADING_EXTENSIONS}"
            ),
            Self::Declaration => f.write_str(
                "a declaration file holds only types, with no code to run. \
                 Fix: import the module it declares, or use 'import type' for its types",
            ),
            Self::SvelteModule => f.write_str(
                "a Svelte module (.svelte.ts, .svelte.js, or a name like .svelte.test.ts) needs \
                 the Svelte compiler, which is not supported yet. Fix: run it with gro on Node \
                 (gro test for a .svelte.test.ts)",
            ),
            Self::UnsupportedExtension(extension) => write!(
                f,
                "the extension '.{extension}' does not load as a module; {LOADING_EXTENSIONS}"
            ),
            Self::SymlinkLoop(path) => write!(
                f,
                "{} leads into a symlink loop, so it names no file or directory. \
                 Fix: remove the looping link, or point it at what it should name (for a \
                 package, reinstall it)",
                path.display()
            ),
            Self::Io(error) => write!(f, "failed to read the file: {error}"),
        }
    }
}

/// Resolves `specifier`, imported by the module at `importer`, which came
/// from `importer_origin` (what resolving the importer returned).
///
/// # Errors
///
/// Returns a [`ResolveError`] when the specifier names no loadable module;
/// its kind says why.
pub fn resolve(
    tree: &impl SourceTree,
    specifier: &str,
    importer: &Url,
    importer_origin: &Origin,
) -> Result<Resolution, ResolveError> {
    let error = |kind| ResolveError {
        specifier: specifier.to_owned(),
        importer: importer.to_string(),
        kind,
    };
    let mut consulted = Consulted::default();
    let (url, origin, hint) = match parse_specifier(specifier, importer).map_err(error)? {
        Request::Url(url) => (url, importer_origin.clone(), Some(specifier)),
        Request::Package(specifier) => {
            let (url, origin) =
                resolve_package(tree, specifier, importer, importer_origin, &mut consulted)
                    .map_err(error)?;
            (url, origin, None)
        }
    };
    let path = url_to_path(&url).map_err(error)?;
    let real_path = match find_module(tree, &path, hint, &mut consulted) {
        Ok(real_path) => real_path,
        Err(kind @ (ResolveErrorKind::Io(_) | ResolveErrorKind::SymlinkLoop(_))) => {
            return Err(error(kind));
        }
        // a package's target: the error names the file, not the specifier
        Err(kind) if hint.is_none() => {
            return Err(error(ResolveErrorKind::Target {
                path,
                error: Box::new(kind),
            }));
        }
        Err(kind) => return Err(error(kind)),
    };
    to_resolution(&real_path, &url, origin, consulted).map_err(error)
}

/// Resolves the entry module at `path`, relative to `cwd`.
///
/// `cwd` is an absolute path, and an absolute `path` ignores it. The same
/// rules apply as to an import: the extension is mandatory and the file must
/// exist. It is a path, not a URL, so `%`, `?`, and `#` are file-name
/// characters. Errors name the path as given, imported from `cwd`. The entry
/// is the project's.
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
    let path = normalize(&cwd.join(path));
    if !path.is_absolute() {
        return Err(error(ResolveErrorKind::InvalidUrl(
            "not an absolute path".to_owned(),
        )));
    }
    let mut consulted = Consulted::default();
    let real_path = find_module(tree, &path, Some(&specifier), &mut consulted).map_err(error)?;
    let url = Url::from_file_path(&real_path).map_err(|()| {
        error(ResolveErrorKind::InvalidUrl(
            "names no local path".to_owned(),
        ))
    })?;
    Ok(Resolution {
        url,
        origin: Origin::Project,
        files_consulted: consulted.0,
    })
}

/// A parsed specifier.
enum Request<'a> {
    /// A relative or absolute specifier, parsed against the importer.
    Url(Url),
    /// A bare specifier, which names a package.
    Package(&'a str),
}

/// Parses a specifier, refusing every form but relative and absolute `file:`
/// ones and bare ones.
fn parse_specifier<'a>(
    specifier: &'a str,
    importer: &Url,
) -> Result<Request<'a>, ResolveErrorKind> {
    // `.` and `..` alone are relative too, as in Node, so they fail as directories
    if specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('/')
        || specifier == "."
        || specifier == ".."
    {
        return importer
            .join(specifier)
            .map(Request::Url)
            .map_err(|error| ResolveErrorKind::InvalidUrl(error.to_string()));
    }
    if let Ok(url) = Url::parse(specifier) {
        return match url.scheme() {
            "file" => Ok(Request::Url(url)),
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
    Ok(Request::Package(specifier))
}

/// The local path a resolved URL names, refusing what Node refuses in one: an
/// encoded `/` or `\`, which would decode to a separator the URL never had,
/// and an encoded NUL, which no path holds.
fn url_to_path(url: &Url) -> Result<PathBuf, ResolveErrorKind> {
    let encoded = url.path().to_ascii_lowercase();
    if encoded.contains("%2f") || encoded.contains("%5c") {
        return Err(ResolveErrorKind::InvalidUrl(format!(
            "the path {} has an encoded '/' or '\\' (%2F or %5C), which never names a separator",
            url.path()
        )));
    }
    if encoded.contains("%00") {
        return Err(ResolveErrorKind::InvalidUrl(format!(
            "the path {} has an encoded NUL (%00)",
            url.path()
        )));
    }
    url.to_file_path()
        .map_err(|()| ResolveErrorKind::InvalidUrl("names no local path".to_owned()))
}

/// The files a resolution read, each `(path, role)` once.
#[derive(Debug, Default)]
struct Consulted(Vec<ConsultedFile>);

impl Consulted {
    fn add(&mut self, path: &Path, role: FileRole) {
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
struct PackageJson {
    /// The directory it was looked up in, which a symlinked `package.json`
    /// isn't in.
    dir: PathBuf,
    /// Its real path.
    path: PathBuf,
    /// Its fields, in the file's key order.
    fields: serde_json::Map<String, Value>,
}

impl PackageJson {
    /// The field `key`, unless absent or not a string.
    fn string(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    /// Its `"exports"`, unless absent or `null`.
    fn exports(&self) -> Option<&Value> {
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
fn lookup(tree: &impl SourceTree, path: &Path) -> Result<Option<Entry>, ResolveErrorKind> {
    tree.lookup(path).map_err(|error| {
        if ELOOP.is_some() && error.raw_os_error() == ELOOP {
            ResolveErrorKind::SymlinkLoop(path.to_owned())
        } else {
            ResolveErrorKind::Io(error.to_string())
        }
    })
}

/// Reads `dir`'s `package.json`, when it has one. A leading byte order mark
/// is skipped, as Node skips it. A `"name"` or `"type"` that isn't a string
/// makes it invalid, as it does in Node, and so does a `package.json` that
/// is a special file (a FIFO, a socket, or a device), which is never opened.
fn read_package_json(
    tree: &impl SourceTree,
    dir: &Path,
    role: FileRole,
    consulted: &mut Consulted,
) -> Result<Option<PackageJson>, ResolveErrorKind> {
    let path = dir.join("package.json");
    let path = match lookup(tree, &path)? {
        Some(Entry::File(path)) => path,
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
    let bytes = match tree.read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::FileTooLarge => {
            return Err(invalid(format!(
                "it is larger than {} MiB, the most a package.json may be",
                MAX_PACKAGE_JSON_SIZE / (1024 * 1024)
            )));
        }
        Err(error) => return Err(ResolveErrorKind::Io(error.to_string())),
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

/// The package scope of `dir`: the nearest `package.json` at or above it,
/// the search ending at a `node_modules` directory, as Node's is.
fn package_scope(
    tree: &impl SourceTree,
    dir: &Path,
    consulted: &mut Consulted,
) -> Result<Option<PackageJson>, ResolveErrorKind> {
    for dir in dir.ancestors() {
        if dir.file_name().is_some_and(|name| name == "node_modules") {
            break;
        }
        if let Some(package_json) = read_package_json(tree, dir, FileRole::PackageScope, consulted)?
        {
            return Ok(Some(package_json));
        }
    }
    Ok(None)
}

/// Whether `path` has a `node_modules` component.
fn in_node_modules(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "node_modules")
}

/// Splits a bare specifier into its package name and its subpath (`.` or
/// `./…`), as Node does, refusing a malformed name. Node accepts empty, `.`,
/// and `..` segments in a scoped name; they're refused here, since they name
/// a directory other than the package's.
fn parse_package_name(specifier: &str) -> Result<(&str, String), ResolveErrorKind> {
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

/// Resolves a bare specifier to the URL its package's `"exports"` name, and
/// the origin of what it names.
fn resolve_package(
    tree: &impl SourceTree,
    specifier: &str,
    importer: &Url,
    importer_origin: &Origin,
    consulted: &mut Consulted,
) -> Result<(Url, Origin), ResolveErrorKind> {
    let (name, subpath) = parse_package_name(specifier)?;
    let importer_path = importer.to_file_path().map_err(|()| {
        ResolveErrorKind::InvalidUrl("the importer names no local path".to_owned())
    })?;
    let importer_dir = importer_path.parent().unwrap_or(&importer_path);

    // a self-reference: the importer's own package, by its name
    let scope = package_scope(tree, importer_dir, consulted)?;
    if let Some(scope) = &scope
        && let Some(exports) = scope.exports()
        && scope.string("name") == Some(name)
    {
        consulted.add(&scope.path, FileRole::Exports);
        let root = scope.path.parent().unwrap_or(&scope.path);
        let url = resolve_exports(root, &scope.path, exports, &subpath)?;
        return Ok((url, importer_origin.clone()));
    }

    let mut bound = None;
    let mut found = None;
    for dir in importer_dir.ancestors() {
        let package_dir = dir.join("node_modules").join(name);
        if let Some(Entry::Dir(root)) = lookup(tree, &package_dir)? {
            found = Some((package_dir, root));
            break;
        }
        if in_node_modules(dir) {
            continue;
        }
        // any package.json but a type marker ends the search, the scope's read once
        let package_json = match &scope {
            Some(scope) if scope.dir == dir => {
                consulted.add(&scope.path, FileRole::SearchBound);
                (!scope.is_type_marker()).then(|| scope.path.clone())
            }
            _ => read_package_json(tree, dir, FileRole::SearchBound, consulted)?
                .filter(|package_json| !package_json.is_type_marker())
                .map(|package_json| package_json.path),
        };
        if let Some(package_json) = package_json {
            bound = Some((dir, package_json));
            break;
        }
    }
    let Some((package_dir, root)) = found else {
        return Err(package_not_found(tree, name, bound));
    };
    let package_json = read_package_json(tree, &root, FileRole::Exports, consulted)?
        .ok_or(ResolveErrorKind::NoPackageJson(package_dir))?;
    let exports = package_json
        .exports()
        .ok_or_else(|| ResolveErrorKind::NoExports(package_json.path.clone()))?;
    let url = resolve_exports(&root, &package_json.path, exports, &subpath)?;
    let origin = Origin::Package {
        name: name.to_owned(),
        version: package_json.string("version").map(str::to_owned),
        linked: !in_node_modules(&root),
    };
    Ok((url, origin))
}

/// The error for a package the search didn't find: `bound` is the directory
/// and the `package.json` that ended it, if one did, and the nearest
/// `package.json` above that directory is named as unsearched.
fn package_not_found(
    tree: &impl SourceTree,
    name: &str,
    bound: Option<(&Path, PathBuf)>,
) -> ResolveErrorKind {
    let Some((dir, package_json)) = bound else {
        return ResolveErrorKind::PackageNotFound {
            name: name.to_owned(),
            bound: None,
        };
    };
    // only for the message, so a failure to look is no `package.json`
    let above =
        dir.ancestors()
            .skip(1)
            .find_map(|dir| match tree.lookup(&dir.join("package.json")) {
                Ok(Some(Entry::File(path))) => Some(path),
                _ => None,
            });
    ResolveErrorKind::PackageNotFound {
        name: name.to_owned(),
        bound: Some(Box::new(PackageSearchBound {
            dir: dir.to_owned(),
            package_json,
            above,
        })),
    }
}

/// What a target in `"exports"` resolved to.
enum Target {
    /// A URL, not yet checked to exist.
    Url(Url),
    /// `null`, or an empty array: not exported, and no fallback.
    Null,
    /// A conditional object none of whose conditions matched, so its parent
    /// tries its next key.
    Unmatched,
}

/// The package and the `"exports"` key a target is resolved for.
struct TargetContext<'a> {
    /// The package's directory, as a URL.
    root: &'a Url,
    /// The `"exports"` key that matched.
    key: &'a str,
    /// What a pattern key's `*` matched.
    matched: Option<&'a str>,
    /// The package's `package.json`, for errors.
    package_json: &'a Path,
}

/// Resolves `subpath` through a package's `"exports"`, Node's
/// `PACKAGE_EXPORTS_RESOLVE`.
fn resolve_exports(
    root: &Path,
    package_json: &Path,
    exports: &Value,
    subpath: &str,
) -> Result<Url, ResolveErrorKind> {
    let invalid = |reason: &str| ResolveErrorKind::InvalidPackageJson {
        path: package_json.to_owned(),
        reason: reason.to_owned(),
    };
    let root = Url::from_directory_path(root)
        .map_err(|()| ResolveErrorKind::InvalidUrl("names no local path".to_owned()))?;
    // a string, an array, or an object of conditions alone is the main entry
    let entries: Vec<(&str, &Value)> = match exports {
        Value::String(_) | Value::Array(_) => vec![(".", exports)],
        Value::Object(map) => {
            let mut conditional = None;
            for key in map.keys() {
                let is_condition = key.is_empty() || !key.starts_with('.');
                if *conditional.get_or_insert(is_condition) != is_condition {
                    return Err(invalid(
                        "\"exports\" cannot mix keys starting with '.' with keys that don't",
                    ));
                }
            }
            if conditional == Some(true) {
                vec![(".", exports)]
            } else {
                map.iter()
                    .map(|(key, value)| (key.as_str(), value))
                    .collect()
            }
        }
        _ => Vec::new(),
    };
    let not_exported = || ResolveErrorKind::NotExported {
        subpath: subpath.to_owned(),
        package_json: package_json.to_owned(),
        exported: entries.iter().map(|(key, _)| (*key).to_owned()).collect(),
    };

    let Some((key, target, matched)) = select_export(&entries, subpath) else {
        return Err(not_exported());
    };
    let context = TargetContext {
        root: &root,
        key,
        matched,
        package_json,
    };
    match resolve_target(&context, target)? {
        Target::Url(url) => Ok(url),
        Target::Null => Err(not_exported()),
        Target::Unmatched => {
            let mut unmatched = Vec::new();
            unmatched_conditions(target, "", &mut unmatched);
            // no conditions at all (`{}`) is no export, as Node words it
            if unmatched.is_empty() {
                return Err(not_exported());
            }
            Err(ResolveErrorKind::NoMatchingCondition {
                subpath: subpath.to_owned(),
                unmatched,
            })
        }
    }
}

/// Collects the conditions in `target` that didn't match, each once, in the
/// package's order: a key outside [`CONDITIONS`], or, under a key in it, its
/// own unmatched conditions, written after `prefix` and the key and a dot.
fn unmatched_conditions(target: &Value, prefix: &str, unmatched: &mut Vec<String>) {
    match target {
        Value::Array(targets) => {
            for target in targets {
                unmatched_conditions(target, prefix, unmatched);
            }
        }
        Value::Object(conditions) => {
            for (condition, target) in conditions {
                if CONDITIONS.contains(&condition.as_str()) {
                    unmatched_conditions(target, &format!("{prefix}{condition}."), unmatched);
                } else {
                    let condition = format!("{prefix}{condition}");
                    if !unmatched.contains(&condition) {
                        unmatched.push(condition);
                    }
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// The `"exports"` entry `subpath` selects, with what a pattern key's `*`
/// matched: the exact key, unless the subpath has a `*` or ends in `/`, else
/// the most specific single-`*` pattern by [`pattern_key_compare`]. An exact
/// key is never passed over for a pattern, even when its target is `null`.
fn select_export<'a>(
    entries: &[(&'a str, &'a Value)],
    subpath: &'a str,
) -> Option<(&'a str, &'a Value, Option<&'a str>)> {
    if !subpath.contains('*')
        && !subpath.ends_with('/')
        && let Some(&(key, target)) = entries.iter().find(|(key, _)| *key == subpath)
    {
        return Some((key, target, None));
    }
    let mut best: Option<(&str, &Value, &str)> = None;
    for &(key, target) in entries {
        let Some(star) = key.find('*') else {
            continue;
        };
        let (prefix, trailer) = (&key[..star], &key[star + 1..]);
        if key.rfind('*') == Some(star)
            && subpath.len() >= key.len()
            && subpath.starts_with(prefix)
            && subpath.ends_with(trailer)
            && best.is_none_or(|(best, _, _)| pattern_key_compare(best, key).is_gt())
        {
            best = Some((key, target, &subpath[star..subpath.len() - trailer.len()]));
        }
    }
    best.map(|(key, target, matched)| (key, target, Some(matched)))
}

/// Node's `PATTERN_KEY_COMPARE`: `Greater` when `b` is the more specific
/// pattern, by the length of its prefix up to and including `*`, then by its
/// whole length.
fn pattern_key_compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let base_length = |key: &str| key.find('*').map_or(key.len(), |index| index + 1);
    match base_length(b).cmp(&base_length(a)) {
        Ordering::Equal => {}
        ordering => return ordering,
    }
    if !a.contains('*') {
        return Ordering::Greater;
    }
    if !b.contains('*') {
        return Ordering::Less;
    }
    b.len().cmp(&a.len())
}

/// Resolves one `"exports"` target, Node's `PACKAGE_TARGET_RESOLVE`. File
/// existence isn't checked, so an array falls back past an invalid target,
/// never past a missing file.
fn resolve_target(context: &TargetContext<'_>, target: &Value) -> Result<Target, ResolveErrorKind> {
    match target {
        Value::String(target) => resolve_target_string(context, target).map(Target::Url),
        Value::Array(targets) => {
            // as Node: the last invalid target is the error, unless a `null` came after it
            let mut last = None;
            for target in targets {
                match resolve_target(context, target) {
                    Ok(Target::Url(url)) => return Ok(Target::Url(url)),
                    Ok(Target::Unmatched) => {}
                    Ok(Target::Null) => last = Some(Ok(())),
                    Err(error @ ResolveErrorKind::InvalidTarget { .. }) => last = Some(Err(error)),
                    Err(error) => return Err(error),
                }
            }
            match last {
                None if !targets.is_empty() => Ok(Target::Unmatched),
                None | Some(Ok(())) => Ok(Target::Null),
                Some(Err(error)) => Err(error),
            }
        }
        Value::Object(conditions) => {
            if conditions.keys().any(|key| is_array_index(key)) {
                return Err(ResolveErrorKind::InvalidPackageJson {
                    path: context.package_json.to_owned(),
                    reason: "\"exports\" cannot have numeric keys".to_owned(),
                });
            }
            for (condition, target) in conditions {
                if CONDITIONS.contains(&condition.as_str()) {
                    match resolve_target(context, target)? {
                        Target::Unmatched => {}
                        resolved => return Ok(resolved),
                    }
                }
            }
            Ok(Target::Unmatched)
        }
        Value::Null => Ok(Target::Null),
        Value::Bool(_) | Value::Number(_) => Err(ResolveErrorKind::InvalidTarget {
            key: context.key.to_owned(),
            target: target.to_string(),
            reason: TargetFault::NotATarget,
        }),
    }
}

/// Whether `key` is an array index, which a conditional object can't have.
/// Node's check also counts the canonical form of a non-integer number
/// (`1.5`); only integers count here.
fn is_array_index(key: &str) -> bool {
    (key == "0" || (!key.starts_with('0') && !key.is_empty()))
        && key.bytes().all(|byte| byte.is_ascii_digit())
        && key.parse::<u64>().is_ok_and(|index| index < 0xFFFF_FFFF)
}

/// Resolves a string target, refusing one that could leave the package:
/// it must start with `./`, and neither it nor what a pattern's `*` matched
/// may have a `.`, `..`, or `node_modules` segment.
fn resolve_target_string(
    context: &TargetContext<'_>,
    target: &str,
) -> Result<Url, ResolveErrorKind> {
    let invalid = |reason| ResolveErrorKind::InvalidTarget {
        key: context.key.to_owned(),
        target: Value::from(target).to_string(),
        reason,
    };
    let Some(rest) = target.strip_prefix("./") else {
        return Err(invalid(TargetFault::NotRelative));
    };
    if has_invalid_segment(rest) {
        return Err(invalid(TargetFault::Segment));
    }
    let inside = |url: &Url| url.path().starts_with(context.root.path());
    let unsubstituted = context
        .root
        .join(target)
        .map_err(|error| ResolveErrorKind::InvalidUrl(error.to_string()))?;
    if !inside(&unsubstituted) {
        return Err(invalid(TargetFault::Outside));
    }
    let Some(matched) = context.matched else {
        return Ok(unsubstituted);
    };
    let invalid_subpath = || ResolveErrorKind::InvalidSubpath {
        key: context.key.to_owned(),
        matched: matched.to_owned(),
    };
    if has_invalid_segment(matched) {
        return Err(invalid_subpath());
    }
    // every `*` in the target, substituted before parsing, so a `?` or `#` in
    // the match is the URL's query or fragment, as in Node
    let url = context
        .root
        .join(&target.replace('*', matched))
        .map_err(|error| ResolveErrorKind::InvalidUrl(error.to_string()))?;
    // the target alone stayed inside, so the match is what left (`.?` after a `.*`)
    if !inside(&url) {
        return Err(invalid_subpath());
    }
    Ok(url)
}

/// Whether `path` has a `.`, `..`, or `node_modules` segment (any case),
/// split at `/` and `\`, its percent-encoded forms included, the segments
/// Node's `invalidSegmentRegEx` refuses. Tabs and line breaks are removed
/// first, as URL parsing removes them, so `.\t.` is `..`; Node's check
/// misses that form, and only its containment check catches it, for a
/// target and not for a pattern's match. Empty segments pass, as in Node.
fn has_invalid_segment(path: &str) -> bool {
    let path: String = path
        .chars()
        .filter(|char| !matches!(char, '\t' | '\n' | '\r'))
        .collect();
    path.split(['/', '\\']).any(|segment| {
        let decoded = percent_decode(segment);
        decoded == b"." || decoded == b".." || decoded.eq_ignore_ascii_case(b"node_modules")
    })
}

/// `segment` with each `%XX` escape decoded to its byte.
fn percent_decode(segment: &str) -> Vec<u8> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = bytes
            .get(index + 1..index + 3)
            .filter(|_| bytes[index] == b'%')
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        if let Some(byte) = escape {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    decoded
}

/// Checks the extension, finds the real file, and checks that it loads: its
/// format by name, and a `.js` file's `"type"`. `hint` is the specifier for
/// the messages' suggestions, unset for a package's target, whose messages
/// name the path instead.
fn find_module(
    tree: &impl SourceTree,
    path: &Path,
    hint: Option<&str>,
    consulted: &mut Consulted,
) -> Result<PathBuf, ResolveErrorKind> {
    let trailing_slash = path.as_os_str().as_encoded_bytes().ends_with(b"/");

    if path.extension().is_none() || trailing_slash {
        let mut suggestion = None;
        if let Some(hint) = hint
            && !trailing_slash
        {
            for extension in SUGGESTED_EXTENSIONS {
                if let Some(Entry::File(_)) = lookup(tree, &path.with_extension(extension))? {
                    suggestion = Some(with_extension(hint, extension));
                    break;
                }
            }
        }
        return Err(ResolveErrorKind::NoExtension { suggestion });
    }

    let real_path = match lookup(tree, path)? {
        Some(Entry::File(real_path)) => real_path,
        Some(Entry::Dir(_) | Entry::Other) => return Err(ResolveErrorKind::NotAFile),
        None => {
            let ts_extension = match path.extension().and_then(|extension| extension.to_str()) {
                Some("js") => Some("ts"),
                Some("mjs") => Some("mts"),
                _ => None,
            };
            let ts_sibling = if let Some(ts_extension) = ts_extension
                && let Some(Entry::File(_)) = lookup(tree, &path.with_extension(ts_extension))?
            {
                Some(hint.map_or_else(
                    || path.with_extension(ts_extension).display().to_string(),
                    |hint| replace_extension(hint, ts_extension),
                ))
            } else {
                None
            };
            return Err(ResolveErrorKind::NotFound { ts_sibling });
        }
    };

    // format by the real file's name, which a symlink can change, as the loader reads it
    module_format(&real_path)?;
    if real_path
        .extension()
        .is_some_and(|extension| extension == "js")
    {
        let dir = real_path.parent().unwrap_or(&real_path);
        let scope = package_scope(tree, dir, consulted)?;
        if scope.as_ref().and_then(|scope| scope.string("type")) != Some("module") {
            return Err(ResolveErrorKind::NotModuleType {
                package_json: scope.map(|scope| scope.path),
            });
        }
    }
    Ok(real_path)
}

/// The resolution of the module at `real_path`, with `url`'s query and
/// fragment re-attached.
fn to_resolution(
    real_path: &Path,
    url: &Url,
    origin: Origin,
    consulted: Consulted,
) -> Result<Resolution, ResolveErrorKind> {
    let mut resolved = Url::from_file_path(real_path)
        .map_err(|()| ResolveErrorKind::InvalidUrl("names no local path".to_owned()))?;
    resolved.set_query(url.query());
    resolved.set_fragment(url.fragment());
    Ok(Resolution {
        url: resolved,
        origin,
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

    /// An in-memory tree: files by path with their contents, symlinks as a
    /// map to their target, and symlink loops, which fail with `ELOOP`. A
    /// directory is any path a file or a listed directory is under.
    #[derive(Default)]
    struct MemoryTree {
        files: BTreeMap<&'static str, &'static str>,
        dirs: Vec<&'static str>,
        links: BTreeMap<&'static str, &'static str>,
        loops: Vec<&'static str>,
    }

    impl SourceTree for MemoryTree {
        fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
            // as `canonicalize` does, `a//b` is `a/b`
            let path: PathBuf = path.components().collect();
            if self.loops.iter().any(|link| path.starts_with(link)) {
                return Err(io::Error::from_raw_os_error(
                    ELOOP.expect("the loop tests need this platform's ELOOP"),
                ));
            }
            let path = path.to_str().unwrap();
            let path = self.links.get(path).copied().unwrap_or(path);
            let is_under = |entry: &str| {
                entry
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'))
            };
            Ok(if self.files.contains_key(path) {
                Some(Entry::File(PathBuf::from(path)))
            } else if self.dirs.contains(&path)
                || self.files.keys().copied().any(is_under)
                || self.dirs.iter().copied().any(is_under)
            {
                Some(Entry::Dir(PathBuf::from(path)))
            } else {
                None
            })
        }

        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            let content = self
                .files
                .get(path.to_str().unwrap())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            if content.len() as u64 > MAX_PACKAGE_JSON_SIZE {
                return Err(io::Error::from(io::ErrorKind::FileTooLarge));
            }
            Ok(content.as_bytes().to_vec())
        }
    }

    const MODULE: &str = r#"{"type": "module"}"#;

    fn tree() -> MemoryTree {
        let files = [
            "/p/main.js",
            "/p/a.js",
            "/p/b.mjs",
            "/p/lib/c.js",
            "/p/typed.ts",
            "/p/typed.mts",
            "/p/only_ts.ts",
            "/p/only_mts.mts",
            "/p/common.cts",
            "/p/view.tsx",
            "/p/types.d.ts",
            "/p/types.d.mts",
            "/p/types.d.cts",
            "/p/state.svelte.ts",
            "/p/state.svelte.js",
            "/p/state.svelte.test.ts",
            "/p/data.json",
            "/p/schema.ts",
            "/real/linked.js",
            "/abs.js",
        ];
        let mut files: BTreeMap<_, _> = files.into_iter().map(|path| (path, "")).collect();
        files.insert("/package.json", MODULE);
        files.insert("/p/package.json", MODULE);
        MemoryTree {
            files,
            dirs: vec!["/p/dir.js"],
            links: BTreeMap::from([
                ("/p/link.js", "/real/linked.js"),
                ("/p/link_to_ts.js", "/p/typed.ts"),
            ]),
            loops: Vec::new(),
        }
    }

    fn importer() -> Url {
        Url::parse("file:///p/main.js").unwrap()
    }

    fn ok(specifier: &str) -> String {
        resolve(&tree(), specifier, &importer(), &Origin::Project)
            .unwrap_or_else(|error| panic!("{error}"))
            .url
            .to_string()
    }

    fn kind(specifier: &str) -> ResolveErrorKind {
        resolve(&tree(), specifier, &importer(), &Origin::Project)
            .unwrap_err()
            .kind
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
        let resolution = resolve(&tree(), "../a.js", &importer, &Origin::Project).unwrap();
        assert_eq!(resolution.url.as_str(), "file:///p/a.js");
        // `..` past the root stays at the root, as URL parsing does
        let resolution = resolve(&tree(), "../../../abs.js", &importer, &Origin::Project).unwrap();
        assert_eq!(resolution.url.as_str(), "file:///abs.js");
    }

    #[test]
    fn absolute() {
        assert_eq!(ok("/abs.js"), "file:///abs.js");
        assert_eq!(ok("file:///abs.js"), "file:///abs.js");
    }

    #[test]
    fn record() {
        let resolution = resolve(&tree(), "./a.js", &importer(), &Origin::Project).unwrap();
        assert_eq!(resolution.origin, Origin::Project);
        // a `.js` file's scope, read for its `"type"`
        assert_eq!(
            resolution.files_consulted,
            [ConsultedFile {
                path: PathBuf::from("/p/package.json"),
                role: FileRole::PackageScope
            }]
        );
        let resolution = resolve(&tree(), "./typed.ts", &importer(), &Origin::Project).unwrap();
        assert!(resolution.files_consulted.is_empty());
        // a relative specifier resolves with the importer's origin
        let origin = Origin::Package {
            name: "x".to_owned(),
            version: None,
            linked: true,
        };
        let resolution = resolve(&tree(), "./a.js", &importer(), &origin).unwrap();
        assert_eq!(resolution.origin, origin);
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
            kind("./only_mts"),
            ResolveErrorKind::NoExtension {
                suggestion: Some("./only_mts.mts".to_owned())
            }
        );
        assert_eq!(
            kind("./nothing"),
            ResolveErrorKind::NoExtension { suggestion: None }
        );
        // a directory never resolves to an index
        for specifier in ["./lib", "./lib/", "./dir.js/", ".", ".."] {
            assert_eq!(
                kind(specifier),
                ResolveErrorKind::NoExtension { suggestion: None },
                "{specifier}"
            );
        }
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
        assert_eq!(
            kind("./only_mts.mjs?x"),
            ResolveErrorKind::NotFound {
                ts_sibling: Some("./only_mts.mts?x".to_owned())
            }
        );
        // only the module extension's own TypeScript twin is hinted
        assert_eq!(
            kind("./only_mts.js"),
            ResolveErrorKind::NotFound { ts_sibling: None }
        );
        assert_eq!(kind("./dir.js"), ResolveErrorKind::NotAFile);
    }

    #[test]
    fn encoded_separators_and_nul_are_refused() {
        for specifier in [
            "./..%2Fa.js",
            "./lib%2fc.js",
            "./lib%5Cc.js",
            "./%00.js",
            "file:///p/lib%2Fc.js",
        ] {
            assert!(
                matches!(kind(specifier), ResolveErrorKind::InvalidUrl(_)),
                "{specifier}"
            );
        }
        // the query isn't part of the path
        assert_eq!(ok("./a.js?%2F"), "file:///p/a.js?%2F");
    }

    #[test]
    fn extensions() {
        assert_eq!(ok("./typed.ts"), "file:///p/typed.ts");
        assert_eq!(ok("./typed.mts"), "file:///p/typed.mts");
        assert_eq!(kind("./common.cts"), ResolveErrorKind::CommonJs);
        assert_eq!(kind("./view.tsx"), ResolveErrorKind::Jsx);
        for specifier in ["./types.d.ts", "./types.d.mts", "./types.d.cts"] {
            assert_eq!(
                kind(specifier),
                ResolveErrorKind::Declaration,
                "{specifier}"
            );
        }
        for specifier in [
            "./state.svelte.ts",
            "./state.svelte.js",
            "./state.svelte.test.ts",
        ] {
            assert_eq!(
                kind(specifier),
                ResolveErrorKind::SvelteModule,
                "{specifier}"
            );
        }
        assert_eq!(
            kind("./data.json"),
            ResolveErrorKind::UnsupportedExtension("json".to_owned())
        );
        // the real file's name decides, not the specifier's
        assert_eq!(ok("./link_to_ts.js"), "file:///p/typed.ts");
    }

    #[test]
    fn formats() {
        let format = |name: &str| module_format(Path::new(name));
        assert_eq!(format("/p/a.js"), Ok(ModuleFormat::JavaScript));
        assert_eq!(format("/p/a.mjs"), Ok(ModuleFormat::JavaScript));
        assert_eq!(format("/p/a.ts"), Ok(ModuleFormat::TypeScript));
        assert_eq!(format("/p/a.mts"), Ok(ModuleFormat::TypeScript));
        // only the compound suffix is a declaration or a Svelte module
        assert_eq!(format("/p/d.ts"), Ok(ModuleFormat::TypeScript));
        assert_eq!(format("/p/svelte.ts"), Ok(ModuleFormat::TypeScript));
        // vite-plugin-svelte's default rule: `.svelte.`, any segments, then `js` or `ts`
        for name in [
            "/p/a.svelte.test.ts",
            "/p/a.svelte.test.js",
            "/p/a.svelte.x.y.ts",
            "/p/.svelte.ts",
            "/p/a.svelte.svelte.ts",
        ] {
            assert_eq!(format(name), Err(ResolveErrorKind::SvelteModule), "{name}");
        }
        // outside it, so plain TypeScript
        assert_eq!(format("/p/a.svelte.mts"), Ok(ModuleFormat::TypeScript));
        assert_eq!(format("/p/a.svelte.test.mts"), Ok(ModuleFormat::TypeScript));
        assert_eq!(format("/p/a.svelte..ts"), Ok(ModuleFormat::TypeScript));
        assert_eq!(format("/p/a.svelte_x.ts"), Ok(ModuleFormat::TypeScript));
        assert_eq!(format("/p/a.d.ts"), Err(ResolveErrorKind::Declaration));
        assert_eq!(
            format("/p/a.svelte.js"),
            Err(ResolveErrorKind::SvelteModule)
        );
        assert_eq!(
            format("/p/a.svelte"),
            Err(ResolveErrorKind::UnsupportedExtension("svelte".to_owned()))
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
    fn package_imports_and_empty() {
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
            entry("typed.ts").unwrap().url.as_str(),
            "file:///p/typed.ts"
        );
        assert_eq!(entry("main.js").unwrap().origin, Origin::Project);
        assert_eq!(entry("view.tsx").unwrap_err().kind, ResolveErrorKind::Jsx);
        let error = entry("missing.js").unwrap_err();
        assert_eq!(error.kind, ResolveErrorKind::NotFound { ts_sibling: None });
        assert_eq!(
            error.to_string(),
            "cannot resolve 'missing.js' from /p: no such file"
        );
        // a path, not a URL: `?`, `#`, and `%` are file name characters
        assert_eq!(
            entry("a.js?x").unwrap_err().kind,
            ResolveErrorKind::NotFound { ts_sibling: None }
        );
        assert_eq!(
            entry("lib%2Fc.js").unwrap_err().kind,
            ResolveErrorKind::NotFound { ts_sibling: None }
        );
    }

    #[test]
    fn messages() {
        let message = |specifier| {
            resolve(&tree(), specifier, &importer(), &Origin::Project)
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
        assert!(message("jsr:@std/toml").contains("@jsr/<scope>__<name>"));
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
        assert_eq!(
            message("./common.cts"),
            "cannot resolve './common.cts' from file:///p/main.js: a .cts file is CommonJS, and \
             only ES modules load. Fix: rename it to .ts or .mts and use import and export"
        );
        assert!(message("./view.tsx").contains("JSX is not supported"));
        assert!(message("./types.d.ts").contains("a declaration file holds only types"));
        assert!(message("./state.svelte.ts").contains("needs the Svelte compiler"));
        assert!(message("./data.json").ends_with(".js, .mjs, .ts, and .mts modules load"));
        assert_eq!(
            message("./..%2Fa.js"),
            "cannot resolve './..%2Fa.js' from file:///p/main.js: not a valid file URL: the path \
             /p/..%2Fa.js has an encoded '/' or '\\' (%2F or %5C), which never names a separator"
        );
    }

    /// A project at `/w/proj` with its packages installed, under `/w`, a
    /// workspace root with packages of its own.
    fn packages() -> MemoryTree {
        let mut files = BTreeMap::from([
            (
                "/w/package.json",
                r#"{"name": "w", "type": "module", "workspaces": ["proj"]}"#,
            ),
            (
                "/w/node_modules/parent_only/package.json",
                r#"{"exports": "./i.js"}"#,
            ),
            (
                "/w/proj/package.json",
                r#"{"name": "proj", "type": "module", "exports": {"./self.js": "./self.js"}}"#,
            ),
            ("/w/proj/main.js", ""),
            ("/w/proj/self.js", ""),
            ("/w/proj/sub/x.js", ""),
            // a stale installed copy of the project, which a self-reference passes over
            (
                "/w/proj/node_modules/proj/package.json",
                r#"{"name": "proj", "exports": {"./self.js": "./self.js"}}"#,
            ),
            (
                "/w/proj/node_modules/t/package.json",
                r#"{
                    "name": "t",
                    "version": "2.3.4",
                    "type": "module",
                    "exports": {
                        ".": "./main.js",
                        "./up": "../escape.js",
                        "./bare": "x.js",
                        "./abs": "/abs/x.js",
                        "./https": "https://example.com/x.js",
                        "./dotdot": "./a/../../x.js",
                        "./nm": "./node_modules/y/i.js",
                        "./encoded": "./%2e%2E/x.js",
                        "./encoded_nm": "./%6eode_%4dodules/x.js",
                        "./dir": "./lib",
                        "./dir_slash": "./lib/",
                        "./no_extension": "./no_extension",
                        "./missing": "./missing.js",
                        "./ts": "./only_ts.js",
                        "./empty_segment": "./lib//a.js",
                        "./fall": {"gro": {"browser": "./b.js"}, "default": "./d.js"},
                        "./blocked": {"gro": null, "default": "./d.js"},
                        "./unmatched": {"node": "./d.js", "require": "./d.js"},
                        "./numeric": {"0": "./d.js", "default": "./d.js"},
                        "./two/*/*": "./lib/*/*.js",
                        "./array_invalid_first": ["x.js", "./d.js"],
                        "./array_missing_first": ["./missing.js", "./d.js"],
                        "./array_null_first": [null, "./d.js"],
                        "./array_invalid_after_null": [null, "x.js"],
                        "./array_null_after_invalid": ["x.js", null],
                        "./array_unmatched": [{"node": "./x.js"}],
                        "./array_object": [{"node": "./x.js"}, "./d.js"],
                        "./array_empty": [],
                        "./number": 5,
                        "./order": {"default": "./d.js", "gro": "./b.js"},
                        "./import": {"import": "./b.js", "default": "./d.js"},
                        "./types": {"types": "./d.d.ts", "gro": "./b.js"},
                        "./lib/*": "./lib/*.js",
                        "./lib/a/*": "./lib/a/*.js",
                        "./star/*.js": "./lib/*.js",
                        "./everywhere/*": "./lib/*/*.js",
                        "./cjs": "./cjs.cjs",
                        "./src/*": "./src/*",
                        "./pattern/*": "./lib/*",
                        "./null_pattern/*": null,
                        "./trailing/": "./lib/",
                        "./tab": "./.\t./escape.js",
                        "./query_up": "./..?x",
                        "./fragment_up": "./..#x",
                        "./tabbed/*": "./star/*.js",
                        "./dot_star/*": "./.*/i.js",
                        "./empty_conditions": {},
                        "./array_empty_conditions": [{}],
                        "./nested_unmatched": {"gro": {"browser": "./b.js"}, "node": "./d.js"},
                        "./nested_array_unmatched": [{"import": [{"node": "./d.js"}]}, {"node": "./d.js"}]
                    }
                }"#,
            ),
            ("/w/proj/node_modules/escape.js", ""),
            ("/w/proj/node_modules/escape_tab.js", ""),
            ("/w/proj/node_modules/loose.js", ""),
            ("/w/proj/node_modules/t/main.js", ""),
            ("/w/proj/node_modules/t/b.js", ""),
            ("/w/proj/node_modules/t/d.js", ""),
            ("/w/proj/node_modules/t/no_extension", ""),
            ("/w/proj/node_modules/t/only_ts.ts", ""),
            ("/w/proj/node_modules/t/cjs.cjs", ""),
            ("/w/proj/node_modules/t/lib/a.js", ""),
            ("/w/proj/node_modules/t/lib/x.js", ""),
            ("/w/proj/node_modules/t/lib/a/b.js", ""),
            ("/w/proj/node_modules/t/lib/x/x.js", ""),
            ("/w/proj/node_modules/t/src/s.ts", ""),
            (
                "/w/proj/node_modules/sugar_string/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            (
                "/w/proj/node_modules/sugar_array/package.json",
                r#"{"exports": ["x.js", "./i.mjs"]}"#,
            ),
            (
                "/w/proj/node_modules/sugar_conditions/package.json",
                r#"{"exports": {"gro": "./i.mjs", "default": "./d.mjs"}}"#,
            ),
            (
                "/w/proj/node_modules/mixed/package.json",
                r#"{"exports": {".": "./i.mjs", "default": "./i.mjs"}}"#,
            ),
            (
                "/w/proj/node_modules/exports_null/package.json",
                r#"{"main": "./i.mjs", "exports": null}"#,
            ),
            (
                "/w/proj/node_modules/no_exports/package.json",
                r#"{"main": "./i.mjs"}"#,
            ),
            (
                "/w/proj/node_modules/empty_exports/package.json",
                r#"{"exports": {}}"#,
            ),
            ("/w/proj/node_modules/not_json/package.json", "{"),
            ("/w/proj/node_modules/not_object/package.json", "[]"),
            // a directory with no package.json, which shadows the parent's real one
            ("/w/proj/node_modules/no_package/index.js", ""),
            (
                "/w/node_modules/no_package/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            (
                "/w/proj/node_modules/cjs/package.json",
                r#"{"exports": "./i.js"}"#,
            ),
            ("/w/proj/node_modules/cjs/i.js", ""),
            (
                "/w/proj/node_modules/@scope/pkg/package.json",
                r#"{"exports": {".": "./i.mjs", "./x.mjs": "./x.mjs"}}"#,
            ),
            ("/w/proj/node_modules/@scope/pkg/i.mjs", ""),
            ("/w/proj/node_modules/@scope/pkg/x.mjs", ""),
            // linked: a symlink to a checkout outside node_modules, with its own deps
            (
                "/w/linked/package.json",
                r#"{"name": "linked", "version": "0.1.0", "type": "module", "exports": {".": "./i.js"}}"#,
            ),
            ("/w/linked/i.js", ""),
            (
                "/w/linked/node_modules/linked_dep/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/w/linked/node_modules/linked_dep/i.mjs", ""),
            // a nested layout, linked into the project's node_modules, its deps beside it
            (
                "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested/package.json",
                r#"{"version": "1.0.0", "exports": "./i.mjs"}"#,
            ),
            (
                "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested/i.mjs",
                "",
            ),
            (
                "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested_dep/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            (
                "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested_dep/i.mjs",
                "",
            ),
            // hoisted: a package's dependency installed beside it
            (
                "/w/proj/node_modules/hoisted/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/w/proj/node_modules/hoisted/i.mjs", ""),
            (
                "/w/proj/node_modules/needs_hoisted/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/w/proj/node_modules/needs_hoisted/node_modules/.keep", ""),
            ("/w/proj/node_modules/needs_hoisted/i.mjs", ""),
            // a project file under a package.json without `"type": "module"`
            ("/w/proj/no_type/package.json", r#"{"name": "no_type"}"#),
            ("/w/proj/no_type/a.js", ""),
            // type markers, which the search walks past
            ("/w/proj/dist/package.json", r#"{"type": "commonjs"}"#),
            ("/w/proj/dist/esm/package.json", MODULE),
            ("/w/proj/dist/esm/x.js", ""),
            ("/w/proj/scripts/package.json", r#"{"type": "commonjs"}"#),
            ("/w/proj/scripts/x.mjs", ""),
            ("/w/proj/bom/package.json", "\u{FEFF}{\"type\": \"module\"}"),
            ("/w/proj/bom/x.js", ""),
            (
                "/w/proj/side/package.json",
                r#"{"type": "module", "sideEffects": false}"#,
            ),
            ("/w/proj/side/x.js", ""),
            ("/w/proj/empty/package.json", "{}"),
            ("/w/proj/empty/x.mjs", ""),
            // a nameless package.json that isn't a marker, which ends the search
            (
                "/w/proj/deps/package.json",
                r#"{"dependencies": {"t": "^2.0.0"}}"#,
            ),
            ("/w/proj/deps/x.mjs", ""),
            (
                "/w/proj/imports/package.json",
                r##"{"type": "module", "imports": {"#x": "./x.js"}}"##,
            ),
            ("/w/proj/imports/x.js", ""),
            // a "name" or "type" that isn't a string, invalid on every read
            (
                "/w/proj/name_number/package.json",
                r#"{"name": 5, "type": "module"}"#,
            ),
            ("/w/proj/name_number/a.js", ""),
            ("/w/proj/name_number/x.mjs", ""),
            ("/w/proj/type_number/package.json", r#"{"type": 5}"#),
            ("/w/proj/type_number/a.js", ""),
            ("/w/proj/name_bool/package.json", r#"{"name": true}"#),
            ("/w/proj/name_bool/inner/package.json", MODULE),
            ("/w/proj/name_bool/inner/x.mjs", ""),
            (
                "/w/proj/node_modules/name_null/package.json",
                r#"{"name": null, "exports": "./i.mjs"}"#,
            ),
            (
                "/w/proj/node_modules/name_array/package.json",
                r#"{"name": ["x"], "exports": "./i.mjs"}"#,
            ),
            (
                "/w/proj/node_modules/type_array/package.json",
                r#"{"type": ["module"], "exports": "./i.mjs"}"#,
            ),
            // what Node accepts: an empty name, and a "main", "imports", or "version" of any type
            (
                "/w/proj/name_empty/package.json",
                r#"{"name": "", "type": "module"}"#,
            ),
            ("/w/proj/name_empty/a.js", ""),
            (
                "/w/proj/node_modules/loose_fields/package.json",
                r#"{"main": 5, "imports": 5, "version": 5, "exports": "./i.mjs"}"#,
            ),
            // a named nested project, which ends the search
            (
                "/w/proj/inner/package.json",
                r#"{"name": "inner", "type": "module"}"#,
            ),
            ("/w/proj/inner/x.js", ""),
            // a package.json on the search that isn't JSON, under a marker scope
            ("/w/proj/broken/package.json", "{"),
            ("/w/proj/broken/inner/package.json", MODULE),
            ("/w/proj/broken/inner/x.mjs", ""),
            // outside any package.json
            ("/loose/a.js", ""),
            ("/loose/b.mjs", ""),
        ]);
        for file in [
            "/w/node_modules/parent_only/i.js",
            "/w/node_modules/no_package/i.mjs",
            "/w/proj/node_modules/sugar_string/i.mjs",
            "/w/proj/node_modules/sugar_array/i.mjs",
            "/w/proj/node_modules/sugar_conditions/i.mjs",
            "/w/proj/node_modules/proj/self.js",
            "/w/proj/node_modules/name_null/i.mjs",
            "/w/proj/node_modules/name_array/i.mjs",
            "/w/proj/node_modules/type_array/i.mjs",
            "/w/proj/node_modules/loose_fields/i.mjs",
        ] {
            files.insert(file, "");
        }
        MemoryTree {
            files,
            dirs: vec!["/w/proj/node_modules/t/lib/dir.js"],
            links: BTreeMap::from([
                ("/w/proj/node_modules/linked", "/w/linked"),
                (
                    "/w/proj/node_modules/nested",
                    "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested",
                ),
            ]),
            loops: vec!["/w/proj/node_modules/looped"],
        }
    }

    fn package(specifier: &str) -> Result<Resolution, ResolveErrorKind> {
        let importer = Url::parse("file:///w/proj/main.js").unwrap();
        resolve(&packages(), specifier, &importer, &Origin::Project).map_err(|error| error.kind)
    }

    /// The cases checked against Node's resolver: the URL a specifier resolves
    /// to, or the parts of its error's `Debug` form.
    #[test]
    fn exports_resolve_as_node_does() {
        let t = "file:///w/proj/node_modules/t";
        let cases: &[(&str, Result<String, &[&str]>)] = &[
            ("t", Ok(format!("{t}/main.js"))),
            ("t/up", Err(&["InvalidTarget", "./up", "NotRelative"])),
            ("t/bare", Err(&["InvalidTarget", "NotRelative"])),
            ("t/abs", Err(&["InvalidTarget", "NotRelative"])),
            ("t/https", Err(&["InvalidTarget", "NotRelative"])),
            ("t/dotdot", Err(&["InvalidTarget", "Segment"])),
            ("t/nm", Err(&["InvalidTarget", "Segment"])),
            ("t/encoded", Err(&["InvalidTarget", "Segment"])),
            ("t/encoded_nm", Err(&["InvalidTarget", "Segment"])),
            // Node: a directory import
            ("t/dir", Err(&["Target", "NoExtension"])),
            ("t/dir_slash", Err(&["Target", "NoExtension"])),
            // Node loads it: the extension is mandatory here
            ("t/no_extension", Err(&["Target", "NoExtension"])),
            (
                "t/missing",
                Err(&["Target", "NotFound { ts_sibling: None }"]),
            ),
            ("t/ts", Err(&["Target", "only_ts.ts"])),
            // Node allows empty segments, with a deprecation warning
            ("t/empty_segment", Ok(format!("{t}/lib/a.js"))),
            ("t/fall", Ok(format!("{t}/d.js"))),
            ("t/blocked", Err(&["NotExported", "./blocked"])),
            // Node matches its `node` condition
            (
                "t/unmatched",
                Err(&["NoMatchingCondition", "\"node\", \"require\""]),
            ),
            ("t/numeric", Err(&["InvalidPackageJson", "numeric"])),
            // a two-star key never matches
            ("t/two/a/b", Err(&["NotExported"])),
            ("t/array_invalid_first", Ok(format!("{t}/d.js"))),
            (
                "t/array_missing_first",
                Err(&["Target", "missing.js", "NotFound"]),
            ),
            ("t/array_null_first", Ok(format!("{t}/d.js"))),
            (
                "t/array_invalid_after_null",
                Err(&["InvalidTarget", "NotRelative"]),
            ),
            ("t/array_null_after_invalid", Err(&["NotExported"])),
            (
                "t/array_unmatched",
                Err(&["NoMatchingCondition", "unmatched: [\"node\"]"]),
            ),
            // Node matches `node` and fails on the missing file
            ("t/array_object", Ok(format!("{t}/d.js"))),
            ("t/array_empty", Err(&["NotExported"])),
            ("t/number", Err(&["InvalidTarget", "NotATarget"])),
            ("t/order", Ok(format!("{t}/d.js"))),
            ("t/import", Ok(format!("{t}/b.js"))),
            ("t/types", Ok(format!("{t}/b.js"))),
            ("t/lib/a", Ok(format!("{t}/lib/a.js"))),
            // the longer prefix wins
            ("t/lib/a/b", Ok(format!("{t}/lib/a/b.js"))),
            ("t/star/a.js", Ok(format!("{t}/lib/a.js"))),
            ("t/star/a", Err(&["NotExported"])),
            ("t/everywhere/x", Ok(format!("{t}/lib/x/x.js"))),
            // Node loads it as CommonJS
            ("t/cjs", Err(&["Target", "UnsupportedExtension(\"cjs\")"])),
            ("t/lib/%2F..%2Fa", Err(&["InvalidUrl", "%2F"])),
            ("t/lib/..%2Fa", Err(&["InvalidUrl", "%2F"])),
            ("t/lib/x/../a", Err(&["InvalidSubpath", "x/../a"])),
            ("t/lib/node_modules/q", Err(&["InvalidSubpath"])),
            ("t/lib/%6eode_modules/q", Err(&["InvalidSubpath"])),
            ("t/lib/%2e%2e/a", Err(&["InvalidSubpath"])),
            ("t/pattern/x.js?v=1", Ok(format!("{t}/lib/x.js?v=1"))),
            ("t/pattern/x.js#frag", Ok(format!("{t}/lib/x.js#frag"))),
            ("t/pattern/dir.js", Err(&["Target", "NotAFile"])),
            // Node refuses to strip under node_modules
            ("t/src/s.ts", Ok(format!("{t}/src/s.ts"))),
            ("t/null_pattern/x", Err(&["NotExported"])),
            ("t/trailing/a.js", Err(&["NotExported"])),
            ("t/", Err(&["NotExported"])),
            ("t/lib/", Err(&["NotExported"])),
            ("t/pattern/", Err(&["NotExported"])),
            ("t/unknown", Err(&["NotExported", "./unknown"])),
            // Node: tabs are dropped, and the containment check refuses it
            ("t/tab", Err(&["InvalidTarget", "Segment"])),
            ("t/query_up", Err(&["InvalidTarget", "Outside"])),
            ("t/fragment_up", Err(&["InvalidTarget", "Outside"])),
            // Node loads node_modules/escape_tab.js
            ("t/tabbed/.\t./.\t./escape_tab", Err(&["InvalidSubpath"])),
            ("t/pattern/.\t./.\t./x", Err(&["InvalidSubpath"])),
            // Node: a directory import, of the package's parent
            ("t/dot_star/.?", Err(&["InvalidSubpath", ".?"])),
            (
                "t/empty_conditions",
                Err(&["NotExported", "./empty_conditions"]),
            ),
            ("t/array_empty_conditions", Err(&["NotExported"])),
            (
                "t/nested_unmatched",
                Err(&["NoMatchingCondition", "[\"gro.browser\", \"node\"]"]),
            ),
            (
                "t/nested_array_unmatched",
                Err(&["NoMatchingCondition", "[\"import.node\", \"node\"]"]),
            ),
            (
                "looped",
                Err(&["SymlinkLoop", "/w/proj/node_modules/looped"]),
            ),
            // Node's scope search stops at node_modules too, then it guesses the format
            (
                "./node_modules/loose.js",
                Err(&["NotModuleType { package_json: None }"]),
            ),
            (
                "sugar_string",
                Ok("file:///w/proj/node_modules/sugar_string/i.mjs".to_owned()),
            ),
            (
                "sugar_array",
                Ok("file:///w/proj/node_modules/sugar_array/i.mjs".to_owned()),
            ),
            (
                "sugar_conditions",
                Ok("file:///w/proj/node_modules/sugar_conditions/i.mjs".to_owned()),
            ),
            ("mixed", Err(&["InvalidPackageJson", "mix"])),
            // Node falls back to `main`
            ("exports_null", Err(&["NoExports"])),
            ("no_exports", Err(&["NoExports"])),
            ("empty_exports", Err(&["NotExported", "exported: []"])),
            ("not_json", Err(&["InvalidPackageJson", "EOF"])),
            (
                "not_object",
                Err(&["InvalidPackageJson", "not a JSON object"]),
            ),
            // Node loads its index.js, and neither looks further up
            (
                "no_package",
                Err(&["NoPackageJson", "/w/proj/node_modules/no_package"]),
            ),
            // Node loads it as CommonJS
            (
                "cjs",
                Err(&["Target", "NotModuleType", "/cjs/package.json"]),
            ),
            (
                "@scope/pkg",
                Ok("file:///w/proj/node_modules/@scope/pkg/i.mjs".to_owned()),
            ),
            (
                "@scope/pkg/x.mjs",
                Ok("file:///w/proj/node_modules/@scope/pkg/x.mjs".to_owned()),
            ),
            ("@scope/pkg/y.mjs", Err(&["NotExported", "./y.mjs"])),
            ("linked", Ok("file:///w/linked/i.js".to_owned())),
            (
                "nested",
                Ok(
                    "file:///w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested/i.mjs"
                        .to_owned(),
                ),
            ),
            // the importer's own package, not the stale copy in node_modules
            ("proj/self.js", Ok("file:///w/proj/self.js".to_owned())),
            // Node reaches the workspace root's node_modules
            (
                "parent_only",
                Err(&[
                    "PackageNotFound",
                    "package_json: \"/w/proj/package.json\"",
                    "above: Some(\"/w/package.json\")",
                ]),
            ),
            ("fs", Err(&["PackageNotFound", "\"fs\""])),
            (".x", Err(&["InvalidPackageName"])),
            ("@scope", Err(&["InvalidPackageName"])),
            ("@scope/", Err(&["InvalidPackageName"])),
            ("@scope/..", Err(&["InvalidPackageName"])),
            ("a%20b", Err(&["InvalidPackageName"])),
            ("a\\b", Err(&["InvalidPackageName"])),
            // Node guesses the format from the code
            (
                "./no_type/a.js",
                Err(&["NotModuleType", "/w/proj/no_type/package.json"]),
            ),
            (
                "../../loose/a.js",
                Err(&["NotModuleType { package_json: None }"]),
            ),
            ("../../loose/b.mjs", Ok("file:///loose/b.mjs".to_owned())),
            (
                "./name_number/a.js",
                Err(&["InvalidPackageJson", "\\\"name\\\" is not a string"]),
            ),
            (
                "./type_number/a.js",
                Err(&["InvalidPackageJson", "\\\"type\\\" is not a string"]),
            ),
            (
                "name_null",
                Err(&[
                    "InvalidPackageJson",
                    "name_null/package.json",
                    "\\\"name\\\"",
                ]),
            ),
            ("name_array", Err(&["InvalidPackageJson", "\\\"name\\\""])),
            ("type_array", Err(&["InvalidPackageJson", "\\\"type\\\""])),
            (
                "./name_empty/a.js",
                Ok("file:///w/proj/name_empty/a.js".to_owned()),
            ),
            (
                "loose_fields",
                Ok("file:///w/proj/node_modules/loose_fields/i.mjs".to_owned()),
            ),
        ];
        for (specifier, expected) in cases {
            match (package(specifier), expected) {
                (Ok(resolution), Ok(url)) => {
                    assert_eq!(resolution.url.as_str(), url, "{specifier}");
                }
                (Err(kind), Err(parts)) => {
                    let debug = format!("{kind:?}");
                    for part in *parts {
                        assert!(debug.contains(part), "{specifier}: {debug}");
                    }
                }
                (actual, expected) => panic!("{specifier}: {actual:?}, expected {expected:?}"),
            }
        }
    }

    #[test]
    fn package_origins_and_consulted_files() {
        let resolution = package("t").unwrap();
        assert_eq!(
            resolution.origin,
            Origin::Package {
                name: "t".to_owned(),
                version: Some("2.3.4".to_owned()),
                linked: false
            }
        );
        let consulted = |files: &[(&str, FileRole)]| {
            files
                .iter()
                .map(|&(path, role)| ConsultedFile {
                    path: PathBuf::from(path),
                    role,
                })
                .collect::<Vec<_>>()
        };
        // the importer's scope (a self-reference's), the package's exports, then the
        // module's scope, which is the same file in another role
        assert_eq!(
            resolution.files_consulted,
            consulted(&[
                ("/w/proj/package.json", FileRole::PackageScope),
                ("/w/proj/node_modules/t/package.json", FileRole::Exports),
                (
                    "/w/proj/node_modules/t/package.json",
                    FileRole::PackageScope
                ),
            ])
        );
        let resolution = package("linked").unwrap();
        assert_eq!(
            resolution.origin,
            Origin::Package {
                name: "linked".to_owned(),
                version: Some("0.1.0".to_owned()),
                linked: true
            }
        );
        // the real paths, not the symlink's
        assert_eq!(
            resolution.files_consulted[1].path,
            Path::new("/w/linked/package.json")
        );
        // a nested layout's real root is inside node_modules, so it isn't linked
        let resolution = package("nested").unwrap();
        assert_eq!(
            resolution.origin,
            Origin::Package {
                name: "nested".to_owned(),
                version: Some("1.0.0".to_owned()),
                linked: false
            }
        );
        // a self-reference keeps the importer's origin
        let resolution = package("proj/self.js").unwrap();
        assert_eq!(resolution.origin, Origin::Project);
        assert_eq!(
            resolution.files_consulted,
            consulted(&[
                ("/w/proj/package.json", FileRole::PackageScope),
                ("/w/proj/package.json", FileRole::Exports),
            ])
        );
    }

    #[test]
    fn packages_resolve_their_own_imports_from_where_they_live() {
        let tree = packages();
        let from = |importer: &str, specifier: &str| {
            let importer = Url::parse(importer).unwrap();
            resolve(&tree, specifier, &importer, &Origin::Project).map(|resolution| resolution.url)
        };
        // a linked package's dependency, from its own node_modules, past the project's
        assert_eq!(
            from("file:///w/linked/i.js", "linked_dep")
                .unwrap()
                .as_str(),
            "file:///w/linked/node_modules/linked_dep/i.mjs"
        );
        // a linked package is a project root of its own
        assert_eq!(
            from("file:///w/linked/i.js", "t").unwrap_err().kind,
            ResolveErrorKind::PackageNotFound {
                name: "t".to_owned(),
                bound: Some(Box::new(PackageSearchBound {
                    dir: PathBuf::from("/w/linked"),
                    package_json: PathBuf::from("/w/linked/package.json"),
                    above: Some(PathBuf::from("/w/package.json")),
                })),
            }
        );
        // inside node_modules, the search goes on past the package's root
        assert_eq!(
            from("file:///w/proj/node_modules/needs_hoisted/i.mjs", "hoisted")
                .unwrap()
                .as_str(),
            "file:///w/proj/node_modules/hoisted/i.mjs"
        );
        assert_eq!(
            from(
                "file:///w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested/i.mjs",
                "nested_dep"
            )
            .unwrap()
            .as_str(),
            "file:///w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested_dep/i.mjs"
        );
        // and still ends at the project root
        assert_eq!(
            from("file:///w/proj/node_modules/hoisted/i.mjs", "parent_only")
                .unwrap_err()
                .kind,
            ResolveErrorKind::PackageNotFound {
                name: "parent_only".to_owned(),
                bound: Some(Box::new(PackageSearchBound {
                    dir: PathBuf::from("/w/proj"),
                    package_json: PathBuf::from("/w/proj/package.json"),
                    above: Some(PathBuf::from("/w/package.json")),
                })),
            }
        );
        // with no project root, it reaches the filesystem's root
        assert_eq!(
            from("file:///loose/b.mjs", "t").unwrap_err().kind,
            ResolveErrorKind::PackageNotFound {
                name: "t".to_owned(),
                bound: None,
            }
        );
    }

    /// The search ends at the first `package.json` outside `node_modules`
    /// that isn't a type marker, one holding only `"type"` and
    /// `"sideEffects"`, which is walked past.
    #[test]
    fn the_search_ends_at_a_package_json_that_is_not_a_type_marker() {
        let tree = packages();
        let from = |importer: &str, specifier: &str| {
            let importer = Url::parse(importer).unwrap();
            resolve(&tree, specifier, &importer, &Origin::Project)
        };
        let t = "file:///w/proj/node_modules/t/main.js";
        for importer in [
            "file:///w/proj/dist/esm/x.js",
            "file:///w/proj/scripts/x.mjs",
            "file:///w/proj/bom/x.js",
            "file:///w/proj/side/x.js",
            "file:///w/proj/empty/x.mjs",
        ] {
            assert_eq!(from(importer, "t").unwrap().url.as_str(), t, "{importer}");
        }
        // a nameless package.json that isn't a marker ends it, as an `npm i` root's does
        let bound = |dir: &str| {
            Some(Box::new(PackageSearchBound {
                dir: PathBuf::from(dir),
                package_json: PathBuf::from(format!("{dir}/package.json")),
                above: Some(PathBuf::from("/w/proj/package.json")),
            }))
        };
        assert_eq!(
            from("file:///w/proj/deps/x.mjs", "t").unwrap_err().kind,
            ResolveErrorKind::PackageNotFound {
                name: "t".to_owned(),
                bound: bound("/w/proj/deps"),
            }
        );
        let error = from("file:///w/proj/imports/x.js", "t").unwrap_err();
        assert_eq!(
            error.kind,
            ResolveErrorKind::PackageNotFound {
                name: "t".to_owned(),
                bound: bound("/w/proj/imports"),
            }
        );
        assert_eq!(
            error.to_string(),
            "cannot resolve 't' from file:///w/proj/imports/x.js: no package 't' is installed: \
             no node_modules/t directory from the importer up to /w/proj/imports/package.json, \
             which ends the search, as every package.json outside node_modules does except one \
             holding nothing but \"type\" and \"sideEffects\", and the node_modules beside \
             /w/proj/package.json is not searched (npm and yarn workspaces, which install at \
             their root, aren't supported). Fix: install it into /w/proj/imports/node_modules"
        );
        // the marker is the scope, read once and consulted in both roles, and
        // the project's package.json ends no search that finds the package
        assert_eq!(
            from("file:///w/proj/dist/esm/x.js", "t")
                .unwrap()
                .files_consulted[..2],
            [
                ConsultedFile {
                    path: PathBuf::from("/w/proj/dist/esm/package.json"),
                    role: FileRole::PackageScope
                },
                ConsultedFile {
                    path: PathBuf::from("/w/proj/dist/esm/package.json"),
                    role: FileRole::SearchBound
                },
            ]
        );
        // a named nested project ends it, with the workspace's root above unsearched
        assert_eq!(
            from("file:///w/proj/inner/x.js", "t").unwrap_err().kind,
            ResolveErrorKind::PackageNotFound {
                name: "t".to_owned(),
                bound: Some(Box::new(PackageSearchBound {
                    dir: PathBuf::from("/w/proj/inner"),
                    package_json: PathBuf::from("/w/proj/inner/package.json"),
                    above: Some(PathBuf::from("/w/proj/package.json")),
                })),
            }
        );
        // a package.json on the search that isn't JSON is an error, not passed over
        assert!(matches!(
            from("file:///w/proj/broken/inner/x.mjs", "t").unwrap_err().kind,
            ResolveErrorKind::InvalidPackageJson { path, .. }
                if path == Path::new("/w/proj/broken/package.json")
        ));
        // and so is one whose "name" isn't a string, which Node never reads here
        assert_eq!(
            from("file:///w/proj/name_bool/inner/x.mjs", "t")
                .unwrap_err()
                .kind,
            ResolveErrorKind::InvalidPackageJson {
                path: PathBuf::from("/w/proj/name_bool/package.json"),
                reason: "its \"name\" is not a string".to_owned(),
            }
        );
        // as the importer's scope, read for a self-reference, as Node reads it
        assert_eq!(
            from("file:///w/proj/name_number/x.mjs", "t")
                .unwrap_err()
                .kind,
            ResolveErrorKind::InvalidPackageJson {
                path: PathBuf::from("/w/proj/name_number/package.json"),
                reason: "its \"name\" is not a string".to_owned(),
            }
        );
    }

    /// A tree that records the path of each read.
    struct CountingTree {
        tree: MemoryTree,
        reads: std::cell::RefCell<Vec<PathBuf>>,
    }

    impl SourceTree for CountingTree {
        fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
            self.tree.lookup(path)
        }

        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.reads.borrow_mut().push(path.to_owned());
            self.tree.read(path)
        }
    }

    /// The importer's scope, read for a self-reference, isn't read again
    /// when the search passes its directory.
    #[test]
    fn the_scope_is_read_once() {
        let tree = CountingTree {
            tree: packages(),
            reads: std::cell::RefCell::default(),
        };
        let importer = Url::parse("file:///w/proj/dist/esm/x.js").unwrap();
        resolve(&tree, "t", &importer, &Origin::Project).unwrap();
        let reads = tree.reads.borrow();
        let scope_reads = reads
            .iter()
            .filter(|path| *path == Path::new("/w/proj/dist/esm/package.json"))
            .count();
        assert_eq!(scope_reads, 1, "{reads:?}");
        // the search passes the next marker up, reading it
        assert!(
            reads.contains(&PathBuf::from("/w/proj/dist/package.json")),
            "{reads:?}"
        );
    }

    /// A `package.json` that is a FIFO is refused by its type, never opened,
    /// so the read can't block: a package's, and a module's scope.
    #[test]
    fn a_special_file_package_json_is_refused() {
        let root = std::env::temp_dir().join(format!("gro_resolver_fifo_{}", std::process::id()));
        std::fs::create_dir_all(root.join("node_modules/fifo")).unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name": "r", "type": "module"}"#,
        )
        .unwrap();
        std::fs::write(root.join("main.mjs"), "").unwrap();
        std::fs::write(root.join("sub/a.js"), "").unwrap();
        for fifo in ["node_modules/fifo/package.json", "sub/package.json"] {
            let status = std::process::Command::new("mkfifo")
                .arg(root.join(fifo))
                .status()
                .expect("coreutils `mkfifo` is required for the special file test");
            assert!(status.success(), "mkfifo {fifo}");
        }
        let real_root = root.canonicalize().unwrap();
        let importer = Url::from_file_path(real_root.join("main.mjs")).unwrap();
        let from = |specifier| resolve(&DiskTree, specifier, &importer, &Origin::Project);
        let package = from("fifo");
        let scope = from("./sub/a.js");
        // removed before any assertion, so a failure leaves no FIFO behind
        std::fs::remove_dir_all(&root).unwrap();
        let package = package.unwrap_err().kind;
        let scope = scope.unwrap_err().kind;
        let invalid = |path: &str| ResolveErrorKind::InvalidPackageJson {
            path: real_root.join(path),
            reason: "it is not a regular file (a FIFO, a socket, or a device)".to_owned(),
        };
        assert_eq!(package, invalid("node_modules/fifo/package.json"));
        assert_eq!(scope, invalid("sub/package.json"));
        assert_eq!(
            package.to_string(),
            format!(
                "{}/node_modules/fifo/package.json is not a valid package.json: it is not a \
                 regular file (a FIFO, a socket, or a device). Fix: correct it, or reinstall the \
                 package",
                real_root.display()
            )
        );
    }

    #[test]
    fn package_json_reads() {
        // a byte order mark is skipped, as Node skips it
        assert_eq!(
            package("./bom/x.js").unwrap().url.as_str(),
            "file:///w/proj/bom/x.js"
        );
        // one too large is refused by its size
        let mut tree = packages();
        let large = "x".repeat(usize::try_from(MAX_PACKAGE_JSON_SIZE).unwrap() + 1);
        tree.files
            .insert("/w/proj/node_modules/large/package.json", large.leak());
        let importer = Url::parse("file:///w/proj/main.js").unwrap();
        let error = resolve(&tree, "large", &importer, &Origin::Project).unwrap_err();
        assert_eq!(
            error.kind,
            ResolveErrorKind::InvalidPackageJson {
                path: PathBuf::from("/w/proj/node_modules/large/package.json"),
                reason: "it is larger than 16 MiB, the most a package.json may be".to_owned(),
            }
        );
    }

    #[test]
    fn disk_reads_are_bounded() {
        let path =
            std::env::temp_dir().join(format!("gro_resolver_bounded_{}.json", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        // sparse, so nothing is written
        file.set_len(MAX_PACKAGE_JSON_SIZE + 1).unwrap();
        let too_large = DiskTree.read(&path).unwrap_err().kind();
        file.set_len(MAX_PACKAGE_JSON_SIZE).unwrap();
        let at_most = DiskTree.read(&path).map(|bytes| bytes.len() as u64);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(too_large, io::ErrorKind::FileTooLarge);
        assert_eq!(at_most.unwrap(), MAX_PACKAGE_JSON_SIZE);
    }

    #[test]
    fn pattern_keys_compare_as_node_does() {
        use std::cmp::Ordering;
        assert_eq!(pattern_key_compare("", "./*"), Ordering::Greater);
        assert_eq!(pattern_key_compare("./*", "./a/*"), Ordering::Greater);
        assert_eq!(pattern_key_compare("./a/*", "./*"), Ordering::Less);
        // the same prefix: the longer trailer wins
        assert_eq!(pattern_key_compare("./*", "./*.js"), Ordering::Greater);
        assert_eq!(pattern_key_compare("./*.js", "./*.js"), Ordering::Equal);
    }

    #[test]
    fn segments() {
        for path in [
            ".",
            "..",
            "a/./b",
            "a/../b",
            "%2e",
            "%2E%2e",
            "node_modules",
            "NODE_MODULES",
        ] {
            assert!(has_invalid_segment(path), "{path}");
        }
        for path in [
            "a/b",
            "",
            "a//b",
            "...",
            ".a",
            "%2e%2e%2e",
            "node_modules_x",
            "%2",
        ] {
            assert!(!has_invalid_segment(path), "{path}");
        }
        assert!(has_invalid_segment("a\\..\\b"));
        // tabs and line breaks are removed first, as URL parsing removes them
        for path in [".\t.", "a/.\n./b", "\r.", "%2e\t%2e", "node\t_modules"] {
            assert!(has_invalid_segment(path), "{path:?}");
        }
        assert!(is_array_index("0") && is_array_index("12"));
        assert!(!is_array_index("01") && !is_array_index("-1") && !is_array_index("4294967295"));
        assert!(!is_array_index("") && !is_array_index("a1"));
    }

    #[test]
    fn package_messages() {
        let message = |specifier| package(specifier).unwrap_err().to_string();
        assert_eq!(
            message("parent_only"),
            "no package 'parent_only' is installed: no node_modules/parent_only directory from \
             the importer up to /w/proj/package.json, which ends the search, as every \
             package.json outside node_modules does except one holding nothing but \"type\" and \
             \"sideEffects\", and the node_modules beside /w/package.json is not searched (npm \
             and yarn workspaces, which install at their root, aren't supported). Fix: install it \
             into /w/proj/node_modules"
        );
        let importer = Url::parse("file:///loose/b.mjs").unwrap();
        assert_eq!(
            resolve(&packages(), "t", &importer, &Origin::Project)
                .unwrap_err()
                .kind
                .to_string(),
            "no package 't' is installed: no node_modules/t directory in any directory above \
             the importer. Fix: install it into node_modules"
        );
        assert_eq!(
            message("t/nested_unmatched"),
            "no condition matches for subpath './nested_unmatched': the package's conditions \
             [gro.browser, node] don't, and only gro, import, default are matched. Fix: add a \
             \"gro\" or \"import\" condition before the others"
        );
        assert!(message("t/empty_conditions").starts_with(
            "subpath './empty_conditions' is not exported by \
             /w/proj/node_modules/t/package.json (exports: ., ./up, "
        ));
        assert_eq!(
            message("looped"),
            "/w/proj/node_modules/looped leads into a symlink loop, so it names no file or \
             directory. Fix: remove the looping link, or point it at what it should name (for a \
             package, reinstall it)"
        );
        assert_eq!(
            message("t/dot_star/.?"),
            "the part of the subpath the pattern './dot_star/*' matches, '.?', has a '.', '..', \
             or 'node_modules' segment, percent-encoded or not, or takes the target out of the \
             package's directory, and a pattern never substitutes those. Fix: import the module \
             by its plain subpath"
        );
        assert_eq!(
            message("no_exports"),
            "/w/proj/node_modules/no_exports/package.json has no \"exports\" field, and only \
             \"exports\" is read (not \"main\" or \"module\"). Fix: use a version with an ESM \
             build, vendor an ESM bundle of it into the project, or run it under Node"
        );
        assert_eq!(
            message("@scope/pkg/y.mjs"),
            "subpath './y.mjs' is not exported by /w/proj/node_modules/@scope/pkg/package.json \
             (exports: ., ./x.mjs). Fix: import an exported subpath, or add one to the package's \
             \"exports\""
        );
        assert_eq!(
            message("t/unmatched"),
            "no condition matches for subpath './unmatched': the package's conditions [node, \
             require] don't, and only gro, import, default are matched. Fix: add a \"gro\" or \
             \"import\" condition before the others"
        );
        assert_eq!(
            message("t/up"),
            "the \"exports\" target \"../escape.js\" for './up' is invalid: a target must start \
             with './'. Fix: use a version of the package whose \"exports\" are valid"
        );
        assert_eq!(
            message("t/ts"),
            "the package exports it as /w/proj/node_modules/t/only_ts.js, which does not exist \
             ('/w/proj/node_modules/t/only_ts.ts' does). Fix: reinstall the package"
        );
        assert_eq!(
            message("cjs"),
            "the package exports it as /w/proj/node_modules/cjs/i.js, which is CommonJS (no \
             \"type\": \"module\", not .mjs), and only ES modules load. Fix: use a version with \
             an ESM build, vendor an ESM bundle of it into the project, or run it under Node"
        );
        assert_eq!(
            message("t/no_extension"),
            "the package exports it as /w/proj/node_modules/t/no_extension: the path has no \
             extension; extensions and directory indexes are never probed"
        );
        assert_eq!(
            message("./no_type/a.js"),
            "a .js file is an ES module only under \"type\": \"module\", which \
             /w/proj/no_type/package.json does not set, and CommonJS is not supported. Fix: add \
             \"type\": \"module\" to it, or rename the file to .mjs"
        );
        assert!(message("../../loose/a.js").contains("Fix: rename the file to .mjs"));
        assert!(message("@scope").contains("not a valid package name"));
        assert!(message("no_package").contains("has no package.json"));
        assert!(message("empty_exports").contains("(its \"exports\" are empty)"));
    }
}
