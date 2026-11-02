//! The `deno_core` module loader: a thin adapter over [`crate::resolver`] that
//! reads the resolved modules from disk, stripping TypeScript's types in
//! process.
//!
//! Static imports, `import()`, and `import.meta.resolve` all reach
//! `resolver::resolve_from`, from the importer's recorded root. The entry module is resolved before the runtime
//! starts ([`resolver::resolve_entry`](crate::resolver::resolve_entry)), so its URL arrives here already
//! final, with the roots its graph starts with.
//!
//! The loader owns the graph's [`Roots`] and the [`Root`](crate::resolver::Root) each
//! URL resolved inside, and passes the roots into every resolution. A bare
//! specifier's package root is admitted when it resolves. Admitting a root
//! around a module already recorded inside an outer one is an error naming
//! both ([`ResolveErrorKind::RootConflict`](crate::resolver::ResolveErrorKind::RootConflict)),
//! since otherwise the import order would decide the module's origin;
//! resolving a URL again inside the same root changes nothing.
//! `import.meta.resolve` runs the same resolution and admits and records
//! nothing, so importing the URL it returns resolves it again, as an absolute
//! specifier: an installed package's file is then a path into `node_modules`,
//! refused.
//!
//! The loader also records each URL's [`FileId`](crate::resolver::FileId), the device and inode
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
//! print. Like every load failure, it reaches JS as a `TypeError`, which
//! `import()` can catch: `deno_core` rethrows each as one, whatever its class.
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
//! read's bytes ([`RecordingTree`](reads::RecordingTree)) included, is checked first: its real path
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
//! record, which a load completes with the module's [`ModuleFormat`](crate::resolver::ModuleFormat) and the
//! BLAKE3 hash of the bytes read from the checked file, before any strip:
//! the bytes V8's source was made from, run as read for JS and stripped for
//! TypeScript; each import that resolved as an edge, by importer, specifier,
//! and [`Reach`] (`deno_core` resolves every import of a graph `import()`
//! loads as dynamic, not only the call's own); every metadata file a
//! resolution consulted, the entry's and `import.meta.resolve`'s included;
//! and each failed resolution or load by its site, with the reason as data
//! built where the message is, never parsed from it. Recording in full,
//! resolutions also read through a [`RecordingTree`](reads::RecordingTree):
//! each metadata file is read once per run, its later reads served the
//! first's bytes, a file of another identity at its path refused, and its
//! hash kept, and every path a resolution went on past finding absent is
//! recorded. Recording changes nothing JS sees but that refusal: every
//! error keeps its message and class.
//! `import.meta.resolve` adds no edge, since it loads nothing, but its
//! consulted files and failures are recorded.

mod boxed;
pub mod graph;
mod load;
pub mod reads;
mod resolve;
pub mod strip;

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};

use deno_core::error::ModuleLoaderError;
use deno_core::{
    ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader, ModuleSource,
    ModuleSpecifier, RequestedModuleType, ResolutionKind,
};
use url::Url;

use crate::confine::BoxRoots;
use crate::resolver::{DiskTree, Resolution, Roots, SourceTree};
use boxed::Boxed;
use graph::{Failure, FailureError, Graph, LoadFailure, Position, Reach, Reached, Recording};
use load::load;
use reads::Reads;
use resolve::resolve_into;

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
/// [`ResolveErrorKind`](crate::resolver::ResolveErrorKind), and a failure is the
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

#[cfg(test)]
mod tests;
