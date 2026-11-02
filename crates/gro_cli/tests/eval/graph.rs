//! The `--graph` dump: its contents, determinism, what it moves with, and its
//! refused and failed runs.

use serde_json::{Value, json};

use crate::limits::HEAP_BACKSTOP;
use crate::support::Scratch;
use crate::{eval, fixtures, memory_heavy_turn};

/// The files of a project with a self-reference, a scoped package with
/// another nested in its own `node_modules`, and a package linked into its
/// `node_modules` from `lib/` beside it, which [`graph_project`] writes.
const GRAPH_FILES: [(&str, &str); 11] = [
    (
        "project/package.json",
        r#"{"name": "app", "type": "module", "exports": {"./self.js": "./src/self.js"}}"#,
    ),
    (
        "project/src/config.ts",
        "import {a} from './a.js';\nimport './a.js?x#y';\nimport {s} from 'app/self.js';\n\
         import {v} from '@sc/pkg';\nimport {l} from 'lnk/l.js';\n\
         export default {a, s, v, l} as const;\n",
    ),
    ("project/src/a.js", "export const a = 'a';\n"),
    ("project/src/self.js", "export const s = 'self';\n"),
    (
        "project/node_modules/@sc/pkg/package.json",
        r#"{"name": "@sc/pkg", "version": "1.0.0", "type": "module", "exports": "./i.js"}"#,
    ),
    (
        "project/node_modules/@sc/pkg/i.js",
        "import {n} from 'nested';\nexport const v = n + 1;\n",
    ),
    (
        "project/node_modules/@sc/pkg/node_modules/nested/package.json",
        r#"{"name": "nested", "version": "2.0.0", "exports": "./n.mjs"}"#,
    ),
    (
        "project/node_modules/@sc/pkg/node_modules/nested/n.mjs",
        "export const n = 1;\n",
    ),
    (
        "lib/package.json",
        r#"{"name": "lnk", "version": "0.1.0", "type": "module", "exports": {"./l.js": "./l.js"}}"#,
    ),
    ("lib/l.js", "export const l = 'linked';\n"),
    // a type marker, outside every walk: it changes nothing
    ("lib/sub/package.json", r#"{"type": "module"}"#),
];

/// [`GRAPH_FILES`] written under a scratch directory named `name`, with
/// `project/node_modules/lnk` linked to `../../lib`.
fn graph_project(name: &str) -> Scratch {
    let scratch = Scratch::new(name, &GRAPH_FILES);
    std::os::unix::fs::symlink("../../lib", scratch.0.join("project/node_modules/lnk")).unwrap();
    scratch
}

/// The BLAKE3 hash of `content`, in hex.
fn hex(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

/// The content [`GRAPH_FILES`] gives `path`.
fn graph_file(path: &str) -> &'static str {
    GRAPH_FILES
        .iter()
        .find(|(file, _)| *file == path)
        .unwrap()
        .1
}

/// The expected `graph` of [`graph_project`]'s config, but for `local`.
fn expected_graph() -> Value {
    let file = |path: &str| hex(graph_file(path));
    let project = json!({"kind": "project"});
    let import = |specifier: &str, resolved: &str| json!({"specifier": specifier, "resolved": resolved, "reach": "static"});
    let consulted =
        |id: &str, path: &str, role: &str| json!({"id": id, "blake3": file(path), "role": role});
    json!({
        "graph_version": 1,
        "roots": ["project:src/config.ts"],
        "modules": [
            {
                "id": "linked:../lib/l.js",
                "blake3": file("lib/l.js"),
                "origin": {"kind": "package", "name": "lnk", "version": "0.1.0", "linked": true},
                "format": "javascript",
                "imports": [],
            },
            {
                "id": "project:node_modules/@sc/pkg/i.js",
                "blake3": file("project/node_modules/@sc/pkg/i.js"),
                "origin": {"kind": "package", "name": "@sc/pkg", "version": "1.0.0", "linked": false},
                "format": "javascript",
                "imports": [import("nested", "project:node_modules/@sc/pkg/node_modules/nested/n.mjs")],
            },
            {
                "id": "project:node_modules/@sc/pkg/node_modules/nested/n.mjs",
                "blake3": file("project/node_modules/@sc/pkg/node_modules/nested/n.mjs"),
                "origin": {"kind": "package", "name": "nested", "version": "2.0.0", "linked": false},
                "format": "javascript",
                "imports": [],
            },
            {
                "id": "project:src/a.js",
                "blake3": file("project/src/a.js"),
                "origin": project,
                "format": "javascript",
                "imports": [],
            },
            {
                "id": "project:src/a.js?x#y",
                "blake3": file("project/src/a.js"),
                "origin": project,
                "format": "javascript",
                "imports": [],
            },
            {
                "id": "project:src/config.ts",
                "blake3": file("project/src/config.ts"),
                "origin": project,
                "format": "typescript",
                "imports": [
                    import("./a.js", "project:src/a.js"),
                    import("./a.js?x#y", "project:src/a.js?x#y"),
                    import("@sc/pkg", "project:node_modules/@sc/pkg/i.js"),
                    import("app/self.js", "project:src/self.js"),
                    import("lnk/l.js", "linked:../lib/l.js"),
                ],
            },
            {
                "id": "project:src/self.js",
                "blake3": file("project/src/self.js"),
                "origin": project,
                "format": "javascript",
                "imports": [],
            },
        ],
        "consulted": [
            consulted("linked:../lib/package.json", "lib/package.json", "exports"),
            consulted("linked:../lib/package.json", "lib/package.json", "package_scope"),
            consulted(
                "project:node_modules/@sc/pkg/node_modules/nested/package.json",
                "project/node_modules/@sc/pkg/node_modules/nested/package.json",
                "exports",
            ),
            consulted(
                "project:node_modules/@sc/pkg/package.json",
                "project/node_modules/@sc/pkg/package.json",
                "exports",
            ),
            consulted(
                "project:node_modules/@sc/pkg/package.json",
                "project/node_modules/@sc/pkg/package.json",
                "package_scope",
            ),
            consulted("project:package.json", "project/package.json", "exports"),
            consulted("project:package.json", "project/package.json", "package_scope"),
            consulted("project:package.json", "project/package.json", "search_bound"),
        ],
        // the source directory's own package.json, on the walks up from it,
        // and its node_modules entries, on the package searches from it
        "absent": [
            "project:src/node_modules/@sc/pkg",
            "project:src/node_modules/lnk",
            "project:src/package.json",
        ],
    })
}

/// `--graph` adds the graph to the envelope as its last field, every file by
/// its path relative to the project, `local` alone holding absolute paths;
/// without the flag the envelope has none.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_graph_dump_names_every_file_by_its_path() {
    let scratch = graph_project("graph");
    let config = scratch.path("project/src/config.ts");
    let run = eval(&[&config, "--graph"]);
    run.assert_value(&json!({"a": "a", "s": "self", "v": 2, "l": "linked"}));
    let keys: Vec<&str> = run
        .envelope
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "eval_protocol",
            "ok",
            "runtime",
            "entry",
            "value",
            "access",
            "graph"
        ]
    );
    let mut graph = run.envelope["graph"].clone();
    let local = graph.as_object_mut().unwrap().remove("local").unwrap();
    assert_eq!(graph, expected_graph(), "{:#}", run.envelope["graph"]);
    let project = scratch.path("project");
    let lib = scratch.path("lib");
    assert_eq!(local["project"], project.as_str());
    assert_eq!(local["box"], json!([project, lib]));
    // every id but a query's has its absolute path, and nothing else does
    let paths = local["paths"].as_object().unwrap();
    for (id, path) in paths {
        let path = path.as_str().unwrap();
        let expected = id.strip_prefix("project:").map_or_else(
            || {
                format!(
                    "{}/{}",
                    scratch.0.display(),
                    id.strip_prefix("linked:../").unwrap()
                )
            },
            |relative| format!("{project}/{relative}"),
        );
        assert_eq!(path, expected, "{id}");
    }
    let mut ids: Vec<&str> = paths.keys().map(String::as_str).collect();
    ids.sort_unstable();
    assert_eq!(
        ids,
        [
            "linked:../lib/l.js",
            "linked:../lib/package.json",
            "project:node_modules/@sc/pkg/i.js",
            "project:node_modules/@sc/pkg/node_modules/nested/n.mjs",
            "project:node_modules/@sc/pkg/node_modules/nested/package.json",
            "project:node_modules/@sc/pkg/package.json",
            "project:package.json",
            "project:src/a.js",
            "project:src/config.ts",
            "project:src/node_modules/@sc/pkg",
            "project:src/node_modules/lnk",
            "project:src/package.json",
            "project:src/self.js",
        ]
    );
    let without = eval(&[&config]);
    assert!(
        without.envelope.get("graph").is_none(),
        "{}",
        without.stdout
    );
    // the same envelope but for the graph
    assert_eq!(
        without.stdout.trim_end(),
        run.stdout
            .trim_end()
            .rsplit_once(",\"graph\":")
            .map(|(before, _)| format!("{before}}}"))
            .unwrap()
    );
}

/// The dump is the same on every run, and the same for the project moved
/// elsewhere, but for `local`.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_graph_dump_is_deterministic_and_location_independent() {
    let here = graph_project("graph_here");
    let moved = graph_project("graph_moved_elsewhere");
    let graph = |scratch: &Scratch| {
        let run = eval(&[&scratch.path("project/src/config.ts"), "--graph"]);
        assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
        run.envelope["graph"].clone()
    };
    let first = graph(&here);
    assert_eq!(graph(&here), first);
    let mut elsewhere = graph(&moved);
    assert_ne!(elsewhere["local"], first["local"]);
    elsewhere["local"] = first["local"].clone();
    assert_eq!(elsewhere, first);
}

/// What a cache would key on moves with what the run depends on: an
/// edited `package.json` changes its hash and nothing else, and a path the
/// package search passed appearing (a package in the source directory's
/// own `node_modules`, which then shadows the linked one) leaves `absent`
/// and changes the value.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_graph_dump_moves_with_what_the_run_read() {
    let scratch = graph_project("graph_changes");
    let config = scratch.path("project/src/config.ts");
    let graph = || {
        let run = eval(&[&config, "--graph"]);
        assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
        let mut graph = run.envelope["graph"].clone();
        graph.as_object_mut().unwrap().remove("local");
        (run.envelope["value"].clone(), graph)
    };
    assert_eq!(graph().1, expected_graph());

    let edited = r#"{"name": "lnk", "version": "0.1.0", "type": "module", "description": "x", "exports": {"./l.js": "./l.js"}}"#;
    std::fs::write(scratch.0.join("lib/package.json"), edited).unwrap();
    let mut expected = expected_graph();
    for file in expected["consulted"].as_array_mut().unwrap() {
        if file["id"] == "linked:../lib/package.json" {
            file["blake3"] = json!(hex(edited));
        }
    }
    assert_eq!(graph().1, expected);

    let shadow =
        r#"{"name": "lnk", "version": "9.0.0", "type": "module", "exports": {"./l.js": "./l.js"}}"#;
    let dir = scratch.0.join("project/src/node_modules/lnk");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("package.json"), shadow).unwrap();
    std::fs::write(dir.join("l.js"), "export const l = 'shadow';\n").unwrap();
    let (value, after) = graph();
    assert_eq!(value["l"], "shadow");
    assert_eq!(
        after["absent"],
        json!([
            "project:src/node_modules/@sc/pkg",
            "project:src/package.json"
        ])
    );
    let ids = |key: &str| -> Vec<String> {
        after[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert!(ids("modules").contains(&"project:src/node_modules/lnk/l.js".to_owned()));
    assert!(!ids("modules").contains(&"linked:../lib/l.js".to_owned()));
    assert!(ids("consulted").contains(&"project:src/node_modules/lnk/package.json".to_owned()));
}

/// A run that refuses its `import()`s dumps the graph without them, since
/// nothing an `import()` names is resolved; a failed run dumps the graph
/// as far as it got, under `ok: false`; and with no graph to dump (the
/// entry didn't resolve, or the process ran out of memory) `graph` is
/// `null`.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_graph_dump_of_refused_and_failed_runs() {
    let fixtures = fixtures().canonicalize().unwrap();
    let id = |file: &str| format!("project:{file}");
    let hash = |file: &str| hex(&std::fs::read_to_string(fixtures.join(file)).unwrap());

    let run = eval(&["dynamic.js", "--graph"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert_eq!(
        run.envelope["access"]["dynamic"].as_array().unwrap().len(),
        2
    );
    let graph = &run.envelope["graph"];
    assert_eq!(
        graph["modules"],
        json!([{
            "id": id("dynamic.js"),
            "blake3": hash("dynamic.js"),
            "origin": {"kind": "project"},
            "format": "javascript",
            "imports": [],
        }])
    );
    assert_eq!(graph["roots"], json!([id("dynamic.js")]));
    assert_eq!(
        graph["consulted"],
        json!([
            {"id": "project:package.json", "blake3": hash("package.json"), "role": "package_scope"},
            {"id": "project:package.json", "blake3": hash("package.json"), "role": "search_bound"},
        ])
    );
    assert_eq!(graph["absent"], json!([]));

    let run = eval(&["import_missing.js", "--graph"]);
    run.assert_failed("resolve", 1);
    assert_eq!(
        run.envelope["graph"]["modules"],
        json!([{
            "id": id("import_missing.js"),
            "blake3": hash("import_missing.js"),
            "origin": {"kind": "project"},
            "format": "javascript",
            "imports": [],
        }])
    );

    let run = eval(&["missing.js", "--graph"]);
    run.assert_failed("resolve", 2);
    assert_eq!(run.envelope["graph"], Value::Null);
    assert!(
        run.stdout.trim_end().ends_with(",\"graph\":null}"),
        "{}",
        run.stdout
    );

    let run = {
        let _turn = memory_heavy_turn();
        eval(&["heap_builtin.js", "--graph"])
    };
    run.assert_failed("heap", 1);
    assert_eq!(run.message(), HEAP_BACKSTOP);
    assert_eq!(run.envelope["graph"], Value::Null);
}
