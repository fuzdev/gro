//! Tests for the graph: root conflicts and the scan's index, re-resolving,
//! the consulted files, edges and failures, and what recording off keeps.

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
    assert_eq!(graph.records[&url].module().unwrap().file_id.ino, 1);
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
    assert_eq!(graph.records[&x].module().unwrap().loaded, Some(first));
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
        failure(FailureReason::Load(LoadFailure::NoLocalPath)),
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
                vec![failure(FailureReason::Load(LoadFailure::NoLocalPath))]
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
    assert!(
        graph
            .records
            .values()
            .all(|record| record.module().unwrap().loaded.is_none())
    );
    assert!(graph.edges.is_empty());
    assert!(graph.consulted.is_empty());
    assert!(graph.failures.is_empty());
}
