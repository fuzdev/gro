//! What a module graph has resolved and loaded: its admitted roots, the
//! root and file of each URL, the imports between them, the metadata files
//! read on the way, and the failures. Free of `deno_core` types.
//!
//! A run's graph is its record, kept for the host to read after the run,
//! whether it succeeded or failed. How much it records is all or nothing
//! ([`Recording`]): off, it keeps only what loading needs, the roots and
//! each URL's root and file, and every other insert returns before it
//! allocates or clones anything; full, it keeps the whole record:
//!
//! - **Records**: each URL resolved, the entry's included, with the root it
//!   is inside and its file's identity, and, recording in full, once loaded,
//!   its [`ModuleFormat`] and the BLAKE3 hash of the bytes read for it. The
//!   first record stands, so its identity, format, and hash describe one
//!   file, the one the load checked and V8 ran.
//! - **Edges**: each import that resolved and was admitted, by its importer,
//!   its specifier as written, and how it was reached ([`Reach`]), to the
//!   URL it resolved to. `import.meta.resolve` adds none: it imports nothing.
//! - **Consulted files**: every metadata file a resolution read, the entry's
//!   and `import.meta.resolve`'s included, each once per role, and, in
//!   [`Graph::reads`], each read once per run with the hash of its bytes.
//! - **Absent paths**: each path a resolution went on past finding nothing
//!   of the kind it looked for ([`Reads::absent`]), since a resolution that
//!   succeeds depends on them staying absent.
//! - **Failures**: each resolution or load that failed, by where it was
//!   reached from, with the reason as data ([`FailureReason`]), never parsed
//!   from a message. A site keeps each distinct failure once, in the order
//!   each first happened.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::ops::Bound;
use std::path::{Path, PathBuf};

use tsv_ts::UnsupportedConstruct;
use url::Url;

use super::reads::{Reads, RecordingTree};
use crate::resolver::{
    self, ConsultedFile, FileId, ModuleFormat, Resolution, ResolveError, ResolveErrorKind, Root,
    Roots, SourceTree,
};

/// How much a graph records beyond what loading needs.
#[derive(Debug, Clone, Copy)]
pub enum Recording {
    /// Only the roots and each URL's root and file, which loading reads;
    /// nothing else is recorded or built.
    Off,
    /// The whole record: besides what loading needs, each load's format and
    /// hash, the edges, the consulted files with their reads' hashes, the
    /// paths found absent, and the failures.
    Full,
}

impl Recording {
    /// Whether this is the whole record.
    pub const fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// What a graph has resolved and loaded: the roots it admitted, the record of
/// each URL, and, recording in full, the imports between them, the files
/// consulted, and the failures.
///
/// Its fields are read through accessors and written only through its
/// methods, which keep the paths index in step with the records.
#[derive(Debug, Clone)]
pub struct Graph {
    /// How much it records, fixed when it is made.
    recording: Recording,
    /// See [`Graph::roots`].
    roots: Roots,
    /// See [`Graph::records`].
    records: HashMap<Url, Record>,
    /// See [`Graph::edges`].
    edges: BTreeMap<Edge, Url>,
    /// See [`Graph::consulted`].
    consulted: BTreeSet<ConsultedFile>,
    /// See [`Graph::reads`].
    reads: Reads,
    /// See [`Graph::failures`].
    failures: BTreeMap<FailureSite, Vec<Failure>>,
    /// The recorded URLs by their real path, which `Path`'s ordering
    /// compares by component, so the paths under a directory are one range.
    /// A path has more than one URL when specifiers differ in their query or
    /// fragment. Built at the first admission that can conflict, so a graph
    /// whose new roots are all outside every admitted one never builds it.
    paths: Option<BTreeMap<PathBuf, Vec<Url>>>,
}

/// What a URL resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The root it resolved inside.
    pub root: Root,
    /// Its file's identity, the one loading it must find.
    pub file_id: FileId,
    /// What its load read, once it loaded, recording in full; unset
    /// otherwise.
    pub loaded: Option<Loaded>,
}

/// What a module's load read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loaded {
    /// How its source became the JS V8 evaluated.
    pub format: ModuleFormat,
    /// The BLAKE3 hash of the bytes read from its file, before any strip.
    pub hash: blake3::Hash,
}

/// An import: the module that imported, the specifier, and how it was
/// reached.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    /// The importing module's URL.
    pub importer: Url,
    /// The specifier as written.
    pub specifier: String,
    /// How it was reached.
    pub reach: Reach,
}

/// How an import was reached, as `deno_core` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    /// A static import of a module loaded with the entry's graph.
    Static,
    /// Resolved for a graph an `import()` loads: the `import()` call's own
    /// specifier, and every static import of the modules that load brings
    /// in, since `deno_core` resolves them all as dynamic.
    Dynamic,
}

/// How a failed specifier was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reached {
    /// By an import, static or dynamic.
    Import(Reach),
    /// By `import.meta.resolve`, which resolves without loading.
    MetaResolve,
}

/// Where a failure happened: what failed, from where, and how it was
/// reached.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FailureSite {
    /// The specifier as written for a failed resolution, the resolved URL for
    /// a failed load.
    pub specifier: String,
    /// The importing module's URL as `deno_core` named it; unset for a load
    /// `deno_core` names no importer for: the entry's, and the module an
    /// `import()` call loads.
    pub importer: Option<String>,
    /// How it was reached.
    pub reached: Reached,
}

/// A failed resolution or load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Where the import is in its importer, when `deno_core` says: it gives a
    /// position only for a static import's load. `deno_core` finds it by
    /// applying V8's UTF-16 source offset to the importer's bytes, so after
    /// non-ASCII text in the importer the line and column can both be wrong.
    pub at: Option<Position>,
    /// Why it failed.
    pub reason: FailureReason,
}

/// Why a resolution or load failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureReason {
    /// The resolver refused the specifier, or admitting its root conflicted
    /// with a module already recorded ([`ResolveErrorKind::RootConflict`]).
    Resolve(ResolveErrorKind),
    /// The importer is not a URL this graph resolved: not a URL at all, or
    /// never resolved here.
    UnknownImporter,
    /// The resolved module didn't load.
    Load(LoadFailure),
    /// An import or `import.meta.resolve` from the driver, a module whose
    /// source the host provides itself and which imports nothing.
    DriverImport,
    /// An `import()`, in a graph that refuses every one, whatever its
    /// specifier.
    DynamicImport,
}

/// Why a resolved module didn't load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadFailure {
    /// Its URL was never resolved here, so there is no file to check.
    NeverResolved,
    /// The import has attributes (`with {type: 'json'}`), which aren't
    /// supported yet.
    ImportAttributes,
    /// Its URL names no local file. Defense in depth: a recorded URL is a
    /// resolved file's, so it can't happen.
    NoLocalPath,
    /// Its file's name doesn't load as a module. Defense in depth: a
    /// recorded URL passed the same table when it resolved, so it can't
    /// happen.
    Format(ResolveErrorKind),
    /// The file at its path isn't the one resolved: replaced since, or a
    /// directory on its path was.
    ReplacedSinceLookup,
    /// Opening, inspecting, or reading the file failed.
    Read(io::ErrorKind),
    /// The file lies outside the box the reads are confined to
    /// ([`ReadRefusal::OutsideBox`](crate::resolver::ReadRefusal::OutsideBox)),
    /// so it wasn't read.
    OutsideBox,
    /// The file holds more than [`MAX_MODULE_SIZE`](crate::resolver::MAX_MODULE_SIZE)
    /// bytes, so it wasn't read whole.
    TooLarge,
    /// Its TypeScript was refused by the strip, at a position in its own
    /// source when tsv reports one.
    Strip {
        /// Why the strip refused it.
        failure: StripFailure,
        /// Where in the module's source.
        at: Option<Position>,
    },
}

/// A position in a module's source: a 1-based line and column.
///
/// A strip's column counts UTF-16 units, as V8's frames do; an importer's is
/// `deno_core`'s (see [`Failure::at`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The line, 1-based.
    pub line: u32,
    /// The column, 1-based.
    pub column: u32,
}

/// Why a strip was refused, as the record of a failed load says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripFailure {
    /// The source doesn't parse as a TypeScript module.
    Parse,
    /// The source nests or chains past tsv's depth limits.
    TooDeep,
    /// TypeScript with runtime semantics, which blanking can't erase.
    Unsupported(UnsupportedConstruct),
    /// A failure a later tsv adds.
    Other,
}

impl Record {
    /// The record of `resolution`, not yet loaded.
    fn new(resolution: &Resolution) -> Self {
        Self {
            root: resolution.root.clone(),
            file_id: resolution.file_id,
            loaded: None,
        }
    }
}

impl Graph {
    /// The graph of `entry`, resolved with `roots`, recording as `recording`
    /// says, with the files its resolution consulted.
    pub fn new(entry: &Resolution, roots: Roots, recording: Recording) -> Self {
        let mut graph = Self {
            recording,
            roots,
            records: HashMap::new(),
            edges: BTreeMap::new(),
            consulted: BTreeSet::new(),
            reads: Reads::default(),
            failures: BTreeMap::new(),
            paths: None,
        };
        graph.consult(entry.files_consulted.iter().cloned());
        graph.record(&entry.url, Record::new(entry));
        graph
    }

    /// The graph, its reads starting from `reads`, what the entry's
    /// resolution read and found absent, recording in full; off, `reads`
    /// is dropped.
    #[must_use]
    pub fn with_reads(self, reads: Reads) -> Self {
        if self.is_full() {
            Self { reads, ..self }
        } else {
            self
        }
    }

    /// Admits the root of `resolution` and records its URL inside it, with
    /// its file, changing nothing on an error.
    ///
    /// A new root around a module already recorded inside an outer one is
    /// a conflict, never first-wins: the module's origin would depend on
    /// which import ran first. A URL resolved again inside the same root is
    /// no change, since `deno_core` can resolve a specifier twice, and its
    /// first record stands: a load checks that file, and a URL already
    /// loaded is served from `deno_core`'s module map, never read again.
    /// Resolved again inside another root, it is a conflict.
    pub fn admit(&mut self, resolution: &Resolution) -> Result<(), ResolveErrorKind> {
        let root = &resolution.root;
        let conflict =
            |path: PathBuf, recorded: &Root, admitted: &Root| ResolveErrorKind::RootConflict {
                path,
                recorded: Box::new(recorded.clone()),
                admitted: Box::new(admitted.clone()),
            };
        let new_root = self.roots.get(&root.dir).is_none();
        // a module recorded inside a root holds no `node_modules` segment
        // below it, so only a new root some admitted root contains can hold one
        if new_root
            && self.roots.containing(&root.dir).is_some()
            && let Some((path, recorded)) =
                self.recorded_under(&root.dir).find(|(path, recorded)| {
                    // one nested in the recorded root would now hold the module
                    root.dir != recorded.dir
                        && root.dir.starts_with(&recorded.dir)
                        && root.contains(path)
                })
        {
            return Err(conflict(path.to_owned(), recorded, root));
        }
        if let Some(recorded) = self.records.get(&resolution.url) {
            let path = || resolution.url.to_file_path().unwrap_or_default();
            if recorded.root.dir != root.dir {
                return Err(conflict(path(), &recorded.root, root));
            }
            // the first record stands
            return Ok(());
        }
        if new_root {
            self.roots.admit(root.clone());
        }
        // built only now, so a URL already recorded clones nothing
        self.record(&resolution.url, Record::new(resolution));
        Ok(())
    }

    /// The project's root, and every package root a bare specifier resolved
    /// into.
    pub const fn roots(&self) -> &Roots {
        &self.roots
    }

    /// What each URL resolved to, the entry's included.
    pub const fn records(&self) -> &HashMap<Url, Record> {
        &self.records
    }

    /// The URL each import resolved to, the first resolution's, recording
    /// in full.
    pub const fn edges(&self) -> &BTreeMap<Edge, Url> {
        &self.edges
    }

    /// Every metadata file the graph's resolutions read, at its real path,
    /// once per role, recording in full.
    pub const fn consulted(&self) -> &BTreeSet<ConsultedFile> {
        &self.consulted
    }

    /// Each metadata file's single read, with its hash, and the paths found
    /// absent, the entry's resolution's among them when the caller passed
    /// its record in ([`Graph::with_reads`]), recording in full.
    pub const fn reads(&self) -> &Reads {
        &self.reads
    }

    /// What failed at each site, recording in full: each distinct failure
    /// once, in the order each first happened. One equal to a failure
    /// already there is skipped, since `deno_core` can resolve a specifier
    /// twice, but another reason at the same site is kept: a caught
    /// `import()` with attributes and a later plain `import()` of the same
    /// URL are one site, and the second can be the failure that ends the
    /// run.
    ///
    /// A run records at most one failure reached statically: `deno_core`
    /// stops the entry's load at its first error, and linking reuses the
    /// specifiers it already resolved, so it resolves none again.
    pub const fn failures(&self) -> &BTreeMap<FailureSite, Vec<Failure>> {
        &self.failures
    }

    /// Resolves `specifier`, imported by the recorded module at `importer`,
    /// from the root it was recorded inside, reading `tree`, through the
    /// graph's reads when recording in full. Admits and records nothing.
    /// `None` when `importer` was never recorded.
    pub(crate) fn resolve(
        &mut self,
        tree: &impl SourceTree,
        specifier: &str,
        importer: &Url,
    ) -> Option<Result<Resolution, ResolveError>> {
        // the record's root is the one its path is inside: admitting a root
        // around a recorded module is a conflict, so none moves it
        let importer_root = &self.records.get(importer)?.root;
        Some(if self.recording.is_full() {
            let tree = RecordingTree::new(tree, &mut self.reads);
            resolver::resolve_from(&tree, specifier, importer, importer_root, &self.roots)
        } else {
            resolver::resolve_from(tree, specifier, importer, importer_root, &self.roots)
        })
    }

    /// How much the graph records.
    pub const fn recording(&self) -> Recording {
        self.recording
    }

    /// Whether the graph records in full.
    pub const fn is_full(&self) -> bool {
        self.recording.is_full()
    }

    /// Adds `files` to the files consulted, each kept once, recording in
    /// full; otherwise `files` is never iterated.
    pub fn consult(&mut self, files: impl IntoIterator<Item = ConsultedFile>) {
        if self.is_full() {
            self.consulted.extend(files);
        }
    }

    /// Records that the import of `specifier` by `importer`, reached as
    /// `reach`, resolved to `url`, recording in full, unless it resolved
    /// before: the first resolution stands.
    pub fn link(&mut self, importer: &Url, specifier: &str, reach: Reach, url: &Url) {
        if !self.is_full() {
            return;
        }
        let edge = Edge {
            importer: importer.clone(),
            specifier: specifier.to_owned(),
            reach,
        };
        self.edges.entry(edge).or_insert_with(|| url.clone());
    }

    /// Records what loading `url` read, recording in full, unless it loaded
    /// before: the first load stands, as `deno_core` evaluates only the
    /// first.
    pub fn load(&mut self, url: &Url, loaded: Loaded) {
        if !self.is_full() {
            return;
        }
        if let Some(record) = self.records.get_mut(url) {
            record.loaded.get_or_insert(loaded);
        }
    }

    /// Records `failure` at the site of `specifier`, imported by `importer`
    /// and reached as `reached`, recording in full, unless an equal failure
    /// is recorded there (see [`Graph::failures`]); off, it does nothing.
    pub fn fail(
        &mut self,
        specifier: &str,
        importer: Option<&str>,
        reached: Reached,
        failure: Failure,
    ) {
        if !self.is_full() {
            return;
        }
        let site = FailureSite {
            specifier: specifier.to_owned(),
            importer: importer.map(str::to_owned),
            reached,
        };
        let failures = self.failures.entry(site).or_default();
        if !failures.contains(&failure) {
            failures.push(failure);
        }
    }

    /// Records `url` as `record`, by its path too once the paths are indexed.
    fn record(&mut self, url: &Url, record: Record) {
        if let Some(paths) = &mut self.paths {
            index_path(paths, url);
        }
        self.records.insert(url.clone(), record);
    }

    /// Every recorded real path under the directory `dir`, its own included,
    /// in path order, with the root of each of its records, indexing the
    /// paths first if they aren't yet.
    fn recorded_under<'a>(
        &'a mut self,
        dir: &'a Path,
    ) -> impl Iterator<Item = (&'a Path, &'a Root)> {
        let records = &self.records;
        let paths = self.paths.get_or_insert_with(|| {
            let mut paths = BTreeMap::new();
            for url in records.keys() {
                index_path(&mut paths, url);
            }
            paths
        });
        paths
            .range::<Path, _>((Bound::Included(dir), Bound::Unbounded))
            .take_while(move |(path, _)| path.starts_with(dir))
            .flat_map(|(path, urls)| {
                urls.iter()
                    .filter_map(|url| records.get(url))
                    .map(move |record| (path.as_path(), &record.root))
            })
    }
}

/// Adds `url` to `paths` by its real path, if it names one.
fn index_path(paths: &mut BTreeMap<PathBuf, Vec<Url>>, url: &Url) {
    if let Ok(path) = url.to_file_path() {
        paths.entry(path).or_default().push(url.clone());
    }
}

#[cfg(test)]
mod tests;
