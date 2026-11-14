//! Tests for the graph's record: edges, consulted files, failures, and
//! `Recording::Off`.

use std::collections::BTreeSet;

use super::super::graph::{Edge, FailureSite, Loaded, StripFailure};
use super::*;
use crate::resolver::{ConsultedFile, FileRole, ResolveErrorKind};

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
        *graph(&loader).edges(),
        BTreeMap::from([
            (edge(MAIN, "bar", Reach::Static), bar.clone()),
            (edge(MAIN, "bar", Reach::Dynamic), bar),
        ])
    );
    assert!(graph(&loader).failures().is_empty());
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
        *graph(&loader).consulted(),
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
        *graph(&loader).consulted(),
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
        *graph(&loader).consulted(),
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
        graph.reads().hash(Path::new("/p/package.json")),
        Some(blake3::hash(br#"{"name": "proj", "type": "module"}"#))
    );
    assert_eq!(
        graph
            .reads()
            .hash(Path::new("/p/node_modules/bar/package.json")),
        Some(blake3::hash(br#"{"exports": "./i.mjs"}"#))
    );
    // every consulted file has its read's hash
    assert!(
        graph
            .consulted()
            .iter()
            .all(|file| graph.reads().hash(&file.path).is_some())
    );
    assert_eq!(
        graph.reads().absent(),
        &BTreeSet::from([
            PathBuf::from("/p/src/node_modules/bar"),
            PathBuf::from("/p/src/package.json"),
        ])
    );
    let (off, off_reads) = resolve_bar_twice(Recording::Off);
    assert_eq!(off_reads, 4);
    assert_eq!(off.reads().hash(Path::new("/p/package.json")), None);
    assert!(off.reads().absent().is_empty());
    // and a record passed in is dropped off
    let mut reads = Reads::default();
    RecordingTree::new(&tree(), &mut reads).note_absent(Path::new("/p/x"));
    let (entry, roots) =
        resolver::resolve_entry(&tree(), Path::new("/p/main.ts"), Path::new("/")).unwrap();
    let loader = Loader::new(&entry, roots.clone(), Recording::Off).with_reads(reads.clone());
    assert!(loader.into_graph().reads().absent().is_empty());
    let loader = Loader::new(&entry, roots, Recording::Full).with_reads(reads);
    assert_eq!(loader.into_graph().reads().absent().len(), 1);
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
    loader
        .resolve_reached(&DiskTree, "./sub/b.js?again", main.as_str(), STATIC)
        .unwrap_err();
    assert_failed(
        &graph(&loader),
        "./sub/b.js?again",
        &FailureReason::Resolve(ResolveErrorKind::ReadReplacedSinceFirstRead(
            fixtures.path("sub/package.json"),
        )),
    );
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
            .reads()
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
    assert!(graph(&loader).failures().is_empty());
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
    let failures = graph(&loader).failures().clone();
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
                failure.reason,
                FailureReason::Resolve(ResolveErrorKind::NotFound { .. })
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
    let failures = graph(&loader).failures().clone();
    assert_eq!(failures.len(), 3, "{failures:?}");
    assert!(matches!(
        only(&failures[&site("lnk/x.ts", Some(MAIN), Reached::Import(Reach::Static))]).reason,
        FailureReason::Resolve(ResolveErrorKind::RootConflict { .. })
    ));
    assert_eq!(
        only(
            &failures[&site(
                "./x.ts",
                Some("file:///p/other.ts"),
                Reached::Import(Reach::Static)
            )]
        )
        .reason,
        FailureReason::UnknownImporter
    );
    assert_eq!(
        only(&failures[&site("./x.ts", Some("not a url"), Reached::MetaResolve)]).reason,
        FailureReason::UnknownImporter
    );
    // the conflict admitted nothing, so no edge for it
    assert_eq!(
        graph(&loader).edges().keys().cloned().collect::<Vec<_>>(),
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
        graph.records()[&a].module().unwrap().loaded,
        Some(Loaded {
            format: ModuleFormat::TypeScript,
            hash: blake3::hash(TYPED.as_bytes()),
        })
    );
    assert_eq!(
        graph.records()[&b].module().unwrap().loaded,
        Some(Loaded {
            format: ModuleFormat::JavaScript,
            hash: blake3::hash(SOURCE.as_bytes()),
        })
    );
    // not loaded yet
    assert_eq!(graph.records()[&main].module().unwrap().loaded, None);
    assert!(graph.failures().is_empty());
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
    // off, the message is all a refused load leaves, and the docs quote it
    let error = load_disk(&loader, &b).unwrap_err();
    assert!(
        error.contains(&format!(
            "cannot load {b}: the file at its path is not the one resolved, so it isn't loaded"
        )),
        "{error}"
    );
    let graph = graph(&loader);
    assert!(!graph.is_full());
    let mut recorded: Vec<_> = graph.records().keys().cloned().collect();
    recorded.sort();
    let mut expected = vec![main, a, b];
    expected.sort();
    assert_eq!(recorded, expected);
    assert!(
        graph
            .records()
            .values()
            .all(|record| record.module().unwrap().loaded.is_none())
    );
    assert!(graph.edges().is_empty());
    assert!(graph.consulted().is_empty());
    assert!(graph.failures().is_empty());
}

/// Resolving a loaded URL again, after its file was replaced, keeps the first
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
    let before = graph(&loader).records()[&a].clone();
    fixtures.replace("a.ts", "export const a = 2;\n");
    loader
        .resolve_reached(&DiskTree, "./a.ts", main.as_str(), STATIC)
        .unwrap();
    let after = graph(&loader).records()[&a].clone();
    assert_eq!(after, before);
    assert_eq!(
        after.module().unwrap().loaded.map(|loaded| loaded.hash),
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
    let failures = graph(&loader).failures().clone();
    assert_eq!(failures.len(), 4, "{failures:?}");
    let at = Some(Position {
        line: 3,
        column: 17,
    });
    assert_eq!(
        *only(&failures[&site(enum_ts.as_str(), Some(main.as_str()), static_)]),
        Failure {
            at,
            reason: FailureReason::Load(LoadFailure::Strip {
                failure: StripFailure::Unsupported(tsv_ts::UnsupportedConstruct::Enum),
                at: Some(Position { line: 2, column: 3 }),
            }),
        }
    );
    assert_eq!(
        *only(&failures[&site(a.as_str(), None, dynamic)]),
        Failure {
            at: None,
            reason: FailureReason::Load(LoadFailure::ReplacedSinceLookup),
        }
    );
    assert_eq!(
        *only(&failures[&site(b.as_str(), Some(main.as_str()), static_)]),
        Failure {
            at,
            reason: FailureReason::Load(LoadFailure::ModuleType),
        }
    );
    assert_eq!(
        only(&failures[&site(never.as_str(), None, static_)]).reason,
        FailureReason::Load(LoadFailure::NeverResolved)
    );
    // no record was loaded
    assert!(
        graph(&loader)
            .records()
            .values()
            .all(|record| record.module().unwrap().loaded.is_none())
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
        only(&graph(&loader).failures()[&site(a.as_str(), None, Reached::Import(Reach::Static))])
            .reason,
        FailureReason::Load(LoadFailure::Read(io::ErrorKind::NotFound))
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
    let failures = graph(&loader).failures().clone();
    assert_eq!(failures.len(), 1, "{failures:?}");
    let failure = |reason| Failure {
        at: None,
        reason: FailureReason::Load(reason),
    };
    assert_eq!(
        failures[&site(a.as_str(), None, Reached::Import(Reach::Dynamic))],
        [
            failure(LoadFailure::ModuleType),
            failure(LoadFailure::ReplacedSinceLookup),
        ]
    );
}
