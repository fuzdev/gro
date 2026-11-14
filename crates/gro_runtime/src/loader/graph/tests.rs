//! Tests for the graph: root conflicts and the scan's index, re-resolving,
//! the consulted files, edges and failures, what recording off keeps, and
//! which failures are refusals by rule.

use super::*;
use crate::resolver::{
    BUILTINS, FileRole, Origin, Reserved, SPLIT_MODULES, UNBUILT_HOST_MODULES, UrlFault,
};

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
    let failure = |reason| Failure {
        at: None,
        reason,
        message: "refused".to_owned(),
        message_hash: None,
    };
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
/// consulted file, or static failure is recorded.
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
            message: "refused".to_owned(),
            message_hash: None,
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

/// Recording off, a refusal by rule reached by an `import()` is kept, once,
/// while one reached statically or by `import.meta.resolve`, and an
/// `import()` that found nothing there, are not.
#[test]
fn recording_off_keeps_the_dynamic_refusals_by_rule() {
    let project = Root {
        dir: PathBuf::from("/a"),
        origin: Origin::Project,
    };
    let entry = resolution("/a/main.ts", &project);
    let mut graph = Graph::new(&entry, Roots::new(project.dir), Recording::Off);
    let main = entry.url.as_str();
    let failure = |reason| Failure {
        at: None,
        reason,
        message: "refused".to_owned(),
        message_hash: None,
    };
    let rule = || FailureReason::Resolve(ResolveErrorKind::UnsupportedScheme("ext".to_owned()));
    let nothing = || FailureReason::Resolve(ResolveErrorKind::NotFound { ts_sibling: None });
    let dynamic = Reached::Import(Reach::Dynamic);
    for (specifier, reached, reason) in [
        ("ext:a", dynamic, rule()),
        ("ext:a", dynamic, rule()),
        ("ext:b", Reached::Import(Reach::Static), rule()),
        ("ext:c", Reached::MetaResolve, rule()),
        ("./d.ts", dynamic, nothing()),
    ] {
        graph.fail(specifier, Some(main), reached, failure(reason));
    }
    let site = FailureSite {
        specifier: "ext:a".to_owned(),
        importer: Some(main.to_owned()),
        reached: dynamic,
    };
    assert_eq!(
        graph.failures,
        BTreeMap::from([(site, vec![failure(rule())])])
    );
}

/// Recording off, the first [`MAX_KEPT_REFUSALS`] refusals naming no host
/// module are kept and the rest counted, each time, a retry of a kept one
/// neither; each kept one's specifier is clipped to [`MAX_KEPT_TEXT`] with
/// its hash in the marker, its reason's text to [`MAX_KEPT_TEXT`], and its
/// message to [`MAX_KEPT_MESSAGE`], keeping the hash of the whole.
/// Recording in full keeps every one as it is.
#[test]
fn recording_off_bounds_the_refusals_it_keeps() {
    let project = Root {
        dir: PathBuf::from("/a"),
        origin: Origin::Project,
    };
    let entry = resolution("/a/main.ts", &project);
    let main = entry.url.as_str();
    let dynamic = Reached::Import(Reach::Dynamic);
    let scheme = |i: usize| format!("s{i}{}", "x".repeat(MAX_KEPT_TEXT));
    let refusal = |i: usize| {
        let scheme = scheme(i);
        Failure {
            at: None,
            message: format!(
                "cannot resolve '{scheme}:a' from {main}: {}",
                "y".repeat(10_000)
            ),
            reason: FailureReason::Resolve(ResolveErrorKind::UnsupportedScheme(scheme)),
            message_hash: None,
        }
    };
    for recording in [Recording::Off, Recording::Full] {
        let mut graph = Graph::new(&entry, Roots::new(project.dir.clone()), recording);
        for i in 0..MAX_KEPT_REFUSALS + 3 {
            graph.fail(&format!("{}:a", scheme(i)), Some(main), dynamic, refusal(i));
            // a retry of each
            graph.fail(&format!("{}:a", scheme(i)), Some(main), dynamic, refusal(i));
        }
        let kept: Vec<_> = graph.failures.iter().collect();
        if recording.is_full() {
            assert_eq!(kept.len(), MAX_KEPT_REFUSALS + 3);
            assert_eq!(graph.unkept_refusals(), 0);
            assert!(kept.iter().all(|(site, failures)| {
                failures.len() == 1 && site.specifier.len() > MAX_KEPT_TEXT
            }));
            continue;
        }
        assert_eq!(kept.len(), MAX_KEPT_REFUSALS);
        // the three past the cap, and their retries
        assert_eq!(graph.unkept_refusals(), 6);
        for (site, failures) in kept {
            let [failure] = failures.as_slice() else {
                panic!("{failures:?}");
            };
            assert!(
                site.specifier.contains(" bytes, clipped, blake3 "),
                "{}",
                site.specifier
            );
            assert!(
                site.specifier.len() < MAX_KEPT_TEXT + 64,
                "{}",
                site.specifier
            );
            assert!(failure.message.len() < MAX_KEPT_MESSAGE + 32);
            assert!(failure.message.contains(" bytes clipped) …"));
            // the clipped specifier stands in the message for the whole
            assert!(
                failure.message.starts_with(&format!(
                    "cannot resolve '{}' from {main}: ",
                    site.specifier
                )),
                "{}",
                failure.message
            );
            let i: usize = site.specifier[1..]
                .split('x')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let whole = blake3::hash(refusal(i).message.as_bytes());
            assert_eq!(failure.message_hash, Some(whole));
            assert!(failure.is_message(&refusal(i).message, &whole));
            let FailureReason::Resolve(ResolveErrorKind::UnsupportedScheme(scheme)) =
                &failure.reason
            else {
                panic!("{:?}", failure.reason);
            };
            assert!(scheme.ends_with(" bytes, clipped)"), "{scheme}");
            assert!(scheme.len() < MAX_KEPT_TEXT + 32, "{scheme}");
        }
    }
}

/// The refusal of `args` to an importer of `origin`, which holds no grant.
fn not_granted(origin: &Origin) -> FailureReason {
    FailureReason::NotGranted {
        builtin: &BUILTINS[0],
        origin: origin.clone(),
    }
}

/// Recording off, the first refusal naming each host module by the project
/// and by each of the first [`MAX_PROBING_ORIGINS`] packages is kept
/// whatever else was refused,
/// then up to [`MAX_KEPT_REFUSALS`] more naming a host module and as many
/// naming none, each kind counted apart: a flood of other refusals, before
/// or after, can't push a probe out, nor can one origin's flood of
/// respellings push out another's probe, or its own of another host
/// module; an origin past the cap shares the respellings' bound.
#[test]
fn recording_off_keeps_each_origin_s_probe_of_each_host_module() {
    let project = Root {
        dir: PathBuf::from("/a"),
        origin: Origin::Project,
    };
    let entry = resolution("/a/main.ts", &project);
    let main = entry.url.as_str();
    let mut graph = Graph::new(&entry, Roots::new(project.dir), Recording::Off);
    let dynamic = Reached::Import(Reach::Dynamic);
    let failure = |reason| Failure {
        at: None,
        reason,
        message: "refused".to_owned(),
        message_hash: None,
    };
    let package = |name: &str| Origin::Package {
        name: name.to_owned(),
        version: None,
        linked: false,
    };
    let other = || FailureReason::Resolve(ResolveErrorKind::UnsupportedScheme("ext".to_owned()));
    let respelled = || {
        FailureReason::Resolve(ResolveErrorKind::Reserved(Reserved::Respelled(
            &BUILTINS[0],
        )))
    };
    let unbuilt = FailureReason::Resolve(ResolveErrorKind::Reserved(Reserved::Unbuilt(
        UNBUILT_HOST_MODULES[0],
    )));
    let args = BUILTINS[0].specifier;
    let kept = |graph: &Graph, specifier: &str| {
        graph
            .failures
            .keys()
            .any(|site| site.specifier == specifier)
    };
    // a package's probe before the flood, then a flood of other refusals
    graph.fail(
        args,
        Some("file:///a/node_modules/early/x.js"),
        dynamic,
        failure(not_granted(&package("early"))),
    );
    for i in 0..MAX_KEPT_REFUSALS + 10 {
        graph.fail(&format!("ext:{i}"), Some(main), dynamic, failure(other()));
    }
    assert_eq!(graph.unkept_refusals(), 10);
    // the project's flood of respellings: its first is its probe, then the
    // bound of the rest naming a host module
    for i in 0..MAX_KEPT_REFUSALS + 20 {
        graph.fail(
            &format!("{args}?{i}"),
            Some(main),
            dynamic,
            failure(respelled()),
        );
    }
    assert_eq!(graph.unkept_refusals(), 10 + 19);
    assert!(kept(&graph, &format!("{args}?{MAX_KEPT_REFUSALS}")));
    assert!(!kept(&graph, &format!("{args}?{}", MAX_KEPT_REFUSALS + 1)));
    // past both floods: the project's probe of another host module, and
    // another package's probe
    graph.fail("@fuzdev/gro/fs.ts", Some(main), dynamic, failure(unbuilt));
    graph.fail(
        args,
        Some("file:///a/node_modules/late/x.js"),
        dynamic,
        failure(not_granted(&package("late"))),
    );
    assert!(kept(&graph, "@fuzdev/gro/fs.ts"));
    let importers: Vec<_> = graph
        .failures
        .keys()
        .filter(|site| site.specifier == args)
        .map(|site| site.importer.as_deref().unwrap())
        .collect();
    assert_eq!(
        importers,
        [
            "file:///a/node_modules/early/x.js",
            "file:///a/node_modules/late/x.js"
        ]
    );
    // a retry of a probe is neither kept again nor counted, and another of
    // a known origin's probes of the same host module is past the bound
    graph.fail(
        args,
        Some("file:///a/node_modules/late/x.js"),
        dynamic,
        failure(not_granted(&package("late"))),
    );
    graph.fail(
        args,
        Some("file:///a/node_modules/late/y.js"),
        dynamic,
        failure(not_granted(&package("late"))),
    );
    assert_eq!(graph.unkept_refusals(), 10 + 19 + 1);
    // the packages so far are `early` and `late`, the project outside the
    // cap; the cap's last ones are kept, and the next shares the full bound
    let room = MAX_PROBING_ORIGINS - 2;
    for i in 0..room + 2 {
        let importer = format!("file:///a/node_modules/p{i}/x.js");
        graph.fail(
            args,
            Some(&importer),
            dynamic,
            failure(not_granted(&package(&format!("p{i}")))),
        );
    }
    let probed = graph
        .failures
        .keys()
        .filter(|site| site.specifier == args)
        .count();
    assert_eq!(probed, 2 + room);
    assert_eq!(graph.unkept_refusals(), 10 + 19 + 1 + 2);
    assert_eq!(graph.unkept_host_module_refusals(), 19 + 1 + 2);
    assert_eq!(
        graph.failures.len(),
        MAX_KEPT_REFUSALS + 1 + MAX_KEPT_REFUSALS + 1 + 2 + room
    );
    // the project's probe of a third host module is kept past the cap
    let third = UNBUILT_HOST_MODULES[1];
    graph.fail(
        &format!("{third}.ts"),
        Some(main),
        dynamic,
        failure(FailureReason::Resolve(ResolveErrorKind::Reserved(
            Reserved::Unbuilt(third),
        ))),
    );
    assert!(kept(&graph, &format!("{third}.ts")));
    assert_eq!(graph.unkept_refusals(), 10 + 19 + 1 + 2);
}

/// The project is outside [`MAX_PROBING_ORIGINS`]: its first probe of each
/// host module is kept even when the packages' probes filled the cap and
/// the bound past it first, and a package past both is counted as a
/// refusal naming a host module.
#[test]
fn recording_off_keeps_the_project_s_probe_past_a_full_cap() {
    let project = Root {
        dir: PathBuf::from("/a"),
        origin: Origin::Project,
    };
    let entry = resolution("/a/main.ts", &project);
    let main = entry.url.as_str();
    let mut graph = Graph::new(&entry, Roots::new(project.dir), Recording::Off);
    let dynamic = Reached::Import(Reach::Dynamic);
    let failure = |reason| Failure {
        at: None,
        reason,
        message: "refused".to_owned(),
        message_hash: None,
    };
    let args = BUILTINS[0].specifier;
    for i in 0..=MAX_PROBING_ORIGINS + MAX_KEPT_REFUSALS {
        let origin = Origin::Package {
            name: format!("p{i}"),
            version: None,
            linked: false,
        };
        graph.fail(
            args,
            Some(&format!("file:///a/node_modules/p{i}/x.js")),
            dynamic,
            failure(not_granted(&origin)),
        );
    }
    assert_eq!(graph.unkept_refusals(), 1);
    assert_eq!(graph.unkept_host_module_refusals(), 1);
    // the project's probes, by a grant's origin and by the recorded importer
    graph.fail(
        args,
        Some(main),
        dynamic,
        failure(not_granted(&Origin::Project)),
    );
    let fs = UNBUILT_HOST_MODULES[0];
    graph.fail(
        &format!("{fs}.ts"),
        Some(main),
        dynamic,
        failure(FailureReason::Resolve(ResolveErrorKind::Reserved(
            Reserved::Unbuilt(fs),
        ))),
    );
    assert!(
        graph
            .failures
            .keys()
            .any(|site| site.specifier == args && site.importer.as_deref() == Some(main))
    );
    assert!(
        graph
            .failures
            .keys()
            .any(|site| site.specifier == format!("{fs}.ts"))
    );
    assert_eq!(graph.unkept_refusals(), 1);
}

/// The refusals naming a host module are a builtin the importer isn't
/// granted or asked for with attributes, and each kind of reserved
/// specifier, through an `"exports"` target too; none of the other
/// refusals by rule names one.
#[test]
fn a_probe_names_its_host_module() {
    let path = || PathBuf::from("/a/x.js");
    let args = BUILTINS[0].specifier;
    for (reason, host_module) in [
        (not_granted(&Origin::Project), args),
        (
            FailureReason::Load(LoadFailure::BuiltinAttributes(&BUILTINS[0])),
            args,
        ),
        (
            FailureReason::Resolve(ResolveErrorKind::Reserved(Reserved::JsSpelling(
                &BUILTINS[0],
            ))),
            args,
        ),
        (
            FailureReason::Resolve(ResolveErrorKind::Reserved(Reserved::Respelled(
                &BUILTINS[0],
            ))),
            args,
        ),
        (
            FailureReason::Resolve(ResolveErrorKind::Target {
                path: path(),
                error: Box::new(ResolveErrorKind::Reserved(Reserved::Unbuilt(
                    "@fuzdev/gro/fs",
                ))),
            }),
            "@fuzdev/gro/fs",
        ),
        (
            FailureReason::Resolve(ResolveErrorKind::Reserved(Reserved::Split(
                &SPLIT_MODULES[0],
            ))),
            "@fuzdev/gro/process",
        ),
    ] {
        assert_eq!(reason.host_module(), Some(host_module), "{reason:?}");
        assert!(reason.guards_a_boundary(), "{reason:?}");
    }
    for reason in [
        FailureReason::StringCompiledImport,
        FailureReason::UnknownImporter,
        FailureReason::DynamicImport,
        FailureReason::DriverImport,
        FailureReason::Load(LoadFailure::NeverResolved),
        FailureReason::Load(LoadFailure::OutsideBox),
        FailureReason::Resolve(ResolveErrorKind::Remote),
        FailureReason::Resolve(ResolveErrorKind::UnsupportedScheme("builtin".to_owned())),
        FailureReason::Resolve(ResolveErrorKind::PackageImportsProject {
            package: Box::from("p"),
            path: path(),
            project: path(),
        }),
        FailureReason::Resolve(ResolveErrorKind::IntoNodeModules {
            path: path(),
            root: path(),
        }),
    ] {
        assert_eq!(reason.host_module(), None, "{reason:?}");
    }
}

/// A long refusal's message keeps who refused what: the specifier is
/// clipped where it stands, an importer past [`MAX_KEPT_IMPORTER`] keeps
/// its first installed package's name and its file, and only then is the
/// rest's middle cut; a short one is kept whole, unhashed.
#[test]
fn a_message_is_clipped_part_by_part() {
    let specifier = format!("https://x/{}", "a".repeat(5000));
    let clipped = clip_str_hashed(&specifier, MAX_KEPT_TEXT);
    let deep = format!(
        "file:///p/node_modules/@scope/pkg/{}probe.js",
        "d/".repeat(1500)
    );
    let message = format!("cannot resolve '{specifier}' from {deep}: it is remote");
    let (kept, hash) = clip_message(message.clone(), &specifier, &clipped, Some(&deep));
    assert_eq!(hash, Some(blake3::hash(message.as_bytes())));
    let importer = clip_importer(&deep);
    assert!(
        importer.starts_with("file:///p/node_modules/@scope/pkg… ("),
        "{importer}"
    );
    assert!(importer.ends_with("d/d/probe.js"), "{importer}");
    assert_eq!(
        kept,
        format!("cannot resolve '{clipped}' from {importer}: it is remote")
    );
    // the reason's own long text is what a middle cut takes
    let scheme = "s".repeat(10_000);
    let message = format!("cannot resolve '{specifier}' from {deep}: the scheme '{scheme}:'");
    let (kept, _) = clip_message(message, &specifier, &clipped, Some(&deep));
    assert!(kept.len() <= MAX_KEPT_MESSAGE + 32, "{}", kept.len());
    assert!(
        kept.starts_with(&format!(
            "cannot resolve '{clipped}' from {importer}: the scheme 's"
        )),
        "{kept}"
    );
    // no package to keep: the URL's two halves
    let flat = format!("file:///p/{}x.js", "f".repeat(2000));
    assert_eq!(
        clip_importer(&flat),
        clip_middle_at(&flat, MAX_KEPT_IMPORTER / 2, MAX_KEPT_IMPORTER)
    );
    // a package whose name ends past half the bound: the URL's two halves
    let long_name = format!(
        "file:///p/node_modules/{}/{}x.js",
        "n".repeat(MAX_KEPT_IMPORTER / 2),
        "d/".repeat(1000)
    );
    assert_eq!(
        clip_importer(&long_name),
        clip_middle_at(&long_name, MAX_KEPT_IMPORTER / 2, MAX_KEPT_IMPORTER)
    );
    let short = "cannot resolve 'ext:a' from file:///p/main.js: refused".to_owned();
    assert_eq!(
        clip_message(short.clone(), "ext:a", "ext:a", Some("file:///p/main.js")),
        (short, None)
    );
}

/// Clipping a reason clips every text and path in it, a package's
/// `"version"` in a grant refusal's origin among them, and leaves short
/// ones as they are.
#[test]
fn a_reason_s_text_is_clipped() {
    let long = "v".repeat(5000);
    let reason = FailureReason::NotGranted {
        builtin: &BUILTINS[0],
        origin: Origin::Package {
            name: "p".to_owned(),
            version: Some(long.clone()),
            linked: false,
        },
    };
    let FailureReason::NotGranted { origin, .. } = reason.clipped(10) else {
        panic!("not a grant refusal");
    };
    assert_eq!(
        origin,
        Origin::Package {
            name: "p".to_owned(),
            version: Some("vvvvvvvvvv… (5000 bytes, clipped)".to_owned()),
            linked: false,
        }
    );
    let outside = FailureReason::Resolve(ResolveErrorKind::OutsideRoot {
        path: PathBuf::from(format!("/a/{long}")),
        root: PathBuf::from("/a"),
        package: None,
    });
    assert_eq!(
        outside.clipped(10),
        FailureReason::Resolve(ResolveErrorKind::OutsideRoot {
            path: PathBuf::from("/a/vvvvvvv… (5003 bytes, clipped)"),
            root: PathBuf::from("/a"),
            package: None,
        })
    );
}

/// Each failure that guards the run's authority or boundaries is a refusal
/// by rule; each that found nothing there, or failed to read, is not.
#[test]
fn the_refusals_by_rule_are_the_ones_that_guard_the_run() {
    let path = || PathBuf::from("/a/x.js");
    let rules = [
        FailureReason::NotGranted {
            builtin: &BUILTINS[0],
            origin: Origin::Project,
        },
        FailureReason::StringCompiledImport,
        FailureReason::UnknownImporter,
        FailureReason::DynamicImport,
        FailureReason::DriverImport,
        FailureReason::Load(LoadFailure::BuiltinAttributes(&BUILTINS[0])),
        FailureReason::Load(LoadFailure::NeverResolved),
        FailureReason::Load(LoadFailure::OutsideBox),
        FailureReason::Resolve(ResolveErrorKind::Remote),
        FailureReason::Resolve(ResolveErrorKind::UnsupportedScheme("builtin".to_owned())),
        FailureReason::Resolve(ResolveErrorKind::OutsideRoot {
            path: path(),
            root: path(),
            package: None,
        }),
        FailureReason::Resolve(ResolveErrorKind::IntoNodeModules {
            path: path(),
            root: path(),
        }),
        FailureReason::Resolve(ResolveErrorKind::PackageImportsProject {
            package: Box::from("p"),
            path: path(),
            project: path(),
        }),
        FailureReason::Resolve(ResolveErrorKind::WorldWritablePackage {
            name: "p".to_owned(),
            dir: path(),
        }),
        FailureReason::Resolve(ResolveErrorKind::InvalidUrl(UrlFault::EncodedSeparator(
            "%2F".to_owned(),
        ))),
        // an `"exports"` target by what refused it
        FailureReason::Resolve(ResolveErrorKind::Target {
            path: path(),
            error: Box::new(ResolveErrorKind::IntoNodeModules {
                path: path(),
                root: path(),
            }),
        }),
    ];
    let nothing_there = [
        FailureReason::Resolve(ResolveErrorKind::NodeBuiltin),
        FailureReason::Resolve(ResolveErrorKind::Registry),
        FailureReason::Resolve(ResolveErrorKind::PackageImport),
        FailureReason::Resolve(ResolveErrorKind::NotFound { ts_sibling: None }),
        FailureReason::Resolve(ResolveErrorKind::NoExtension { suggestion: None }),
        FailureReason::Resolve(ResolveErrorKind::UnsupportedExtension(".json".to_owned())),
        FailureReason::Resolve(ResolveErrorKind::CommonJs),
        FailureReason::Resolve(ResolveErrorKind::NoMatchingCondition {
            subpath: ".".to_owned(),
            unmatched: Vec::new(),
        }),
        FailureReason::Resolve(ResolveErrorKind::InvalidUrl(UrlFault::Join(
            url::ParseError::EmptyHost,
        ))),
        FailureReason::Resolve(ResolveErrorKind::Target {
            path: path(),
            error: Box::new(ResolveErrorKind::NotFound { ts_sibling: None }),
        }),
        // a feature that doesn't exist, the same whatever the specifier
        FailureReason::Load(LoadFailure::UnsupportedAttribute),
        FailureReason::Load(LoadFailure::ModuleType),
        FailureReason::Load(LoadFailure::Read(io::ErrorKind::PermissionDenied)),
        FailureReason::Load(LoadFailure::ReplacedSinceLookup),
        FailureReason::Load(LoadFailure::TooLarge),
        FailureReason::Load(LoadFailure::Strip {
            failure: StripFailure::Parse,
            at: None,
        }),
    ];
    for reason in rules {
        assert!(reason.guards_a_boundary(), "{reason:?}");
    }
    for reason in nothing_there {
        assert!(!reason.guards_a_boundary(), "{reason:?}");
    }
}
