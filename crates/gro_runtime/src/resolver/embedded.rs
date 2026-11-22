//! Gro's embedded library: the specifiers of Gro's package, from any
//! importer, and every specifier of an embedded module.
//!
//! A specifier of Gro's package (`@fuzdev/gro`, or one past
//! `@fuzdev/gro/`) that the builtin table neither names nor reserves names
//! a module of Gro's library by its path in `src/lib`, `@fuzdev/gro` alone
//! its `index.ts`, and resolves to the copy the run holds, never to a
//! package, whatever `node_modules` or the importer's own package holds: a
//! planted or stale install of Gro never runs as Gro. A path under
//! `internal/` is refused, as the package's `"exports"` null it on Node,
//! and so is one the copy leaves out to run on Node, and one Gro has no
//! module at, the `.js` spelling of a module's `.ts` path among them.
//!
//! An embedded module's URL is its path in the `gro:` scheme,
//! `gro:lib/<path>`, which the resolver refuses from source like every
//! scheme but `file:`, so only a specifier of Gro's reaches one. It's an
//! opaque-path URL, no base for another (`new URL('./x.ts', import.meta.url)`
//! throws), so nothing in the embed may resolve a path through its
//! `import.meta.url`, and none does: its own imports are specifiers. Its
//! relative specifiers resolve by their text, inside `src/lib`, never above
//! it, and row first: a path that is a host module's file is that builtin
//! ([`HostModule::file`]), or refused as unbuilt, so the Node file never
//! loads. A bare one is a builtin, a reserved specifier, or one of Gro's
//! package as from any importer, or else a package resolved from the
//! project root's `node_modules`, with no self-reference, as Gro installed
//! there would find it, so one package serves Gro's library and the
//! project's modules alike. That package must be one of Gro's peer
//! dependencies, at a version in the range Gro's `package.json` declares,
//! which the copy carries ([`super::semver`]): one Gro doesn't declare, one
//! the project doesn't install, and one at another version are each
//! refused, naming the package, the range, and the install that fixes it.
//! The check holds against version skew, not against a project that means
//! to defeat it, whose `package.json` names any version it likes. Any other
//! specifier is refused: an embedded module reaches nothing outside
//! `src/lib` but by a package's name.

use url::Url;

use super::builtin::{
    Builtin, GRO_PACKAGE, HostModule, Reserved, find_builtin, find_host_module_file, find_reserved,
    gro_module_path,
};
use super::error::{GroModuleFault, PeerFault, ResolveError, ResolveErrorKind};
use super::package::{parse_package_name, resolve_package};
use super::roots::{Origin, Roots};
use super::semver::{Version, VersionRange};
use super::tree::SourceTree;
use super::walk::Consulted;
use super::{Conditions, Resolved, find_target};
use crate::embed::Embed;

/// Gro's library as a resolution is handed it, as data.
///
/// It's the run's copy, or why there's none: `None` when the run holds none
/// and the reason when its copy failed to load. The caller obtains it, so a
/// resolution reads
/// nothing but its [`SourceTree`]; one from a module on disk needs it only
/// when [`needs_gro_library`] says so, and is handed `Err(None)`, never
/// read, otherwise.
pub type GroLibrary<'a> = Result<Embed<'static>, Option<&'a str>>;

/// Whether resolving `specifier` from a module on disk reads Gro's library:
/// a specifier of Gro's package that the builtin table neither names nor
/// reserves, the one [`resolve_gro_package`] serves.
pub fn needs_gro_library(specifier: &str) -> bool {
    (specifier == GRO_PACKAGE || gro_module_path(specifier).is_some())
        && find_builtin(specifier).is_none()
        && find_reserved(specifier).is_none()
}

/// What an embedded module's URL begins with: the rest is its path in
/// `src/lib`.
pub const EMBEDDED_URL_PREFIX: &str = "gro:lib/";

/// The module `@fuzdev/gro` alone names, the package's entry.
const PACKAGE_ENTRY: &str = "index.ts";

/// The directory of Gro's modules its package doesn't export.
const INTERNAL_DIR: &str = "internal/";

/// A module of Gro's embedded library.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EmbeddedModule {
    /// Its path in Gro's `src/lib`.
    pub path: &'static str,
    /// Its code, stripped of its types when the copy was made, so it loads
    /// as it is.
    pub code: &'static str,
}

impl std::fmt::Debug for EmbeddedModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedModule")
            .field("path", &self.path)
            .field("code", &self.code.len())
            .finish()
    }
}

impl EmbeddedModule {
    /// Its URL, `gro:lib/<path>`.
    ///
    /// # Errors
    ///
    /// Returns why the URL doesn't parse, which a path the copy holds never
    /// makes.
    pub fn url(&self) -> Result<Url, url::ParseError> {
        Url::parse(&format!("{EMBEDDED_URL_PREFIX}{}", self.path))
    }
}

/// What a specifier of Gro's package names, when `specifier` is one:
/// `@fuzdev/gro` alone, or one past `@fuzdev/gro/`. The builtin table is
/// asked first, by the caller.
pub(super) fn resolve_gro_package(
    library: GroLibrary<'_>,
    specifier: &str,
) -> Option<Result<Resolved, ResolveErrorKind>> {
    let path = if specifier == GRO_PACKAGE {
        PACKAGE_ENTRY
    } else {
        gro_module_path(specifier)?
    };
    if path.starts_with(INTERNAL_DIR) {
        return Some(Err(ResolveErrorKind::GroModule {
            path: Box::from(path),
            fault: GroModuleFault::Internal,
        }));
    }
    Some(resolve_lib_path(library, path))
}

/// What the path `path` in `src/lib` names: a host module's builtin, or
/// its refusal as unbuilt, else the run's copy of the module, else the
/// refusal of a path the copy leaves out or doesn't have.
fn resolve_lib_path(library: GroLibrary<'_>, path: &str) -> Result<Resolved, ResolveErrorKind> {
    match find_host_module_file(path) {
        Some(HostModule::Builtin(builtin)) => return Ok(Resolved::Builtin(builtin)),
        Some(HostModule::Unbuilt(stem)) => {
            return Err(ResolveErrorKind::Reserved(Reserved::Unbuilt(stem)));
        }
        None => {}
    }
    let embed = library.map_err(|reason| ResolveErrorKind::NoGroLibrary(reason.map(Box::from)))?;
    if let Some(module) = find_embedded(embed, path) {
        return Ok(Resolved::Embedded(module));
    }
    let fault = if embed.is_left_out(path) {
        GroModuleFault::NodeOnly
    } else if path
        .strip_suffix(".js")
        .is_some_and(|stem| embed.get(&format!("{stem}.ts")).is_some())
    {
        GroModuleFault::JsSpelling
    } else {
        GroModuleFault::Missing
    };
    Err(ResolveErrorKind::GroModule {
        path: Box::from(path),
        fault,
    })
}

/// The module at `path` in `embed`, with the path and code it holds.
fn find_embedded(embed: Embed<'static>, path: &str) -> Option<EmbeddedModule> {
    embed
        .module(path)
        .map(|(path, code)| EmbeddedModule { path, code })
}

/// Resolves `specifier`, imported by the embedded module at `importer`,
/// whose path in `src/lib` is `importer_path`, as the module doc says, in
/// `library`, the run's copy, which the importer was loaded from. A bare
/// specifier for a package resolves from the project's root in `roots`,
/// reading `tree`, its `"exports"` matching `conditions`.
///
/// # Errors
///
/// Returns a [`ResolveError`] when the specifier names nothing an embedded
/// module may import, or a package that doesn't resolve.
pub fn resolve_from_embedded(
    tree: &impl SourceTree,
    library: GroLibrary<'_>,
    specifier: &str,
    importer: &Url,
    importer_path: &str,
    roots: &Roots,
    conditions: Conditions,
) -> Result<Resolved, ResolveError> {
    let error = |kind| ResolveError {
        specifier: specifier.to_owned(),
        importer: importer.to_string(),
        kind,
    };
    if specifier.starts_with("./") || specifier.starts_with("../") {
        let path = join_lib_path(importer_path, specifier)
            .ok_or_else(|| error(ResolveErrorKind::OutsideGroLibrary))?;
        return resolve_lib_path(library, &path).map_err(error);
    }
    if specifier.is_empty() {
        return Err(error(ResolveErrorKind::Empty));
    }
    if specifier.starts_with(['/', '#'])
        || specifier == "."
        || specifier == ".."
        || Url::parse(specifier).is_ok()
    {
        return Err(error(ResolveErrorKind::OutsideGroLibrary));
    }
    if let Some(builtin) = find_builtin(specifier) {
        return Ok(Resolved::Builtin(builtin));
    }
    if let Some(reserved) = find_reserved(specifier) {
        return Err(error(ResolveErrorKind::Reserved(reserved)));
    }
    if let Some(resolved) = resolve_gro_package(library, specifier) {
        return resolved.map_err(error);
    }
    let embed =
        library.map_err(|reason| error(ResolveErrorKind::NoGroLibrary(reason.map(Box::from))))?;
    resolve_peer(tree, embed, specifier, roots, conditions).map_err(error)
}

/// Resolves the bare `specifier` of a package, imported by Gro's library,
/// from the project root's `node_modules`, once its version is checked
/// against the range of Gro's peer dependency of its name in `embed`.
fn resolve_peer(
    tree: &impl SourceTree,
    embed: Embed<'static>,
    specifier: &str,
    roots: &Roots,
    conditions: Conditions,
) -> Result<Resolved, ResolveErrorKind> {
    let (name, _) = parse_package_name(specifier)?;
    let peer = |fault| ResolveErrorKind::GroPeer {
        name: Box::from(name),
        fault: Box::new(fault),
    };
    // before anything is read: Gro imports only what it declares
    let range = embed
        .peer_range(name)
        .ok_or_else(|| peer(PeerFault::Undeclared))?;
    let mut consulted = Consulted::default();
    let project = roots.project();
    let package = resolve_package(
        tree,
        specifier,
        &project.dir,
        None,
        roots,
        &mut consulted,
        conditions,
    )
    .map_err(|kind| match kind {
        ResolveErrorKind::PackageNotFound { .. } => peer(PeerFault::Missing {
            range: Box::from(range),
            node_modules: project.dir.join("node_modules"),
        }),
        kind => kind,
    })?;
    let path = super::specifier::url_to_path(&package.url)?;
    // before the target is read: only the project's root has its origin
    let version = match &package.root.origin {
        Origin::Package { version, .. } => version.as_deref(),
        Origin::Project | Origin::Embedded => {
            return Err(ResolveErrorKind::GroImportsProject {
                path,
                project: package.root.dir,
            });
        }
    };
    let parsed =
        VersionRange::parse(range).map_err(|_| peer(PeerFault::UnreadRange(Box::from(range))))?;
    if !version
        .and_then(Version::parse)
        .is_some_and(|version| parsed.contains(&version))
    {
        return Err(peer(PeerFault::OutOfRange {
            range: Box::from(range),
            version: version.map(Box::from),
            dir: package.root.dir,
        }));
    }
    find_target(
        tree,
        package,
        path,
        None,
        roots,
        consulted,
        |path, error| ResolveErrorKind::Target { path, error },
    )
    .map(Resolved::Module)
}

/// The builtin module the relative `specifier` names from the embedded
/// module whose URL is `importer`, by its text, as
/// [`resolve_from_embedded`] resolves it, reading nothing: the loader
/// refuses an import of a builtin with attributes before it resolves, and
/// an embedded module names one by a relative specifier that isn't the
/// table's.
pub fn find_embedded_builtin(importer: &str, specifier: &str) -> Option<&'static Builtin> {
    let importer_path = importer.strip_prefix(EMBEDDED_URL_PREFIX)?;
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return None;
    }
    match find_host_module_file(&join_lib_path(importer_path, specifier)?)? {
        HostModule::Builtin(builtin) => Some(builtin),
        HostModule::Unbuilt(_) => None,
    }
}

/// The path in `src/lib` that the relative `specifier` names from the
/// module at `importer_path`, by its text alone: `.` segments dropped and
/// each `..` taking the segment before it, or `None` past `src/lib`.
fn join_lib_path(importer_path: &str, specifier: &str) -> Option<String> {
    let mut segments: Vec<&str> = importer_path.split('/').collect();
    // the importer's own name
    segments.pop();
    for segment in specifier.split('/') {
        match segment {
            "." => {}
            ".." => {
                segments.pop()?;
            }
            segment => segments.push(segment),
        }
    }
    Some(segments.join("/"))
}
