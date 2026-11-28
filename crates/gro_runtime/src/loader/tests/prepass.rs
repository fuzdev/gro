//! Tests for an import graph read before the run and served to its loader
//! (`prepass.rs`): what the walk follows, what it keeps, and what the loader
//! serves, loads from disk, and refuses.

use super::*;
use crate::resolver::ResolveErrorKind;

/// A dynamic import, as a module loads one.
const DYNAMIC: Reached = Reached::Import(Reach::Dynamic);

/// The graph of `entry` among `fixtures`, read as a run reads it, the walk
/// on tsv's stack.
fn read(fixtures: &DiskFixtures, entry: &str) -> ReadGraph {
    read_with(fixtures, entry, &[])
}

/// The graph of `entry` among `fixtures`, read as a run reads it with
/// `--allow-read=` naming `read`, real paths, the walk on tsv's stack.
fn read_with(fixtures: &DiskFixtures, entry: &str, read: &[PathBuf]) -> ReadGraph {
    let (entry, roots) =
        resolver::resolve_entry(&DiskTree, Path::new(entry), fixtures.dir()).unwrap();
    tsv_ts::run_with_stack(|| {
        crate::loader::prepass::read_graph(
            &entry,
            roots,
            Conditions::default(),
            Recording::Off,
            read,
        )
    })
    .unwrap()
}

/// A loader for the graph of `entry` serving `graph`, reading from disk
/// beneath `disk`.
fn served(fixtures: &DiskFixtures, entry: &str, graph: ReadGraph, disk: &[&str]) -> Loader {
    let disk = disk.iter().map(|dir| fixtures.path(dir)).collect();
    fixtures
        .loader_recording(entry, Recording::Full)
        .with_read_graph(graph, disk)
}

/// Resolves `specifier` from `referrer` through the loader's own tree, as
/// a run resolves it, reached as `reached`.
fn resolve(
    loader: &Loader,
    specifier: &str,
    referrer: &Url,
    reached: Reached,
) -> Result<Url, String> {
    loader
        .resolve_on_disk(specifier, referrer.as_str(), reached)
        .map_err(|error| error.to_string())
}

/// Loads `url`, reached as `reach` from `referrer`, returning the code or
/// the error's message.
fn load_from(
    loader: &Loader,
    url: &Url,
    reach: Reach,
    referrer: Option<&Url>,
) -> Result<String, String> {
    let referrer = referrer.map(|referrer| ModuleLoadReferrer {
        specifier: referrer.clone(),
        line_number: 1,
        column_number: 1,
    });
    loader
        .load_reached(url, &RequestedModuleType::None, reach, referrer.as_ref())
        .map(|source| String::from_utf8(source.code.as_bytes().to_vec()).unwrap())
        .map_err(|error| error.to_string())
}

/// The modules a graph read, by their paths relative to `fixtures`, sorted.
fn modules(fixtures: &DiskFixtures, graph: &ReadGraph) -> Vec<String> {
    let mut modules: Vec<String> = graph
        .modules()
        .map(|path| {
            path.strip_prefix(fixtures.dir())
                .unwrap()
                .display()
                .to_string()
        })
        .collect();
    modules.sort();
    modules
}

/// The walk follows static imports, re-exports, an import whose every
/// specifier is a type (the strip keeps it), and `import()` of a string
/// literal, in code that never runs too, and the `package.json` files
/// resolving them; never an import only of types, an `import()` of a name
/// built at run time, or a builtin.
#[test]
fn the_walk_follows_what_literal_imports_load() {
    let fixtures = DiskFixtures::new(
        "prepass_walk",
        &[
            (
                "main.ts",
                "import {a} from './a.ts';\nimport type {T} from './types.ts';\n\
                 import {type S} from './spec.ts';\nexport {b} from './b.mjs';\n\
                 import {fs} from '@fuzdev/gro/fs.ts';\nawait import('./c.ts');\n\
                 await import(['./computed', '.ts'].join(''));\n\
                 if (Math.random() > 2) { await import(`./dead.ts`); }\n",
            ),
            (
                "a.ts",
                "import {e} from './sub/e.js';\nexport const a = e;\n",
            ),
            ("types.ts", "export type T = 1;\n"),
            ("spec.ts", "export type S = 1;\n"),
            ("b.mjs", "export const b = 1;\n"),
            ("c.ts", ""),
            ("computed.ts", ""),
            ("dead.ts", ""),
            ("sub/package.json", r#"{"type": "module"}"#),
            ("sub/e.js", "export const e = 1;\n"),
        ],
    );
    let graph = read(&fixtures, "main.ts");
    assert_eq!(
        modules(&fixtures, &graph),
        [
            "a.ts", "b.mjs", "c.ts", "dead.ts", "main.ts", "spec.ts", "sub/e.js"
        ]
    );
    assert!(
        graph
            .files()
            .any(|file| file == fixtures.path("sub/package.json")),
        "{graph:?}"
    );
}

/// The loader serves a module the walk read as the walk read it, a file
/// rewritten in place since included, and refuses one replaced since; one
/// the walk never read loads only beneath a directory read from disk, and
/// is refused otherwise, static or an `import()`, naming the read flag and
/// recording the need for it.
#[test]
fn the_loader_serves_the_walk_s_reads_and_refuses_the_rest() {
    let fixtures = DiskFixtures::new(
        "prepass_serve",
        &[
            ("main.ts", "import './a.js';\nawait import('./b.js');\n"),
            ("a.js", SOURCE),
            ("b.js", SOURCE),
            ("c.js", SOURCE),
        ],
    );
    let main = fixtures.url("main.ts");
    let loader = served(&fixtures, "main.ts", read(&fixtures, "main.ts"), &[]);
    // rewritten in place: the same file, served the bytes the walk read
    std::fs::write(fixtures.path("a.js"), "export const a = 2;\n").unwrap();
    let a = resolve(&loader, "./a.js", &main, STATIC).unwrap();
    assert_eq!(
        load_from(&loader, &a, Reach::Static, Some(&main)).unwrap(),
        SOURCE
    );
    fixtures.replace("b.js", SOURCE);
    let b = resolve(&loader, "./b.js", &main, DYNAMIC).unwrap();
    assert_eq!(
        load_from(&loader, &b, Reach::Dynamic, Some(&main)).unwrap_err(),
        format!(
            "cannot load {b}: the file at its path isn't the one the run read before \
             it started, so it isn't loaded"
        )
    );
    let c = resolve(&loader, "./c.js", &main, DYNAMIC).unwrap();
    let outside = format!(
        "cannot load {c}: the entry is in no project, so the run loads only the modules its \
         imports name by a string literal, read before it started, and this isn't one. Fix: add \
         --allow-read= naming its directory"
    );
    assert_eq!(
        load_from(&loader, &c, Reach::Dynamic, Some(&main)).unwrap_err(),
        outside
    );
    assert_eq!(
        load_from(&loader, &c, Reach::Static, Some(&main)).unwrap_err(),
        outside
    );
    assert_failed(
        &graph(&loader),
        c.as_str(),
        &FailureReason::Load(LoadFailure::OutsideGraph),
    );
    let refused = loader.take_refused();
    assert_eq!(refused.needed, [crate::NeededGrant::EntryDirImport]);
    assert_eq!(refused.listed.len(), 1, "{refused:?}");

    // beneath a directory read from disk, what the walk didn't read loads
    let loader = served(&fixtures, "main.ts", read(&fixtures, "main.ts"), &["."]);
    let c = resolve(&loader, "./c.js", &main, DYNAMIC).unwrap();
    assert_eq!(
        load_from(&loader, &c, Reach::Dynamic, Some(&main)).unwrap(),
        SOURCE
    );
}

/// A TypeScript module the strip refuses fails its load with the strip's
/// own refusal, as a run reading from disk gives it; a JS module tsv
/// can't parse fails with its own, loading from disk beneath a directory
/// read from disk.
#[test]
fn a_module_the_walk_couldn_t_read_fails_as_its_load_would() {
    let deep = format!(
        "export const d = {}1{};\n",
        "[".repeat(300),
        "]".repeat(300)
    );
    let fixtures = DiskFixtures::new(
        "prepass_unread",
        &[
            (
                "main.ts",
                "await import('./e.ts');\nawait import('./deep.js');\n",
            ),
            ("e.ts", "enum E { A }\n"),
            ("deep.js", &deep),
        ],
    );
    let main = fixtures.url("main.ts");
    let loader = served(&fixtures, "main.ts", read(&fixtures, "main.ts"), &[]);
    let e = resolve(&loader, "./e.ts", &main, DYNAMIC).unwrap();
    let plain = fixtures.loader("main.ts");
    let e_plain = plain
        .resolve_reached(&DiskTree, "./e.ts", main.as_str(), DYNAMIC)
        .unwrap();
    assert_eq!(
        load_from(&loader, &e, Reach::Dynamic, None).unwrap_err(),
        load_from(&plain, &e_plain, Reach::Dynamic, None).unwrap_err()
    );
    let deep_url = resolve(&loader, "./deep.js", &main, DYNAMIC).unwrap();
    let error = load_from(&loader, &deep_url, Reach::Dynamic, None).unwrap_err();
    assert!(
        error.starts_with(
            "cannot parse the JS for its imports: Nesting exceeds tsv's depth limit of 256 \
             wire levels. Fix: add --allow-read= naming the entry's directory, which loads it \
             and what it imports from disk without that parse\n    at "
        ),
        "{error}"
    );
    let loader = served(&fixtures, "main.ts", read(&fixtures, "main.ts"), &["."]);
    let deep_url = resolve(&loader, "./deep.js", &main, DYNAMIC).unwrap();
    assert_eq!(
        load_from(&loader, &deep_url, Reach::Dynamic, None).unwrap(),
        deep
    );
}

/// A `package.json` the walk didn't read is refused to a resolution,
/// naming the read flag, unless a directory read from disk holds it.
#[test]
fn a_package_json_the_walk_didn_t_read_is_refused() {
    let fixtures = DiskFixtures::new(
        "prepass_files",
        &[
            ("main.ts", ""),
            ("sub/package.json", r#"{"type": "module"}"#),
            ("sub/x.js", ""),
        ],
    );
    let main = fixtures.url("main.ts");
    let loader = served(&fixtures, "main.ts", read(&fixtures, "main.ts"), &[]);
    let error = resolve(&loader, "./sub/x.js", &main, DYNAMIC).unwrap_err();
    let json = fixtures.path("sub/package.json");
    assert!(
        error.ends_with(&format!(
            "cannot read {}: the entry is in no project, so the run reads only the package.json \
             files its import graph's resolution read before it started, and this isn't one. \
             Fix: add --allow-read= naming its directory",
            json.display()
        )),
        "{error}"
    );
    assert_failed(
        &graph(&loader),
        "./sub/x.js",
        &FailureReason::Resolve(ResolveErrorKind::ReadOutsideGraph(json)),
    );
    assert_eq!(
        loader.take_refused().needed,
        [crate::NeededGrant::EntryDirImport]
    );
    let loader = served(&fixtures, "main.ts", read(&fixtures, "main.ts"), &["sub"]);
    resolve(&loader, "./sub/x.js", &main, DYNAMIC).unwrap();
}

/// The walk reads nothing past the project's directory but what
/// `--allow-read=` names: a package a project nested there links from
/// outside, by its `node_modules` or by the package's own directory, is
/// refused as the box refuses a read, which the loader gives again, needing
/// no flag for the project's directory, while a package installed beneath
/// that directory loads, and one in a directory `--allow-read=` names is
/// read.
#[test]
fn the_walk_reads_nothing_past_the_project_s_directory() {
    let fixtures = DiskFixtures::new(
        "prepass_reach",
        &[
            (
                "main.ts",
                "import './nested/a.ts';\nimport './linked/b.ts';\nimport './inside/d.ts';\n",
            ),
            ("nested/package.json", r#"{"name": "nested"}"#),
            ("nested/a.ts", "import 'pkg';\n"),
            ("linked/package.json", r#"{"name": "linked"}"#),
            ("linked/b.ts", "import 'pkg';\n"),
            ("inside/package.json", r#"{"name": "inside"}"#),
            ("inside/d.ts", "import 'pkg';\n"),
            (
                "inside/node_modules/pkg/package.json",
                r#"{"name": "pkg", "type": "module", "exports": "./i.js"}"#,
            ),
            ("inside/node_modules/pkg/i.js", "export const i = 1;\n"),
        ],
    );
    let elsewhere = DiskFixtures::new(
        "prepass_reach_elsewhere",
        &[
            (
                "node_modules/pkg/package.json",
                r#"{"name": "pkg", "type": "module", "exports": "./i.js"}"#,
            ),
            ("node_modules/pkg/i.js", "export const i = 2;\n"),
        ],
    );
    std::fs::create_dir(fixtures.path("nested/node_modules")).unwrap();
    std::os::unix::fs::symlink(
        elsewhere.path("node_modules/pkg"),
        fixtures.path("nested/node_modules/pkg"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        elsewhere.path("node_modules"),
        fixtures.path("linked/node_modules"),
    )
    .unwrap();
    let graph = read(&fixtures, "main.ts");
    assert!(
        graph.files().all(|file| !file.starts_with(elsewhere.dir())),
        "{graph:?}"
    );
    assert!(
        graph
            .modules()
            .all(|file| !file.starts_with(elsewhere.dir())),
        "{graph:?}"
    );
    assert!(
        graph
            .files()
            .any(|file| file == fixtures.path("inside/node_modules/pkg/package.json")),
        "{graph:?}"
    );
    let loader = served(&fixtures, "main.ts", graph, &[]);
    let main = fixtures.url("main.ts");
    let json = elsewhere.path("node_modules/pkg/package.json");
    for nested in ["nested/a.ts", "linked/b.ts"] {
        let importer = resolve(&loader, &format!("./{nested}"), &main, STATIC).unwrap();
        let error = resolve(&loader, "pkg", &importer, STATIC).unwrap_err();
        assert!(
            error.ends_with(&format!(
                "cannot read {}: it lies outside the box's roots",
                json.display()
            )),
            "{nested}: {error}"
        );
    }
    let inside = resolve(&loader, "./inside/d.ts", &main, STATIC).unwrap();
    let i = resolve(&loader, "pkg", &inside, STATIC).unwrap();
    assert_eq!(
        load_from(&loader, &i, Reach::Static, Some(&inside)).unwrap(),
        "export const i = 1;\n"
    );
    assert!(loader.take_refused().needed.is_empty());

    // a directory --allow-read= names is within the walk's reach
    let read = vec![elsewhere.dir().to_owned()];
    let graph = read_with(&fixtures, "main.ts", &read);
    assert!(graph.files().any(|file| file == json), "{graph:?}");
    let loader = fixtures
        .loader_recording("main.ts", Recording::Full)
        .with_read_graph(graph, read);
    let nested = resolve(&loader, "./nested/a.ts", &main, STATIC).unwrap();
    let pkg = resolve(&loader, "pkg", &nested, STATIC).unwrap();
    assert_eq!(
        load_from(&loader, &pkg, Reach::Static, Some(&nested)).unwrap(),
        "export const i = 2;\n"
    );
}
