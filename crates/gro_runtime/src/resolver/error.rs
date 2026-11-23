//! Why a specifier didn't resolve, and the message each reason carries.

use std::fmt;
use std::io;
use std::path::PathBuf;

use super::CONDITIONS;
use super::builtin::{GRO_PACKAGE, Reserved, UNBUILT_HOST_MODULES};
use super::roots::{Origin, Root};
use super::tree::ReadRefusal;
use crate::BINARY_NAME;

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

/// Why a specifier didn't resolve. Each message reads after
/// [`ResolveError`]'s, which names the specifier and its importer.
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
    /// A `#` specifier that names no mapping: `#` alone, or one ending in
    /// `/`, as Node refuses them.
    InvalidImportSpecifier,
    /// A `#` specifier from a module with no `package.json` at or above its
    /// directory, below any `node_modules` and any world-writable directory,
    /// whose `"imports"` it would resolve through.
    NoImportScope {
        /// The world-writable directory the search stopped at, reading
        /// nothing in it; unset when it reached the filesystem's root or a
        /// `node_modules` directory.
        stopped_at: Option<PathBuf>,
    },
    /// A `#` specifier the nearest `package.json` above its importer doesn't
    /// map: it has no `"imports"` object, no key matches, or the target is
    /// `null` or has no conditions.
    ImportNotDefined {
        /// The nearest `package.json`'s real path.
        package_json: PathBuf,
        /// Its `"imports"` keys, unset when it has no `"imports"` object.
        defined: Option<Vec<String>>,
    },
    /// A `#` specifier whose `"imports"` target names a package or a builtin
    /// module rather than a file: not a URL, and, a pattern's match
    /// substituted, a valid package name not starting with `#`.
    ImportsPackage {
        /// The `"imports"` key that matched.
        key: String,
        /// The target, as JSON.
        target: Box<str>,
        /// What the target names, a pattern's match substituted.
        specifier: Box<str>,
    },
    /// A specifier the builtin table reserves without serving: the `.js`
    /// spelling of a builtin module's, a host module's not built yet, a
    /// module's whose members are modules of their own, or a respelling of
    /// any of these or of a builtin module's. It's refused ahead of
    /// any package lookup, so it never resolves to a package, gro's own Node
    /// implementation included.
    Reserved(Reserved),
    /// A specifier of Gro's package, or an embedded module's relative one,
    /// naming a module of Gro's library the run doesn't serve. Gro's
    /// package is the binary's own, so such a specifier never resolves to a
    /// package, Gro's Node copy included.
    GroModule {
        /// The module's path in Gro's `src/lib`, as the specifier names it.
        path: Box<str>,
        /// Why it isn't served.
        fault: GroModuleFault,
    },
    /// A specifier of Gro's package that isn't a builtin, in a run that
    /// holds no copy of Gro's library: its embedder handed none in, as
    /// eval's doesn't, or the copy failed to load, for the reason given.
    NoGroLibrary(Option<Box<str>>),
    /// A specifier of an embedded module that leaves Gro's library: a
    /// relative one above `src/lib`, or one that is neither relative nor
    /// bare (absolute, a URL, a `#` specifier). An embedded module reaches
    /// Gro's other modules only by relative specifiers, and packages by
    /// name.
    OutsideGroLibrary,
    /// An embedded module's bare import that resolves to the project's own
    /// modules, a package of that name linked to the project's directory.
    /// The project's modules hold its grants, and Gro's library, which
    /// holds none, never imports them by name.
    GroImportsProject {
        /// The module's path, as the project's `"exports"` name it.
        path: PathBuf,
        /// The project root's real directory.
        project: PathBuf,
    },
    /// A package Gro's library imports that the project doesn't hold as Gro
    /// needs it: one of Gro's peer dependencies, missing or at a version
    /// outside its range, or one Gro doesn't declare.
    GroPeer {
        /// The package's name.
        name: Box<str>,
        /// What's wrong, boxed: a failure is the rare path, and it would
        /// size every resolution's error.
        fault: Box<PeerFault>,
    },
    /// The empty string.
    Empty,
    /// A specifier, its resolved URL, or its importer that isn't a valid
    /// file URL.
    InvalidUrl(UrlFault),
    /// A bare specifier whose package name is malformed: it starts with `.`,
    /// holds `%` or `\`, has an empty, `.`, or `..` segment, or is a scope
    /// with no name after it.
    InvalidPackageName,
    /// A bare specifier, other than a self-reference, from a module with no
    /// project root above it: no `package.json` outside `node_modules` that
    /// isn't a type marker, at or above the importer's directory and below
    /// any world-writable directory. No `node_modules` is searched without
    /// one.
    NoProjectRoot {
        /// The importer's directory, where the walk started.
        dir: PathBuf,
        /// The world-writable directory the walk stopped at, reading nothing
        /// in it; unset when it reached the filesystem's root.
        stopped_at: Option<PathBuf>,
    },
    /// No `node_modules/<name>` directory was found, from the importer up to
    /// the project root.
    PackageNotFound {
        /// The package name.
        name: String,
        /// The project root that bounded the search.
        bound: Box<PackageSearchBound>,
    },
    /// A package the search found through a world-writable directory: the
    /// `node_modules` it is in, its scope's directory, its own root, for a
    /// linked package the real directories holding it (up to the real
    /// `node_modules` it was found through or the importer's directories,
    /// else as many as its name has segments), or for a linked
    /// `node_modules` or scope directory its real parent. Any user can plant
    /// or replace a package there, so nothing of the package is read.
    WorldWritablePackage {
        /// The package name.
        name: String,
        /// The world-writable directory, the first checked of those that are
        /// (the `node_modules`, then the scope's, then a linked
        /// `node_modules`'s real parent, then a linked scope directory's,
        /// then a linked root's holders, outermost first, then the root).
        dir: PathBuf,
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
        /// The field the target is in.
        field: MapField,
        /// The subpath, `.` or `./…`, or the `#` specifier.
        subpath: String,
        /// The conditions that didn't match, in the package's order, a
        /// nested one after its matched parent and a dot (`gro.browser`).
        unmatched: Vec<String>,
    },
    /// The `"exports"` or `"imports"` target the subpath selected is
    /// invalid.
    InvalidTarget {
        /// The field the target is in.
        field: MapField,
        /// The key that matched.
        key: String,
        /// The target, as JSON.
        target: String,
        /// Why it's invalid.
        reason: TargetFault,
    },
    /// The part of the subpath a pattern's `*` matched has a `.`, `..`, or
    /// `node_modules` segment, or in `"imports"` spells one with the target
    /// around it, or substituted into the target, it makes an
    /// empty segment the target doesn't have (as a subpath ending in `/`
    /// does before a `/` in the target) or leaves the directory of the
    /// `package.json`.
    InvalidSubpath {
        /// The field the pattern is in.
        field: MapField,
        /// The pattern that matched.
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
    /// The `"imports"` of `package_json` map a `#` specifier to `path`, and
    /// loading it fails with `error`.
    ImportTarget {
        /// The `package.json` whose `"imports"` map it.
        package_json: PathBuf,
        /// The file they map it to.
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
    /// A `.js` file whose nearest `package.json` doesn't set `"type":
    /// "module"`, so it isn't an ES module.
    NotModuleType {
        /// The `package.json` that doesn't set `"type": "module"`.
        package_json: PathBuf,
    },
    /// A `.js` file with no `package.json` above it to set `"type":
    /// "module"`, so it isn't an ES module.
    NoTypeScope {
        /// The world-writable directory the search stopped at, reading
        /// nothing in it; unset when it reached the filesystem's root or a
        /// `node_modules` directory.
        stopped_at: Option<PathBuf>,
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
    /// The module's real path is outside the root it must stay inside: the
    /// importer's, for a relative or absolute specifier; the package's, for
    /// an `"exports"` target; and for a self-reference, its package's
    /// directory and the importer's root both.
    OutsideRoot {
        /// The module's real path, or its path as written when that is
        /// already outside the root.
        path: PathBuf,
        /// The root's real directory.
        root: PathBuf,
        /// The package's name when the root is an installed package's, not
        /// linked, whose files are the package's to fix, not the user's.
        package: Option<Box<str>>,
    },
    /// The module's path has a `node_modules` segment below the root it must
    /// stay inside, by its real path or as written. Only a bare specifier
    /// reaches into `node_modules`, through the package's `"exports"`.
    IntoNodeModules {
        /// The module's path: its real path, or the path as written when
        /// that is where the segment is.
        path: PathBuf,
        /// The root's real directory.
        root: PathBuf,
    },
    /// The module's real path is inside another admitted root, nested in the
    /// one it must stay inside: a package's, which only a bare specifier
    /// reaches, through its `"exports"`, or the project's, which a package
    /// never reaches.
    IntoPackageRoot {
        /// The module's real path.
        path: PathBuf,
        /// The nested root.
        root: Box<Root>,
    },
    /// A package's module imports, by a bare specifier other than its own
    /// package's self-reference, a module of the project's own root: the
    /// project linked into `node_modules` (`node_modules/app -> ..`, a
    /// workspace sibling that depends back on the app). Relative and
    /// absolute specifiers stay inside the package's root, so a bare one is
    /// the only way across, and through one a package would reach the
    /// grants the project's modules hold. The project's own bare imports
    /// of itself are unaffected.
    PackageImportsProject {
        /// The importing package's name.
        package: Box<str>,
        /// The module's path, as the project's `"exports"` name it.
        path: PathBuf,
        /// The project root's real directory.
        project: PathBuf,
    },
    /// An entry inside `node_modules` but in no package: a file directly in
    /// it, or in a directory that names no package (`.bin`, `.deno`), so it
    /// has no origin.
    InNoPackage(PathBuf),
    /// A module recorded inside one root is inside another one admitted
    /// after it, so which root it has would depend on the import order: a
    /// file reached by a relative specifier and then admitted with its
    /// package, or the reverse. The module loader raises it, since the
    /// resolver records nothing.
    RootConflict {
        /// The module's real path.
        path: PathBuf,
        /// The root it was recorded inside.
        recorded: Box<Root>,
        /// The root admitted around it.
        admitted: Box<Root>,
    },
    /// The path leads into a symlink loop, so it names no file or directory.
    SymlinkLoop(PathBuf),
    /// Reading the tree failed.
    Io {
        /// The path that failed to read.
        path: PathBuf,
        /// The failure's kind.
        kind: io::ErrorKind,
        /// The failure's message, for display.
        message: String,
    },
    /// A file a resolution read is not, at its path, the one an earlier
    /// resolution in the run read there
    /// ([`ReadRefusal::ReplacedSinceFirstRead`]).
    ReadReplacedSinceFirstRead(PathBuf),
    /// A file a resolution read was replaced, or a directory on its path
    /// was, between its lookup and its read
    /// ([`ReadRefusal::ReplacedSinceLookup`]).
    ReadReplacedSinceLookup(PathBuf),
    /// A file a resolution read lies outside the box the reads are confined
    /// to ([`ReadRefusal::OutsideBox`]).
    ReadOutsideBox(PathBuf),
}

/// Why a package Gro's library imports isn't one it may load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerFault {
    /// Gro's `package.json` declares no peer dependency of the name, so
    /// no range says which versions Gro's library works with.
    Undeclared,
    /// The project root's `node_modules` holds no package of the name.
    Missing {
        /// The range Gro declares for it.
        range: Box<str>,
        /// The `node_modules` searched.
        node_modules: PathBuf,
    },
    /// The package's version is outside the range Gro declares, or isn't
    /// one a range can be checked against.
    OutOfRange {
        /// The range Gro declares for it.
        range: Box<str>,
        /// Its `package.json`'s `"version"`, unset when that isn't a
        /// string.
        version: Option<Box<str>>,
        /// The package's real root.
        dir: PathBuf,
    },
    /// The range Gro declares isn't one the resolver reads, which a copy
    /// the build made never holds.
    UnreadRange(Box<str>),
}

/// Why a module of Gro's library isn't served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroModuleFault {
    /// One of Gro's modules that run only on Node, which the binary leaves
    /// out of its copy.
    NodeOnly,
    /// No module of Gro's has the path.
    Missing,
    /// A module under `internal/`, which Gro's package doesn't export, as
    /// its `"exports"` null `./internal/*`.
    Internal,
    /// The `.js` spelling of an embedded module's `.ts` path.
    JsSpelling,
}

/// Why a specifier, its resolved URL, or its importer isn't a valid file
/// URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlFault {
    /// Joining the specifier to the importer, or an `"exports"` target to
    /// its package's root, failed.
    Join(url::ParseError),
    /// The URL names no local path.
    NoLocalPath,
    /// The importer's URL names no local path.
    ImporterNoLocalPath,
    /// The resolved path, as the URL writes it, has an encoded `/` or `\`
    /// (`%2F` or `%5C`), which would decode to a separator the URL never
    /// had.
    EncodedSeparator(String),
    /// The resolved path, as the URL writes it, has an encoded NUL (`%00`),
    /// which no path holds.
    EncodedNul(String),
    /// The entry's path, joined to the working directory, isn't absolute.
    NotAbsolute,
}

/// The project root that bounded a `node_modules` search: the first
/// `package.json` above the importer, outside `node_modules`, that isn't a
/// type marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSearchBound {
    /// Its directory, whose `node_modules` was the last searched.
    pub dir: PathBuf,
    /// Its `package.json`'s real path; none where the directory bounds the
    /// search as the holder of the `node_modules` the importer is in.
    pub package_json: Option<PathBuf>,
    /// The nearest `package.json` above it and below any world-writable
    /// directory, whose `node_modules` aren't searched, as an npm or yarn
    /// workspace's root's aren't.
    pub above: Option<PathBuf>,
}

/// The field of a `package.json` that maps specifiers to targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapField {
    /// `"exports"`, a package's subpaths.
    Exports,
    /// `"imports"`, the `#` specifiers of the modules below it.
    Imports,
}

impl fmt::Display for MapField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Exports => "exports",
            Self::Imports => "imports",
        })
    }
}

/// Why an `"exports"` or `"imports"` target is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFault {
    /// It doesn't start with `./`: a bare name, an absolute path, `../`, or
    /// a URL. An `"imports"` target naming a package is
    /// [`ResolveErrorKind::ImportsPackage`] instead, and one naming nothing
    /// is [`TargetFault::NamesNothing`].
    NotRelative,
    /// An `"imports"` target that doesn't start with `./`, `../`, or `/`,
    /// isn't a URL, and, a pattern's match substituted, names no package
    /// (`""`, `.`, `.x`, `@x`, a `#` specifier). Node resolves it
    /// as a bare specifier, never as a target, so an array doesn't fall back past it.
    NamesNothing,
    /// It has a `.`, `..`, or `node_modules` segment, percent-encoded or not.
    Segment,
    /// An `"imports"` target has an empty segment: a `//`, or a trailing `/`.
    EmptySegment,
    /// It resolves outside the directory of its `package.json`.
    Outside,
    /// It isn't a string, an array, an object, or `null`.
    NotATarget,
}

impl ResolveErrorKind {
    /// Whether the resolver refused the import for what the specifier names
    /// and the files on disk hold, a missing file included: the specifier, the
    /// project, or a package must change for it to resolve, the common case.
    /// Otherwise reading the tree failed (an I/O error, a symlink loop, a
    /// file replaced mid-run, a read refused) or the caller broke an
    /// invariant (an importer with no local path,
    /// [`UrlFault::ImporterNoLocalPath`]), which no change to the import
    /// fixes.
    pub const fn is_refusal(&self) -> bool {
        !matches!(
            self,
            Self::Io { .. }
                | Self::SymlinkLoop(_)
                | Self::ReadReplacedSinceFirstRead(_)
                | Self::ReadReplacedSinceLookup(_)
                | Self::ReadOutsideBox(_)
                | Self::InvalidUrl(UrlFault::ImporterNoLocalPath)
        )
    }

    /// Whether this refuses by a rule that guards the run's boundaries,
    /// rather than finding nothing there to load: a remote, inline, or other
    /// scheme (`https:`, `data:`, `ext:`, `builtin:`), whose module would have
    /// no path in a root to take an origin from, a
    /// name the builtin table reserves, a package found through a
    /// world-writable directory, a path or `"exports"` target that leaves its
    /// root or crosses into another's, a package's import of the project, a
    /// module of Gro's library reaching outside it or importing the project
    /// by name, a module whose origin is unclear, or a read outside the box.
    /// Nothing there is a missing file or package, a module of Gro's the run
    /// doesn't serve, a peer of Gro's missing or out of its range, a
    /// malformed specifier or package,
    /// a format or feature `fgro` doesn't load (a `.cts` file or a `.js` one
    /// outside `"type": "module"`, JSX, an unsupported extension, `node:`, a
    /// registry, a `#` specifier mapped to a package), or a failed read: the
    /// same for every importer, and what probing for an optional dependency
    /// meets. A rule's refusal of an `import()` fails an `fgro run` even when
    /// the program catches it.
    pub fn guards_a_boundary(&self) -> bool {
        match self {
            Self::Remote
            | Self::UnsupportedScheme(_)
            | Self::Reserved(_)
            | Self::InvalidUrl(UrlFault::EncodedSeparator(_) | UrlFault::EncodedNul(_))
            | Self::WorldWritablePackage { .. }
            | Self::InvalidTarget { .. }
            | Self::InvalidSubpath { .. }
            | Self::OutsideRoot { .. }
            | Self::IntoNodeModules { .. }
            | Self::IntoPackageRoot { .. }
            | Self::PackageImportsProject { .. }
            | Self::OutsideGroLibrary
            | Self::GroImportsProject { .. }
            | Self::InNoPackage(_)
            | Self::RootConflict { .. }
            | Self::ReadOutsideBox(_) => true,
            Self::Target { error, .. } | Self::ImportTarget { error, .. } => {
                error.guards_a_boundary()
            }
            Self::NodeBuiltin
            | Self::Registry
            | Self::InvalidImportSpecifier
            | Self::NoImportScope { .. }
            | Self::ImportNotDefined { .. }
            | Self::ImportsPackage { .. }
            | Self::GroModule { .. }
            | Self::GroPeer { .. }
            | Self::NoGroLibrary(_)
            | Self::Empty
            | Self::InvalidUrl(
                UrlFault::Join(_)
                | UrlFault::NoLocalPath
                | UrlFault::ImporterNoLocalPath
                | UrlFault::NotAbsolute,
            )
            | Self::InvalidPackageName
            | Self::NoProjectRoot { .. }
            | Self::PackageNotFound { .. }
            | Self::NoPackageJson(_)
            | Self::InvalidPackageJson { .. }
            | Self::NoExports(_)
            | Self::NotExported { .. }
            | Self::NoMatchingCondition { .. }
            | Self::NoExtension { .. }
            | Self::NotFound { .. }
            | Self::NotAFile
            | Self::CommonJs
            | Self::NotModuleType { .. }
            | Self::NoTypeScope { .. }
            | Self::Jsx
            | Self::Declaration
            | Self::SvelteModule
            | Self::UnsupportedExtension(_)
            | Self::SymlinkLoop(_)
            | Self::Io { .. }
            | Self::ReadReplacedSinceFirstRead(_)
            | Self::ReadReplacedSinceLookup(_) => false,
        }
    }
}

/// What every extension message lists as loading.
const LOADING_EXTENSIONS: &str = ".js, .mjs, .ts, and .mts modules load";

/// The fix for a package that can't load as an ES module.
const PACKAGE_FIX: &str = "Fix: use a version with an ESM build, vendor an ESM bundle of it into \
                           the project, or run it under Node";

/// The fix for an `"exports"` target whose real path leaves its package.
const TARGET_FIX: &str = "Fix: use a version of the package whose \"exports\" stay inside it";

impl fmt::Display for ResolveErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // the specifier is quoted by `ResolveError`, so these restate it only where the spec's wording does
        match self {
            Self::NodeBuiltin => f.write_str(
                "a node: specifier names a Node built-in, and there is no Node compat. \
                 Fix: import Gro's host module for it, or run this under Node",
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
            Self::InvalidImportSpecifier => f.write_str(
                "it is not a valid # specifier: '#' alone and a specifier ending in '/' map to \
                 nothing, as in Node. Fix: import a module the package.json's \"imports\" map, \
                 like '#lib/x.ts'",
            ),
            Self::NoImportScope { stopped_at } => {
                f.write_str(
                    "a # specifier resolves through the \"imports\" of the nearest package.json \
                     above its importer, and ",
                )?;
                match stopped_at {
                    Some(stopped_at) => write!(
                        f,
                        "the search for one stopped at {}, which is world-writable, so nothing in \
                         it or above it is read",
                        stopped_at.display()
                    )?,
                    None => f.write_str("none is above it")?,
                }
                f.write_str(
                    ". Fix: add a package.json with \"imports\" above it, or import the file by \
                     a relative path",
                )
            }
            Self::ImportNotDefined {
                package_json,
                defined: None,
            } => write!(
                f,
                "a # specifier resolves through the \"imports\" of the nearest package.json above \
                 its importer, {}, which has none, and one in a subdirectory hides those above it, \
                 as in Node. Fix: add the mapping to its \"imports\", or import the file by a \
                 relative path",
                package_json.display()
            ),
            Self::ImportNotDefined {
                package_json,
                defined: Some(defined),
            } => {
                write!(
                    f,
                    "it is not mapped by the \"imports\" of {} ",
                    package_json.display()
                )?;
                if defined.is_empty() {
                    f.write_str("(its \"imports\" are empty)")?;
                } else {
                    write!(f, "(imports: {})", defined.join(", "))?;
                }
                f.write_str(". Fix: import a specifier it maps, or add one to its \"imports\"")
            }
            Self::ImportsPackage {
                key,
                target,
                specifier,
            } => write!(
                f,
                "the \"imports\" target {target} for '{key}' names a package or a builtin module, \
                 and a # specifier maps only to a file in the directory of its package.json. Fix: \
                 import '{specifier}' by its name"
            ),
            Self::Reserved(Reserved::JsSpelling(builtin)) => write!(
                f,
                "it is the .js spelling of the builtin module '{}', reserved so it never \
                 resolves to a package. Fix: import '{0}'",
                builtin.specifier
            ),
            Self::Reserved(Reserved::Respelled(builtin)) => write!(
                f,
                "it respells the builtin module '{}', reserved so no spelling of it resolves to \
                 a package. Fix: import '{0}'",
                builtin.specifier
            ),
            Self::Reserved(Reserved::Unbuilt(stem)) => write!(
                f,
                "it names Gro's host module '{stem}.ts', which isn't on {BINARY_NAME} yet, and a \
                 host module's specifier never resolves to a package. Fix: run this under Node \
                 with gro"
            ),
            Self::Reserved(Reserved::Split(split)) => {
                write!(
                    f,
                    "it names '{}.ts', which isn't one of Gro's modules: its members are modules \
                     of their own, and its specifier never resolves to a package. Fix: import \
                     what you use from ",
                    split.stem
                )?;
                write_specifiers(f, split.modules.iter().copied(), split.modules.len(), "or")?;
                // the ones refused on fgro, which the fix can't promise there
                let is_unbuilt = |module: &str| {
                    module
                        .strip_suffix(".ts")
                        .is_some_and(|stem| UNBUILT_HOST_MODULES.contains(&stem))
                };
                let available = split.modules.iter().filter(|module| !is_unbuilt(module));
                let count = available.clone().count();
                if count == split.modules.len() {
                    return Ok(());
                }
                if count == 0 {
                    return write!(
                        f,
                        "; none of them is on {BINARY_NAME} yet, so run code that uses them \
                         under Node with gro"
                    );
                }
                f.write_str("; of these, only ")?;
                write_specifiers(f, available.copied(), count, "and")?;
                let verb = if count == 1 { "is" } else { "are" };
                write!(
                    f,
                    " {verb} on {BINARY_NAME} yet, so run code that uses the others under Node \
                     with gro"
                )
            }
            Self::GroModule { path, fault } => match fault {
                GroModuleFault::NodeOnly => write!(
                    f,
                    "it names Gro's module '{path}', which runs only on Node, so {BINARY_NAME} \
                     doesn't carry it, and a specifier of Gro's never resolves to a package. \
                     Fix: run this under Node with gro"
                ),
                GroModuleFault::Missing => write!(
                    f,
                    "Gro has no module '{path}': {BINARY_NAME} carries Gro's library, so a \
                     specifier of Gro's names one of its modules by its path in src/lib, and \
                     never resolves to a package"
                ),
                GroModuleFault::Internal => write!(
                    f,
                    "it names Gro's internal module '{path}', which Gro's package doesn't \
                     export. Fix: import what Gro exports"
                ),
                GroModuleFault::JsSpelling => {
                    let ts = path.strip_suffix(".js").unwrap_or(path);
                    write!(
                        f,
                        "it is the .js spelling of Gro's module '{ts}.ts', and a specifier of \
                         Gro's never resolves to a package. Fix: import '{GRO_PACKAGE}/{ts}.ts'"
                    )
                }
            },
            Self::GroPeer { name, fault } => match fault.as_ref() {
                PeerFault::Undeclared => write!(
                    f,
                    "Gro's library imports the package '{name}', which Gro's package.json \
                     doesn't declare as a peer dependency, so no range says which of its \
                     versions Gro works with. Fix: run this under Node with gro"
                ),
                PeerFault::Missing {
                    range,
                    node_modules,
                } => write!(
                    f,
                    "Gro's library imports the package '{name}', a peer dependency of Gro's at \
                     {range}, which the project doesn't install: no {}. Fix: npm i -D {name}",
                    node_modules.join(name.as_ref()).display()
                ),
                PeerFault::OutOfRange {
                    range,
                    version,
                    dir,
                } => {
                    write!(
                        f,
                        "Gro's library imports the package '{name}', a peer dependency of \
                         Gro's at {range}, and the project installs "
                    )?;
                    match version {
                        Some(version) => write!(f, "version {version}")?,
                        None => f.write_str("it with no version")?,
                    }
                    write!(
                        f,
                        " at {}. Fix: npm i -D '{name}@{range}'",
                        dir.display()
                    )
                }
                PeerFault::UnreadRange(range) => write!(
                    f,
                    "Gro's library imports the package '{name}', a peer dependency of Gro's at \
                     {range}, a range {BINARY_NAME} doesn't read, so its version can't be \
                     checked. Fix: run this under Node with gro"
                ),
            },
            Self::NoGroLibrary(None) => write!(
                f,
                "this run holds no copy of Gro's library, so of Gro's modules only its host \
                 modules load, and a specifier of Gro's never resolves to a package"
            ),
            Self::NoGroLibrary(Some(reason)) => write!(
                f,
                "this run's copy of Gro's library failed to load ({reason}), and a specifier \
                 of Gro's never resolves to a package"
            ),
            Self::OutsideGroLibrary => f.write_str(
                "a module of Gro's library imports only Gro's other modules, by a relative \
                 specifier that stays inside its src/lib, and packages by name",
            ),
            Self::GroImportsProject { path, project } => write!(
                f,
                "Gro's library imports it by name, and it resolves to {}, a module of the \
                 project at {}: Gro's library holds no grant and never imports the project's \
                 own modules, which hold the project's. Fix: install the package into \
                 node_modules rather than linking the project",
                path.display(),
                project.display()
            ),
            Self::Empty => f.write_str("the specifier is empty"),
            Self::InvalidUrl(fault) => write!(f, "not a valid file URL: {fault}"),
            Self::InvalidPackageName => f.write_str(
                "it is not a valid package name: a name can't start with '.' or hold '%' or '\\', \
                 and a scoped one is '@scope/name'",
            ),
            Self::NoProjectRoot {
                dir,
                stopped_at: Some(stopped_at),
            } if super::walk::in_node_modules(dir) =>
            {
                write!(
                    f,
                    "no project root is above {}: a module inside node_modules resolves bare \
                     specifiers within the directory holding its outermost node_modules, and the \
                     walk up to it stopped at {}, which is world-writable, so nothing in it or \
                     above it is read. Fix: remove that directory's other-write bit (chmod o-w)",
                    dir.display(),
                    stopped_at.display()
                )
            }
            Self::NoProjectRoot { dir, stopped_at } => {
                write!(
                    f,
                    "no project root is above {}: a bare specifier resolves only under a \
                     package.json outside node_modules that holds more than \"type\" and \
                     \"sideEffects\", and the search for one",
                    dir.display()
                )?;
                match stopped_at {
                    Some(stopped_at) => write!(
                        f,
                        " stopped at {}, which is world-writable, so nothing in it or above it \
                         is read",
                        stopped_at.display()
                    )?,
                    None => f.write_str(" reached the filesystem's root")?,
                }
                f.write_str(
                    ". Fix: add a package.json at the project root, with the package installed in \
                     the node_modules beside it",
                )
            }
            Self::PackageNotFound { name, bound } => {
                match &bound.package_json {
                    Some(package_json) => write!(
                        f,
                        "no package '{name}' is installed: no node_modules/{name} directory from \
                         the importer up to {}, which ends the search, as every package.json \
                         outside node_modules does except one holding nothing but \"type\" and \
                         \"sideEffects\"",
                        package_json.display()
                    )?,
                    None => write!(
                        f,
                        "no package '{name}' is installed: no node_modules/{name} directory from \
                         the importer up to {}, which holds the node_modules the importer is in \
                         and ends the search",
                        bound.dir.display()
                    )?,
                }
                if let Some(above) = &bound.above {
                    write!(
                        f,
                        ", and the node_modules beside {} is not searched (npm and yarn \
                         workspaces, which install at their root, aren't supported yet)",
                        above.display()
                    )?;
                }
                write!(
                    f,
                    ". Fix: install it into {}",
                    bound.dir.join("node_modules").display()
                )
            }
            Self::WorldWritablePackage { name, dir } => write!(
                f,
                "package '{name}' is found through {}, which is world-writable, so nothing in it \
                 is read: any user can plant or replace a package there. Fix: remove the \
                 directory's other-write bit (chmod o-w), then reinstall the package",
                dir.display()
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
            Self::NoMatchingCondition {
                field,
                subpath,
                unmatched,
            } => {
                match field {
                    MapField::Exports => write!(f, "no condition matches for subpath '{subpath}'")?,
                    MapField::Imports => {
                        write!(f, "no condition matches for '{subpath}' in \"imports\"")?;
                    }
                }
                write!(
                    f,
                    ": the package's conditions [{}] don't, and only {} are matched. Fix: add a \
                     \"gro\" or \"import\" condition before the others",
                    unmatched.join(", "),
                    CONDITIONS.join(", ")
                )
            }
            Self::InvalidTarget {
                field,
                key,
                target,
                reason,
            } => {
                write!(f, "the \"{field}\" target {target} for '{key}' is invalid: ")?;
                f.write_str(match (reason, field) {
                    (TargetFault::NotRelative | TargetFault::NamesNothing, _) => {
                        "a target must start with './'"
                    }
                    (TargetFault::Segment, _) => {
                        "a target can't have a '.', '..', or 'node_modules' segment, \
                         percent-encoded or not"
                    }
                    (TargetFault::EmptySegment, _) => {
                        "a target can't have an empty segment ('//' or a trailing '/')"
                    }
                    (TargetFault::Outside, MapField::Exports) => "it leaves the package's directory",
                    (TargetFault::Outside, MapField::Imports) => {
                        "it leaves the directory of its package.json"
                    }
                    (TargetFault::NotATarget, _) => {
                        "a target is a string, an array, an object of conditions, or null"
                    }
                })?;
                match field {
                    MapField::Exports => f.write_str(
                        ". Fix: use a version of the package whose \"exports\" are valid",
                    ),
                    MapField::Imports => f.write_str(
                        ". Fix: correct the target, or for an installed package, use a version \
                         whose \"imports\" are valid",
                    ),
                }
            }
            Self::InvalidSubpath {
                field: MapField::Exports,
                key,
                matched,
            } => write!(
                f,
                "the part of the subpath the pattern '{key}' matches, '{matched}', has a '.', \
                 '..', or 'node_modules' segment, percent-encoded or not, makes an empty \
                 segment in the target, or takes the target out of the package's directory, and \
                 a pattern never substitutes those. Fix: import the module by its plain subpath"
            ),
            Self::InvalidSubpath {
                field: MapField::Imports,
                key,
                matched,
            } => write!(
                f,
                "the part of it the \"imports\" pattern '{key}' matches, '{matched}', has a '.', \
                 '..', or 'node_modules' segment or spells one with the target around it, \
                 percent-encoded or not, makes an empty segment in the target, or takes the target \
                 out of the directory of its package.json, and a pattern never substitutes those. \
                 Fix: import the module by its plain path"
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
                    Self::NotModuleType { .. } | Self::NoTypeScope { stopped_at: None } => write!(
                        f,
                        "the package exports it as {path}, which is CommonJS (no \"type\": \
                         \"module\", not .mjs), and only ES modules load. {PACKAGE_FIX}"
                    ),
                    Self::OutsideRoot {
                        path: real, root, ..
                    } => write!(
                        f,
                        "the package exports it as {path}, which resolves to {}, outside {}, and \
                         an \"exports\" target stays inside its package. {TARGET_FIX}",
                        real.display(),
                        root.display()
                    ),
                    Self::IntoNodeModules { path: real, root } => write!(
                        f,
                        "the package exports it as {path}, which resolves to {}, inside \
                         node_modules below {}, and an \"exports\" target stays inside its \
                         package. {TARGET_FIX}",
                        real.display(),
                        root.display()
                    ),
                    Self::IntoPackageRoot { path: real, root } => write!(
                        f,
                        "the package exports it as {path}, which resolves to {}, inside {root}, \
                         and an \"exports\" target stays inside its package. {TARGET_FIX}",
                        real.display()
                    ),
                    error => write!(f, "the package exports it as {path}: {error}"),
                }
            }
            Self::ImportTarget {
                package_json,
                path,
                error,
            } => {
                let mapped = format!(
                    "the \"imports\" of {} map it to {}",
                    package_json.display(),
                    path.display()
                );
                match &**error {
                    Self::OutsideRoot {
                        path: real,
                        root,
                        package,
                    } => {
                        write!(
                            f,
                            "{mapped}, which resolves to {}, outside {}, and a # specifier's \
                             target stays inside the directory of the package.json that maps it \
                             and inside its importer's root. ",
                            real.display(),
                            root.display()
                        )?;
                        match package {
                            Some(_) => f.write_str(
                                "Fix: use a version of the package whose \"imports\" stay inside \
                                 it",
                            ),
                            None => write!(
                                f,
                                "Fix: map it to a file inside {}, or import the file by name if \
                                 it is a package's",
                                root.display()
                            ),
                        }
                    }
                    Self::IntoNodeModules { path: real, root } => write!(
                        f,
                        "{mapped}, which resolves to {}, inside node_modules below {}, which only \
                         a bare specifier reaches, through the package's \"exports\". Fix: \
                         import it by name",
                        real.display(),
                        root.display()
                    ),
                    Self::IntoPackageRoot { path: real, root } => {
                        write!(f, "{mapped}, which resolves to {}, inside {root}", real.display())?;
                        match &root.origin {
                            Origin::Package { name, .. } => write!(
                                f,
                                ", which only a bare specifier reaches, through its \"exports\". \
                                 Fix: import it by name ('{name}' or a subpath it exports)"
                            ),
                            Origin::Project | Origin::Embedded => {
                                f.write_str(", which a package's modules never reach")
                            }
                        }
                    }
                    error => write!(f, "{mapped}: {error}"),
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
            Self::NotModuleType { package_json } => write!(
                f,
                "a .js file is an ES module only under \"type\": \"module\", which {} does not \
                 set, and CommonJS is not supported. Fix: add \"type\": \"module\" to it, or \
                 rename the file to .mjs",
                package_json.display()
            ),
            Self::NoTypeScope { stopped_at: None } => f.write_str(
                "a .js file is an ES module only under a package.json with \"type\": \"module\", \
                 and none is above it (the format is never guessed from the code). \
                 Fix: rename the file to .mjs, or add a package.json with \"type\": \"module\"",
            ),
            Self::NoTypeScope {
                stopped_at: Some(stopped_at),
            } => write!(
                f,
                "a .js file is an ES module only under a package.json with \"type\": \"module\", \
                 and the search for one stopped at {}, which is world-writable, so nothing in it \
                 or above it is read (the format is never guessed from the code). Fix: rename the \
                 file to .mjs, or move it under a package.json with \"type\": \"module\" in a \
                 directory that isn't world-writable",
                stopped_at.display()
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
            Self::OutsideRoot {
                path,
                root,
                package: None,
            } => write!(
                f,
                "it resolves to {}, outside {}, and a relative or absolute specifier resolves \
                 only inside its importer's root, the project's directory or a package's own. \
                 Fix: move the module inside {1}, or install it as a package and import it by name",
                path.display(),
                root.display()
            ),
            Self::OutsideRoot {
                path,
                root,
                package: Some(name),
            } => write!(
                f,
                "it resolves to {}, outside package '{name}' at {}, and a package's relative and \
                 absolute specifiers resolve only inside it, so the package reaches outside \
                 itself. Fix: use a version of the package that doesn't",
                path.display(),
                root.display()
            ),
            Self::PackageImportsProject {
                package,
                path,
                project,
            } => write!(
                f,
                "package '{package}' imports it by name, and it resolves to {}, a module of the \
                 project at {}: a package can't import the project's own modules by name, since \
                 through one it would reach the grants the project's modules hold",
                path.display(),
                project.display()
            ),
            Self::InNoPackage(path) => write!(
                f,
                "it resolves to {}, inside node_modules but in no package, so it has no origin. \
                 Fix: run a file inside a package (node_modules/<name>/...)",
                path.display()
            ),
            Self::IntoNodeModules { path, root } => write!(
                f,
                "it resolves to {}, inside node_modules below {}, which only a bare specifier \
                 reaches, through the package's \"exports\". Fix: import it by name",
                path.display(),
                root.display()
            ),
            Self::IntoPackageRoot { path, root } => {
                write!(f, "it resolves to {}, inside {root}", path.display())?;
                match &root.origin {
                    Origin::Package { name, .. } => write!(
                        f,
                        ", which only a bare specifier reaches, through its \"exports\". Fix: \
                         import it by name ('{name}' or a subpath it exports)"
                    ),
                    Origin::Project | Origin::Embedded => {
                        f.write_str(", which a package's modules never reach")
                    }
                }
            }
            Self::RootConflict {
                path,
                recorded,
                admitted,
            } => {
                write!(
                    f,
                    "{} was resolved inside {recorded}, and {admitted} was then admitted around \
                     it, so its origin would depend on the import order",
                    path.display()
                )?;
                // the project's root is admitted first, so only a package's is admitted around one
                let Origin::Package { name, .. } = &admitted.origin else {
                    return Ok(());
                };
                match &recorded.origin {
                    Origin::Project | Origin::Embedded => write!(
                        f,
                        ". Fix: import it by name ('{name}' or a subpath it exports) everywhere, \
                         not by a relative path"
                    ),
                    Origin::Package { name: holder, .. } => write!(
                        f,
                        ". Two packages hold it, '{name}' inside '{holder}'. Fix: import it \
                         through only one of them, or install '{name}' on its own rather than \
                         inside '{holder}'"
                    ),
                }
            }
            Self::SymlinkLoop(path) => write!(
                f,
                "{} leads into a symlink loop, so it names no file or directory. \
                 Fix: remove the looping link, or point it at what it should name (for a \
                 package, reinstall it)",
                path.display()
            ),
            Self::Io { path, message, .. } => {
                write!(f, "failed to read {}: {message}", path.display())
            }
            Self::ReadReplacedSinceFirstRead(path) => write!(
                f,
                "cannot read {}: {}",
                path.display(),
                ReadRefusal::ReplacedSinceFirstRead
            ),
            Self::ReadReplacedSinceLookup(path) => write!(
                f,
                "cannot read {}: {}",
                path.display(),
                ReadRefusal::ReplacedSinceLookup
            ),
            Self::ReadOutsideBox(path) => write!(
                f,
                "cannot read {}: it lies outside the box's roots",
                path.display(),
            ),
        }
    }
}

impl fmt::Display for UrlFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Join(error) => write!(f, "{error}"),
            Self::NoLocalPath => f.write_str("names no local path"),
            Self::ImporterNoLocalPath => f.write_str("the importer names no local path"),
            Self::EncodedSeparator(path) => write!(
                f,
                "the path {path} has an encoded '/' or '\\' (%2F or %5C), which never names a \
                 separator"
            ),
            Self::EncodedNul(path) => write!(f, "the path {path} has an encoded NUL (%00)"),
            Self::NotAbsolute => f.write_str("not an absolute path"),
        }
    }
}

/// Writes `count` specifiers quoted, joined as a list by `conjunction`
/// (`or`, `and`): `'a'`, `'a' or 'b'`, `'a', 'b', or 'c'`.
fn write_specifiers<'a>(
    f: &mut fmt::Formatter<'_>,
    specifiers: impl Iterator<Item = &'a str>,
    count: usize,
    conjunction: &str,
) -> fmt::Result {
    let last = count.saturating_sub(1);
    for (i, specifier) in specifiers.enumerate() {
        match i {
            0 => {}
            _ if i == last && last == 1 => write!(f, " {conjunction} ")?,
            _ if i == last => write!(f, ", {conjunction} ")?,
            _ => f.write_str(", ")?,
        }
        write!(f, "'{specifier}'")?;
    }
    Ok(())
}
