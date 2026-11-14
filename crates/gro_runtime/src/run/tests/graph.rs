//! Tests for the graph a run records.

use super::super::{RunError, run_recorded};
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};
use crate::loader::graph::Graph;
use crate::loader::graph::Recording;
use crate::test_support::TestDir;
use crate::{RuntimeOptions, STACK_SIZE};

/// Writes `files` into a fresh directory named for `name`, runs its
/// `entry` recording as `recording` says, and returns the run's result and
/// graph, with the directory, removed when it drops. A second run of the
/// same `name` remakes the directory at the same path, so its graph names
/// the same files.
fn run_graph(
    name: &str,
    files: &[(&str, &str)],
    entry: &str,
    recording: Recording,
) -> (Result<(), RunError>, Option<Graph>, TestDir) {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let test_dir = TestDir::on_disk(name);
    let dir = test_dir.path();
    for (file, content) in files {
        std::fs::write(dir.join(file), content).unwrap();
    }
    let entry = dir.join(entry);
    let cwd = dir.to_owned();
    let (outcome, graph) = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || run_recorded(engine, &RuntimeOptions::default(), &entry, &cwd, recording))
        .unwrap()
        .join()
        .unwrap();
    (outcome.result, graph, test_dir)
}

const PACKAGE_JSON: (&str, &str) = ("package.json", r#"{"name": "fixture", "type": "module"}"#);

/// A run records its graph in full: the static import and the modules a
/// dynamic one loads as edges, an `import()` that failed and was caught
/// as a failure, `import.meta.resolve` as neither, the files consulted
/// from the entry on, and the format and hash of each module's bytes
/// before the strip. Recording off, the same run keeps only its roots
/// and records.
#[test]
fn a_run_records_its_graph() {
    use std::collections::{BTreeMap, BTreeSet};

    use url::Url;

    use crate::loader::graph::{Edge, Failure, FailureReason, FailureSite, Loaded, Reach, Reached};
    use crate::resolver::{ConsultedFile, FileRole, ModuleFormat, ResolveErrorKind};

    const MAIN: &str = "import {a} from './a.ts';
let caught = false;
try {
\tawait import('./missing.js');
} catch (error) {
\tcaught = error instanceof TypeError;
}
if (!caught) throw new Error('the import() did not fail with a TypeError');
const url = import.meta.resolve('./b.js');
if (!url.endsWith('/b.js')) throw new Error(url);
const {d} = await import('./c.js');
if (a !== 1 || d !== 4) throw new Error('wrong values');
";
    const A: &str = "export const a: number = 1;\n";
    const C: &str = "export {d} from './d.js';\n";
    const D: &str = "export const d = 4;\n";
    let files = [
        PACKAGE_JSON,
        ("main.js", MAIN),
        ("a.ts", A),
        ("b.js", "export {};\n"),
        ("c.js", C),
        ("d.js", D),
    ];
    let name = "run_records_its_graph";
    let (result, graph, test_dir) = run_graph(name, &files, "main.js", Recording::Full);
    assert!(result.is_ok(), "{result:?}");
    let (off_result, off, _off_dir) = run_graph(name, &files, "main.js", Recording::Off);
    let dir = test_dir.path();
    assert!(off_result.is_ok(), "{off_result:?}");
    let graph = graph.unwrap();
    let off = off.unwrap();
    let url = |file: &str| Url::from_file_path(dir.join(file)).unwrap();

    let main = url("main.js");
    let edge = |importer: &Url, specifier: &str, reach| Edge {
        importer: importer.clone(),
        specifier: specifier.to_owned(),
        reach,
    };
    assert_eq!(
        *graph.edges(),
        BTreeMap::from([
            (edge(&main, "./a.ts", Reach::Static), url("a.ts")),
            (edge(&main, "./c.js", Reach::Dynamic), url("c.js")),
            // a static import inside a dynamically loaded module
            (edge(&url("c.js"), "./d.js", Reach::Dynamic), url("d.js")),
        ])
    );
    // `import.meta.resolve` loaded nothing, and the failed import nothing
    let mut recorded: Vec<_> = graph.records().keys().cloned().collect();
    recorded.sort();
    let mut expected = vec![main.clone(), url("a.ts"), url("c.js"), url("d.js")];
    expected.sort();
    assert_eq!(recorded, expected);
    for (file, source, format) in [
        ("main.js", MAIN, ModuleFormat::JavaScript),
        ("a.ts", A, ModuleFormat::TypeScript),
        ("c.js", C, ModuleFormat::JavaScript),
        ("d.js", D, ModuleFormat::JavaScript),
    ] {
        let loaded = Loaded {
            format,
            hash: blake3::hash(source.as_bytes()),
        };
        assert_eq!(
            graph.records()[&url(file)].module().unwrap().loaded,
            Some(loaded),
            "{file}"
        );
    }

    assert_eq!(graph.failures().len(), 1, "{:?}", graph.failures());
    let site = FailureSite {
        specifier: "./missing.js".to_owned(),
        importer: Some(main.to_string()),
        reached: Reached::Import(Reach::Dynamic),
    };
    assert!(
        matches!(
            graph.failures()[&site].as_slice(),
            [Failure {
                at: None,
                reason: FailureReason::Resolve(ResolveErrorKind::NotFound { .. }),
                ..
            }]
        ),
        "{:?}",
        graph.failures()
    );

    let package_json = dir.join("package.json");
    let consulted = |role| ConsultedFile {
        path: package_json.clone(),
        role,
    };
    assert_eq!(
        *graph.consulted(),
        BTreeSet::from([
            consulted(FileRole::SearchBound),
            consulted(FileRole::PackageScope),
        ])
    );

    // off, the same roots and records, with nothing else
    assert_eq!(off.roots(), graph.roots());
    let mut off_recorded: Vec<_> = off.records().keys().cloned().collect();
    off_recorded.sort();
    assert_eq!(off_recorded, expected);
    assert!(
        off.records()
            .values()
            .all(|record| record.module().unwrap().loaded.is_none())
    );
    assert!(off.edges().is_empty());
    assert!(off.consulted().is_empty());
    assert!(off.failures().is_empty());
}

/// The graph is recorded for a run that fails, and there is none when
/// the entry doesn't resolve.
#[test]
fn a_failed_run_still_records_its_graph() {
    use crate::loader::graph::{FailureReason, LoadFailure};

    let files = [("main.ts", "enum E { A }\n")];
    let name = "failed_run_records_its_graph";
    let (result, graph, _dir) = run_graph(name, &files, "main.ts", Recording::Full);
    let (missing, none, _missing_dir) = run_graph(name, &files, "missing.ts", Recording::Full);
    assert!(matches!(result, Err(RunError::Uncaught(_))), "{result:?}");
    let failures = graph.unwrap().failures().clone();
    assert_eq!(failures.len(), 1, "{failures:?}");
    let (site, failures) = failures.iter().next().unwrap();
    // the entry's own load names no importer
    assert_eq!(site.importer, None);
    assert!(matches!(
        failures.as_slice(),
        [crate::loader::graph::Failure {
            reason: FailureReason::Load(LoadFailure::Strip { .. }),
            ..
        }]
    ));
    assert!(matches!(missing, Err(RunError::Entry(_))), "{missing:?}");
    assert!(none.is_none());
}

/// A load that fails is recorded by how `deno_core` reached it: the
/// module an `import()` call loads and every static import below it as
/// dynamic, the root with no importer and the import below it with its
/// importer and position, and a static import of the entry's graph as
/// static.
#[test]
fn a_load_failure_is_recorded_by_its_reach() {
    use url::Url;

    use crate::loader::graph::Position;
    use crate::loader::graph::{Failure, FailureReason, FailureSite, LoadFailure, Reach, Reached};

    const MAIN: &str = "try {
\tawait import('./e.ts');
} catch {}
try {
\tawait import('./c.js');
} catch {}
await import('./s.js');
";
    let mut files = [
        PACKAGE_JSON,
        ("main.js", MAIN),
        ("e.ts", "enum E { A }\n"),
        ("c.js", "import './f.ts';\n"),
        ("f.ts", "enum F { A }\n"),
        ("s.js", "export {};\n"),
    ];
    let name = "load_failure_reach";
    let (result, graph, test_dir) = run_graph(name, &files, "main.js", Recording::Full);
    // the entry's own load, failing, is static with no importer
    files[1].1 = "import './e.ts';\n";
    let (failed, static_graph, _static_dir) = run_graph(name, &files, "main.js", Recording::Full);
    let dir = test_dir.path();
    let url = |file: &str| Url::from_file_path(dir.join(file)).unwrap().to_string();
    assert!(result.is_ok(), "{result:?}");
    assert!(failed.is_err());
    let failures = graph.unwrap().failures().clone();
    let dynamic = Reached::Import(Reach::Dynamic);
    let sites: Vec<_> = failures.keys().cloned().collect();
    assert_eq!(
        sites,
        [
            FailureSite {
                specifier: url("e.ts"),
                importer: None,
                reached: dynamic,
            },
            FailureSite {
                specifier: url("f.ts"),
                importer: Some(url("c.js")),
                reached: dynamic,
            },
        ]
    );
    assert!(matches!(
        failures[&sites[1]].as_slice(),
        [Failure {
            at: Some(Position { line: 1, column: 8 }),
            reason: FailureReason::Load(LoadFailure::Strip { .. }),
            ..
        }]
    ));
    let failures = static_graph.unwrap().failures().clone();
    let sites: Vec<_> = failures.keys().cloned().collect();
    assert_eq!(
        sites,
        [FailureSite {
            specifier: url("e.ts"),
            importer: Some(url("main.js")),
            reached: Reached::Import(Reach::Static),
        }]
    );
}

/// A run records at most one failure reached statically: `deno_core`
/// stops the entry's load at its first error, and linking reuses the
/// specifiers it already resolved, so the other refused imports are
/// never loaded or resolved again.
#[test]
fn a_run_records_at_most_one_static_failure() {
    use crate::loader::graph::{FailureReason, LoadFailure, Reach, Reached};

    let files = [
        PACKAGE_JSON,
        (
            "main.js",
            "import './z.ts';\nimport './a.ts';\nimport './y.ts';\n",
        ),
        ("z.ts", "enum E { A }\n"),
        ("a.ts", "enum E { A }\n"),
        ("y.ts", "enum E { A }\n"),
    ];
    let (result, graph, _) = run_graph("one_static_failure", &files, "main.js", Recording::Full);
    assert!(matches!(result, Err(RunError::Uncaught(_))), "{result:?}");
    let failures = graph.unwrap().failures().clone();
    let all: Vec<_> = failures
        .iter()
        .flat_map(|(site, failures)| failures.iter().map(move |failure| (site, failure)))
        .collect();
    let [(site, failure)] = all.as_slice() else {
        panic!("{failures:?}");
    };
    assert_eq!(site.reached, Reached::Import(Reach::Static));
    assert!(matches!(
        failure.reason,
        FailureReason::Load(LoadFailure::Strip { .. })
    ));
}
