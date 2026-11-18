//! `#` specifiers through the nearest `package.json`'s `"imports"`: matching,
//! conditions, targets, containment, origins, and the refusals.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use url::Url;

use super::{MODULE, from_entry, roots};
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{
    ConsultedFile, MapField, Origin, PackageField, PackageFields, Resolution, ResolveError,
    ResolveErrorKind, TargetFault,
};

const PROJECT_IMPORTS: &str = r##"{
    "name": "p",
    "type": "module",
    "imports": {
        "#lib/*": "./src/lib/*",
        "#lib/special/*": "./src/routes/*",
        "#exact": "./src/lib/a.js",
        "#/x": "./src/lib/a.js",
        "#star/*/end.js": "./src/lib/*.js",
        "#cond": {"node": "./src/lib/node.js", "gro": "./src/lib/gro.js", "default": "./src/lib/default.js"},
        "#nested": {"browser": "./src/lib/b.js", "import": {"svelte": "./src/lib/s.js", "default": "./src/lib/import.js"}},
        "#unmatched": {"node": "./src/lib/node.js", "require": "./src/lib/r.js"},
        "#no_conditions": {},
        "#null": null,
        "#arr": ["../bad.js", "./src/lib/a.js"],
        "#arr_null": ["../bad.js", null],
        "#arr_bad": ["/bad.js"],
        "#dep": "dep",
        "#dep/*": "dep/*",
        "#node": "node:fs",
        "#up": "../outside/x.js",
        "#abs": "/w/outside/x.js",
        "#url": "file:///w/outside/x.js",
        "#nm": "./node_modules/dep/index.js",
        "#dotdot": "./src/../src/lib/a.js",
        "#encoded": "./src/%2e%2e/src/lib/a.js",
        "#empty_segment": "./src//lib/a.js",
        "#trailing": "./src/lib/",
        "#bool": true,
        "#numeric": {"0": "./src/lib/a.js"},
        "#link_out": "./src/link_out.js",
        "#missing": "./src/lib/missing.js",
        "#ts/*": "./src/ts/*",
        "#ts_exact": "./src/ts/only.js",
        "#ts_dir/*": "./src/ts/*/index.js",
        "#ts_js/*.js": "./src/ts/*",
        "#loop": "./src/loop.js",
        "#bare/*": "*",
        "#empty": "",
        "#dot": ".",
        "#parent": "..",
        "#dot_x": ".x",
        "#hash": "#lib/a.js",
        "#arr_hash": ["#lib/a.js", "./src/lib/a.js"],
        "#arr_dot_x/*": [".*", "./src/lib/a.js"],
        "#arr_empty_seg": ["./src//lib/a.js", "./src/lib/b.js"]
    }
}"##;

/// A project at `/w/p` whose `package.json` maps `#` specifiers, a
/// subdirectory hiding them behind a type marker, one with its own
/// `"imports"`, an installed package with its own, and a file outside.
fn tree() -> MemoryTree {
    let files = [
        "/w/p/src/main.ts",
        "/w/p/src/lib/a.js",
        "/w/p/src/lib/gro.js",
        "/w/p/src/lib/default.js",
        "/w/p/src/lib/import.js",
        "/w/p/src/lib/foo.js",
        "/w/p/src/lib/x/b.ts",
        "/w/p/src/routes/r.js",
        "/w/p/src/ts/only.ts",
        "/w/p/src/ts/dir/index.ts",
        "/w/p/sub/m.js",
        "/w/p/inner/m.js",
        "/w/p/node_modules/dep/index.js",
        "/w/p/node_modules/dep/lib/i.js",
        "/w/outside/x.js",
        "/w/loose/m.mjs",
        "/shared/m.mjs",
    ];
    let mut files: BTreeMap<_, _> = files.into_iter().map(|path| (path, "")).collect();
    files.insert("/w/p/package.json", PROJECT_IMPORTS);
    files.insert("/w/p/sub/package.json", MODULE);
    files.insert(
        "/w/p/inner/package.json",
        r##"{"type": "module", "imports": {"#up_link": "./link.js", "#here": "./m.js"}}"##,
    );
    files.insert(
        "/w/p/node_modules/dep/package.json",
        r##"{"name": "dep", "type": "module", "exports": "./index.js",
            "imports": {"#internal": "./lib/i.js", "#project": "./link.js"}}"##,
    );
    files.insert("/shared/package.json", PROJECT_IMPORTS);
    MemoryTree {
        files,
        links: BTreeMap::from([
            ("/w/p/src/link_out.js", "/w/outside/x.js"),
            ("/w/p/inner/link.js", "/w/p/src/lib/a.js"),
            ("/w/p/node_modules/dep/link.js", "/w/p/src/lib/a.js"),
        ]),
        loops: vec!["/w/p/src/loop.js"],
        world_writable: vec!["/shared"],
        ..MemoryTree::default()
    }
}

fn url(path: &str) -> Url {
    Url::from_file_path(path).unwrap()
}

/// Resolves `specifier` from the project's `src/main.ts`.
fn resolve(specifier: &str) -> Result<Resolution, ResolveError> {
    from_entry(&tree(), specifier, &url("/w/p/src/main.ts"))
}

fn ok(specifier: &str) -> String {
    resolve(specifier)
        .unwrap_or_else(|error| panic!("{error}"))
        .url
        .to_string()
}

fn kind(specifier: &str) -> ResolveErrorKind {
    resolve(specifier).unwrap_err().kind
}

/// Exact keys and single-`*` patterns, the most specific first, as Node
/// matches them, the importer's query and fragment kept.
#[test]
fn keys_and_patterns_match_as_in_node() {
    assert_eq!(ok("#exact"), "file:///w/p/src/lib/a.js");
    assert_eq!(ok("#/x"), "file:///w/p/src/lib/a.js");
    assert_eq!(ok("#lib/a.js"), "file:///w/p/src/lib/a.js");
    assert_eq!(ok("#lib/x/b.ts"), "file:///w/p/src/lib/x/b.ts");
    assert_eq!(ok("#lib/special/r.js"), "file:///w/p/src/routes/r.js");
    assert_eq!(ok("#star/foo/end.js"), "file:///w/p/src/lib/foo.js");
    assert_eq!(ok("#lib/a.js?q=1#f"), "file:///w/p/src/lib/a.js?q=1#f");
    // a key that isn't one, and a pattern needing a nonempty match
    for specifier in ["#nope", "#lib", "#star/end.js"] {
        assert!(
            matches!(
                kind(specifier),
                ResolveErrorKind::ImportNotDefined {
                    defined: Some(_),
                    ..
                }
            ),
            "{specifier}"
        );
    }
}

/// The `"exports"` conditions, in the package's key order, arrays as
/// fallbacks past an invalid target, and `null` or no conditions as not
/// defined.
#[test]
fn conditions_and_fallbacks() {
    assert_eq!(ok("#cond"), "file:///w/p/src/lib/gro.js");
    assert_eq!(ok("#nested"), "file:///w/p/src/lib/import.js");
    assert_eq!(ok("#arr"), "file:///w/p/src/lib/a.js");
    assert_eq!(
        kind("#unmatched"),
        ResolveErrorKind::NoMatchingCondition {
            field: MapField::Imports,
            subpath: "#unmatched".to_owned(),
            unmatched: vec!["node".to_owned(), "require".to_owned()],
        }
    );
    for specifier in ["#null", "#arr_null", "#no_conditions"] {
        assert!(
            matches!(kind(specifier), ResolveErrorKind::ImportNotDefined { .. }),
            "{specifier}"
        );
    }
    assert!(matches!(
        kind("#arr_bad"),
        ResolveErrorKind::InvalidTarget {
            field: MapField::Imports,
            reason: TargetFault::NotRelative,
            ..
        }
    ));
    assert!(matches!(
        kind("#numeric"),
        ResolveErrorKind::InvalidPackageJson { reason, .. }
            if reason == "\"imports\" cannot have numeric keys"
    ));
}

/// A target is a `./` path with no `.`, `..`, `node_modules`, or empty
/// segment, encoded or not; `../`, `/`, and URL targets are invalid, as in
/// Node, and so is one naming no package or a `#` specifier, while one
/// naming a package or builtin, a pattern's match substituted, is refused
/// with its name.
#[test]
fn targets_are_paths_in_the_package_json_directory() {
    let invalid = |specifier: &str| match kind(specifier) {
        ResolveErrorKind::InvalidTarget {
            field: MapField::Imports,
            reason,
            ..
        } => reason,
        kind => panic!("{specifier}: {kind:?}"),
    };
    for specifier in ["#up", "#abs", "#url", "#node"] {
        assert_eq!(invalid(specifier), TargetFault::NotRelative, "{specifier}");
    }
    // Node resolves these as bare specifiers and refuses them there, so an
    // array never falls back past one
    for specifier in [
        "#empty",
        "#dot",
        "#parent",
        "#dot_x",
        "#hash",
        "#bare/.x",
        "#bare/#x",
        "#bare/@x",
        "#arr_hash",
        "#arr_dot_x/x",
    ] {
        assert_eq!(invalid(specifier), TargetFault::NamesNothing, "{specifier}");
    }
    for specifier in ["#nm", "#dotdot", "#encoded"] {
        assert_eq!(invalid(specifier), TargetFault::Segment, "{specifier}");
    }
    for specifier in ["#empty_segment", "#trailing", "#arr_empty_seg"] {
        assert_eq!(invalid(specifier), TargetFault::EmptySegment, "{specifier}");
    }
    assert_eq!(invalid("#bool"), TargetFault::NotATarget);
    assert_eq!(
        kind("#dep"),
        ResolveErrorKind::ImportsPackage {
            key: "#dep".to_owned(),
            target: "\"dep\"".into(),
            specifier: "dep".into(),
        }
    );
    assert_eq!(
        kind("#dep/x.js"),
        ResolveErrorKind::ImportsPackage {
            key: "#dep/*".to_owned(),
            target: "\"dep/*\"".into(),
            specifier: "dep/x.js".into(),
        }
    );
    assert_eq!(
        kind("#bare/x"),
        ResolveErrorKind::ImportsPackage {
            key: "#bare/*".to_owned(),
            target: "\"*\"".into(),
            specifier: "x".into(),
        }
    );
    // a pattern's match can't add what the target can't have
    for specifier in ["#lib/x/../a.js", "#lib/%2e%2e/a.js", "#lib/x//b.ts"] {
        assert!(
            matches!(
                kind(specifier),
                ResolveErrorKind::InvalidSubpath {
                    field: MapField::Imports,
                    ..
                }
            ),
            "{specifier}"
        );
    }
}

/// The target's real path stays inside the directory of the `package.json`
/// that maps it and the importer's root, and a missing file is named with
/// the specifier that would find the `.ts` sibling where its text ends the
/// path.
#[test]
fn a_target_is_contained_and_found() {
    let target = |specifier: &str| match kind(specifier) {
        ResolveErrorKind::ImportTarget {
            package_json,
            path,
            error,
        } => {
            assert_eq!(package_json, Path::new("/w/p/package.json"));
            (path, *error)
        }
        kind => panic!("{specifier}: {kind:?}"),
    };
    assert_eq!(
        target("#link_out"),
        (
            PathBuf::from("/w/p/src/link_out.js"),
            ResolveErrorKind::OutsideRoot {
                path: PathBuf::from("/w/outside/x.js"),
                root: PathBuf::from("/w/p"),
                package: None,
            }
        )
    );
    assert_eq!(
        target("#ts/only.js").1,
        ResolveErrorKind::NotFound {
            ts_sibling: Some("#ts/only.ts".to_owned())
        }
    );
    assert_eq!(
        target("#ts/only").1,
        ResolveErrorKind::NoExtension {
            suggestion: Some("#ts/only.ts".to_owned())
        }
    );
    // the specifier's text doesn't end the path, so the path is named
    assert_eq!(
        target("#ts_exact").1,
        ResolveErrorKind::NotFound {
            ts_sibling: Some("/w/p/src/ts/only.ts".to_owned())
        }
    );
    // nor where the key goes on past its `*`
    assert_eq!(
        target("#ts_js/only.js").1,
        ResolveErrorKind::NoExtension { suggestion: None }
    );
    assert_eq!(
        target("#ts_dir/dir").1,
        ResolveErrorKind::NotFound {
            ts_sibling: Some("/w/p/src/ts/dir/index.ts".to_owned())
        }
    );
    assert_eq!(
        target("#missing").1,
        ResolveErrorKind::NotFound { ts_sibling: None }
    );
    // the tree's failure is the tree's, not the target's
    assert_eq!(
        kind("#loop"),
        ResolveErrorKind::SymlinkLoop(PathBuf::from("/w/p/src/loop.js"))
    );
    // a subdirectory's `package.json` holds its targets inside it, though
    // the project's root, the importer's here, holds the file too
    let tree = tree();
    let roots = roots(&tree, &url("/w/p/src/main.ts"));
    let inner = super::resolve(&tree, "#up_link", &url("/w/p/inner/m.js"), &roots).unwrap_err();
    assert_eq!(
        inner.kind,
        ResolveErrorKind::ImportTarget {
            package_json: PathBuf::from("/w/p/inner/package.json"),
            path: PathBuf::from("/w/p/inner/link.js"),
            error: Box::new(ResolveErrorKind::OutsideRoot {
                path: PathBuf::from("/w/p/src/lib/a.js"),
                root: PathBuf::from("/w/p/inner"),
                package: None,
            }),
        }
    );
    let here = super::resolve(&tree, "#here", &url("/w/p/inner/m.js"), &roots).unwrap();
    assert_eq!(here.url.as_str(), "file:///w/p/inner/m.js");
    assert_eq!(here.root.dir, Path::new("/w/p"));
}

/// The nearest `package.json` is the one read, as in Node, so a type
/// marker in a subdirectory hides the project's `"imports"`; with none
/// above, or a world-writable directory first, nothing resolves.
#[test]
fn the_nearest_package_json_maps_them() {
    assert_eq!(
        from_entry(&tree(), "#exact", &url("/w/p/sub/m.js"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::ImportNotDefined {
            package_json: PathBuf::from("/w/p/sub/package.json"),
            defined: None,
        }
    );
    assert_eq!(
        from_entry(&tree(), "#exact", &url("/w/loose/m.mjs"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::NoImportScope { stopped_at: None }
    );
    assert_eq!(
        from_entry(&tree(), "#exact", &url("/shared/m.mjs"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::NoImportScope {
            stopped_at: Some(PathBuf::from("/shared"))
        }
    );
}

/// Inside an installed package its own `"imports"` resolve, keeping its
/// origin and staying inside it; the project's are never read.
#[test]
fn a_package_resolves_its_own_imports() {
    let importer = url("/w/p/node_modules/dep/index.js");
    let resolution = from_entry(&tree(), "#internal", &importer).unwrap();
    assert_eq!(
        resolution.url.as_str(),
        "file:///w/p/node_modules/dep/lib/i.js"
    );
    assert_eq!(
        resolution.root.origin,
        Origin::Package {
            name: "dep".to_owned(),
            version: None,
            linked: false,
        }
    );
    assert_eq!(
        resolution.files_consulted,
        [ConsultedFile {
            path: PathBuf::from("/w/p/node_modules/dep/package.json"),
            fields: PackageFields::of([PackageField::Imports, PackageField::Type]),
        }]
    );
    // a link into the project is outside the package
    assert!(matches!(
        from_entry(&tree(), "#project", &importer).unwrap_err().kind,
        ResolveErrorKind::ImportTarget { error, .. }
            if matches!(*error, ResolveErrorKind::OutsideRoot { package: Some(_), .. })
    ));
    assert!(matches!(
        from_entry(&tree(), "#exact", &importer).unwrap_err().kind,
        ResolveErrorKind::ImportNotDefined { package_json, .. }
            if package_json == Path::new("/w/p/node_modules/dep/package.json")
    ));
}

/// The project's `#` import keeps the project's origin, and records the
/// `"imports"` read beside the module's `"type"`.
#[test]
fn a_project_import_keeps_its_origin_and_records_its_read() {
    let resolution = resolve("#exact").unwrap();
    assert_eq!(resolution.root.origin, Origin::Project);
    assert_eq!(resolution.root.dir, Path::new("/w/p"));
    assert_eq!(
        resolution.files_consulted,
        [ConsultedFile {
            path: PathBuf::from("/w/p/package.json"),
            fields: PackageFields::of([PackageField::Imports, PackageField::Type]),
        }]
    );
}

/// Each refusal names what failed and the fix.
#[test]
fn the_messages_name_the_fix() {
    let message = |specifier: &str| resolve(specifier).unwrap_err().to_string();
    assert_eq!(
        message("#"),
        "cannot resolve '#' from file:///w/p/src/main.ts: it is not a valid # specifier: '#' \
         alone and a specifier ending in '/' map to nothing, as in Node. Fix: import a module \
         the package.json's \"imports\" map, like '#lib/x.ts'"
    );
    assert_eq!(
        message("#dep"),
        "cannot resolve '#dep' from file:///w/p/src/main.ts: the \"imports\" target \"dep\" for \
         '#dep' names a package or a builtin module, and a # specifier maps only to a file in \
         the directory of its package.json. Fix: import 'dep' by its name"
    );
    assert_eq!(
        message("#empty_segment"),
        "cannot resolve '#empty_segment' from file:///w/p/src/main.ts: the \"imports\" target \
         \"./src//lib/a.js\" for '#empty_segment' is invalid: a target can't have an empty \
         segment ('//' or a trailing '/'). Fix: correct the target, or for an installed \
         package, use a version whose \"imports\" are valid"
    );
    for (specifier, target) in [("#hash", "\"#lib/a.js\""), ("#empty", "\"\"")] {
        assert_eq!(
            message(specifier),
            format!(
                "cannot resolve '{specifier}' from file:///w/p/src/main.ts: the \"imports\" \
                 target {target} for '{specifier}' is invalid: a target must start with './'. \
                 Fix: correct the target, or for an installed package, use a version whose \
                 \"imports\" are valid"
            )
        );
    }
    assert_eq!(
        message("#ts/only.js"),
        "cannot resolve '#ts/only.js' from file:///w/p/src/main.ts: the \"imports\" of \
         /w/p/package.json map it to /w/p/src/ts/only.js: no such file, but '#ts/only.ts' \
         exists; write the real extension"
    );
    assert_eq!(
        message("#link_out"),
        "cannot resolve '#link_out' from file:///w/p/src/main.ts: the \"imports\" of \
         /w/p/package.json map it to /w/p/src/link_out.js, which resolves to /w/outside/x.js, \
         outside /w/p, and a # specifier's target stays inside the directory of the \
         package.json that maps it and inside its importer's root. Fix: map it to a file \
         inside /w/p, or import the file by name if it is a package's"
    );
    assert_eq!(
        message("#nope"),
        "cannot resolve '#nope' from file:///w/p/src/main.ts: it is not mapped by the \
         \"imports\" of /w/p/package.json (imports: #lib/*, #lib/special/*, #exact, #/x, \
         #star/*/end.js, #cond, #nested, #unmatched, #no_conditions, #null, #arr, #arr_null, \
         #arr_bad, #dep, #dep/*, #node, #up, #abs, #url, #nm, #dotdot, #encoded, \
         #empty_segment, #trailing, #bool, #numeric, #link_out, #missing, #ts/*, #ts_exact, \
         #ts_dir/*, #ts_js/*.js, #loop, #bare/*, #empty, #dot, #parent, #dot_x, #hash, \
         #arr_hash, #arr_dot_x/*, #arr_empty_seg). Fix: import a specifier it maps, or add one to its \
         \"imports\""
    );
    let message = |specifier: &str, importer: &str| {
        from_entry(&tree(), specifier, &url(importer))
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        message("#exact", "/w/p/sub/m.js"),
        "cannot resolve '#exact' from file:///w/p/sub/m.js: a # specifier resolves through the \
         \"imports\" of the nearest package.json above its importer, /w/p/sub/package.json, \
         which has none, and one in a subdirectory hides those above it, as in Node. Fix: add \
         the mapping to its \"imports\", or import the file by a relative path"
    );
    assert_eq!(
        message("#exact", "/shared/m.mjs"),
        "cannot resolve '#exact' from file:///shared/m.mjs: a # specifier resolves through the \
         \"imports\" of the nearest package.json above its importer, and the search for one \
         stopped at /shared, which is world-writable, so nothing in it or above it is read. \
         Fix: add a package.json with \"imports\" above it, or import the file by a relative \
         path"
    );
}

/// A target naming a package is refused wherever an array reaches it, which
/// Node resolves and never falls back past, and a pattern's match can't
/// spell a `..` or `node_modules` segment with the target around it.
#[test]
fn a_package_never_falls_back_and_a_match_spells_no_segment() {
    let mut files: BTreeMap<_, _> = ["/w/p/src/main.ts", "/w/p/x.js", "/w/p/store/x.js"]
        .into_iter()
        .map(|path| (path, ""))
        .collect();
    files.insert(
        "/w/p/package.json",
        r##"{"type": "module", "imports": {"#first": ["dep", "./x.js"],
            "#after": ["../bad.js", "dep", "./x.js"], "#pct/*": "./src/%2*",
            "#nm/*": "./node_*"}}"##,
    );
    // a `node_modules` whose real path has no such segment
    let tree = MemoryTree {
        files,
        links: BTreeMap::from([("/w/p/node_modules", "/w/p/store")]),
        ..MemoryTree::default()
    };
    let main = url("/w/p/src/main.ts");
    let resolve = |specifier| from_entry(&tree, specifier, &main);
    for specifier in ["#first", "#after"] {
        assert!(
            matches!(
                resolve(specifier).unwrap_err().kind,
                ResolveErrorKind::ImportsPackage { .. }
            ),
            "{specifier}"
        );
    }
    for specifier in [
        "#pct/e%2e/x.js",
        "#pct/e%2e/%2e%2e/outside.js",
        "#nm/modules/x.js",
    ] {
        assert!(
            matches!(
                resolve(specifier).unwrap_err().kind,
                ResolveErrorKind::InvalidSubpath {
                    field: MapField::Imports,
                    ..
                }
            ),
            "{specifier}"
        );
    }
    assert_eq!(
        resolve("#nm/modules/x.js").unwrap_err().to_string(),
        "cannot resolve '#nm/modules/x.js' from file:///w/p/src/main.ts: the part of it the \
         \"imports\" pattern '#nm/*' matches, 'modules/x.js', has a '.', '..', or \
         'node_modules' segment or spells one with the target around it, percent-encoded or \
         not, makes an empty segment in the target, or takes the target out of the directory \
         of its package.json, and a pattern never substitutes those. Fix: import the module by \
         its plain path"
    );
}
