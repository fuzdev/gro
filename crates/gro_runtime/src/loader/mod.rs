//! The `deno_core` module loader: a thin adapter over [`crate::resolver`] that
//! reads the resolved modules from disk, stripping TypeScript's types in
//! process.
//!
//! Static imports, `import()`, and `import.meta.resolve` all reach
//! [`resolver::resolve`]. The entry module is resolved before the runtime
//! starts ([`resolver::resolve_entry`]), so its URL arrives here already
//! final, with the roots its graph starts with.
//!
//! The loader owns the graph's [`Roots`] and the [`Root`](resolver::Root) each
//! URL resolved inside, and passes the roots into every resolution. A bare
//! specifier's package root is admitted when it resolves. Admitting a root
//! around a module already recorded inside an outer one is an error naming
//! both ([`ResolveErrorKind::RootConflict`](resolver::ResolveErrorKind::RootConflict)),
//! since otherwise the import order would decide the module's origin;
//! resolving a URL again inside the same root changes nothing.
//! `import.meta.resolve` runs the same resolution and admits and records
//! nothing, so importing the URL it returns resolves it again, as an absolute
//! specifier: an installed package's file is then a path into `node_modules`,
//! refused.
//!
//! The loader also records each URL's [`FileId`], the device and inode
//! numbers and birth time the resolver took when it looked the file up, and
//! reads a module only from a file it opened and found to have that identity:
//! it opens the path without blocking, compares the open file's identity with
//! the record, and reads the source from that same open file, never from the
//! path again. The first resolution's record stands: resolving the URL again
//! changes nothing, and a URL already loaded is served from `deno_core`'s
//! module map. A file, or a directory on its path, swapped between resolving
//! and loading is refused, a FIFO among them, except a file remade where the
//! filesystem records no birth time, or within the same tick of its clock as
//! the original's birth: the identity then falls back to the device and
//! inode numbers, which ext4 reuses for a file made after one is removed.
//! What it can't see is a swap inside the resolver's own lookup, between
//! canonicalizing the path and reading its identity; the lookup reads that
//! identity without following symlinks, so a last component swapped for a
//! symlink there is refused, but a swapped directory above it isn't caught.
//!
//! A TypeScript module is stripped by `tsv_ts`, which blanks every type-only
//! range rather than rewriting the code, so every surviving character keeps
//! its line and column and V8's positions are the source's without a
//! sourcemap. TypeScript with runtime semantics (an `enum`, a parameter
//! property) is refused, not transformed: the load fails naming the construct,
//! a fix, and its position, in the `at <url>:<line>:<column>` form V8's frames
//! print. Like every load failure but an I/O error reading the file, it
//! reaches JS as a `TypeError`, which `import()` can catch.
//!
//! Its [`Rules`] refuse more than the resolver does, for eval: every
//! `import()`, before anything is resolved, and every import from the
//! driver, a module whose source the host provides and that resolves to
//! itself, as the root of its own load, and to nothing else. Both are
//! recorded as failures of their own ([`FailureError::DriverImport`],
//! [`FailureError::DynamicImport`]), and reach JS as a `TypeError`. The
//! default rules refuse nothing more.
//!
//! With [`Rules::box_roots`] set, as eval sets them, every read the loader
//! makes, a module's or a `package.json`'s, one served from an earlier
//! read's bytes ([`RecordingTree`]) included, is checked first: its real path
//! must lie under one of the box's roots, the directories the run's process
//! is confined to (`confine`), or it isn't read. The box is the project and
//! the link closure found from its `node_modules` before the run, and a graph
//! can reach past it: the resolver follows a link the closure doesn't (one
//! in a nested `node_modules`, or in a subdirectory's own) or drops (one to
//! a directory holding the project or another package's directory), and
//! reads a `package.json` symlinked out of the box at its real path. A refused read
//! is recorded once ([`Loader::breach`]) for the caller to fail the run on,
//! whatever the run did with the error.
//!
//! The loader keeps the run's [`Graph`]. With recording off, what a plain
//! run uses, it keeps only what loading needs, the roots and each URL's root
//! and file, and does no work for anything else: no hash, edge, or failure
//! reason is built, and the files a resolution reports consulting are
//! dropped unread. With it full ([`Recording`]),
//! the graph is the whole record of what resolved and loaded: each URL's
//! record, which a load completes with the module's [`ModuleFormat`] and the
//! BLAKE3 hash of the bytes read from the checked file, before any strip:
//! the bytes V8's source was made from, run as read for JS and stripped for
//! TypeScript; each import that resolved as an edge, by importer, specifier,
//! and [`Reach`] (`deno_core` resolves every import of a graph `import()`
//! loads as dynamic, not only the call's own); every metadata file a
//! resolution consulted, the entry's and `import.meta.resolve`'s included;
//! and each failed resolution or load by its site, with the reason as data
//! built where the message is, never parsed from it. Recording in full,
//! resolutions also read through a [`RecordingTree`]:
//! each metadata file is read once per run, its later reads served the
//! first's bytes, a file of another identity at its path refused, and its
//! hash kept, and every path a resolution went on past finding absent is
//! recorded. Recording changes nothing JS sees but that refusal: every
//! error keeps its message and class.
//! `import.meta.resolve` adds no edge, since it loads nothing, but its
//! consulted files and failures are recorded.

pub mod graph;
pub mod reads;
pub mod strip;

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};

use deno_core::error::ModuleLoaderError;
use deno_core::{
    ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader, ModuleSource,
    ModuleSourceCode, ModuleSpecifier, ModuleType, RequestedModuleType, ResolutionKind,
};
use url::Url;

use crate::confine::BoxRoots;
use crate::resolver::{
    self, DiskTree, Entry, FileId, MAX_MODULE_SIZE, ModuleFormat, Resolution, ResolveError, Roots,
    SourceTree,
};
use graph::{
    Failure, FailureError, Graph, LoadFailure, Loaded, Position, Reach, Reached, Recording,
};
use reads::{Reads, RecordingTree};
use strip::strip_module;

/// Resolves through the strict resolver over the disk, and loads `.js` and
/// `.mjs` files as ES modules, and `.ts` and `.mts` files as ES modules
/// stripped of their types.
#[derive(Debug)]
pub struct Loader {
    /// What the graph resolved and loaded, and, recording in full, the rest
    /// of its record.
    graph: RefCell<Graph>,
    /// What it refuses beyond the resolver's rules.
    rules: Rules,
    /// The first read refused for lying outside [`Rules::box_roots`].
    breach: RefCell<Option<PathBuf>>,
}

/// What a loader refuses beyond the resolver's rules, fixed when it is made.
/// The default refuses nothing more, as `fgro run` loads.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    /// Refuses every `import()`, whatever its specifier, before resolving
    /// it, so nothing a dynamic import names is read.
    pub refuse_dynamic_import: bool,
    /// The URL of the driver, a module whose source the host provides
    /// itself (eval's stdin driver): it resolves to itself as the root of
    /// its own load, and every import and `import.meta.resolve` from it is
    /// refused, since it has no place on disk to resolve against.
    pub driver: Option<Url>,
    /// The roots of the box the run's process is confined to: a module or a
    /// `package.json` whose real path lies under none of them isn't read.
    pub box_roots: Option<BoxRoots>,
}

/// Why an import from the driver is refused, the reason in its message.
const DRIVER_IMPORT_FREE: &str = "a stdin driver is import-free";

/// A failed resolution or load: the reason as data, built only when the
/// graph records failures, and the error `deno_core` rethrows to JS. The
/// reason is boxed, since it holds a
/// [`ResolveErrorKind`](resolver::ResolveErrorKind), and a failure is the
/// rare path.
type Refused = (Option<Box<FailureError>>, ModuleLoaderError);

/// The reason `reason` makes, boxed, when `recording` keeps failures; off,
/// none is made.
fn reason(
    recording: Recording,
    reason: impl FnOnce() -> FailureError,
) -> Option<Box<FailureError>> {
    recording.is_full().then(|| Box::new(reason()))
}

impl Loader {
    /// A loader for the graph of `entry`, already resolved with `roots`,
    /// recording as `recording` says.
    pub fn new(entry: &Resolution, roots: Roots, recording: Recording) -> Self {
        Self {
            graph: RefCell::new(Graph::new(entry, roots, recording)),
            rules: Rules::default(),
            breach: RefCell::new(None),
        }
    }

    /// The loader, its graph's reads starting from `reads`, the record of
    /// the entry's resolution, recording in full ([`Graph::with_reads`]).
    #[must_use]
    pub fn with_reads(self, reads: Reads) -> Self {
        let graph = self.graph.into_inner().with_reads(reads);
        Self {
            graph: RefCell::new(graph),
            ..self
        }
    }

    /// The loader, refusing what `rules` says beyond the resolver's rules.
    pub fn with_rules(self, rules: Rules) -> Self {
        Self { rules, ..self }
    }

    /// Refuses `specifier`, reached from `referrer` as `reached`, for a
    /// rule of this loader's, recording `error` as the failure.
    fn refuse(
        &self,
        specifier: &str,
        referrer: &str,
        reached: Reached,
        error: FailureError,
        message: String,
    ) -> ModuleLoaderError {
        let mut graph = self.graph.borrow_mut();
        if let Some(error) = reason(graph.recording(), || error) {
            let failure = Failure {
                at: None,
                error: *error,
            };
            graph.fail(specifier, Some(referrer), reached, failure);
        }
        ModuleLoaderError::type_error(message)
    }

    /// Refuses an import or `import.meta.resolve` reached from the driver,
    /// when `referrer` is the driver.
    fn refuse_from_driver(
        &self,
        specifier: &str,
        referrer: &str,
        reached: Reached,
    ) -> Result<(), ModuleLoaderError> {
        if self
            .rules
            .driver
            .as_ref()
            .is_none_or(|driver| driver.as_str() != referrer)
        {
            return Ok(());
        }
        let verb = if reached == Reached::MetaResolve {
            "resolve"
        } else {
            "import"
        };
        Err(self.refuse(
            specifier,
            referrer,
            reached,
            FailureError::DriverImport,
            format!(
                "cannot {verb} '{specifier}' from the driver: {DRIVER_IMPORT_FREE}, so it can \
                 neither import nor resolve a module"
            ),
        ))
    }

    /// The first path a read was refused at for lying outside
    /// [`Rules::box_roots`], if any.
    pub fn breach(&self) -> Option<PathBuf> {
        self.breach.borrow().clone()
    }

    /// Checks a read of the real path `path` against [`Rules::box_roots`],
    /// recording the first refusal.
    fn check_read(&self, path: &Path) -> io::Result<()> {
        match &self.rules.box_roots {
            Some(roots) if !roots.holds(path) => {
                self.breach
                    .borrow_mut()
                    .get_or_insert_with(|| path.to_owned());
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "{} lies outside the box's roots, so it isn't read",
                        path.display()
                    ),
                ))
            }
            _ => Ok(()),
        }
    }

    /// Resolves as [`Loader::resolve_reached`] over the disk, each read
    /// checked against the box.
    fn resolve_on_disk(
        &self,
        specifier: &str,
        referrer: &str,
        reached: Reached,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        let tree = Boxed {
            loader: self,
            tree: &DiskTree,
        };
        self.resolve_reached(&tree, specifier, referrer, reached)
    }

    /// The graph, taken from the loader.
    pub fn into_graph(self) -> Graph {
        self.graph.into_inner()
    }

    /// A copy of the graph as it stands.
    pub fn graph(&self) -> Graph {
        self.graph.borrow().clone()
    }

    /// Resolves `specifier`, imported by the module at `referrer`, reading
    /// `tree`. Reached by an import, the result's root is admitted and its
    /// URL recorded; reached by `import.meta.resolve`, neither is. Recording
    /// in full, an import is also recorded as an edge, and either way the
    /// files the resolution consulted and a failure are.
    fn resolve_reached(
        &self,
        tree: &impl SourceTree,
        specifier: &str,
        referrer: &str,
        reached: Reached,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        let mut graph = self.graph.borrow_mut();
        resolve_into(&mut graph, tree, specifier, referrer, reached).map_err(|(reason, error)| {
            if let Some(reason) = reason {
                // `deno_core` gives no position to a resolution
                let failure = Failure {
                    at: None,
                    error: *reason,
                };
                graph.fail(specifier, Some(referrer), reached, failure);
            }
            error
        })
    }

    /// Loads the module at `specifier`, a URL resolved here, from the file
    /// its resolution found, reached as `reach` from `referrer` when
    /// `deno_core` names one. Recording in full, records what the load read,
    /// or why it failed.
    fn load_reached(
        &self,
        specifier: &ModuleSpecifier,
        requested: &RequestedModuleType,
        reach: Reach,
        referrer: Option<&ModuleLoadReferrer>,
    ) -> Result<ModuleSource, ModuleLoaderError> {
        let (file_id, recording) = {
            let graph = self.graph.borrow();
            let file_id = graph.records.get(specifier).map(|record| record.file_id);
            (file_id, graph.recording())
        };
        // `deno_core` loads only what `resolve` returned, which recorded it
        let result = file_id.map_or_else(
            || {
                Err((
                    reason(recording, || FailureError::Load(LoadFailure::NeverResolved)),
                    ModuleLoaderError::type_error(format!(
                        "cannot load {specifier}: it was never resolved"
                    )),
                ))
            },
            |file_id| {
                load(specifier, requested, file_id, recording, |path| {
                    self.check_read(path)
                })
            },
        );
        match result {
            Ok((source, loaded)) => {
                if let Some(loaded) = loaded {
                    self.graph.borrow_mut().load(specifier, loaded);
                }
                Ok(source)
            }
            Err((reason, error)) => {
                if let Some(reason) = reason {
                    let at = referrer.and_then(|referrer| {
                        Some(Position {
                            line: u32::try_from(referrer.line_number).ok()?,
                            column: u32::try_from(referrer.column_number).ok()?,
                        })
                    });
                    let failure = Failure { at, error: *reason };
                    self.graph.borrow_mut().fail(
                        specifier.as_str(),
                        referrer.map(|referrer| referrer.specifier.as_str()),
                        Reached::Import(reach),
                        failure,
                    );
                }
                Err(error)
            }
        }
    }
}

impl ModuleLoader for Loader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        let reach = match kind {
            // the entry, already resolved by `resolver::resolve_entry`
            ResolutionKind::MainModule => {
                return Url::parse(specifier)
                    .map_err(|error| ModuleLoaderError::type_error(error.to_string()));
            }
            ResolutionKind::Import => Reach::Static,
            ResolutionKind::DynamicImport => Reach::Dynamic,
        };
        // the root of the driver's own load, which `deno_core` resolves as a
        // static import from "."
        if let Some(driver) = &self.rules.driver
            && reach == Reach::Static
            && referrer == "."
            && specifier == driver.as_str()
        {
            return Ok(driver.clone());
        }
        let reached = Reached::Import(reach);
        self.refuse_from_driver(specifier, referrer, reached)?;
        if reach == Reach::Dynamic && self.rules.refuse_dynamic_import {
            return Err(self.refuse(
                specifier,
                referrer,
                reached,
                FailureError::DynamicImport,
                format!(
                    "cannot import '{specifier}' from {referrer}: import() is refused in eval, \
                     whatever it names. Fix: import the module with a static import"
                ),
            ));
        }
        self.resolve_on_disk(specifier, referrer, reached)
    }

    /// Resolves as an import does, roots and all, but admits no root and
    /// records no URL: the URL it returns isn't imported, and importing it
    /// resolves it again, as an absolute specifier.
    fn import_meta_resolve(
        &self,
        specifier: &str,
        referrer: &str,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        self.refuse_from_driver(specifier, referrer, Reached::MetaResolve)?;
        self.resolve_on_disk(specifier, referrer, Reached::MetaResolve)
    }

    fn load(
        &self,
        specifier: &ModuleSpecifier,
        referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        let reach = if options.is_dynamic_import {
            Reach::Dynamic
        } else {
            Reach::Static
        };
        ModuleLoadResponse::Sync(self.load_reached(
            specifier,
            &options.requested_module_type,
            reach,
            referrer,
        ))
    }
}

/// A tree whose reads are checked against the loader's box first
/// ([`Loader::check_read`]).
struct Boxed<'a, T> {
    /// The loader whose box checks the reads.
    loader: &'a Loader,
    /// The tree read.
    tree: &'a T,
}

impl<T: SourceTree> SourceTree for Boxed<'_, T> {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        self.tree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        self.check_read(path)?;
        self.tree.read(path, id)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        self.tree.is_world_writable(dir)
    }

    fn note_absent(&self, path: &Path) {
        self.tree.note_absent(path);
    }

    fn check_read(&self, path: &Path) -> io::Result<()> {
        self.loader.check_read(path)?;
        self.tree.check_read(path)
    }
}

/// Resolves `specifier`, imported by the module at `referrer`, into `graph`,
/// as [`Loader::resolve_reached`] describes.
fn resolve_into(
    graph: &mut Graph,
    tree: &impl SourceTree,
    specifier: &str,
    referrer: &str,
    reached: Reached,
) -> Result<ModuleSpecifier, Refused> {
    let recording = graph.recording();
    let importer = Url::parse(referrer).map_err(|error| {
        (
            reason(recording, || FailureError::UnknownImporter),
            ModuleLoaderError::type_error(format!(
                "cannot resolve '{specifier}': the importer '{referrer}' is not a URL: {error}"
            )),
        )
    })?;
    // every importer was resolved here first, the entry by `Loader::new`
    let Some(importer_record) = graph.records.get(&importer) else {
        return Err((
            reason(recording, || FailureError::UnknownImporter),
            ModuleLoaderError::type_error(format!(
                "cannot resolve '{specifier}': the importer '{referrer}' was never resolved"
            )),
        ));
    };
    // the record's root is the one its path is inside: admitting a root
    // around a recorded module is a conflict, so none moves it
    let importer_root = &importer_record.root;
    debug_assert_eq!(
        importer
            .to_file_path()
            .ok()
            .and_then(|path| graph.roots.containing(&path)),
        Some(importer_root),
    );
    let refused = |error: ResolveError| {
        let message = error.to_string();
        (
            reason(recording, || FailureError::Resolve(error.kind)),
            ModuleLoaderError::type_error(message),
        )
    };
    let resolved = if graph.is_full() {
        let tree = RecordingTree::new(tree, &mut graph.reads);
        resolver::resolve_from(&tree, specifier, &importer, importer_root, &graph.roots)
    } else {
        resolver::resolve_from(tree, specifier, &importer, importer_root, &graph.roots)
    };
    let mut resolution = resolved.map_err(refused)?;
    // read whether or not the root is admitted
    graph.consult(std::mem::take(&mut resolution.files_consulted));
    let Reached::Import(reach) = reached else {
        return Ok(resolution.url);
    };
    graph.admit(&resolution).map_err(|kind| {
        refused(ResolveError {
            specifier: specifier.to_owned(),
            importer: referrer.to_owned(),
            kind,
        })
    })?;
    graph.link(&importer, specifier, reach, &resolution.url);
    Ok(resolution.url)
}

/// Reads the module at `specifier`, a URL the resolver produced for the file
/// `file_id` names, once `check` allows its path, stripping a TypeScript
/// module's types, and returns it with, when `recording` is full, what was
/// read: its format and the hash of the bytes read.
fn load(
    specifier: &ModuleSpecifier,
    requested: &RequestedModuleType,
    file_id: FileId,
    recording: Recording,
    check: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(ModuleSource, Option<Loaded>), Refused> {
    let refused = |failure, error| (reason(recording, || FailureError::Load(failure)), error);
    // TODO: JSON modules (`with {type: 'json'}`) join with the module formats past JS
    if *requested != RequestedModuleType::None {
        return Err(refused(
            LoadFailure::ImportAttributes,
            ModuleLoaderError::type_error(format!(
                "cannot load {specifier}: import attributes are not supported yet"
            )),
        ));
    }
    let path = specifier.to_file_path().map_err(|()| {
        refused(
            LoadFailure::NotAFile,
            ModuleLoaderError::type_error(format!("cannot load {specifier}: not a local file")),
        )
    })?;
    // the resolver read the same table, so a refusal here means the URL skipped it
    let format = resolver::module_format(&path).map_err(|kind| {
        let error = ModuleLoaderError::type_error(format!("cannot load {specifier}: {kind}"));
        refused(LoadFailure::Format(kind), error)
    })?;
    let io_error = |error: io::Error| {
        refused(
            LoadFailure::Read(error.kind()),
            ModuleLoaderError::generic(format!("cannot load {specifier}: {error}")),
        )
    };
    check(&path).map_err(io_error)?;
    // read from the file checked, never the path again, which can change in
    // between, and never past the cap, so a huge file or a FIFO isn't read whole
    let read = resolver::read_checked(&path, file_id, Some(MAX_MODULE_SIZE)).map_err(|error| {
        if error.kind() != io::ErrorKind::FileTooLarge {
            return io_error(error);
        }
        refused(
            LoadFailure::TooLarge,
            ModuleLoaderError::type_error(format!(
                "cannot load {specifier}: the module is larger than {} MiB, the cap on a \
                 module's source",
                MAX_MODULE_SIZE >> 20
            )),
        )
    })?;
    let Some(code) = read else {
        return Err(refused(
            LoadFailure::IdentityMismatch,
            ModuleLoaderError::type_error(format!(
                "cannot load {specifier}: the file at its path is not the one resolved, so it \
                 isn't loaded"
            )),
        ));
    };
    // the bytes read, which V8 runs as is or stripped
    let loaded = recording.is_full().then(|| Loaded {
        format,
        hash: blake3::hash(&code),
    });
    let code = match format {
        ModuleFormat::JavaScript => ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
        ModuleFormat::TypeScript => {
            // invalid UTF-8 reads as U+FFFD, as V8 decodes a JS module's bytes
            let source = String::from_utf8(code)
                .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
            // the class `deno_core` drops: it rethrows every load failure to JS as a
            // `TypeError` (`js_class_and_message_to_exception`)
            let code = strip_module(source, specifier).map_err(|refusal| {
                let error = ModuleLoaderError::new("SyntaxError", refusal.message);
                let failure = LoadFailure::Strip {
                    failure: refusal.failure,
                    at: refusal.at,
                };
                refused(failure, error)
            })?;
            ModuleSourceCode::String(code.into())
        }
    };
    Ok((
        ModuleSource::new(ModuleType::JavaScript, code, specifier, None),
        loaded,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::path::{Path, PathBuf};

    use std::collections::BTreeSet;

    use super::graph::{Edge, FailureSite, Record};
    use super::strip::StripFailure;
    use super::*;
    use crate::confine::BoxRoots;
    use crate::resolver::test_tree::{MemoryTree, disk_test_dir};
    use crate::resolver::{ConsultedFile, FileRole, ResolveErrorKind};

    /// A project at `/p` with `bar` installed and `lnk` linked to a checkout
    /// inside the project.
    fn tree() -> MemoryTree {
        MemoryTree {
            files: BTreeMap::from([
                ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
                ("/p/main.ts", ""),
                (
                    "/p/packages/lnk/package.json",
                    r#"{"name": "lnk", "type": "module", "exports": {"./x.ts": "./x.ts"}}"#,
                ),
                ("/p/packages/lnk/x.ts", ""),
                (
                    "/p/node_modules/bar/package.json",
                    r#"{"exports": "./i.mjs"}"#,
                ),
                ("/p/node_modules/bar/i.mjs", ""),
            ]),
            links: BTreeMap::from([("/p/node_modules/lnk", "/p/packages/lnk")]),
            ..MemoryTree::default()
        }
    }

    const MAIN: &str = "file:///p/main.ts";

    /// A static import, the reach most tests resolve by.
    const STATIC: Reached = Reached::Import(Reach::Static);

    /// A loader for the graph of `/p/main.ts` in `tree`, recording in full.
    fn new_loader(tree: &MemoryTree) -> Loader {
        let (entry, roots) =
            resolver::resolve_entry(tree, Path::new("/p/main.ts"), Path::new("/")).unwrap();
        Loader::new(&entry, roots, Recording::Full)
    }

    fn state(loader: &Loader) -> (Roots, HashMap<Url, Record>) {
        let graph = loader.graph.borrow();
        (graph.roots.clone(), graph.records.clone())
    }

    /// A file reached both by a relative specifier and through its package
    /// is refused in either order, never loaded under the first one's root.
    #[test]
    fn a_root_conflict_is_refused_in_either_order() {
        let tree = tree();
        // the package first: the relative specifier is refused by the resolver
        let loader = new_loader(&tree);
        loader
            .resolve_reached(&tree, "lnk/x.ts", MAIN, STATIC)
            .unwrap();
        let error = loader
            .resolve_reached(&tree, "./packages/lnk/x.ts", MAIN, STATIC)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "cannot resolve './packages/lnk/x.ts' from file:///p/main.ts: it resolves to \
                 /p/packages/lnk/x.ts, inside package 'lnk' at /p/packages/lnk, which only a \
                 bare specifier reaches"
            ),
            "{error}"
        );
        // the relative specifier first: admitting the package is the conflict
        let loader = new_loader(&tree);
        loader
            .resolve_reached(&tree, "./packages/lnk/x.ts", MAIN, STATIC)
            .unwrap();
        let before = state(&loader);
        let error = loader
            .resolve_reached(&tree, "lnk/x.ts", MAIN, STATIC)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "cannot resolve 'lnk/x.ts' from file:///p/main.ts: /p/packages/lnk/x.ts was \
                 resolved inside the project at /p, and package 'lnk' at /p/packages/lnk was \
                 then admitted around it, so its origin would depend on the import order. Fix: \
                 import it by name ('lnk' or a subpath it exports) everywhere, not by a relative \
                 path"
            ),
            "{error}"
        );
        // nothing was admitted, so it fails the same way again
        assert_eq!(state(&loader), before);
        assert!(
            loader
                .resolve_reached(&tree, "lnk/x.ts", MAIN, STATIC)
                .is_err()
        );
    }

    /// Each record's root is the one its path is inside, which the loader
    /// relies on to resolve from the importer's record without walking its
    /// path: across project, installed, in-tree linked, and out-of-tree
    /// linked roots, their modules' own imports, and a query.
    #[test]
    fn a_record_s_root_is_the_one_its_path_is_inside() {
        let tree = MemoryTree {
            files: BTreeMap::from([
                ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
                ("/p/main.ts", ""),
                ("/p/a.ts", ""),
                (
                    "/p/packages/lnk/package.json",
                    r#"{"name": "lnk", "type": "module", "exports": {"./x.ts": "./x.ts"}}"#,
                ),
                ("/p/packages/lnk/x.ts", ""),
                ("/p/packages/lnk/y.ts", ""),
                (
                    "/p/node_modules/bar/package.json",
                    r#"{"exports": "./i.mjs"}"#,
                ),
                ("/p/node_modules/bar/i.mjs", ""),
                ("/p/node_modules/bar/sub/j.mjs", ""),
                (
                    "/q/far/package.json",
                    r#"{"name": "far", "type": "module", "exports": "./f.ts"}"#,
                ),
                ("/q/far/f.ts", ""),
                ("/q/far/g.ts", ""),
            ]),
            links: BTreeMap::from([
                ("/p/node_modules/lnk", "/p/packages/lnk"),
                ("/p/node_modules/far", "/q/far"),
            ]),
            ..MemoryTree::default()
        };
        let loader = new_loader(&tree);
        for (specifier, referrer) in [
            ("./a.ts?v=1", MAIN),
            ("bar", MAIN),
            ("./sub/j.mjs", "file:///p/node_modules/bar/i.mjs"),
            ("lnk/x.ts", MAIN),
            ("./y.ts", "file:///p/packages/lnk/x.ts"),
            ("far", MAIN),
            ("./g.ts", "file:///q/far/f.ts"),
        ] {
            loader
                .resolve_reached(&tree, specifier, referrer, STATIC)
                .unwrap_or_else(|error| panic!("{specifier} from {referrer}: {error}"));
        }
        let (roots, records) = state(&loader);
        assert_eq!(records.len(), 8);
        for (url, record) in &records {
            let path = url.to_file_path().unwrap();
            assert_eq!(roots.containing(&path), Some(&record.root), "{url}");
        }
    }

    /// `deno_core` can resolve a specifier twice: the same URL inside the
    /// same root changes nothing.
    #[test]
    fn resolving_twice_is_no_change() {
        let tree = tree();
        let loader = new_loader(&tree);
        let first = loader.resolve_reached(&tree, "bar", MAIN, STATIC).unwrap();
        let after_first = state(&loader);
        let second = loader.resolve_reached(&tree, "bar", MAIN, STATIC).unwrap();
        assert_eq!(first, second);
        assert_eq!(state(&loader), after_first);
        assert_eq!(after_first.0.iter().count(), 2);
        assert_eq!(after_first.1.len(), 2);
    }

    /// `import.meta.resolve` resolves as an import does but admits and
    /// records nothing, so importing what it returned for a package is a
    /// path into `node_modules`, refused.
    #[test]
    fn import_meta_resolve_changes_nothing() {
        let tree = tree();
        let loader = new_loader(&tree);
        let before = state(&loader);
        let url = loader
            .resolve_reached(&tree, "bar", MAIN, Reached::MetaResolve)
            .unwrap();
        assert_eq!(url.as_str(), "file:///p/node_modules/bar/i.mjs");
        assert_eq!(state(&loader), before);
        let error = loader
            .resolve_reached(&tree, url.as_str(), MAIN, STATIC)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("inside node_modules below /p") && error.contains("import it by name"),
            "{error}"
        );
        // refused as an import is
        assert!(
            loader
                .resolve_reached(
                    &tree,
                    "./node_modules/bar/i.mjs",
                    MAIN,
                    Reached::MetaResolve
                )
                .is_err()
        );
        // and an importer must have been resolved here
        let error = loader
            .resolve_reached(&tree, "./main.ts", url.as_str(), Reached::MetaResolve)
            .unwrap_err()
            .to_string();
        assert!(error.contains("was never resolved"), "{error}");
    }

    /// Files on disk in a fresh directory (see [`disk_test_dir`]), removed on
    /// drop, under a named `package.json`, so the project root is the
    /// directory itself. Every directory is `0o755`, so a umask of 0 can't
    /// make one world-writable.
    struct DiskFixtures(PathBuf);

    impl DiskFixtures {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            use std::os::unix::fs::PermissionsExt as _;
            let dir = disk_test_dir(&format!("loader_{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            let package_json = [("package.json", r#"{"name": "fixture", "type": "module"}"#)];
            for (file, content) in files.iter().chain(&package_json) {
                let path = dir.join(file);
                let parent = path.parent().unwrap();
                std::fs::create_dir_all(parent).unwrap();
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
                std::fs::write(path, content).unwrap();
            }
            Self(dir.canonicalize().unwrap())
        }

        fn path(&self, file: &str) -> PathBuf {
            self.0.join(file)
        }

        fn url(&self, file: &str) -> Url {
            Url::from_file_path(self.path(file)).unwrap()
        }

        /// A loader for the graph of `entry`, recording in full.
        fn loader(&self, entry: &str) -> Loader {
            self.loader_recording(entry, Recording::Full)
        }

        /// A loader for the graph of `entry`, recording as `recording` says.
        fn loader_recording(&self, entry: &str, recording: Recording) -> Loader {
            let (entry, roots) =
                resolver::resolve_entry(&DiskTree, Path::new(entry), &self.0).unwrap();
            Loader::new(&entry, roots, recording)
        }

        /// Replaces the file at `file` with a new one holding `content`,
        /// written beside it first and renamed over it, so the old file's
        /// inode is still in use when the new one is made and can't be
        /// reused for it.
        fn replace(&self, file: &str, content: &str) {
            let replacement = self.path(&format!("{file}.replacement"));
            std::fs::write(&replacement, content).unwrap();
            std::fs::rename(replacement, self.path(file)).unwrap();
        }

        /// Puts a symlink to `target` at `file`, made beside it and renamed
        /// over whatever is there.
        fn link(&self, file: &str, target: &str) {
            let link = self.path(&format!("{file}.link"));
            std::os::unix::fs::symlink(target, &link).unwrap();
            std::fs::rename(link, self.path(file)).unwrap();
        }
    }

    impl Drop for DiskFixtures {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SOURCE: &str = "export const a = 1;\n";

    /// A loader over `/p/main.ts` in `tree`, its reads checked against the
    /// box `roots`.
    fn boxed_loader(tree: &MemoryTree, roots: &[&str]) -> Loader {
        let roots = roots.iter().map(PathBuf::from).collect();
        new_loader(tree).with_rules(Rules {
            box_roots: Some(BoxRoots::for_test(roots)),
            ..Rules::default()
        })
    }

    /// A `package.json` read outside the box is refused before it is read,
    /// and the loader records the breach; inside it, nothing is.
    #[test]
    fn a_read_outside_the_box_is_a_breach() {
        let mut tree = tree();
        tree.files.insert(
            "/q/ext/package.json",
            r#"{"name": "ext", "type": "module", "exports": "./x.js"}"#,
        );
        tree.files.insert("/q/ext/x.js", "");
        tree.links.insert("/p/node_modules/ext", "/q/ext");
        let resolve = |loader: &Loader, specifier: &str| {
            let boxed = Boxed {
                loader,
                tree: &tree,
            };
            loader.resolve_reached(&boxed, specifier, MAIN, STATIC)
        };

        let inside = boxed_loader(&tree, &["/p", "/q/ext"]);
        assert!(resolve(&inside, "ext").is_ok());
        assert!(resolve(&inside, "bar").is_ok());
        assert_eq!(inside.breach(), None);

        let outside = boxed_loader(&tree, &["/p"]);
        assert!(resolve(&outside, "bar").is_ok());
        assert_eq!(outside.breach(), None);
        let error = resolve(&outside, "ext").unwrap_err();
        assert!(error.to_string().contains("outside the box"), "{error}");
        assert_eq!(outside.breach(), Some(PathBuf::from("/q/ext/package.json")));
        // the first breach stands
        let _ = resolve(&outside, "ext");
        assert_eq!(outside.breach(), Some(PathBuf::from("/q/ext/package.json")));
    }

    /// A `package.json` the pre-pass read before the box existed is served
    /// from that read only inside the box: symlinked out of it, the walk up
    /// a bare import makes is refused as a breach, as a fresh read would be.
    #[test]
    fn a_read_the_prepass_kept_is_checked_against_the_box() {
        let mut tree = tree();
        let project = tree.files.remove("/p/package.json").unwrap();
        tree.files.insert("/q/pkg.json", project);
        tree.links.insert("/p/package.json", "/q/pkg.json");
        let mut reads = Reads::default();
        let (entry, roots) = resolver::resolve_entry(
            &RecordingTree::new(&tree, &mut reads),
            Path::new("/p/main.ts"),
            Path::new("/"),
        )
        .unwrap();
        assert!(reads.hash(Path::new("/q/pkg.json")).is_some());
        let loader = Loader::new(&entry, roots, Recording::Full)
            .with_reads(reads)
            .with_rules(Rules {
                box_roots: Some(BoxRoots::for_test(vec![PathBuf::from("/p")])),
                ..Rules::default()
            });
        let boxed = Boxed {
            loader: &loader,
            tree: &tree,
        };
        let error = loader
            .resolve_reached(&boxed, "bar", MAIN, STATIC)
            .unwrap_err();
        assert!(error.to_string().contains("outside the box"), "{error}");
        assert_eq!(loader.breach(), Some(PathBuf::from("/q/pkg.json")));
    }

    /// A module outside the box isn't read, and the loader records the
    /// breach; with no box, reads aren't checked.
    #[test]
    fn a_module_outside_the_box_is_not_read() {
        let fixtures = DiskFixtures::new("box", &[("main.js", SOURCE)]);
        let url = fixtures.url("main.js");
        let unboxed = fixtures.loader("main.js");
        assert_eq!(load_disk(&unboxed, &url).as_deref(), Ok(SOURCE));
        assert_eq!(unboxed.breach(), None);
        let inside = fixtures.loader("main.js").with_rules(Rules {
            box_roots: Some(BoxRoots::for_test(vec![fixtures.0.clone()])),
            ..Rules::default()
        });
        assert_eq!(load_disk(&inside, &url).as_deref(), Ok(SOURCE));
        assert_eq!(inside.breach(), None);
        let elsewhere = fixtures.0.with_file_name("elsewhere");
        let outside = fixtures.loader("main.js").with_rules(Rules {
            box_roots: Some(BoxRoots::for_test(vec![elsewhere])),
            ..Rules::default()
        });
        let error = load_disk(&outside, &url).unwrap_err();
        assert!(error.contains("outside the box"), "{error}");
        assert_eq!(outside.breach(), Some(fixtures.path("main.js")));
    }

    fn load_disk(loader: &Loader, url: &Url) -> Result<String, String> {
        loader
            .load_reached(url, &RequestedModuleType::None, Reach::Static, None)
            .map(|source| String::from_utf8(source.code.as_bytes().to_vec()).unwrap())
            .map_err(|error| error.to_string())
    }

    fn assert_refused(result: Result<String, String>, url: &Url) {
        let error = result.unwrap_err();
        assert!(
            error.contains(&format!(
                "cannot load {url}: the file at its path is not the one resolved, so it isn't \
                 loaded"
            )),
            "{error}"
        );
    }

    /// The control: a file left alone between resolving and loading loads,
    /// the entry's and an import's alike.
    #[test]
    fn an_unchanged_file_loads() {
        let fixtures = DiskFixtures::new("unchanged", &[("main.ts", ""), ("a.js", SOURCE)]);
        let loader = fixtures.loader("main.ts");
        let main = fixtures.url("main.ts");
        let url = loader
            .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
            .unwrap();
        assert_eq!(load_disk(&loader, &url).unwrap(), SOURCE);
        assert_eq!(load_disk(&loader, &main).unwrap(), "");
        // resolving it again finds the same file
        loader
            .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
            .unwrap();
        // and a URL never resolved here isn't loaded
        let error = load_disk(&loader, &fixtures.url("package.json")).unwrap_err();
        assert!(error.contains("it was never resolved"), "{error}");
    }

    /// A file replaced by a rename after it was resolved isn't loaded, for an
    /// import and the entry, and resolving its URL again keeps the first
    /// record, so the load is still refused.
    #[test]
    fn a_file_renamed_over_is_refused() {
        let fixtures = DiskFixtures::new("renamed", &[("main.ts", ""), ("a.js", SOURCE)]);
        let loader = fixtures.loader("main.ts");
        let main = fixtures.url("main.ts");
        let url = loader
            .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
            .unwrap();
        fixtures.replace("a.js", "export const a = 2;\n");
        assert_refused(load_disk(&loader, &url), &url);
        // resolving it again keeps the first file, which the load still checks
        loader
            .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
            .unwrap();
        assert_refused(load_disk(&loader, &url), &url);
        fixtures.replace("main.ts", "");
        assert_refused(load_disk(&loader, &main), &main);
    }

    /// A directory on the path swapped for a symlink to a twin directory
    /// holding a file of the same name and content: the path now opens
    /// another file, refused.
    #[test]
    fn a_directory_swapped_for_a_link_is_refused() {
        let fixtures = DiskFixtures::new(
            "component",
            &[("main.ts", ""), ("sub/a.js", SOURCE), ("twin/a.js", SOURCE)],
        );
        let loader = fixtures.loader("main.ts");
        let url = loader
            .resolve_reached(
                &DiskTree,
                "./sub/a.js",
                fixtures.url("main.ts").as_str(),
                STATIC,
            )
            .unwrap();
        std::fs::rename(fixtures.path("sub"), fixtures.path("old_sub")).unwrap();
        fixtures.link("sub", "twin");
        assert_refused(load_disk(&loader, &url), &url);
    }

    /// The file swapped for a symlink to an identical file: the open follows
    /// it to a file with another identity, refused.
    #[test]
    fn a_file_swapped_for_a_link_is_refused() {
        let fixtures = DiskFixtures::new(
            "last_component",
            &[("main.ts", ""), ("a.js", SOURCE), ("copy.js", SOURCE)],
        );
        let loader = fixtures.loader("main.ts");
        let url = loader
            .resolve_reached(
                &DiskTree,
                "./a.js",
                fixtures.url("main.ts").as_str(),
                STATIC,
            )
            .unwrap();
        fixtures.link("a.js", "copy.js");
        assert_refused(load_disk(&loader, &url), &url);
    }

    /// A file removed and made again after it was resolved isn't loaded,
    /// though ext4 hands the new file the old one's freed inode number.
    #[test]
    fn a_file_removed_and_remade_is_refused() {
        let fixtures = DiskFixtures::new("remade", &[("main.ts", ""), ("a.js", SOURCE)]);
        let loader = fixtures.loader("main.ts");
        let url = loader
            .resolve_reached(
                &DiskTree,
                "./a.js",
                fixtures.url("main.ts").as_str(),
                STATIC,
            )
            .unwrap();
        // past the filesystem clock's tick, so the birth times differ
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::remove_file(fixtures.path("a.js")).unwrap();
        std::fs::write(fixtures.path("a.js"), "export const a = 2;\n").unwrap();
        assert_refused(load_disk(&loader, &url), &url);
    }

    /// A FIFO put in the file's place after it was resolved is refused
    /// without blocking on the open. The load runs on its own thread, so a
    /// blocking open fails the test by its timeout rather than hanging it.
    #[test]
    fn a_fifo_swapped_in_is_refused_without_blocking() {
        let fixtures = DiskFixtures::new("fifo", &[("main.ts", ""), ("a.js", SOURCE)]);
        let loader = fixtures.loader("main.ts");
        let url = loader
            .resolve_reached(
                &DiskTree,
                "./a.js",
                fixtures.url("main.ts").as_str(),
                STATIC,
            )
            .unwrap();
        let fifo = fixtures.path("a.js.fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        std::fs::rename(fifo, fixtures.path("a.js")).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let load_url = url.clone();
        std::thread::spawn(move || {
            let _ = send.send(load_disk(&loader, &load_url));
        });
        let Ok(result) = receive.recv_timeout(std::time::Duration::from_secs(5)) else {
            // a writer releases the blocked open, so the thread ends
            drop(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(fixtures.path("a.js")),
            );
            panic!("opening a FIFO in the file's place blocked");
        };
        assert_refused(result, &url);
    }

    fn graph(loader: &Loader) -> Graph {
        loader.graph()
    }

    fn url(path: &str) -> Url {
        Url::from_file_path(path).unwrap()
    }

    fn edge(importer: &str, specifier: &str, reach: Reach) -> Edge {
        Edge {
            importer: Url::parse(importer).unwrap(),
            specifier: specifier.to_owned(),
            reach,
        }
    }

    fn site(specifier: &str, importer: Option<&str>, reached: Reached) -> FailureSite {
        FailureSite {
            specifier: specifier.to_owned(),
            importer: importer.map(str::to_owned),
            reached,
        }
    }

    fn consulted(path: &str, role: FileRole) -> ConsultedFile {
        ConsultedFile {
            path: PathBuf::from(path),
            role,
        }
    }

    /// The one failure at a site.
    fn only(failures: &[Failure]) -> &Failure {
        assert_eq!(failures.len(), 1, "{failures:?}");
        &failures[0]
    }

    /// An import is an edge per reach, once however often `deno_core`
    /// resolves it, and `import.meta.resolve` adds none.
    #[test]
    fn each_import_is_one_edge_per_reach() {
        let tree = tree();
        let loader = new_loader(&tree);
        let static_ = Reached::Import(Reach::Static);
        let dynamic = Reached::Import(Reach::Dynamic);
        for reached in [static_, static_, dynamic, dynamic, Reached::MetaResolve] {
            loader.resolve_reached(&tree, "bar", MAIN, reached).unwrap();
        }
        loader
            .resolve_reached(&tree, "lnk/x.ts", MAIN, Reached::MetaResolve)
            .unwrap();
        let bar = url("/p/node_modules/bar/i.mjs");
        assert_eq!(
            graph(&loader).edges,
            BTreeMap::from([
                (edge(MAIN, "bar", Reach::Static), bar.clone()),
                (edge(MAIN, "bar", Reach::Dynamic), bar),
            ])
        );
        assert!(graph(&loader).failures.is_empty());
    }

    /// The entry's consulted files are in the graph from the start, and every
    /// resolution's join them once each, `import.meta.resolve`'s and a
    /// refused admission's included.
    #[test]
    fn the_consulted_files_are_the_union_of_every_resolution() {
        let tree = tree();
        let loader = new_loader(&tree);
        let search_bound = consulted("/p/package.json", FileRole::SearchBound);
        assert_eq!(
            graph(&loader).consulted,
            BTreeSet::from([search_bound.clone()])
        );
        // read without admitting anything
        loader
            .resolve_reached(&tree, "bar", MAIN, Reached::MetaResolve)
            .unwrap();
        // the importer's scope, for a self-reference, then the package's
        let scope = consulted("/p/package.json", FileRole::PackageScope);
        let bar = consulted("/p/node_modules/bar/package.json", FileRole::Exports);
        assert_eq!(
            graph(&loader).consulted,
            BTreeSet::from([search_bound.clone(), scope.clone(), bar.clone()])
        );
        loader.resolve_reached(&tree, "bar", MAIN, STATIC).unwrap();
        loader
            .resolve_reached(&tree, "./packages/lnk/x.ts", MAIN, STATIC)
            .unwrap();
        // refused when admitting its root, after it was read
        loader
            .resolve_reached(&tree, "lnk/x.ts", MAIN, STATIC)
            .unwrap_err();
        let lnk = consulted("/p/packages/lnk/package.json", FileRole::Exports);
        assert_eq!(
            graph(&loader).consulted,
            BTreeSet::from([search_bound, scope, bar, lnk])
        );
    }

    /// A tree that counts its reads.
    struct Counting {
        tree: MemoryTree,
        reads: std::cell::Cell<usize>,
    }

    impl SourceTree for Counting {
        fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
            self.tree.lookup(path)
        }

        fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
            self.reads.set(self.reads.get() + 1);
            self.tree.read(path, id)
        }

        fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
            self.tree.is_world_writable(dir)
        }
    }

    /// Resolves `bar` from the entry and from a module in a subdirectory,
    /// recording as `recording` says, and returns the graph and how many
    /// reads reached the tree.
    fn resolve_bar_twice(recording: Recording) -> (Graph, usize) {
        let mut tree = tree();
        tree.files.insert("/p/src/deep.ts", "");
        let (entry, roots) =
            resolver::resolve_entry(&tree, Path::new("/p/main.ts"), Path::new("/")).unwrap();
        let tree = Counting {
            tree,
            reads: std::cell::Cell::new(0),
        };
        let loader = Loader::new(&entry, roots, recording);
        loader.resolve_reached(&tree, "bar", MAIN, STATIC).unwrap();
        let deep = loader
            .resolve_reached(&tree, "./src/deep.ts", MAIN, STATIC)
            .unwrap();
        loader
            .resolve_reached(&tree, "bar", deep.as_str(), STATIC)
            .unwrap();
        (loader.into_graph(), tree.reads.get())
    }

    /// Recording in full, each `package.json` is read once however many
    /// resolutions consult it, and hashed as read, and the paths a
    /// resolution went on past are recorded absent: the subdirectory's own
    /// `package.json`, on the walks up, and its `node_modules` entry, on
    /// the search. Off, every resolution reads again and nothing is kept.
    #[test]
    fn a_consulted_file_is_read_once_and_each_path_passed_is_absent() {
        let (graph, reads) = resolve_bar_twice(Recording::Full);
        assert_eq!(reads, 2);
        assert_eq!(
            graph.reads.hash(Path::new("/p/package.json")),
            Some(blake3::hash(br#"{"name": "proj", "type": "module"}"#))
        );
        assert_eq!(
            graph
                .reads
                .hash(Path::new("/p/node_modules/bar/package.json")),
            Some(blake3::hash(br#"{"exports": "./i.mjs"}"#))
        );
        // every consulted file has its read's hash
        assert!(
            graph
                .consulted
                .iter()
                .all(|file| graph.reads.hash(&file.path).is_some())
        );
        assert_eq!(
            graph.reads.absent(),
            &BTreeSet::from([
                PathBuf::from("/p/src/node_modules/bar"),
                PathBuf::from("/p/src/package.json"),
            ])
        );
        let (off, off_reads) = resolve_bar_twice(Recording::Off);
        assert_eq!(off_reads, 4);
        assert_eq!(off.reads.hash(Path::new("/p/package.json")), None);
        assert!(off.reads.absent().is_empty());
        // and a record passed in is dropped off
        let mut reads = Reads::default();
        RecordingTree::new(&tree(), &mut reads).note_absent(Path::new("/p/x"));
        let (entry, roots) =
            resolver::resolve_entry(&tree(), Path::new("/p/main.ts"), Path::new("/")).unwrap();
        let loader = Loader::new(&entry, roots.clone(), Recording::Off).with_reads(reads.clone());
        assert!(loader.into_graph().reads.absent().is_empty());
        let loader = Loader::new(&entry, roots, Recording::Full).with_reads(reads);
        assert_eq!(loader.into_graph().reads.absent().len(), 1);
    }

    /// A `package.json` swapped for another file between two resolutions
    /// that read it is refused at the second, recording in full, rather
    /// than read as another file under the same path.
    #[test]
    fn a_consulted_file_of_another_identity_is_refused_in_one_run() {
        let fixtures = DiskFixtures::new(
            "consulted_swap",
            &[
                ("main.ts", ""),
                ("sub/package.json", r#"{"type": "module"}"#),
                ("sub/b.js", SOURCE),
            ],
        );
        let main = fixtures.url("main.ts");
        let loader = fixtures.loader("main.ts");
        loader
            .resolve_reached(&DiskTree, "./sub/b.js", main.as_str(), STATIC)
            .unwrap();
        fixtures.replace("sub/package.json", r#"{"type": "module"}"#);
        let error = loader
            .resolve_reached(&DiskTree, "./sub/b.js?again", main.as_str(), STATIC)
            .unwrap_err()
            .to_string();
        assert!(error.contains(Reads::CHANGED), "{error}");
        // off, the new file is read
        let loader = fixtures.loader_recording("main.ts", Recording::Off);
        loader
            .resolve_reached(&DiskTree, "./sub/b.js", main.as_str(), STATIC)
            .unwrap();
        fixtures.replace("sub/package.json", r#"{"type": "module"}"#);
        loader
            .resolve_reached(&DiskTree, "./sub/b.js?again", main.as_str(), STATIC)
            .unwrap();
    }

    /// A `package.json` rewritten in place between two resolutions that
    /// read it, its identity unchanged, is served the first read's bytes,
    /// recording in full, and its hash is theirs; off, the second
    /// resolution reads the new bytes.
    #[test]
    fn a_consulted_file_rewritten_in_place_serves_the_first_read() {
        const MODULE: &str = r#"{"type": "module"}"#;
        let fixtures = DiskFixtures::new(
            "consulted_rewrite",
            &[
                ("main.ts", ""),
                ("sub/package.json", MODULE),
                ("sub/b.js", SOURCE),
            ],
        );
        let main = fixtures.url("main.ts");
        let rewrite = || {
            std::fs::write(fixtures.path("sub/package.json"), r#"{"type": "commonjs"}"#).unwrap();
        };
        let loader = fixtures.loader("main.ts");
        loader
            .resolve_reached(&DiskTree, "./sub/b.js", main.as_str(), STATIC)
            .unwrap();
        rewrite();
        loader
            .resolve_reached(&DiskTree, "./sub/b.js?again", main.as_str(), STATIC)
            .unwrap();
        assert_eq!(
            loader
                .into_graph()
                .reads
                .hash(&fixtures.path("sub/package.json")),
            Some(blake3::hash(MODULE.as_bytes()))
        );
        // off, the rewritten file is read
        std::fs::write(fixtures.path("sub/package.json"), MODULE).unwrap();
        let loader = fixtures.loader_recording("main.ts", Recording::Off);
        loader
            .resolve_reached(&DiskTree, "./sub/b.js", main.as_str(), STATIC)
            .unwrap();
        rewrite();
        let error = loader
            .resolve_reached(&DiskTree, "./sub/b.js?again", main.as_str(), STATIC)
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not set"), "{error}");
    }

    /// A refused resolution is recorded by its specifier, importer, and
    /// reach, with the resolver's reason, once however often it is retried;
    /// a success records none.
    #[test]
    fn a_refused_resolution_is_recorded_once_per_site() {
        let tree = tree();
        let loader = new_loader(&tree);
        loader.resolve_reached(&tree, "bar", MAIN, STATIC).unwrap();
        assert!(graph(&loader).failures.is_empty());
        for reached in [
            Reached::Import(Reach::Static),
            Reached::Import(Reach::Dynamic),
            Reached::Import(Reach::Dynamic),
            Reached::MetaResolve,
        ] {
            loader
                .resolve_reached(&tree, "./nope.js", MAIN, reached)
                .unwrap_err();
        }
        let failures = graph(&loader).failures;
        assert_eq!(failures.len(), 3, "{failures:?}");
        for reached in [
            Reached::Import(Reach::Static),
            Reached::Import(Reach::Dynamic),
            Reached::MetaResolve,
        ] {
            let failure = only(&failures[&site("./nope.js", Some(MAIN), reached)]);
            assert_eq!(failure.at, None);
            assert!(
                matches!(
                    failure.error,
                    FailureError::Resolve(ResolveErrorKind::NotFound { .. })
                ),
                "{failure:?}"
            );
        }
    }

    /// A root conflict is a resolve failure, and an importer this graph
    /// never resolved is its own.
    #[test]
    fn a_root_conflict_and_an_unknown_importer_are_recorded() {
        let tree = tree();
        let loader = new_loader(&tree);
        loader
            .resolve_reached(&tree, "./packages/lnk/x.ts", MAIN, STATIC)
            .unwrap();
        loader
            .resolve_reached(&tree, "lnk/x.ts", MAIN, STATIC)
            .unwrap_err();
        loader
            .resolve_reached(&tree, "./x.ts", "file:///p/other.ts", STATIC)
            .unwrap_err();
        loader
            .resolve_reached(&tree, "./x.ts", "not a url", Reached::MetaResolve)
            .unwrap_err();
        let failures = graph(&loader).failures;
        assert_eq!(failures.len(), 3, "{failures:?}");
        assert!(matches!(
            only(&failures[&site("lnk/x.ts", Some(MAIN), Reached::Import(Reach::Static))]).error,
            FailureError::Resolve(ResolveErrorKind::RootConflict { .. })
        ));
        assert_eq!(
            only(
                &failures[&site(
                    "./x.ts",
                    Some("file:///p/other.ts"),
                    Reached::Import(Reach::Static)
                )]
            )
            .error,
            FailureError::UnknownImporter
        );
        assert_eq!(
            only(&failures[&site("./x.ts", Some("not a url"), Reached::MetaResolve)]).error,
            FailureError::UnknownImporter
        );
        // the conflict admitted nothing, so no edge for it
        assert_eq!(
            graph(&loader).edges.keys().cloned().collect::<Vec<_>>(),
            [edge(MAIN, "./packages/lnk/x.ts", Reach::Static)]
        );
    }

    const TYPED: &str = "export const a: number = 1;\n";

    /// Recording in full, a load records its format and the hash of the
    /// bytes read, before the strip.
    #[test]
    fn a_load_records_its_format_and_hash() {
        let fixtures = DiskFixtures::new(
            "hash",
            &[("main.ts", ""), ("a.ts", TYPED), ("b.js", SOURCE)],
        );
        let main = fixtures.url("main.ts");
        let loader = fixtures.loader("main.ts");
        let a = loader
            .resolve_reached(&DiskTree, "./a.ts", main.as_str(), STATIC)
            .unwrap();
        let b = loader
            .resolve_reached(&DiskTree, "./b.js", main.as_str(), STATIC)
            .unwrap();
        let stripped = load_disk(&loader, &a).unwrap();
        assert_ne!(stripped, TYPED);
        load_disk(&loader, &b).unwrap();
        let graph = graph(&loader);
        assert_eq!(
            graph.records[&a].loaded,
            Some(Loaded {
                format: ModuleFormat::TypeScript,
                hash: blake3::hash(TYPED.as_bytes()),
            })
        );
        assert_eq!(
            graph.records[&b].loaded,
            Some(Loaded {
                format: ModuleFormat::JavaScript,
                hash: blake3::hash(SOURCE.as_bytes()),
            })
        );
        // not loaded yet
        assert_eq!(graph.records[&main].loaded, None);
        assert!(graph.failures.is_empty());
    }

    /// Recording off, the loader resolves, admits, and loads as it does in
    /// full, and refuses the same, but records no load, edge, consulted file,
    /// or failure.
    #[test]
    fn recording_off_records_only_roots_and_files() {
        let fixtures = DiskFixtures::new(
            "recording_off",
            &[("main.ts", ""), ("a.ts", TYPED), ("b.js", SOURCE)],
        );
        let main = fixtures.url("main.ts");
        let loader = fixtures.loader_recording("main.ts", Recording::Off);
        let a = loader
            .resolve_reached(&DiskTree, "./a.ts", main.as_str(), STATIC)
            .unwrap();
        let b = loader
            .resolve_reached(
                &DiskTree,
                "./b.js",
                main.as_str(),
                Reached::Import(Reach::Dynamic),
            )
            .unwrap();
        loader
            .resolve_reached(&DiskTree, "./a.ts", main.as_str(), Reached::MetaResolve)
            .unwrap();
        assert_ne!(load_disk(&loader, &a).unwrap(), TYPED);
        // a refused resolution, and a refused load
        loader
            .resolve_reached(&DiskTree, "./nope.js", main.as_str(), STATIC)
            .unwrap_err();
        fixtures.replace("b.js", SOURCE);
        assert_refused(load_disk(&loader, &b), &b);
        let graph = graph(&loader);
        assert!(!graph.is_full());
        let mut recorded: Vec<_> = graph.records.keys().cloned().collect();
        recorded.sort();
        let mut expected = vec![main, a, b];
        expected.sort();
        assert_eq!(recorded, expected);
        assert!(graph.records.values().all(|record| record.loaded.is_none()));
        assert!(graph.edges.is_empty());
        assert!(graph.consulted.is_empty());
        assert!(graph.failures.is_empty());
    }

    /// Resolving a loaded URL again, after its file changed, keeps the first
    /// record, its load and hash included, so they still describe the file
    /// V8 ran.
    #[test]
    fn a_re_resolve_keeps_the_loaded_record() {
        let fixtures = DiskFixtures::new("re_resolve", &[("main.ts", ""), ("a.ts", TYPED)]);
        let main = fixtures.url("main.ts");
        let loader = fixtures.loader("main.ts");
        let a = loader
            .resolve_reached(&DiskTree, "./a.ts", main.as_str(), STATIC)
            .unwrap();
        load_disk(&loader, &a).unwrap();
        let before = graph(&loader).records[&a].clone();
        fixtures.replace("a.ts", "export const a = 2;\n");
        loader
            .resolve_reached(&DiskTree, "./a.ts", main.as_str(), STATIC)
            .unwrap();
        let after = &graph(&loader).records[&a];
        assert_eq!(*after, before);
        assert_eq!(
            after.loaded.map(|loaded| loaded.hash),
            Some(blake3::hash(TYPED.as_bytes()))
        );
    }

    /// Each refused load is recorded with its reason, by its URL, and with
    /// the importer and position `deno_core` gives for a static import.
    #[test]
    fn a_refused_load_is_recorded_with_its_reason() {
        let fixtures = DiskFixtures::new(
            "load_failures",
            &[
                ("main.ts", ""),
                ("enum.ts", "const a = 1;\n  enum E { A }\n"),
                ("a.js", SOURCE),
                ("b.js", SOURCE),
            ],
        );
        let main = fixtures.url("main.ts");
        let loader = fixtures.loader("main.ts");
        let resolve = |specifier: &str| {
            loader
                .resolve_reached(&DiskTree, specifier, main.as_str(), STATIC)
                .unwrap()
        };
        let referrer = ModuleLoadReferrer {
            specifier: main.clone(),
            line_number: 3,
            column_number: 17,
        };
        let static_ = Reached::Import(Reach::Static);
        let dynamic = Reached::Import(Reach::Dynamic);
        // a strip refusal, its message as before
        let enum_ts = resolve("./enum.ts");
        let error = loader
            .load_reached(
                &enum_ts,
                &RequestedModuleType::None,
                Reach::Static,
                Some(&referrer),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot strip an enum. Fix: "), "{error}");
        // a replaced file, twice, recorded once
        let a = resolve("./a.js");
        fixtures.replace("a.js", SOURCE);
        for _ in 0..2 {
            loader
                .load_reached(&a, &RequestedModuleType::None, Reach::Dynamic, None)
                .unwrap_err();
        }
        // import attributes
        let b = resolve("./b.js");
        loader
            .load_reached(
                &b,
                &RequestedModuleType::Json,
                Reach::Static,
                Some(&referrer),
            )
            .unwrap_err();
        // a URL never resolved here
        let never = fixtures.url("package.json");
        load_disk(&loader, &never).unwrap_err();
        let failures = graph(&loader).failures;
        assert_eq!(failures.len(), 4, "{failures:?}");
        let at = Some(Position {
            line: 3,
            column: 17,
        });
        assert_eq!(
            *only(&failures[&site(enum_ts.as_str(), Some(main.as_str()), static_)]),
            Failure {
                at,
                error: FailureError::Load(LoadFailure::Strip {
                    failure: StripFailure::Unsupported(tsv_ts::UnsupportedKind::Enum),
                    at: Some(Position { line: 2, column: 3 }),
                }),
            }
        );
        assert_eq!(
            *only(&failures[&site(a.as_str(), None, dynamic)]),
            Failure {
                at: None,
                error: FailureError::Load(LoadFailure::IdentityMismatch),
            }
        );
        assert_eq!(
            *only(&failures[&site(b.as_str(), Some(main.as_str()), static_)]),
            Failure {
                at,
                error: FailureError::Load(LoadFailure::ImportAttributes),
            }
        );
        assert_eq!(
            only(&failures[&site(never.as_str(), None, static_)]).error,
            FailureError::Load(LoadFailure::NeverResolved)
        );
        // no record was loaded
        assert!(
            graph(&loader)
                .records
                .values()
                .all(|record| record.loaded.is_none())
        );
    }

    /// A read that fails is recorded by its I/O error's kind.
    #[test]
    fn a_failed_read_is_recorded_by_its_kind() {
        let fixtures = DiskFixtures::new("read_failure", &[("main.ts", ""), ("a.js", SOURCE)]);
        let main = fixtures.url("main.ts");
        let loader = fixtures.loader("main.ts");
        let a = loader
            .resolve_reached(&DiskTree, "./a.js", main.as_str(), STATIC)
            .unwrap();
        std::fs::remove_file(fixtures.path("a.js")).unwrap();
        load_disk(&loader, &a).unwrap_err();
        assert_eq!(
            only(&graph(&loader).failures[&site(a.as_str(), None, Reached::Import(Reach::Static))])
                .error,
            FailureError::Load(LoadFailure::Read(io::ErrorKind::NotFound))
        );
    }

    /// A site keeps each distinct failure once, in order: a load with
    /// import attributes refused, then the same URL loaded plainly after its
    /// file was replaced, is two reasons at one site, and neither repeats.
    #[test]
    fn a_site_keeps_each_distinct_failure() {
        let fixtures = DiskFixtures::new("site_failures", &[("main.ts", ""), ("a.js", SOURCE)]);
        let main = fixtures.url("main.ts");
        let loader = fixtures.loader("main.ts");
        let a = loader
            .resolve_reached(
                &DiskTree,
                "./a.js",
                main.as_str(),
                Reached::Import(Reach::Dynamic),
            )
            .unwrap();
        let json = RequestedModuleType::Json;
        let plain = RequestedModuleType::None;
        loader
            .load_reached(&a, &json, Reach::Dynamic, None)
            .unwrap_err();
        fixtures.replace("a.js", SOURCE);
        for requested in [&plain, &json, &plain] {
            loader
                .load_reached(&a, requested, Reach::Dynamic, None)
                .unwrap_err();
        }
        let failures = graph(&loader).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        let failure = |reason| Failure {
            at: None,
            error: FailureError::Load(reason),
        };
        assert_eq!(
            failures[&site(a.as_str(), None, Reached::Import(Reach::Dynamic))],
            [
                failure(LoadFailure::ImportAttributes),
                failure(LoadFailure::IdentityMismatch),
            ]
        );
    }
}
