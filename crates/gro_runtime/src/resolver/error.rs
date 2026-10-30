//! Why a specifier didn't resolve, and the message each reason carries.

use std::fmt;
use std::path::PathBuf;

use super::CONDITIONS;
use super::roots::{Origin, Root};

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
    /// that names no local path, a resolved path with an encoded `/`, `\`,
    /// or NUL in it, or an importer that names no local path or is inside
    /// no admitted root.
    InvalidUrl(String),
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
    /// linked package the real directories holding it, or for a linked
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
        /// The failure.
        error: String,
    },
}

/// The project root that bounded a `node_modules` search: the first
/// `package.json` above the importer, outside `node_modules`, that isn't a
/// type marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageSearchBound {
    /// Its directory, whose `node_modules` was the last searched.
    pub dir: PathBuf,
    /// Its real path.
    pub package_json: PathBuf,
    /// The nearest `package.json` above it and below any world-writable
    /// directory, whose `node_modules` aren't searched, as an npm or yarn
    /// workspace's root's aren't.
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
    /// common case, as opposed to a failure reading the files, a broken
    /// symlink among them, or a file replaced while it was resolved.
    pub const fn is_specifier_error(&self) -> bool {
        !matches!(self, Self::Io { .. } | Self::SymlinkLoop(_))
    }
}

/// The extensions a missing-extension message suggests, in order.
pub(super) const SUGGESTED_EXTENSIONS: [&str; 4] = ["ts", "js", "mjs", "mts"];

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
                    Origin::Project => f.write_str(", which a package's modules never reach"),
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
                    Origin::Project => write!(
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
            Self::Io { path, error } => write!(f, "failed to read {}: {error}", path.display()),
        }
    }
}
