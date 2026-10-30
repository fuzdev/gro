//! Tests for the resolver, split by seam; the fixtures every seam shares live here.

mod containment;
mod exports;
mod identity;
mod packages;
mod specifier;
mod walk;

use std::collections::BTreeMap;

use url::Url;

use crate::resolver::test_tree::MemoryTree;
use crate::resolver::walk::Consulted;
use crate::resolver::{
    Resolution, ResolveError, ResolveErrorKind, Roots, SourceTree, entry_roots, resolve,
};

pub(super) const MODULE: &str = r#"{"type": "module"}"#;

pub(super) fn tree() -> MemoryTree {
    let files = [
        "/p/main.js",
        "/p/a.js",
        "/p/b.mjs",
        "/p/lib/c.js",
        "/p/typed.ts",
        "/p/typed.mts",
        "/p/only_ts.ts",
        "/p/only_mts.mts",
        "/p/common.cts",
        "/p/view.tsx",
        "/p/types.d.ts",
        "/p/types.d.mts",
        "/p/types.d.cts",
        "/p/state.svelte.ts",
        "/p/state.svelte.js",
        "/p/state.svelte.test.ts",
        "/p/data.json",
        "/p/schema.ts",
        "/p/real/linked.js",
        "/real/linked.js",
        "/abs.js",
    ];
    let mut files: BTreeMap<_, _> = files.into_iter().map(|path| (path, "")).collect();
    files.insert("/package.json", MODULE);
    // the project root, which bounds every relative specifier from `/p`
    files.insert("/p/package.json", r#"{"name": "p", "type": "module"}"#);
    MemoryTree {
        files,
        dirs: vec!["/p/dir.js"],
        links: BTreeMap::from([
            ("/p/link.js", "/p/real/linked.js"),
            ("/p/link_out.js", "/real/linked.js"),
            ("/p/link_to_ts.js", "/p/typed.ts"),
        ]),
        loops: Vec::new(),
        world_writable: Vec::new(),
    }
}

pub(super) fn importer() -> Url {
    Url::parse("file:///p/main.js").unwrap()
}

/// The roots of a graph whose entry is `importer`, the entry's own
/// directory alone when the walk for them fails.
pub(super) fn roots(tree: &impl SourceTree, importer: &Url) -> Roots {
    let path = importer.to_file_path().unwrap();
    entry_roots(tree, &path, &mut Consulted::default()).map_or_else(
        |_| Roots::new(path.parent().unwrap().to_owned()),
        |(roots, _)| roots,
    )
}

/// Resolves `specifier` from `importer` in a graph whose entry it is.
pub(super) fn from_entry(
    tree: &impl SourceTree,
    specifier: &str,
    importer: &Url,
) -> Result<Resolution, ResolveError> {
    resolve(tree, specifier, importer, &roots(tree, importer))
}

pub(super) fn ok(specifier: &str) -> String {
    from_entry(&tree(), specifier, &importer())
        .unwrap_or_else(|error| panic!("{error}"))
        .url
        .to_string()
}

pub(super) fn kind(specifier: &str) -> ResolveErrorKind {
    from_entry(&tree(), specifier, &importer())
        .unwrap_err()
        .kind
}

/// A project at `/w/proj` with its packages installed, under `/w`, a
/// workspace root with packages of its own.
pub(super) fn packages() -> MemoryTree {
    let mut files = BTreeMap::from([
        (
            "/w/package.json",
            r#"{"name": "w", "type": "module", "workspaces": ["proj"]}"#,
        ),
        (
            "/w/node_modules/parent_only/package.json",
            r#"{"exports": "./i.js"}"#,
        ),
        (
            "/w/proj/package.json",
            r#"{"name": "proj", "type": "module", "exports": {"./self.js": "./self.js"}}"#,
        ),
        ("/w/proj/main.js", ""),
        ("/w/proj/self.js", ""),
        ("/w/proj/sub/x.js", ""),
        // a stale installed copy of the project, which a self-reference passes over
        (
            "/w/proj/node_modules/proj/package.json",
            r#"{"name": "proj", "exports": {"./self.js": "./self.js"}}"#,
        ),
        (
            "/w/proj/node_modules/t/package.json",
            r#"{
                "name": "t",
                "version": "2.3.4",
                "type": "module",
                "exports": {
                    ".": "./main.js",
                    "./up": "../escape.js",
                    "./bare": "x.js",
                    "./abs": "/abs/x.js",
                    "./https": "https://example.com/x.js",
                    "./dotdot": "./a/../../x.js",
                    "./nm": "./node_modules/y/i.js",
                    "./encoded": "./%2e%2E/x.js",
                    "./encoded_nm": "./%6eode_%4dodules/x.js",
                    "./dir": "./lib",
                    "./dir_slash": "./lib/",
                    "./no_extension": "./no_extension",
                    "./missing": "./missing.js",
                    "./ts": "./only_ts.js",
                    "./empty_segment": "./lib//a.js",
                    "./fall": {"gro": {"browser": "./b.js"}, "default": "./d.js"},
                    "./blocked": {"gro": null, "default": "./d.js"},
                    "./unmatched": {"node": "./d.js", "require": "./d.js"},
                    "./numeric": {"0": "./d.js", "default": "./d.js"},
                    "./two/*/*": "./lib/*/*.js",
                    "./array_invalid_first": ["x.js", "./d.js"],
                    "./array_missing_first": ["./missing.js", "./d.js"],
                    "./array_null_first": [null, "./d.js"],
                    "./array_invalid_after_null": [null, "x.js"],
                    "./array_null_after_invalid": ["x.js", null],
                    "./array_unmatched": [{"node": "./x.js"}],
                    "./array_object": [{"node": "./x.js"}, "./d.js"],
                    "./array_empty": [],
                    "./number": 5,
                    "./order": {"default": "./d.js", "gro": "./b.js"},
                    "./import": {"import": "./b.js", "default": "./d.js"},
                    "./types": {"types": "./d.d.ts", "gro": "./b.js"},
                    "./lib/*": "./lib/*.js",
                    "./lib/a/*": "./lib/a/*.js",
                    "./star/*.js": "./lib/*.js",
                    "./everywhere/*": "./lib/*/*.js",
                    "./cjs": "./cjs.cjs",
                    "./src/*": "./src/*",
                    "./pattern/*": "./lib/*",
                    "./null_pattern/*": null,
                    "./trailing/": "./lib/",
                    "./tab": "./.\t./escape.js",
                    "./query_up": "./..?x",
                    "./fragment_up": "./..#x",
                    "./tabbed/*": "./star/*.js",
                    "./dot_star/*": "./.*/i.js",
                    "./empty_conditions": {},
                    "./array_empty_conditions": [{}],
                    "./nested_unmatched": {"gro": {"browser": "./b.js"}, "node": "./d.js"},
                    "./nested_array_unmatched": [{"import": [{"node": "./d.js"}]}, {"node": "./d.js"}]
                }
            }"#,
        ),
        ("/w/proj/node_modules/escape.js", ""),
        ("/w/proj/node_modules/escape_tab.js", ""),
        ("/w/proj/node_modules/loose.js", ""),
        ("/w/proj/node_modules/t/main.js", ""),
        ("/w/proj/node_modules/t/b.js", ""),
        ("/w/proj/node_modules/t/d.js", ""),
        ("/w/proj/node_modules/t/no_extension", ""),
        ("/w/proj/node_modules/t/only_ts.ts", ""),
        ("/w/proj/node_modules/t/cjs.cjs", ""),
        ("/w/proj/node_modules/t/lib/a.js", ""),
        ("/w/proj/node_modules/t/lib/x.js", ""),
        ("/w/proj/node_modules/t/lib/a/b.js", ""),
        ("/w/proj/node_modules/t/lib/x/x.js", ""),
        ("/w/proj/node_modules/t/src/s.ts", ""),
        (
            "/w/proj/node_modules/sugar_string/package.json",
            r#"{"exports": "./i.mjs"}"#,
        ),
        (
            "/w/proj/node_modules/sugar_array/package.json",
            r#"{"exports": ["x.js", "./i.mjs"]}"#,
        ),
        (
            "/w/proj/node_modules/sugar_conditions/package.json",
            r#"{"exports": {"gro": "./i.mjs", "default": "./d.mjs"}}"#,
        ),
        (
            "/w/proj/node_modules/mixed/package.json",
            r#"{"exports": {".": "./i.mjs", "default": "./i.mjs"}}"#,
        ),
        (
            "/w/proj/node_modules/exports_null/package.json",
            r#"{"main": "./i.mjs", "exports": null}"#,
        ),
        (
            "/w/proj/node_modules/no_exports/package.json",
            r#"{"main": "./i.mjs"}"#,
        ),
        (
            "/w/proj/node_modules/empty_exports/package.json",
            r#"{"exports": {}}"#,
        ),
        ("/w/proj/node_modules/not_json/package.json", "{"),
        ("/w/proj/node_modules/not_object/package.json", "[]"),
        // a directory with no package.json, which shadows the parent's real one
        ("/w/proj/node_modules/no_package/index.js", ""),
        (
            "/w/node_modules/no_package/package.json",
            r#"{"exports": "./i.mjs"}"#,
        ),
        (
            "/w/proj/node_modules/cjs/package.json",
            r#"{"exports": "./i.js"}"#,
        ),
        ("/w/proj/node_modules/cjs/i.js", ""),
        (
            "/w/proj/node_modules/@scope/pkg/package.json",
            r#"{"exports": {".": "./i.mjs", "./x.mjs": "./x.mjs"}}"#,
        ),
        ("/w/proj/node_modules/@scope/pkg/i.mjs", ""),
        ("/w/proj/node_modules/@scope/pkg/x.mjs", ""),
        // linked: a symlink to a checkout outside node_modules, with its own deps
        (
            "/w/linked/package.json",
            r#"{"name": "linked", "version": "0.1.0", "type": "module", "exports": {".": "./i.js"}}"#,
        ),
        ("/w/linked/i.js", ""),
        (
            "/w/linked/node_modules/linked_dep/package.json",
            r#"{"exports": "./i.mjs"}"#,
        ),
        ("/w/linked/node_modules/linked_dep/i.mjs", ""),
        // a nested layout, linked into the project's node_modules, its deps beside it
        (
            "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested/package.json",
            r#"{"version": "1.0.0", "exports": "./i.mjs"}"#,
        ),
        (
            "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested/i.mjs",
            "",
        ),
        (
            "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested_dep/package.json",
            r#"{"exports": "./i.mjs"}"#,
        ),
        (
            "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested_dep/i.mjs",
            "",
        ),
        // hoisted: a package's dependency installed beside it
        (
            "/w/proj/node_modules/hoisted/package.json",
            r#"{"exports": "./i.mjs"}"#,
        ),
        ("/w/proj/node_modules/hoisted/i.mjs", ""),
        (
            "/w/proj/node_modules/needs_hoisted/package.json",
            r#"{"exports": "./i.mjs"}"#,
        ),
        ("/w/proj/node_modules/needs_hoisted/node_modules/.keep", ""),
        ("/w/proj/node_modules/needs_hoisted/i.mjs", ""),
        // a project file under a package.json without `"type": "module"`
        ("/w/proj/no_type/package.json", r#"{"name": "no_type"}"#),
        ("/w/proj/no_type/a.js", ""),
        // type markers, which the search walks past
        ("/w/proj/dist/package.json", r#"{"type": "commonjs"}"#),
        ("/w/proj/dist/esm/package.json", MODULE),
        ("/w/proj/dist/esm/x.js", ""),
        ("/w/proj/scripts/package.json", r#"{"type": "commonjs"}"#),
        ("/w/proj/scripts/x.mjs", ""),
        ("/w/proj/bom/package.json", "\u{FEFF}{\"type\": \"module\"}"),
        ("/w/proj/bom/x.js", ""),
        (
            "/w/proj/side/package.json",
            r#"{"type": "module", "sideEffects": false}"#,
        ),
        ("/w/proj/side/x.js", ""),
        ("/w/proj/empty/package.json", "{}"),
        ("/w/proj/empty/x.mjs", ""),
        // a nameless package.json that isn't a marker, which ends the search
        (
            "/w/proj/deps/package.json",
            r#"{"dependencies": {"t": "^2.0.0"}}"#,
        ),
        ("/w/proj/deps/x.mjs", ""),
        (
            "/w/proj/imports/package.json",
            r##"{"type": "module", "imports": {"#x": "./x.js"}}"##,
        ),
        ("/w/proj/imports/x.js", ""),
        // a "name" or "type" that isn't a string, invalid on every read
        (
            "/w/proj/name_number/package.json",
            r#"{"name": 5, "type": "module"}"#,
        ),
        ("/w/proj/name_number/a.js", ""),
        ("/w/proj/name_number/x.mjs", ""),
        ("/w/proj/type_number/package.json", r#"{"type": 5}"#),
        ("/w/proj/type_number/a.js", ""),
        ("/w/proj/name_bool/package.json", r#"{"name": true}"#),
        ("/w/proj/name_bool/inner/package.json", MODULE),
        ("/w/proj/name_bool/inner/x.mjs", ""),
        (
            "/w/proj/node_modules/name_null/package.json",
            r#"{"name": null, "exports": "./i.mjs"}"#,
        ),
        (
            "/w/proj/node_modules/name_array/package.json",
            r#"{"name": ["x"], "exports": "./i.mjs"}"#,
        ),
        (
            "/w/proj/node_modules/type_array/package.json",
            r#"{"type": ["module"], "exports": "./i.mjs"}"#,
        ),
        // what Node accepts: an empty name, and a "main", "imports", or "version" of any type
        (
            "/w/proj/name_empty/package.json",
            r#"{"name": "", "type": "module"}"#,
        ),
        ("/w/proj/name_empty/a.js", ""),
        (
            "/w/proj/node_modules/loose_fields/package.json",
            r#"{"main": 5, "imports": 5, "version": 5, "exports": "./i.mjs"}"#,
        ),
        // a named nested project, which ends the search
        (
            "/w/proj/inner/package.json",
            r#"{"name": "inner", "type": "module"}"#,
        ),
        ("/w/proj/inner/x.js", ""),
        // a package.json on the search that isn't JSON, under a marker scope
        ("/w/proj/broken/package.json", "{"),
        ("/w/proj/broken/inner/package.json", MODULE),
        ("/w/proj/broken/inner/x.mjs", ""),
        // outside any package.json
        ("/loose/a.js", ""),
        ("/loose/b.mjs", ""),
    ]);
    for file in [
        "/w/node_modules/parent_only/i.js",
        "/w/node_modules/no_package/i.mjs",
        "/w/proj/node_modules/sugar_string/i.mjs",
        "/w/proj/node_modules/sugar_array/i.mjs",
        "/w/proj/node_modules/sugar_conditions/i.mjs",
        "/w/proj/node_modules/proj/self.js",
        "/w/proj/node_modules/name_null/i.mjs",
        "/w/proj/node_modules/name_array/i.mjs",
        "/w/proj/node_modules/type_array/i.mjs",
        "/w/proj/node_modules/loose_fields/i.mjs",
    ] {
        files.insert(file, "");
    }
    MemoryTree {
        files,
        dirs: vec!["/w/proj/node_modules/t/lib/dir.js"],
        links: BTreeMap::from([
            ("/w/proj/node_modules/linked", "/w/linked"),
            (
                "/w/proj/node_modules/nested",
                "/w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested",
            ),
        ]),
        loops: vec!["/w/proj/node_modules/looped"],
        world_writable: Vec::new(),
    }
}

pub(super) fn package(specifier: &str) -> Result<Resolution, ResolveErrorKind> {
    let importer = Url::parse("file:///w/proj/main.js").unwrap();
    from_entry(&packages(), specifier, &importer).map_err(|error| error.kind)
}
