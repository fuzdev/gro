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

use tsv_ts::UnsupportedKind;
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
    NotAFile,
    /// Its file's name doesn't load as a module. Defense in depth: a
    /// recorded URL passed the same table when it resolved, so it can't
    /// happen.
    Format(ResolveErrorKind),
    /// The file at its path isn't the one resolved: replaced since, or a
    /// directory on its path was.
    IdentityMismatch,
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
    Unsupported(UnsupportedKind),
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
        // a module recorded inside a root holds no `node_modules` segment
        // below it, so only a new root some admitted root contains can hold one
        if self.roots.get(&root.dir).is_none()
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
        if self.roots.get(&root.dir).is_none() {
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
mod tests {
    use super::*;
    use crate::resolver::{FileRole, Origin};

    fn package(dir: &str, name: &str) -> Root {
        Root {
            dir: PathBuf::from(dir),
            origin: Origin::Package {
                name: name.to_owned(),
                version: None,
                linked: true,
            },
        }
    }

    /// The resolution of the file at `path` inside `root`.
    fn resolution(path: &str, root: &Root) -> Resolution {
        Resolution {
            url: Url::from_file_path(path).unwrap(),
            file_id: FileId {
                dev: 1,
                ino: 1,
                born: None,
            },
            root: root.clone(),
            files_consulted: Vec::new(),
        }
    }

    /// A graph of the project at `/a` whose entry is `/a/main.ts`, with
    /// `paths` recorded inside the project.
    fn project_graph(paths: &[&str]) -> Graph {
        let project = Root {
            dir: PathBuf::from("/a"),
            origin: Origin::Project,
        };
        let mut graph = Graph::new(
            &resolution("/a/main.ts", &project),
            Roots::new(project.dir.clone()),
            Recording::Full,
        );
        for path in paths {
            graph.admit(&resolution(path, &project)).unwrap();
        }
        graph
    }

    fn conflict_path(result: Result<(), ResolveErrorKind>) -> Option<PathBuf> {
        match result {
            Err(ResolveErrorKind::RootConflict { path, .. }) => Some(path),
            Ok(()) => None,
            Err(error) => panic!("not a conflict: {error}"),
        }
    }

    /// A new root conflicts with a module recorded inside it by component:
    /// one under a sibling whose name extends the root's (`/a/b-c` beside
    /// `/a/b`) or under `node_modules` below it is no conflict.
    #[test]
    fn a_new_root_conflicts_only_with_what_it_would_hold() {
        let b = package("/a/b", "b");
        let mut graph = project_graph(&["/a/b-c/x.ts", "/a/b.ts", "/a/b/node_modules/n/x.mjs"]);
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            None
        );
        let mut graph = project_graph(&["/a/b-c/x.ts", "/a/b/c/x.ts"]);
        let before = (graph.roots.clone(), graph.records.clone());
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/c/x.ts"))
        );
        // nothing changed on the error
        assert_eq!((graph.roots.clone(), graph.records.clone()), before);
        // the root's own directory, were it a module, is inside it
        let mut graph = project_graph(&["/a/b"]);
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b"))
        );
    }

    /// The scan reads past a record the new root doesn't conflict with,
    /// one in a root nested in it, and the entry is recorded by its path.
    #[test]
    fn a_conflict_is_found_past_what_is_none_and_at_the_entry() {
        let b = package("/a/b", "b");
        let mut graph = project_graph(&["/a/b/z.ts"]);
        graph
            .admit(&resolution("/a/b/c/x.ts", &package("/a/b/c", "c")))
            .unwrap();
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/z.ts"))
        );
        let project = Root {
            dir: PathBuf::from("/a"),
            origin: Origin::Project,
        };
        let mut graph = Graph::new(
            &resolution("/a/b/main.ts", &project),
            Roots::new(project.dir.clone()),
            Recording::Off,
        );
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/main.ts"))
        );
    }

    /// A root admitted already isn't checked again, and a module resolved
    /// again inside the same root keeps its first record; inside another
    /// root it is a conflict.
    #[test]
    fn a_re_resolve_keeps_the_first_record() {
        let b = package("/a/b", "b");
        let mut graph = project_graph(&[]);
        graph.admit(&resolution("/a/b/i.ts", &b)).unwrap();
        let mut again = resolution("/a/b/i.ts", &b);
        again.file_id.ino = 2;
        graph.admit(&again).unwrap();
        let url = Url::from_file_path("/a/b/i.ts").unwrap();
        assert_eq!(graph.records[&url].file_id.ino, 1);
        // the same URL inside an outer root, not admitted around it
        let a = Root {
            dir: PathBuf::from("/a"),
            origin: Origin::Project,
        };
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &a))),
            Some(PathBuf::from("/a/b/i.ts"))
        );
        // a query makes another URL for the same file, recorded on its own
        let mut query = resolution("/a/b/i.ts", &b);
        query.url.set_query(Some("q"));
        graph.admit(&query).unwrap();
        assert_eq!(graph.records.len(), 3);
        // a root nested in `b` around neither record is no conflict
        let x = package("/a/b/x", "x");
        graph.admit(&resolution("/a/b/x/y.ts", &x)).unwrap();
        // one around both conflicts with the first in path order
        let mut graph = project_graph(&["/a/b/c/z.ts", "/a/b/c/y.ts"]);
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/c/y.ts"))
        );
    }

    /// A new root no admitted root contains can't hold a recorded module,
    /// so it skips the scan without building the paths index; one that a
    /// root contains builds it, and records made after are indexed too.
    #[test]
    fn only_a_contained_root_scans_and_the_index_keeps_up() {
        let mut graph = project_graph(&["/a/x.ts", "/a/c/x.ts"]);
        let installed = package("/a/node_modules/n", "n");
        graph
            .admit(&resolution("/a/node_modules/n/i.mjs", &installed))
            .unwrap();
        graph
            .admit(&resolution("/q/l/i.ts", &package("/q/l", "l")))
            .unwrap();
        assert!(graph.paths.is_none());
        // a root the project contains scans, building the index
        graph
            .admit(&resolution("/a/b/i.ts", &package("/a/b", "b")))
            .unwrap();
        assert!(graph.paths.is_some());
        // a module recorded after the index is built is found by a later scan
        let project = graph.roots.project().clone();
        graph.admit(&resolution("/a/d/x.ts", &project)).unwrap();
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/d/i.ts", &package("/a/d", "d")))),
            Some(PathBuf::from("/a/d/x.ts"))
        );
        // and one recorded before it
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/c/i.ts", &package("/a/c", "c")))),
            Some(PathBuf::from("/a/c/x.ts"))
        );
    }

    /// The scan's gate is sound: over every set of admitted roots drawn from
    /// a small tree, a new root that would hold a module validly recorded in
    /// an outer root is always inside some admitted root.
    #[test]
    fn a_root_that_skips_the_scan_conflicts_with_nothing() {
        const DIRS: [&str; 8] = [
            "/a/b",
            "/a/b/c",
            "/a/bc",
            "/a/node_modules/n",
            "/a/node_modules/n/node_modules/m",
            "/a/b/node_modules/n",
            "/a/b/node_modules/n/d",
            "/q",
        ];
        let candidates: Vec<Root> = DIRS.iter().map(|dir| package(dir, "x")).collect();
        let paths: Vec<PathBuf> = std::iter::once("/a")
            .chain(DIRS)
            .flat_map(|dir| [format!("{dir}/x.ts"), format!("{dir}/node_modules/z/x.ts")])
            .map(PathBuf::from)
            .collect();
        for subset in 0..1_u32 << DIRS.len() {
            let mut roots = Roots::new(PathBuf::from("/a"));
            for (index, root) in candidates.iter().enumerate() {
                if subset & (1 << index) != 0 {
                    roots.admit(root.clone());
                }
            }
            for path in &paths {
                // a module is recorded only inside the root that contains it
                let Some(recorded) = roots.containing(path) else {
                    continue;
                };
                for new in candidates
                    .iter()
                    .filter(|new| roots.get(&new.dir).is_none())
                {
                    let conflicts = new.dir != recorded.dir
                        && new.dir.starts_with(&recorded.dir)
                        && new.contains(path);
                    assert!(
                        !conflicts || roots.containing(&new.dir).is_some(),
                        "{} would hold {} from {}, skipping the scan",
                        new.dir.display(),
                        path.display(),
                        recorded.dir.display(),
                    );
                }
            }
        }
    }

    fn consulted(path: &str, role: FileRole) -> ConsultedFile {
        ConsultedFile {
            path: PathBuf::from(path),
            role,
        }
    }

    /// The entry's consulted files are the graph's first, and the union
    /// keeps each file once per role.
    #[test]
    fn the_consulted_files_are_a_union_from_the_entry_on() {
        let project = Root {
            dir: PathBuf::from("/a"),
            origin: Origin::Project,
        };
        let mut entry = resolution("/a/main.js", &project);
        entry.files_consulted = vec![
            consulted("/a/package.json", FileRole::SearchBound),
            consulted("/a/package.json", FileRole::PackageScope),
        ];
        let mut graph = Graph::new(&entry, Roots::new(project.dir), Recording::Full);
        assert_eq!(
            graph.consulted,
            BTreeSet::from([
                consulted("/a/package.json", FileRole::SearchBound),
                consulted("/a/package.json", FileRole::PackageScope),
            ])
        );
        graph.consult([
            consulted("/a/package.json", FileRole::PackageScope),
            consulted("/a/node_modules/b/package.json", FileRole::Exports),
        ]);
        assert_eq!(graph.consulted.len(), 3);
    }

    /// An edge and a load each keep their first record, and a site keeps
    /// each distinct failure once, in order.
    #[test]
    fn the_first_edge_and_load_stand_and_distinct_failures_are_kept() {
        let mut graph = project_graph(&["/a/x.ts"]);
        let main = Url::from_file_path("/a/main.ts").unwrap();
        let x = Url::from_file_path("/a/x.ts").unwrap();
        let edge = |reach| Edge {
            importer: main.clone(),
            specifier: "./x.ts".to_owned(),
            reach,
        };
        graph.link(&main, "./x.ts", Reach::Static, &x);
        graph.link(&main, "./x.ts", Reach::Static, &main);
        graph.link(&main, "./x.ts", Reach::Dynamic, &x);
        assert_eq!(
            graph.edges,
            BTreeMap::from([
                (edge(Reach::Static), x.clone()),
                (edge(Reach::Dynamic), x.clone())
            ])
        );
        let first = Loaded {
            format: ModuleFormat::TypeScript,
            hash: blake3::hash(b"first"),
        };
        graph.load(&x, first);
        graph.load(
            &x,
            Loaded {
                format: ModuleFormat::JavaScript,
                hash: blake3::hash(b"second"),
            },
        );
        assert_eq!(graph.records[&x].loaded, Some(first));
        // a URL never recorded gets no load
        graph.load(&Url::from_file_path("/a/y.ts").unwrap(), first);
        assert_eq!(graph.records.len(), 2);
        let failure = |reason| Failure { at: None, reason };
        let dynamic = Reached::Import(Reach::Dynamic);
        let main = main.as_str();
        for error in [
            FailureReason::UnknownImporter,
            FailureReason::Load(LoadFailure::NeverResolved),
            // equal to one already there
            FailureReason::UnknownImporter,
        ] {
            graph.fail("./y.ts", Some(main), dynamic, failure(error));
        }
        // another reach is another site
        graph.fail(
            "./y.ts",
            Some(main),
            Reached::MetaResolve,
            failure(FailureReason::Load(LoadFailure::NotAFile)),
        );
        let site = |reached| FailureSite {
            specifier: "./y.ts".to_owned(),
            importer: Some(main.to_owned()),
            reached,
        };
        assert_eq!(
            graph.failures,
            BTreeMap::from([
                (
                    site(dynamic),
                    vec![
                        failure(FailureReason::UnknownImporter),
                        failure(FailureReason::Load(LoadFailure::NeverResolved)),
                    ]
                ),
                (
                    site(Reached::MetaResolve),
                    vec![failure(FailureReason::Load(LoadFailure::NotAFile))]
                ),
            ])
        );
    }

    /// Recording off, a graph keeps its roots and records, the entry's
    /// included, and conflicts are still refused, but no edge, load,
    /// consulted file, or failure is recorded.
    #[test]
    fn recording_off_keeps_only_what_loading_needs() {
        let project = Root {
            dir: PathBuf::from("/a"),
            origin: Origin::Project,
        };
        let mut entry = resolution("/a/main.ts", &project);
        entry.files_consulted = vec![consulted("/a/package.json", FileRole::SearchBound)];
        let mut graph = Graph::new(&entry, Roots::new(project.dir.clone()), Recording::Off);
        assert!(!graph.is_full());
        graph.admit(&resolution("/a/b/c.ts", &project)).unwrap();
        let b = package("/a/b", "b");
        assert_eq!(
            conflict_path(graph.admit(&resolution("/a/b/i.ts", &b))),
            Some(PathBuf::from("/a/b/c.ts"))
        );
        let main = entry.url;
        let c = Url::from_file_path("/a/b/c.ts").unwrap();
        graph.consult([consulted("/a/package.json", FileRole::PackageScope)]);
        graph.link(&main, "./b/c.ts", Reach::Static, &c);
        graph.load(
            &c,
            Loaded {
                format: ModuleFormat::TypeScript,
                hash: blake3::hash(b""),
            },
        );
        graph.fail(
            "./x.ts",
            Some(main.as_str()),
            Reached::Import(Reach::Static),
            Failure {
                at: None,
                reason: FailureReason::UnknownImporter,
            },
        );
        assert_eq!(graph.records.len(), 2);
        assert!(graph.records.values().all(|record| record.loaded.is_none()));
        assert!(graph.edges.is_empty());
        assert!(graph.consulted.is_empty());
        assert!(graph.failures.is_empty());
    }
}
