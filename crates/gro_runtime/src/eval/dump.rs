//! The graph dump: the run's module graph as data, the envelope's `graph`
//! when the caller asks for it (`graph_version` [`GRAPH_VERSION`]).
//!
//! Built from the [`Graph`] the run's loader recorded in full, after the
//! run, so it names exactly what was read: each module loaded, with the
//! hash of the bytes read for it, each `package.json` consulted, with the
//! hash of its one read, and each path a resolution went on past finding
//! absent. Every array is sorted, so the dump is a function of the graph,
//! not of the order the run reached things in.
//!
//! A file's id is location-independent: its real path relative to the
//! project's real directory P, `project:<path>` under P (`node_modules`
//! included) and `linked:<path>` outside it (with `..` segments), the path
//! joined by `/`, with `%`, `?`, and `#` percent-encoded, so a module's
//! query and fragment, which join its id as its URL has them, can't be
//! mistaken for part of a file's name. The absolute paths are only in
//! `local`, which a cache must never key on.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use url::Url;

use crate::loader::graph::{Graph, Reach};
use crate::resolver::{FileRole, ModuleFormat, Origin};

/// The dump's version, its first field. It moves with any change to its
/// shape that a reader must know about: a new field, a new id prefix, or a
/// new value of `origin.kind`, `format`, `reach`, or `role`.
pub const GRAPH_VERSION: u32 = 1;

/// The module graph of a run, as the envelope's `graph` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphDump {
    /// The dump's version, [`GRAPH_VERSION`].
    pub graph_version: u32,
    /// The modules the graph was loaded from: the entry alone.
    pub roots: Vec<String>,
    /// Every module the run resolved, by id.
    pub modules: Vec<DumpModule>,
    /// Every `package.json` a resolution read, by id, then role.
    pub consulted: Vec<DumpConsulted>,
    /// Every path a resolution went on past, finding nothing of the kind it
    /// looked for: a `package.json` with no file there, on a walk up, or a
    /// package with no directory there, in a `node_modules` it searched.
    /// Each is the path as looked up.
    pub absent: Vec<String>,
    /// The absolute paths, for watching and re-hashing, never for a key.
    pub local: DumpLocal,
}

/// A module the run resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DumpModule {
    /// Its id: its real path's, with its URL's query and fragment.
    pub id: String,
    /// The BLAKE3 hash of the bytes read for it, before any strip, in hex;
    /// unset when it didn't load, which only a failed run leaves.
    pub blake3: Option<String>,
    /// Where it comes from, its root's origin.
    pub origin: DumpOrigin,
    /// How its source became the JS V8 ran; unset when it didn't load.
    pub format: Option<DumpFormat>,
    /// Each import of it that resolved, by specifier, then reach.
    pub imports: Vec<DumpImport>,
}

/// Where a module comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DumpOrigin {
    /// The project's own files.
    Project,
    /// An installed or linked package.
    Package {
        /// The name it was first imported by.
        name: String,
        /// Its `package.json` `"version"`, unset when that isn't a string.
        version: Option<String>,
        /// Whether its real root is outside every `node_modules`.
        linked: bool,
    },
}

/// How a module's source became the JS V8 ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpFormat {
    /// Run as written.
    Javascript,
    /// Stripped of its types.
    Typescript,
}

/// An import that resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DumpImport {
    /// The specifier as written.
    pub specifier: String,
    /// The id of the module it resolved to.
    pub resolved: String,
    /// How it was reached.
    pub reach: DumpReach,
}

/// How an import was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpReach {
    /// A static import of the entry's graph.
    Static,
    /// Reached through an `import()`'s graph, which eval refuses before
    /// resolving, so none appears in its dump today.
    Dynamic,
}

/// A `package.json` a resolution read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DumpConsulted {
    /// Its id.
    pub id: String,
    /// The BLAKE3 hash of the bytes of its one read in the run, in hex.
    pub blake3: String,
    /// What it was read for.
    pub role: DumpRole,
}

/// What a `package.json` was read for, the resolver's [`FileRole`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpRole {
    /// A package's, for its `"exports"`.
    Exports,
    /// The nearest above a module, for its `"type"`, or above an importer,
    /// for a self-reference.
    PackageScope,
    /// One on the walk for the project root, which bounds the
    /// `node_modules` search.
    SearchBound,
    /// The package's an entry inside `node_modules` is in, for its version.
    Version,
}

/// The absolute paths behind the ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DumpLocal {
    /// The project's real directory, P, every id's base.
    pub project: String,
    /// The roots of the box the run's process was confined to: P, then
    /// each root of the link closure.
    #[serde(rename = "box")]
    pub box_roots: Vec<String>,
    /// Each id's absolute path: a module's without its query and fragment.
    pub paths: BTreeMap<String, String>,
}

/// Why a graph has no dump.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DumpError {
    /// A path in it isn't UTF-8, so no id can name it as written.
    #[error("the graph names {}, which isn't a UTF-8 path, so no id can name it", .0.display())]
    NotUtf8(PathBuf),
    /// A module's URL names no local path.
    #[error("the graph names the module {0}, whose URL names no local path")]
    NotAPath(Url),
    /// A consulted file has no read recorded: a host bug, which fails the
    /// eval as `internal`.
    #[error("the graph names {} as consulted with no read of it recorded", .0.display())]
    Unread(PathBuf),
}

/// The dump of `graph`, whose entry is `entry` and whose process was
/// confined to `box_roots`.
///
/// # Errors
///
/// Returns why the graph can't be dumped: a path that isn't UTF-8, a module
/// URL with no path, or a consulted file the run didn't record reading.
pub fn dump_graph(
    graph: &Graph,
    entry: &Url,
    box_roots: &[PathBuf],
) -> Result<GraphDump, DumpError> {
    let project = &graph.roots().project().dir;
    let mut paths = BTreeMap::new();
    let mut path_id = |path: &Path| -> Result<String, DumpError> {
        let id = to_path_id(path, project)?;
        paths.insert(id.clone(), to_utf8(path)?.to_owned());
        Ok(id)
    };
    let mut module_id = |url: &Url| -> Result<String, DumpError> {
        let path = url
            .to_file_path()
            .map_err(|()| DumpError::NotAPath(url.clone()))?;
        let mut id = path_id(&path)?;
        if let Some(query) = url.query() {
            id.push('?');
            id.push_str(query);
        }
        if let Some(fragment) = url.fragment() {
            id.push('#');
            id.push_str(fragment);
        }
        Ok(id)
    };
    let roots = vec![module_id(entry)?];
    let mut imports: BTreeMap<&Url, Vec<DumpImport>> = BTreeMap::new();
    for (edge, resolved) in graph.edges() {
        imports.entry(&edge.importer).or_default().push(DumpImport {
            specifier: edge.specifier.clone(),
            resolved: module_id(resolved)?,
            reach: match edge.reach {
                Reach::Static => DumpReach::Static,
                Reach::Dynamic => DumpReach::Dynamic,
            },
        });
    }
    let mut modules = Vec::with_capacity(graph.records().len());
    for (url, record) in graph.records() {
        let mut imports = imports.remove(url).unwrap_or_default();
        imports.sort_by(|a, b| {
            (&a.specifier, a.reach, &a.resolved).cmp(&(&b.specifier, b.reach, &b.resolved))
        });
        modules.push(DumpModule {
            id: module_id(url)?,
            blake3: record.loaded.map(|loaded| loaded.hash.to_hex().to_string()),
            origin: match &record.root.origin {
                Origin::Project => DumpOrigin::Project,
                Origin::Package {
                    name,
                    version,
                    linked,
                } => DumpOrigin::Package {
                    name: name.clone(),
                    version: version.clone(),
                    linked: *linked,
                },
            },
            format: record.loaded.map(|loaded| match loaded.format {
                ModuleFormat::JavaScript => DumpFormat::Javascript,
                ModuleFormat::TypeScript => DumpFormat::Typescript,
            }),
            imports,
        });
    }
    // an edge is recorded only from a module the loader recorded
    debug_assert!(
        imports.is_empty(),
        "edges from importers with no record: {:?}",
        imports.keys().collect::<Vec<_>>()
    );
    modules.sort_by(|a, b| a.id.cmp(&b.id));
    let mut consulted = Vec::with_capacity(graph.consulted().len());
    for file in graph.consulted() {
        let hash = graph
            .reads()
            .hash(&file.path)
            .ok_or_else(|| DumpError::Unread(file.path.clone()))?;
        consulted.push(DumpConsulted {
            id: path_id(&file.path)?,
            blake3: hash.to_hex().to_string(),
            role: match file.role {
                FileRole::Exports => DumpRole::Exports,
                FileRole::PackageScope => DumpRole::PackageScope,
                FileRole::SearchBound => DumpRole::SearchBound,
                FileRole::Version => DumpRole::Version,
            },
        });
    }
    consulted.sort_by(|a, b| (&a.id, a.role).cmp(&(&b.id, b.role)));
    let mut absent = graph
        .reads()
        .absent()
        .iter()
        .map(|path| path_id(path))
        .collect::<Result<Vec<_>, _>>()?;
    absent.sort();
    absent.dedup();
    let box_roots = box_roots
        .iter()
        .map(|root| to_utf8(root).map(str::to_owned))
        .collect::<Result<_, _>>()?;
    Ok(GraphDump {
        graph_version: GRAPH_VERSION,
        roots,
        modules,
        consulted,
        absent,
        local: DumpLocal {
            project: to_utf8(project)?.to_owned(),
            box_roots,
            paths,
        },
    })
}

/// `path` as UTF-8.
fn to_utf8(path: &Path) -> Result<&str, DumpError> {
    path.to_str()
        .ok_or_else(|| DumpError::NotUtf8(path.to_owned()))
}

/// The id of the absolute path `path`, relative to the project's real
/// directory `project`: `project:` under it, `linked:` outside it.
fn to_path_id(path: &Path, project: &Path) -> Result<String, DumpError> {
    let (prefix, relative) = path.strip_prefix(project).map_or_else(
        |_| ("linked:", relative_outside(path, project)),
        |below| ("project:", below.to_owned()),
    );
    let relative = to_utf8(&relative).map_err(|_| DumpError::NotUtf8(path.to_owned()))?;
    let mut id = String::with_capacity(prefix.len() + relative.len());
    id.push_str(prefix);
    for char in relative.chars() {
        match char {
            '%' => id.push_str("%25"),
            '?' => id.push_str("%3F"),
            '#' => id.push_str("%23"),
            char => id.push(char),
        }
    }
    Ok(id)
}

/// The path from `project` to `path`, outside it, by components: a `..`
/// for each of `project`'s past what they share, then the rest of `path`.
fn relative_outside(path: &Path, project: &Path) -> PathBuf {
    let shared = path
        .components()
        .zip(project.components())
        .take_while(|(a, b)| a == b)
        .count();
    let up = project.components().count() - shared;
    std::iter::repeat_n(Component::ParentDir, up)
        .chain(path.components().skip(shared))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::*;
    use crate::loader::graph::{Loaded, Recording};
    use crate::loader::reads::{Reads, RecordingTree};
    use crate::resolver::test_tree::MemoryTree;
    use crate::resolver::{self, module_format};

    /// A test's `'static` string.
    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    /// A project at `<base>/p` with a self-reference, a scoped package with a
    /// package nested in its own `node_modules`, a package linked from
    /// `<base>/q`, and a directory `src-x` beside `src`, as `(path, content)`
    /// pairs and the link. The linked package's paths sort after the
    /// project's and `src-x`'s after `src`'s, while their ids sort before, so
    /// the dump's order is the ids', not the record's.
    fn tree(base: &str) -> MemoryTree {
        let files = [
            (
                "p/package.json",
                r#"{"name": "app", "type": "module", "exports": {"./self.js": "./self.js"}}"#,
            ),
            ("p/src/main.ts", "// main"),
            ("p/self.js", "// self"),
            ("p/src-x/y.js", "// y"),
            (
                "p/node_modules/@sc/pkg/package.json",
                r#"{"name": "@sc/pkg", "version": "1.0.0", "exports": "./i.mjs"}"#,
            ),
            ("p/node_modules/@sc/pkg/i.mjs", "// i"),
            (
                "p/node_modules/@sc/pkg/node_modules/nested/package.json",
                r#"{"exports": "./n.mjs"}"#,
            ),
            ("p/node_modules/@sc/pkg/node_modules/nested/n.mjs", "// n"),
            (
                "q/package.json",
                r#"{"name": "lnk", "version": "0.1.0", "exports": {"./l.mjs": "./l.mjs"}}"#,
            ),
            ("q/l.mjs", "// l"),
        ];
        MemoryTree {
            files: files
                .into_iter()
                .map(|(path, content)| (leak(format!("{base}/{path}")), content))
                .collect(),
            links: BTreeMap::from([(
                leak(format!("{base}/p/node_modules/lnk")),
                leak(format!("{base}/q")),
            )]),
            ..MemoryTree::default()
        }
    }

    /// The dump of the graph of `<base>/p/src/main.ts` in [`tree`], each of
    /// `imports` (importer relative to `base`, specifier) resolved, admitted,
    /// linked, and loaded in order, as the loader would, recording in full.
    fn dump_of(base: &str, imports: &[(&str, &str)]) -> GraphDump {
        let tree = tree(base);
        let mut reads = Reads::default();
        let (entry, roots, _) = resolver::resolve_entry_with_bound(
            &RecordingTree::new(&tree, &mut reads),
            Path::new(&format!("{base}/p/src/main.ts")),
            Path::new("/"),
        )
        .unwrap();
        let mut graph = Graph::new(&entry, roots, Recording::Full).with_reads(reads);
        let load = |graph: &mut Graph, url: &Url| {
            let path = url.to_file_path().unwrap();
            let content = tree.files[path.to_str().unwrap()];
            graph.load(
                url,
                Loaded {
                    format: module_format(&path).unwrap(),
                    hash: blake3::hash(content.as_bytes()),
                },
            );
        };
        load(&mut graph, &entry.url);
        for (importer, specifier) in imports {
            let importer = Url::from_file_path(format!("{base}/{importer}")).unwrap();
            let resolution = graph.resolve(&tree, specifier, &importer).unwrap().unwrap();
            graph.consult(resolution.files_consulted.iter().cloned());
            graph.admit(&resolution).unwrap();
            graph.link(&importer, specifier, Reach::Static, &resolution.url);
            load(&mut graph, &resolution.url);
        }
        let box_roots = [
            PathBuf::from(format!("{base}/p")),
            PathBuf::from(format!("{base}/q")),
        ];
        dump_graph(&graph, &entry.url, &box_roots).unwrap()
    }

    const IMPORTS: [(&str, &str); 7] = [
        ("p/src/main.ts", "../src-x/y.js"),
        ("p/src/main.ts", "app/self.js"),
        ("p/src/main.ts", "@sc/pkg"),
        ("p/src/main.ts", "lnk/l.mjs"),
        ("p/src/main.ts", "../self.js?q=1#f"),
        ("p/node_modules/@sc/pkg/i.mjs", "nested"),
        ("p/src/main.ts", "../self.js"),
    ];

    fn hash(content: &str) -> String {
        blake3::hash(content.as_bytes()).to_hex().to_string()
    }

    /// The dump of a graph with a self-reference, a scoped package, a
    /// package nested in another's `node_modules`, a linked package, and a
    /// module imported with a query and fragment, pinned whole.
    #[test]
    fn a_graph_dumps_with_path_ids() {
        let dump = serde_json::to_value(dump_of("/w", &IMPORTS)).unwrap();
        let tree = tree("/w");
        let file = |path: &str| hash(tree.files[leak(format!("/w/{path}"))]);
        let project = json!({"kind": "project"});
        let static_import = |specifier: &str, resolved: &str| json!({"specifier": specifier, "resolved": resolved, "reach": "static"});
        let expected = json!({
            "graph_version": 1,
            "roots": ["project:src/main.ts"],
            "modules": [
                {
                    "id": "linked:../q/l.mjs",
                    "blake3": file("q/l.mjs"),
                    "origin": {"kind": "package", "name": "lnk", "version": "0.1.0", "linked": true},
                    "format": "javascript",
                    "imports": [],
                },
                {
                    "id": "project:node_modules/@sc/pkg/i.mjs",
                    "blake3": file("p/node_modules/@sc/pkg/i.mjs"),
                    "origin": {"kind": "package", "name": "@sc/pkg", "version": "1.0.0", "linked": false},
                    "format": "javascript",
                    "imports": [static_import(
                        "nested",
                        "project:node_modules/@sc/pkg/node_modules/nested/n.mjs",
                    )],
                },
                {
                    "id": "project:node_modules/@sc/pkg/node_modules/nested/n.mjs",
                    "blake3": file("p/node_modules/@sc/pkg/node_modules/nested/n.mjs"),
                    "origin": {"kind": "package", "name": "nested", "version": null, "linked": false},
                    "format": "javascript",
                    "imports": [],
                },
                {
                    "id": "project:self.js",
                    "blake3": file("p/self.js"),
                    "origin": project,
                    "format": "javascript",
                    "imports": [],
                },
                {
                    "id": "project:self.js?q=1#f",
                    "blake3": file("p/self.js"),
                    "origin": project,
                    "format": "javascript",
                    "imports": [],
                },
                {
                    "id": "project:src-x/y.js",
                    "blake3": file("p/src-x/y.js"),
                    "origin": project,
                    "format": "javascript",
                    "imports": [],
                },
                {
                    "id": "project:src/main.ts",
                    "blake3": file("p/src/main.ts"),
                    "origin": project,
                    "format": "typescript",
                    "imports": [
                        static_import("../self.js", "project:self.js"),
                        static_import("../self.js?q=1#f", "project:self.js?q=1#f"),
                        static_import("../src-x/y.js", "project:src-x/y.js"),
                        static_import("@sc/pkg", "project:node_modules/@sc/pkg/i.mjs"),
                        static_import("app/self.js", "project:self.js"),
                        static_import("lnk/l.mjs", "linked:../q/l.mjs"),
                    ],
                },
            ],
            "consulted": [
                {"id": "linked:../q/package.json", "blake3": file("q/package.json"), "role": "exports"},
                {"id": "project:node_modules/@sc/pkg/node_modules/nested/package.json", "blake3": file("p/node_modules/@sc/pkg/node_modules/nested/package.json"), "role": "exports"},
                {"id": "project:node_modules/@sc/pkg/package.json", "blake3": file("p/node_modules/@sc/pkg/package.json"), "role": "exports"},
                {"id": "project:node_modules/@sc/pkg/package.json", "blake3": file("p/node_modules/@sc/pkg/package.json"), "role": "package_scope"},
                {"id": "project:package.json", "blake3": file("p/package.json"), "role": "exports"},
                {"id": "project:package.json", "blake3": file("p/package.json"), "role": "package_scope"},
                {"id": "project:package.json", "blake3": file("p/package.json"), "role": "search_bound"},
            ],
            // each subdirectory's own package.json on each walk up, and
            // its node_modules entry on each search
            "absent": [
                "project:src-x/package.json",
                "project:src/node_modules/@sc/pkg",
                "project:src/node_modules/lnk",
                "project:src/package.json",
            ],
            "local": {},
        });
        // pinned apart from local, checked below
        let mut without_local = dump.clone();
        without_local["local"] = json!({});
        assert_eq!(without_local, expected, "{dump:#}");
        // every id has its absolute path, and only local has any
        let mut ids: Vec<&str> = ["linked:../q/l.mjs", "linked:../q/package.json"].into();
        for path in [
            "node_modules/@sc/pkg/i.mjs",
            "node_modules/@sc/pkg/node_modules/nested/n.mjs",
            "node_modules/@sc/pkg/node_modules/nested/package.json",
            "node_modules/@sc/pkg/package.json",
            "package.json",
            "self.js",
            "src-x/package.json",
            "src-x/y.js",
            "src/main.ts",
            "src/node_modules/@sc/pkg",
            "src/node_modules/lnk",
            "src/package.json",
        ] {
            ids.push(leak(format!("project:{path}")));
        }
        let paths: serde_json::Map<String, Value> = ids
            .iter()
            .map(|id| {
                let path = id.strip_prefix("project:").map_or_else(
                    || id.replace("linked:../", "/w/"),
                    |path| format!("/w/p/{path}"),
                );
                ((*id).to_owned(), Value::String(path))
            })
            .collect();
        assert_eq!(
            dump["local"],
            json!({"project": "/w/p", "box": ["/w/p", "/w/q"], "paths": paths})
        );
    }

    /// The same graph reached in another order, from a project at another
    /// path, dumps the same but for `local`.
    #[test]
    fn a_dump_is_independent_of_order_and_location() {
        let mut reversed = IMPORTS;
        reversed.reverse();
        // the nested package needs its importer resolved first
        reversed.swap(1, 4);
        let here = dump_of("/w", &IMPORTS);
        let moved = dump_of("/x/y", &reversed);
        assert_ne!(here.local, moved.local);
        assert_eq!(
            GraphDump {
                local: here.local.clone(),
                ..moved
            },
            here
        );
    }

    /// A path a resolution passed appearing changes the dump: the
    /// subdirectory's own `package.json`, a type marker the walks pass, is
    /// consulted then, not absent.
    #[test]
    fn an_absent_path_appearing_changes_the_dump() {
        let before = dump_of("/w", &IMPORTS);
        assert!(
            before
                .absent
                .contains(&"project:src/package.json".to_owned())
        );
        let mut tree = tree("/w");
        tree.files
            .insert("/w/p/src/package.json", r#"{"type": "module"}"#);
        let mut reads = Reads::default();
        let (entry, roots, _) = resolver::resolve_entry_with_bound(
            &RecordingTree::new(&tree, &mut reads),
            Path::new("/w/p/src/main.ts"),
            Path::new("/"),
        )
        .unwrap();
        let graph = Graph::new(&entry, roots, Recording::Full).with_reads(reads);
        let after = dump_graph(&graph, &entry.url, &[]).unwrap();
        assert!(
            !after
                .absent
                .contains(&"project:src/package.json".to_owned())
        );
        assert!(
            after
                .consulted
                .iter()
                .any(|file| file.id == "project:src/package.json"
                    && file.role == DumpRole::SearchBound
                    && file.blake3 == hash(r#"{"type": "module"}"#))
        );
    }

    #[test]
    fn ids_are_relative_to_the_project_and_escaped() {
        let p = Path::new("/home/u/p");
        let id = |path: &str| to_path_id(Path::new(path), p).unwrap();
        assert_eq!(id("/home/u/p/src/a.ts"), "project:src/a.ts");
        assert_eq!(
            id("/home/u/p/node_modules/@x/y/i.js"),
            "project:node_modules/@x/y/i.js"
        );
        assert_eq!(id("/home/u/lib/x.js"), "linked:../lib/x.js");
        assert_eq!(id("/opt/z/x.js"), "linked:../../../opt/z/x.js");
        // a sibling whose name extends the project's is outside it
        assert_eq!(id("/home/u/p2/x.js"), "linked:../p2/x.js");
        assert_eq!(id("/home/u/p/a?b#c%d.ts"), "project:a%3Fb%23c%25d.ts");
    }

    /// A consulted file with no read recorded, which only a host bug
    /// makes, has no dump: here the entry's reads aren't passed in.
    #[test]
    fn a_consulted_file_with_no_read_is_unread() {
        let tree = tree("/w");
        let (entry, roots) =
            resolver::resolve_entry(&tree, Path::new("/w/p/src/main.ts"), Path::new("/")).unwrap();
        let graph = Graph::new(&entry, roots, Recording::Full);
        assert_eq!(
            dump_graph(&graph, &entry.url, &[]),
            Err(DumpError::Unread(PathBuf::from("/w/p/package.json")))
        );
    }

    #[test]
    fn a_path_that_is_not_utf8_has_no_id() {
        use std::os::unix::ffi::OsStrExt as _;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/p/\xff.js"));
        assert_eq!(
            to_path_id(path, Path::new("/p")),
            Err(DumpError::NotUtf8(path.to_owned()))
        );
    }
}
