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
//!   and `import.meta.resolve`'s included, each once per role.
//! - **Failures**: each resolution or load that failed, by where it was
//!   reached from, with the reason as data ([`FailureError`]), never parsed
//!   from a message. A site keeps each distinct failure once, in the order
//!   each first happened.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::ops::Bound;
use std::path::{Path, PathBuf};

use url::Url;

use super::strip::StripFailure;
use crate::resolver::{
    ConsultedFile, FileId, ModuleFormat, Resolution, ResolveErrorKind, Root, Roots,
};

/// How much a graph records beyond what loading needs.
#[derive(Debug, Clone, Copy)]
pub enum Recording {
    /// Only the roots and each URL's root and file, which loading reads;
    /// nothing else is recorded or built.
    Off,
    /// The whole record: besides what loading needs, each load's format and
    /// hash, the edges, the consulted files, and the failures.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only tests record in full so far")
    )]
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
#[derive(Debug, Clone)]
pub struct Graph {
    /// How much it records, fixed when it is made.
    recording: Recording,
    /// The project's root, and every package root a bare specifier resolved
    /// into.
    pub roots: Roots,
    /// What each URL resolved to, the entry's included.
    pub records: HashMap<Url, Record>,
    /// The URL each import resolved to, the first resolution's.
    pub edges: BTreeMap<Edge, Url>,
    /// Every metadata file the graph's resolutions read, at its real path,
    /// once per role.
    pub consulted: BTreeSet<ConsultedFile>,
    /// What failed at each site: each distinct failure once, in the order
    /// each first happened. One equal to a failure already there is skipped,
    /// since `deno_core` can resolve a specifier twice, but another reason
    /// at the same site is kept: a caught `import()` with attributes and a
    /// later plain `import()` of the same URL are one site, and the second
    /// can be the failure that ends the run.
    ///
    /// A run records at most one failure reached statically: `deno_core`
    /// stops the entry's load at its first error, and linking reuses the
    /// specifiers it already resolved, so it resolves none again.
    pub failures: BTreeMap<FailureSite, Vec<Failure>>,
    /// The recorded URLs by their real path, which `Path`'s ordering
    /// compares by component, so the paths under a directory are one range.
    /// A path has more than one URL when specifiers differ in their query or
    /// fragment.
    paths: BTreeMap<PathBuf, Vec<Url>>,
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
    pub error: FailureError,
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

/// Why a resolution or load failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureError {
    /// The resolver refused the specifier, or admitting its root conflicted
    /// with a module already recorded ([`ResolveErrorKind::RootConflict`]).
    Resolve(ResolveErrorKind),
    /// The importer is not a URL this graph resolved: not a URL at all, or
    /// never resolved here.
    UnknownImporter,
    /// The resolved module didn't load.
    Load(LoadFailure),
}

/// Why a resolved module didn't load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadFailure {
    /// Its URL was never resolved here, so there is no file to check.
    NeverResolved,
    /// The import has attributes (`with {type: 'json'}`), which aren't
    /// supported yet.
    ImportAttributes,
    /// Its URL names no local file.
    NotAFile,
    /// Its file's name doesn't load as a module, which resolution checked
    /// from the same table.
    Format(ResolveErrorKind),
    /// The file at its path isn't the one resolved: replaced since, or a
    /// directory on its path was.
    IdentityMismatch,
    /// Opening, inspecting, or reading the file failed.
    Read(io::ErrorKind),
    /// Its TypeScript was refused by the strip, at a position in its own
    /// source when tsv reports one.
    Strip {
        /// Why the strip refused it.
        failure: StripFailure,
        /// Where in the module's source.
        at: Option<Position>,
    },
}

impl Record {
    /// The record of `resolution`, not yet loaded.
    fn new(resolution: &Resolution) -> Self {
        Self {
            root: Root {
                dir: resolution.root.clone(),
                origin: resolution.origin.clone(),
            },
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
            failures: BTreeMap::new(),
            paths: BTreeMap::new(),
        };
        graph.consult(entry.files_consulted.iter().cloned());
        graph.record(&entry.url, Record::new(entry));
        graph
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
        let record = Record::new(resolution);
        let root = &record.root;
        let conflict =
            |path: PathBuf, recorded: &Root, admitted: &Root| ResolveErrorKind::RootConflict {
                path,
                recorded: Box::new(recorded.clone()),
                admitted: Box::new(admitted.clone()),
            };
        if self.roots.get(&root.dir).is_none()
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
        self.roots.admit(record.root.clone());
        self.record(&resolution.url, record);
        Ok(())
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
    /// is recorded there (see [`Graph::failures`]).
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

    /// Records `url` as `record`, by its path too.
    fn record(&mut self, url: &Url, record: Record) {
        if let Ok(path) = url.to_file_path() {
            self.paths.entry(path).or_default().push(url.clone());
        }
        self.records.insert(url.clone(), record);
    }

    /// Every recorded real path under the directory `dir`, its own included,
    /// in path order, with the root of each of its records.
    fn recorded_under<'a>(&'a self, dir: &'a Path) -> impl Iterator<Item = (&'a Path, &'a Root)> {
        self.paths
            .range::<Path, _>((Bound::Included(dir), Bound::Unbounded))
            .take_while(move |(path, _)| path.starts_with(dir))
            .flat_map(|(path, urls)| {
                urls.iter()
                    .filter_map(|url| self.records.get(url))
                    .map(move |record| (path.as_path(), &record.root))
            })
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
            root: root.dir.clone(),
            origin: root.origin.clone(),
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
        let failure = |error| Failure { at: None, error };
        let dynamic = Reached::Import(Reach::Dynamic);
        let main = main.as_str();
        for error in [
            FailureError::UnknownImporter,
            FailureError::Load(LoadFailure::NeverResolved),
            // equal to one already there
            FailureError::UnknownImporter,
        ] {
            graph.fail("./y.ts", Some(main), dynamic, failure(error));
        }
        // another reach is another site
        graph.fail(
            "./y.ts",
            Some(main),
            Reached::MetaResolve,
            failure(FailureError::Load(LoadFailure::NotAFile)),
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
                        failure(FailureError::UnknownImporter),
                        failure(FailureError::Load(LoadFailure::NeverResolved)),
                    ]
                ),
                (
                    site(Reached::MetaResolve),
                    vec![failure(FailureError::Load(LoadFailure::NotAFile))]
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
                error: FailureError::UnknownImporter,
            },
        );
        assert_eq!(graph.records.len(), 2);
        assert!(graph.records.values().all(|record| record.loaded.is_none()));
        assert!(graph.edges.is_empty());
        assert!(graph.consulted.is_empty());
        assert!(graph.failures.is_empty());
    }
}
