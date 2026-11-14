//! The `deno_core` module loader: a thin adapter over [`crate::resolver`] that
//! reads the resolved modules from disk, stripping TypeScript's types in
//! process ([`strip`]). The rules it applies are `src/docs/resolver.md`'s (a
//! file loaded is the file resolved, origins, root conflicts) and
//! `src/docs/typescript.md`'s; this covers how the loader holds them.
//!
//! Static imports, `import()`, and `import.meta.resolve` all reach
//! `resolver::resolve_from`, from the importer's recorded root. The entry
//! module is resolved before the runtime starts
//! ([`resolver::resolve_entry`](crate::resolver::resolve_entry)), so its URL
//! arrives here already final, with the roots its graph starts with.
//!
//! The loader owns the graph's [`Roots`], the [`Root`](crate::resolver::Root)
//! each URL resolved inside, and its [`FileId`](crate::resolver::FileId),
//! and passes the roots into every resolution. A bare specifier's package
//! root is admitted when it resolves, and one admitted around a module
//! already recorded inside an outer one is refused
//! ([`ResolveErrorKind::RootConflict`](crate::resolver::ResolveErrorKind::RootConflict)).
//! `import.meta.resolve` admits and records nothing. A module is read only
//! from a file opened without blocking and found to have its recorded
//! identity, never from the path again; the first resolution's record
//! stands, and a URL already loaded is served from `deno_core`'s module map.
//!
//! Every load failure, a strip's refusal included, reaches JS as a
//! `TypeError`, which `import()` can catch: `deno_core` rethrows each as one,
//! whatever its class.
//!
//! A builtin ([`Resolved::Builtin`](crate::resolver::Resolved::Builtin)) is
//! checked against the run's [`Authority`] at link time, for a static import
//! and `import()` alike: one whose grant the importer isn't granted is
//! refused ([`FailureReason::NotGranted`]), the message naming the grant. A
//! loader made without an authority, as eval's is, imports no builtin, a
//! grant-free one included, checked before the grant; a package's module is
//! granted nothing whatever the authority. A granted
//! import returns the builtin's `builtin:` URL, which `deno_core` serves as
//! the builtin's synthetic module, never asking the loader to load it.
//! `import.meta.resolve` returns that URL to any importer, granted or not:
//! it imports nothing, and the URL imports nothing either, its scheme
//! refused from source.
//!
//! An `import()` in code compiled from a string (an `eval`, a `Function`
//! constructor, a string passed to a timer) is refused, whatever it names
//! and whatever its attributes, ahead of every other check of the import
//! but eval's own rules ([`FailureReason::StringCompiledImport`]): V8 can
//! cache that code by its source and name as its importer the module that
//! compiled the string first, so another module compiling the same string
//! would import on that module's grants. Its record names no importer. A
//! function such code calls that was defined in a module imports as that
//! module.
//!
//! Import attributes are refused: an attribute under any key but `type`
//! before anything resolves, as the language orders it, for a file and a
//! builtin alike ([`LoadFailure::UnsupportedAttribute`]); any attribute of
//! a builtin before its grant is checked
//! ([`LoadFailure::BuiltinAttributes`]); and a file's `type` when it loads,
//! whatever its value, since no module type past JS loads yet
//! ([`LoadFailure::ModuleType`]). A static import's attributes and an
//! `import()`'s alike reach the loader, the latter through gro's patch of
//! `deno_core`.
//!
//! Its [`Rules`] refuse more than the resolver does, for eval: every
//! `import()`, before anything is resolved, and every import from the
//! driver, a module whose source the host provides and that resolves to
//! itself, as the root of its own load, and to nothing else. Both are
//! recorded as failures of their own ([`FailureReason::DriverImport`],
//! [`FailureReason::DynamicImport`]). The default rules refuse nothing more.
//!
//! With [`Rules::box_roots`] set, as eval sets them, every read the loader
//! makes, a module's or a `package.json`'s, is checked against the box first
//! (`boxed.rs`), one served from an earlier read's bytes
//! ([`RecordingTree`](reads::RecordingTree)) included. A refused read is
//! recorded once ([`Loader::breach`]) for the caller to fail the run on,
//! whatever the run did with the error, and, recording in full, as the
//! failure of its resolution or load
//! ([`ResolveErrorKind::ReadOutsideBox`](crate::resolver::ResolveErrorKind::ReadOutsideBox),
//! [`LoadFailure::OutsideBox`]).
//!
//! The loader keeps the run's [`Graph`], recording as [`Recording`] says (the
//! graph's module doc lists the record, each loaded module's
//! [`ModuleFormat`](crate::resolver::ModuleFormat) among it); recording in
//! full, resolutions read through a [`RecordingTree`](reads::RecordingTree).
//! Recording changes nothing JS sees but how that tree treats a file read again
//! mid-run: one rewritten in place is served its first read's bytes, and one
//! replaced is refused. Every error keeps its message and class. `deno_core`
//! resolves every import of a graph `import()` loads as dynamic ([`Reach`]),
//! not only the call's own.

mod boxed;
pub mod graph;
mod load;
pub mod reads;
mod resolve;
pub mod strip;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use deno_core::error::ModuleLoaderError;
use deno_core::{
    ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader, ModuleSource,
    ModuleSpecifier, RequestedModuleType, ResolutionKind, v8,
};
use url::Url;

use crate::Authority;
use crate::confine::BoxRoots;
use crate::resolver::{DiskTree, Resolution, Roots, SourceTree, find_builtin};
use boxed::{BoxCheck, Boxed};
use graph::{Failure, FailureReason, Graph, LoadFailure, Position, Reach, Reached, Recording};
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
    /// [`Rules::refuse_dynamic_import`].
    refuse_dynamic_import: bool,
    /// [`Rules::driver`].
    driver: Option<Url>,
    /// The box of [`Rules::box_roots`], and the first read it refused.
    box_check: BoxCheck,
    /// The run's authority, which each builtin import is checked against;
    /// unset, no builtin is imported.
    authority: Option<Authority>,
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

/// The one import attribute key supported, as the language names it: an
/// attribute under any other is refused before anything resolves.
const SUPPORTED_ATTRIBUTE: &str = "type";

/// The referrer `deno_core` names for the root of a load it starts itself,
/// as it resolves a side module's own URL, the driver's (an internal of
/// `deno_core`'s, not a URL).
const ROOT_REFERRER: &str = ".";

/// A failed resolution or load: the reason as data and the message of the
/// error `deno_core` rethrows to JS, both of which the graph records when it
/// keeps the failure ([`Graph::fail`]).
struct Refused {
    /// Why, boxed, since it can hold a
    /// [`ResolveErrorKind`](crate::resolver::ResolveErrorKind), and a failure
    /// is the rare path.
    reason: Box<FailureReason>,
    /// The message JS sees.
    message: String,
}

impl Refused {
    /// A refusal for `reason`, reaching JS as a `TypeError` with `message`:
    /// `deno_core` rethrows every load failure to JS as a `TypeError`
    /// (`js_class_and_message_to_exception`), so no other class is given.
    fn new(reason: FailureReason, message: String) -> Self {
        Self {
            reason: Box::new(reason),
            message,
        }
    }
}

impl Loader {
    /// A loader for the graph of `entry`, already resolved with `roots`,
    /// recording as `recording` says, importing no builtin.
    pub fn new(entry: &Resolution, roots: Roots, recording: Recording) -> Self {
        Self {
            graph: RefCell::new(Graph::new(entry, roots, recording)),
            refuse_dynamic_import: false,
            driver: None,
            box_check: BoxCheck::default(),
            authority: None,
        }
    }

    /// The loader, granting the builtins `authority` grants.
    #[must_use]
    pub fn with_authority(self, authority: Authority) -> Self {
        Self {
            authority: Some(authority),
            ..self
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
    #[must_use]
    pub fn with_rules(self, rules: Rules) -> Self {
        let Rules {
            refuse_dynamic_import,
            driver,
            box_roots,
        } = rules;
        Self {
            refuse_dynamic_import,
            driver,
            box_check: BoxCheck::new(box_roots),
            ..self
        }
    }

    /// Records `refused` as the failure of `specifier`, reached from
    /// `referrer` as `reached`, at `at` in its importer when `deno_core`
    /// says, and returns the error JS sees.
    fn record_refused(
        &self,
        specifier: &str,
        referrer: Option<&str>,
        reached: Reached,
        at: Option<Position>,
        refused: Refused,
    ) -> ModuleLoaderError {
        let error = ModuleLoaderError::type_error(refused.message.clone());
        let failure = Failure {
            at,
            reason: *refused.reason,
            message: refused.message,
            message_hash: None,
        };
        self.graph
            .borrow_mut()
            .fail(specifier, referrer, reached, failure);
        error
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
        let refused = Refused::new(
            FailureReason::DriverImport,
            format!(
                "cannot {verb} '{specifier}' from the driver: {DRIVER_IMPORT_FREE}, so it can \
                 neither import nor resolve a module"
            ),
        );
        Err(self.record_refused(specifier, Some(referrer), reached, None, refused))
    }

    /// The first path a read was refused at for lying outside
    /// [`Rules::box_roots`], if any.
    pub fn breach(&self) -> Option<&Path> {
        self.box_check.breach()
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
            check: &self.box_check,
            tree: &DiskTree,
        };
        self.resolve_reached(&tree, specifier, referrer, reached)
    }

    /// The graph as it stands, borrowed.
    pub fn graph(&self) -> std::cell::Ref<'_, Graph> {
        self.graph.borrow()
    }

    /// The graph, taken from the loader.
    pub fn into_graph(self) -> Graph {
        self.graph.into_inner()
    }

    /// The graph, taken from the loader when this is its last share, or a
    /// copy of it as it stands when another share is still alive.
    pub fn take_graph(self: Rc<Self>) -> Graph {
        Rc::try_unwrap(self).map_or_else(|loader| loader.graph_clone(), Self::into_graph)
    }

    /// A copy of the whole graph as it stands.
    pub fn graph_clone(&self) -> Graph {
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
        let resolved = resolve_into(
            &mut self.graph.borrow_mut(),
            self.authority.as_ref(),
            tree,
            specifier,
            referrer,
            reached,
        );
        // `deno_core` gives no position to a resolution
        resolved.map_err(|refused| {
            self.record_refused(specifier, Some(referrer), reached, None, refused)
        })
    }

    /// Resolves `specifier`, imported by the module at `referrer` as `kind`
    /// says, with the import attributes `attributes`: the entry's URL as it
    /// is, already resolved; a refusal of this loader's own rules, of an
    /// `import()` whose calling code `from_string_code` says was compiled
    /// from a string (asked only for an `import()`), of an attribute under a
    /// key the language leaves unsupported, or of a builtin module with
    /// attributes, in that order; else the resolver's answer over the disk.
    fn resolve_import(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
        attributes: &HashMap<String, String>,
        from_string_code: impl FnOnce() -> bool,
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
        // static import from its root referrer
        if let Some(driver) = &self.driver
            && reach == Reach::Static
            && referrer == ROOT_REFERRER
            && specifier == driver.as_str()
        {
            return Ok(driver.clone());
        }
        self.refuse_by_rules(specifier, referrer, reach)?;
        // first, so nothing refused in string code names V8's referrer
        if reach == Reach::Dynamic && from_string_code() {
            return Err(self.refuse_string_code(specifier));
        }
        if !attributes.is_empty() {
            self.refuse_unsupported_attribute(specifier, referrer, reach, attributes)?;
            self.refuse_builtin_attributes(specifier, referrer, reach)?;
        }
        self.resolve_on_disk(specifier, referrer, Reached::Import(reach))
    }

    /// Refuses an import from the driver, and with
    /// [`Rules::refuse_dynamic_import`] every `import()`, recording why.
    fn refuse_by_rules(
        &self,
        specifier: &str,
        referrer: &str,
        reach: Reach,
    ) -> Result<(), ModuleLoaderError> {
        let reached = Reached::Import(reach);
        self.refuse_from_driver(specifier, referrer, reached)?;
        if reach == Reach::Dynamic && self.refuse_dynamic_import {
            let refused = Refused::new(
                FailureReason::DynamicImport,
                format!(
                    "cannot import '{specifier}' from {referrer}: import() is refused in eval, \
                     whatever it names. Fix: import the module with a static import"
                ),
            );
            return Err(self.record_refused(specifier, Some(referrer), reached, None, refused));
        }
        Ok(())
    }

    /// Refuses an import with an attribute under any key but `type`, the one
    /// key supported, reached as `reach` from `referrer`, recording why
    /// ([`LoadFailure::UnsupportedAttribute`]). The language
    /// refuses such an attribute before the host resolves anything, for a
    /// file and a builtin module alike; of several, the message names the
    /// least, so it doesn't follow the map's order.
    fn refuse_unsupported_attribute(
        &self,
        specifier: &str,
        referrer: &str,
        reach: Reach,
        attributes: &HashMap<String, String>,
    ) -> Result<(), ModuleLoaderError> {
        let Some(key) = attributes
            .keys()
            .filter(|key| key.as_str() != SUPPORTED_ATTRIBUTE)
            .min()
        else {
            return Ok(());
        };
        let refused = Refused::new(
            FailureReason::Load(LoadFailure::UnsupportedAttribute),
            format!(
                "cannot import '{specifier}' from {referrer}: the import attribute '{key}' is not \
                 supported; only '{SUPPORTED_ATTRIBUTE}' is"
            ),
        );
        Err(self.record_refused(
            specifier,
            Some(referrer),
            Reached::Import(reach),
            None,
            refused,
        ))
    }

    /// Refuses an `import()` of `specifier` in code compiled from a string,
    /// recording why under no importer: the module V8 names may have
    /// compiled that string first without being the caller.
    fn refuse_string_code(&self, specifier: &str) -> ModuleLoaderError {
        let refused = Refused::new(
            FailureReason::StringCompiledImport,
            format!(
                "cannot import '{specifier}' from code compiled from a string (eval, a Function \
                 constructor, or a string passed to a timer): V8 can share such code between \
                 modules that compile the same string, so it has no importer of its own. Fix: \
                 call import() in a module and pass what it returns to the compiled code"
            ),
        );
        self.record_refused(
            specifier,
            None,
            Reached::Import(Reach::Dynamic),
            None,
            refused,
        )
    }

    /// Refuses an import with import attributes of a builtin module,
    /// reached as `reach` from `referrer`, recording why; any other
    /// specifier passes. A builtin takes no attributes, whatever they say,
    /// and is never loaded, so the load's refusal of a file's attributes
    /// never sees it ([`LoadFailure::BuiltinAttributes`]). A builtin is the
    /// table hit of its exact specifier, which the resolver returns ahead of
    /// anything else ([`find_builtin`]).
    fn refuse_builtin_attributes(
        &self,
        specifier: &str,
        referrer: &str,
        reach: Reach,
    ) -> Result<(), ModuleLoaderError> {
        let Some(builtin) = find_builtin(specifier) else {
            return Ok(());
        };
        let refused = Refused::new(
            FailureReason::Load(LoadFailure::BuiltinAttributes(builtin)),
            format!(
                "cannot import '{specifier}' from {referrer}: a builtin module takes no import \
                 attributes"
            ),
        );
        Err(self.record_refused(
            specifier,
            Some(referrer),
            Reached::Import(reach),
            None,
            refused,
        ))
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
        let (record, recording) = {
            let graph = self.graph.borrow();
            let record = graph
                .records()
                .get(specifier)
                .map(|record| record.module().map(|module| module.file_id));
            (record, graph.recording())
        };
        // `deno_core` loads only what `resolve` returned, which recorded it,
        // and serves a builtin as its synthetic module, never loading it
        let result = match record {
            None => Err(Refused::new(
                FailureReason::Load(LoadFailure::NeverResolved),
                format!("cannot load {specifier}: it was never resolved"),
            )),
            Some(None) => Err(Refused::new(
                FailureReason::Load(LoadFailure::NoLocalPath),
                format!("cannot load {specifier}: a builtin has no file to load"),
            )),
            Some(Some(file_id)) => load(specifier, requested, file_id, recording, |path| {
                self.box_check.check_read(path)
            }),
        };
        match result {
            Ok((source, loaded)) => {
                if let Some(loaded) = loaded {
                    self.graph.borrow_mut().load(specifier, loaded);
                }
                Ok(source)
            }
            Err(refused) => {
                let at = referrer.and_then(|referrer| {
                    Some(Position {
                        line: u32::try_from(referrer.line_number).ok()?,
                        column: u32::try_from(referrer.column_number).ok()?,
                    })
                });
                Err(self.record_refused(
                    specifier.as_str(),
                    referrer.map(|referrer| referrer.specifier.as_str()),
                    Reached::Import(reach),
                    at,
                    refused,
                ))
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
        // `deno_core` resolves an `import()`, and any import with attributes,
        // with a scope, so neither is here
        self.resolve_import(specifier, referrer, kind, &HashMap::new(), || false)
    }

    /// Resolves as [`ModuleLoader::resolve`] does, refusing an `import()` in
    /// code compiled from a string, an attribute under any key but `type`,
    /// and an import of a builtin module with import attributes.
    /// `deno_core` passes a static import's attributes here, and an
    /// `import()`'s through gro's patch of it (`vendor/README.md`).
    ///
    /// `deno_core` resolves an `import()`'s own specifier here inside V8's
    /// dynamic-import callback, the function that called `import()` the top
    /// JS frame on the stack, and each import of the modules it loads later,
    /// from the event loop, with no JS frame on the stack.
    fn resolve_with_scope(
        &self,
        scope: &mut v8::PinScope,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
        import_attributes: &HashMap<String, String>,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        self.resolve_import(specifier, referrer, kind, import_attributes, || {
            called_from_string_code(scope)
        })
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

/// Whether the top JS frame on the stack runs code compiled from a string:
/// an `eval`, direct or indirect, a `Function` constructor (async and
/// generator ones included), or a string passed to a timer, which
/// `deno_web` runs through an indirect `eval`.
///
/// Asked in V8's dynamic-import callback, the top frame is the function
/// holding the `import()` call: V8 calls the callback from that function's
/// own frame, a stack trace lists an inlined function as a frame of its own,
/// innermost first, and the frames it skips (V8's builtins and scripts that
/// aren't user JS) never hold an `import()` call. V8 names the referrer of
/// that call from the same function's script, and from the module that
/// first compiled that source when the script is compiled from a string,
/// which is exactly when the frame `is_eval`.
fn called_from_string_code(scope: &v8::PinScope) -> bool {
    // with no JS frame on the stack, as when the imports of a module an
    // `import()` loads resolve, the trace is empty, and reading a frame past
    // its end reads out of bounds and crashes the process
    v8::StackTrace::current_stack_trace(scope, 1)
        .filter(|trace| trace.get_frame_count() > 0)
        .and_then(|trace| trace.get_frame(scope, 0))
        .is_some_and(|frame| frame.is_eval())
}

#[cfg(test)]
mod tests;
