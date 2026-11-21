//! Gro's embedded library: a specifier of Gro's package resolving to it
//! from any importer, ahead of any package, and an embedded module's own
//! specifiers resolving by text inside it, row first, or to a package from
//! the project's root.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

use url::Url;

use super::builtins::{Watched, shadowed};
use super::{TEST_EMBED_MODULES, resolve_any, resolve_gro, roots, test_library};
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{
    BUILTINS, GroModuleFault, Origin, PeerFault, Reserved, ResolveError, ResolveErrorKind,
    Resolved, Root, Roots, find_builtin, needs_gro_library, resolve_from_embedded,
};

/// The project of [`shadowed`], named `@fuzdev/gro` and exporting every
/// `.ts` file of its own, with an installed `@fuzdev/gro` exporting every
/// one of its own too, each holding the files the test embed does, and a
/// package `dep` admitted beside it.
fn planted() -> (Watched, Roots) {
    let mut files = shadowed().files;
    let exports = r#"{"name": "@fuzdev/gro", "type": "module", "exports": {".": "./index.ts", "./*.ts": "./*.ts", "./*.js": "./*.ts"}}"#;
    files.insert("/p/package.json", exports);
    files.insert("/p/node_modules/@fuzdev/gro/package.json", exports);
    for (path, _) in TEST_EMBED_MODULES {
        let leak = |prefix: &str| -> &'static str { Box::leak(format!("{prefix}{path}").into()) };
        files.insert(leak("/p/"), "export const planted = true;\n");
        files.insert(
            leak("/p/node_modules/@fuzdev/gro/"),
            "export const planted = true;\n",
        );
    }
    let tree = MemoryTree {
        files,
        ..MemoryTree::default()
    };
    let main = Url::parse("file:///p/main.js").unwrap();
    let mut roots = roots(&tree, &main);
    roots.admit(Root {
        dir: PathBuf::from("/p/node_modules/dep"),
        origin: Origin::Package {
            name: "dep".to_owned(),
            version: None,
            linked: false,
        },
    });
    let watched = Watched {
        tree,
        calls: RefCell::default(),
    };
    (watched, roots)
}

/// The importers of [`planted`]: the project's entry and the package's.
const IMPORTERS: [&str; 2] = ["file:///p/main.js", "file:///p/node_modules/dep/index.js"];

/// The embedded module a resolution named, or panics.
fn embedded_path(resolved: Result<Resolved, ResolveError>) -> &'static str {
    match resolved {
        Ok(Resolved::Embedded(module)) => module.path,
        resolved => panic!("not an embedded module: {resolved:?}"),
    }
}

/// A specifier of Gro's package is Gro's library's module, from the
/// project and from a package alike, never the project's self-reference of
/// that name or the installed package, and nothing is read for it;
/// `@fuzdev/gro` alone is its `index.ts`. Its URL is its path in the
/// `gro:` scheme.
#[test]
fn a_specifier_of_gro_s_package_is_gro_s_library_reading_nothing() {
    let (watched, roots) = planted();
    for importer in IMPORTERS {
        let importer = Url::parse(importer).unwrap();
        for (specifier, path) in [
            ("@fuzdev/gro/task_args.ts", "task_args.ts"),
            ("@fuzdev/gro/fs_paths.ts", "fs_paths.ts"),
            ("@fuzdev/gro/dir/a.ts", "dir/a.ts"),
            ("@fuzdev/gro", "index.ts"),
        ] {
            let resolved = resolve_gro(&watched, specifier, &importer, &roots);
            let Ok(Resolved::Embedded(module)) = resolved else {
                panic!("{specifier} from {importer}: {resolved:?}");
            };
            assert_eq!(module.path, path);
            let code = TEST_EMBED_MODULES.iter().find(|(held, _)| *held == path);
            assert_eq!(Some(module.code), code.map(|(_, code)| *code));
            assert_eq!(module.url().unwrap().as_str(), format!("gro:lib/{path}"));
        }
        // the table first, as for every importer
        assert_eq!(
            resolve_gro(&watched, "@fuzdev/gro/args.ts", &importer, &roots),
            Ok(Resolved::Builtin(&BUILTINS[0]))
        );
    }
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
}

/// What Gro's library doesn't serve is refused by kind, naming the module,
/// reading nothing, and never falls through to a package: an internal
/// module, though the library holds it; one left out to run on Node; a
/// `.js` spelling of a module it holds; and a path it has no module at.
#[test]
fn what_gro_s_library_doesn_t_serve_is_refused_by_kind() {
    let (watched, roots) = planted();
    let cases = [
        (
            "@fuzdev/gro/internal/posix_path.ts",
            "internal/posix_path.ts",
            GroModuleFault::Internal,
        ),
        (
            "@fuzdev/gro/internal/nope.ts",
            "internal/nope.ts",
            GroModuleFault::Internal,
        ),
        (
            "@fuzdev/gro/invoke.ts",
            "invoke.ts",
            GroModuleFault::NodeOnly,
        ),
        (
            "@fuzdev/gro/library_load.ts",
            "library_load.ts",
            GroModuleFault::NodeOnly,
        ),
        (
            "@fuzdev/gro/build.task.ts",
            "build.task.ts",
            GroModuleFault::NodeOnly,
        ),
        (
            "@fuzdev/gro/task_args.js",
            "task_args.js",
            GroModuleFault::JsSpelling,
        ),
        ("@fuzdev/gro/nope.js", "nope.js", GroModuleFault::Missing),
        ("@fuzdev/gro/nope.ts", "nope.ts", GroModuleFault::Missing),
        (
            "@fuzdev/gro/task_args",
            "task_args",
            GroModuleFault::Missing,
        ),
        (
            "@fuzdev/gro/task_args.ts?x",
            "task_args.ts?x",
            GroModuleFault::Missing,
        ),
        (
            "@fuzdev/gro/./task_args.ts",
            "./task_args.ts",
            GroModuleFault::Missing,
        ),
        (
            "@fuzdev/gro/package.json",
            "package.json",
            GroModuleFault::Missing,
        ),
        ("@fuzdev/gro/", "", GroModuleFault::Missing),
    ];
    for importer in IMPORTERS {
        let importer = Url::parse(importer).unwrap();
        for (specifier, path, fault) in cases {
            let error = resolve_gro(&watched, specifier, &importer, &roots).unwrap_err();
            assert_eq!(
                error.kind,
                ResolveErrorKind::GroModule {
                    path: Box::from(path),
                    fault,
                },
                "{specifier} from {importer}"
            );
            assert!(!error.kind.guards_a_boundary(), "{specifier}");
            assert!(error.kind.is_refusal(), "{specifier}");
        }
    }
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
    let main = Url::parse(IMPORTERS[0]).unwrap();
    let message = |specifier| {
        resolve_gro(&watched, specifier, &main, &roots)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        message("@fuzdev/gro/invoke.ts"),
        format!(
            "cannot resolve '@fuzdev/gro/invoke.ts' from file:///p/main.js: it names Gro's \
             module 'invoke.ts', which runs only on Node, so {} doesn't carry it, and a \
             specifier of Gro's never resolves to a package. Fix: run this under Node with gro",
            crate::BINARY_NAME
        )
    );
    assert!(message("@fuzdev/gro/task_args.js").ends_with(
        "it is the .js spelling of Gro's module 'task_args.ts', and a specifier of Gro's \
             never resolves to a package. Fix: import '@fuzdev/gro/task_args.ts'"
    ));
    assert!(message("@fuzdev/gro/internal/posix_path.ts").contains(
        "it names Gro's internal module 'internal/posix_path.ts', which Gro's package doesn't \
         export"
    ));
    assert!(message("@fuzdev/gro/nope.ts").contains("Gro has no module 'nope.ts'"));
}

/// A run that holds no copy of Gro's library refuses every specifier of
/// Gro's package but the table's, which need none, and still reads
/// nothing for one: a host module, a reserved specifier, and a respelling
/// are as they are with a copy.
#[test]
fn without_a_copy_only_the_table_s_specifiers_resolve() {
    let (watched, roots) = planted();
    for importer in IMPORTERS {
        let importer = Url::parse(importer).unwrap();
        for specifier in [
            "@fuzdev/gro/task_args.ts",
            "@fuzdev/gro",
            "@fuzdev/gro/invoke.ts",
        ] {
            let error = resolve_any(&watched, specifier, &importer, &roots).unwrap_err();
            assert_eq!(
                error.kind,
                ResolveErrorKind::NoGroLibrary(None),
                "{specifier}"
            );
            assert!(
                error
                    .to_string()
                    .contains("this run holds no copy of Gro's library")
            );
        }
        assert_eq!(
            resolve_any(&watched, "@fuzdev/gro/fs.ts", &importer, &roots),
            Ok(Resolved::Builtin(
                find_builtin("@fuzdev/gro/fs.ts").unwrap()
            ))
        );
        assert_eq!(
            resolve_any(&watched, "@fuzdev/gro/net.ts", &importer, &roots)
                .unwrap_err()
                .kind,
            ResolveErrorKind::Reserved(Reserved::Unbuilt("@fuzdev/gro/net"))
        );
    }
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
    // a copy that failed to load is refused with its reason
    let main = Url::parse(IMPORTERS[0]).unwrap();
    let error = crate::resolver::resolve_from(
        &watched,
        Err(Some("the embed is malformed: broken")),
        "@fuzdev/gro/task_args.ts",
        &main,
        roots.project(),
        &roots,
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::NoGroLibrary(Some(Box::from("the embed is malformed: broken")))
    );
}

/// The specifiers that read Gro's library from a module on disk, which the
/// graph obtains it for: Gro's package's, but the table's and the ones it
/// reserves, and nothing else.
#[test]
fn only_a_specifier_of_gro_s_library_needs_it() {
    for specifier in [
        "@fuzdev/gro",
        "@fuzdev/gro/task_args.ts",
        "@fuzdev/gro/internal/posix_path.ts",
        "@fuzdev/gro/invoke.ts",
        "@fuzdev/gro/",
        "@fuzdev/gro/args",
    ] {
        assert!(needs_gro_library(specifier), "{specifier}");
    }
    for specifier in [
        "@fuzdev/gro/args.ts",
        "@fuzdev/gro/args.js",
        "@fuzdev/gro/net.ts",
        "@fuzdev/gro/arg%73.ts",
        "@fuzdev/gro/process.ts",
        "@fuzdev/grox/task_args.ts",
        "dep/x.js",
        "./task_args.ts",
        "file:///p/x.js",
        "",
    ] {
        assert!(!needs_gro_library(specifier), "{specifier}");
    }
}

/// Resolves `specifier` from the embedded module at `importer_path`.
fn from_embedded(
    tree: &MemoryTree,
    roots: &Roots,
    importer_path: &str,
    specifier: &str,
) -> Result<Resolved, ResolveError> {
    let importer = Url::parse(&format!("gro:lib/{importer_path}")).unwrap();
    resolve_from_embedded(
        tree,
        test_library(),
        specifier,
        &importer,
        importer_path,
        roots,
    )
}

/// An embedded module's relative specifiers resolve by their text inside
/// Gro's `src/lib`, internal modules included, row first, so a host
/// module's file is its builtin or refused as unbuilt; one past `src/lib`,
/// and every specifier that is neither relative nor bare, is refused,
/// naming the specifier and the importer.
#[test]
fn an_embedded_module_s_relative_specifiers_resolve_by_text() {
    let (watched, roots) = planted();
    let tree = &watched.tree;
    for (importer, specifier, path) in [
        ("dir/a.ts", "../task_args.ts", "task_args.ts"),
        ("dir/a.ts", "./../index.ts", "index.ts"),
        ("dir/a.ts", "./a.ts", "dir/a.ts"),
        ("dir/a.ts", "../dir/./a.ts", "dir/a.ts"),
        (
            "dir/a.ts",
            "../internal/posix_path.ts",
            "internal/posix_path.ts",
        ),
        ("index.ts", "./task_error.ts", "task_error.ts"),
        ("index.ts", "./dir/../fs_paths.ts", "fs_paths.ts"),
        ("internal/posix_path.ts", "../task_args.ts", "task_args.ts"),
    ] {
        let resolved = from_embedded(tree, &roots, importer, specifier);
        assert_eq!(embedded_path(resolved), path, "{specifier} from {importer}");
    }
    let fs = find_builtin("@fuzdev/gro/fs.ts").unwrap();
    assert_eq!(
        from_embedded(tree, &roots, "index.ts", "./fs.ts"),
        Ok(Resolved::Builtin(fs))
    );
    assert_eq!(
        from_embedded(tree, &roots, "dir/a.ts", "../fs.ts"),
        Ok(Resolved::Builtin(fs))
    );
    assert_eq!(
        from_embedded(tree, &roots, "index.ts", "./net.ts")
            .unwrap_err()
            .kind,
        ResolveErrorKind::Reserved(Reserved::Unbuilt("@fuzdev/gro/net"))
    );
    for (importer, specifier, path, fault) in [
        (
            "index.ts",
            "./invoke.ts",
            "invoke.ts",
            GroModuleFault::NodeOnly,
        ),
        (
            "dir/a.ts",
            "../build.task.ts",
            "build.task.ts",
            GroModuleFault::NodeOnly,
        ),
        ("dir/a.ts", "./b.ts", "dir/b.ts", GroModuleFault::Missing),
        (
            "index.ts",
            "./task_args.js",
            "task_args.js",
            GroModuleFault::JsSpelling,
        ),
        (
            "index.ts",
            "./task_args.ts?x",
            "task_args.ts?x",
            GroModuleFault::Missing,
        ),
        (
            "index.ts",
            "./dir//a.ts",
            "dir//a.ts",
            GroModuleFault::Missing,
        ),
        ("index.ts", "./", "", GroModuleFault::Missing),
    ] {
        assert_eq!(
            from_embedded(tree, &roots, importer, specifier)
                .unwrap_err()
                .kind,
            ResolveErrorKind::GroModule {
                path: Box::from(path),
                fault
            },
            "{specifier} from {importer}"
        );
    }
    for (importer, specifier) in [
        ("index.ts", "../index.ts"),
        ("dir/a.ts", "../../index.ts"),
        ("dir/a.ts", "./../../dir/a.ts"),
        ("index.ts", "/p/main.js"),
        ("index.ts", "file:///p/main.js"),
        ("index.ts", "gro:lib/task_args.ts"),
        ("index.ts", "https://example.com/x.js"),
        ("index.ts", "node:fs"),
        ("index.ts", "#lib/x.ts"),
        ("index.ts", "."),
        ("index.ts", ".."),
    ] {
        let error = from_embedded(tree, &roots, importer, specifier).unwrap_err();
        assert_eq!(
            error.kind,
            ResolveErrorKind::OutsideGroLibrary,
            "{specifier} from {importer}"
        );
        assert!(error.kind.guards_a_boundary(), "{specifier}");
        assert!(
            error.to_string().starts_with(&format!(
                "cannot resolve '{specifier}' from gro:lib/{importer}: a module of Gro's library \
                 imports only Gro's other modules"
            )),
            "{error}"
        );
    }
    assert_eq!(
        from_embedded(tree, &roots, "index.ts", "")
            .unwrap_err()
            .kind,
        ResolveErrorKind::Empty
    );
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
}

/// An embedded module's bare specifiers: the table's and Gro's package's
/// as from any importer, reading nothing, and a package from the project
/// root's `node_modules`, never through the project's self-reference.
#[test]
fn an_embedded_module_s_bare_specifiers_are_the_table_gro_or_a_package() {
    let (watched, roots) = planted();
    let tree = &watched.tree;
    assert_eq!(
        from_embedded(tree, &roots, "index.ts", "@fuzdev/gro/args.ts"),
        Ok(Resolved::Builtin(&BUILTINS[0]))
    );
    assert_eq!(
        from_embedded(tree, &roots, "index.ts", "@fuzdev/gro/args.js")
            .unwrap_err()
            .kind,
        ResolveErrorKind::Reserved(Reserved::JsSpelling(&BUILTINS[0]))
    );
    assert_eq!(
        embedded_path(from_embedded(
            tree,
            &roots,
            "dir/a.ts",
            "@fuzdev/gro/task_args.ts"
        )),
        "task_args.ts"
    );
    assert_eq!(
        from_embedded(
            tree,
            &roots,
            "index.ts",
            "@fuzdev/gro/internal/posix_path.ts"
        )
        .unwrap_err()
        .kind,
        ResolveErrorKind::GroModule {
            path: Box::from("internal/posix_path.ts"),
            fault: GroModuleFault::Internal,
        }
    );
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
}

/// A project named `dep`, exporting `./x.js` itself, with `dep` installed
/// and the project linked into its own `node_modules` as `app`.
fn project_named_dep() -> (MemoryTree, Roots) {
    let tree = MemoryTree {
        files: BTreeMap::from([
            (
                "/p/package.json",
                r#"{"name": "dep", "type": "module", "exports": {"./x.js": "./x.js"}}"#,
            ),
            ("/p/main.js", ""),
            ("/p/x.js", ""),
            (
                "/p/node_modules/dep/package.json",
                r#"{"name": "dep", "version": "1.2.0", "type": "module", "exports": {"./x.js": "./x.js"}}"#,
            ),
            ("/p/node_modules/dep/x.js", ""),
        ]),
        links: BTreeMap::from([("/p/node_modules/app", "/p")]),
        ..MemoryTree::default()
    };
    let roots = roots(&tree, &Url::parse("file:///p/main.js").unwrap());
    (tree, roots)
}

/// An embedded module's package import resolves from the project root's
/// `node_modules`, never through the project's self-reference, so the
/// project's modules and Gro's library share one copy of the package; one
/// resolving to the project's own modules, linked by name, is refused.
#[test]
fn an_embedded_module_s_package_is_the_project_root_s() {
    let (tree, roots) = project_named_dep();
    // the project's own import of the name is its self-reference
    let main = Url::parse("file:///p/main.js").unwrap();
    assert!(matches!(
        resolve_any(&tree, "dep/x.js", &main, &roots),
        Ok(Resolved::Module(resolution)) if resolution.url.as_str() == "file:///p/x.js"
    ));
    let Ok(Resolved::Module(resolution)) = from_embedded(&tree, &roots, "dir/a.ts", "dep/x.js")
    else {
        panic!("not a module");
    };
    assert_eq!(resolution.url.as_str(), "file:///p/node_modules/dep/x.js");
    assert_eq!(
        resolution.root,
        Root {
            dir: PathBuf::from("/p/node_modules/dep"),
            origin: Origin::Package {
                name: "dep".to_owned(),
                version: Some("1.2.0".to_owned()),
                linked: false,
            },
        }
    );
    let error = from_embedded(&tree, &roots, "index.ts", "app/x.js").unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::GroImportsProject {
            path: PathBuf::from("/p/x.js"),
            project: PathBuf::from("/p"),
        }
    );
    assert!(error.kind.guards_a_boundary());
    assert!(error.to_string().contains(
        "Gro's library imports it by name, and it resolves to /p/x.js, a module of the project \
         at /p"
    ));
}

/// A project at `/p` with `name` installed at `version`, or with no
/// `"version"` when it's unset.
fn project_with(name: &str, version: Option<&str>) -> (MemoryTree, Roots) {
    let leak = |text: String| -> &'static str { Box::leak(text.into()) };
    let version = version.map_or_else(String::new, |version| {
        format!(r#""version": "{version}", "#)
    });
    let tree = MemoryTree {
        files: BTreeMap::from([
            ("/p/package.json", r#"{"name": "app", "type": "module"}"#),
            ("/p/main.js", ""),
            (
                leak(format!("/p/node_modules/{name}/package.json")),
                leak(format!(
                    r#"{{"name": "{name}", {version}"type": "module", "exports": {{"./x.js": "./x.js"}}}}"#
                )),
            ),
            (leak(format!("/p/node_modules/{name}/x.js")), ""),
        ]),
        ..MemoryTree::default()
    };
    let roots = roots(&tree, &Url::parse("file:///p/main.js").unwrap());
    (tree, roots)
}

/// A package Gro's library imports is one of Gro's peers at a version in
/// the range it declares: one in the range loads; one Gro doesn't declare
/// is refused before anything is read; one the project doesn't install,
/// one outside the range, a prerelease the range doesn't name, one with a
/// version no range reads, and one with none are each refused, naming the
/// package, the range, and the install that fixes it.
#[test]
fn an_embedded_module_s_package_is_a_peer_in_its_range() {
    for version in ["1.0.0", "1.2.0", "1.99.0"] {
        let (tree, roots) = project_with("dep", Some(version));
        let resolved = from_embedded(&tree, &roots, "index.ts", "dep/x.js");
        assert!(
            matches!(
                &resolved,
                Ok(Resolved::Module(resolution))
                    if resolution.url.as_str() == "file:///p/node_modules/dep/x.js"
            ),
            "{version}: {resolved:?}"
        );
    }
    let (tree, roots) = project_with("other", Some("1.0.0"));
    let watched = Watched {
        tree,
        calls: RefCell::default(),
    };
    let error = resolve_from_embedded(
        &watched,
        test_library(),
        "other/x.js",
        &Url::parse("gro:lib/index.ts").unwrap(),
        "index.ts",
        &roots,
    )
    .unwrap_err();
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
    assert_eq!(
        error.kind,
        ResolveErrorKind::GroPeer {
            name: Box::from("other"),
            fault: Box::new(PeerFault::Undeclared),
        }
    );
    assert!(error.to_string().contains(
        "Gro's library imports the package 'other', which Gro's package.json doesn't declare as \
         a peer dependency"
    ));
    let error =
        from_embedded(&watched.tree, &roots, "index.ts", "@fuzdev/fuz_util/log.ts").unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::GroPeer {
            name: Box::from("@fuzdev/fuz_util"),
            fault: Box::new(PeerFault::Missing {
                range: Box::from(">=0.83.0"),
                node_modules: PathBuf::from("/p/node_modules"),
            }),
        }
    );
    assert_eq!(
        error.to_string(),
        "cannot resolve '@fuzdev/fuz_util/log.ts' from gro:lib/index.ts: Gro's library imports \
         the package '@fuzdev/fuz_util', a peer dependency of Gro's at >=0.83.0, which the \
         project doesn't install: no /p/node_modules/@fuzdev/fuz_util. Fix: npm i -D \
         @fuzdev/fuz_util"
    );
    for version in [
        Some("2.0.0"),
        Some("0.9.9"),
        Some("1.3.0-rc.1"),
        Some("v1.2.0"),
        None,
    ] {
        let (tree, roots) = project_with("dep", version);
        let error = from_embedded(&tree, &roots, "dir/a.ts", "dep/x.js").unwrap_err();
        assert_eq!(
            error.kind,
            ResolveErrorKind::GroPeer {
                name: Box::from("dep"),
                fault: Box::new(PeerFault::OutOfRange {
                    range: Box::from("^1"),
                    version: version.map(Box::from),
                    dir: PathBuf::from("/p/node_modules/dep"),
                }),
            },
            "{version:?}"
        );
        assert!(!error.kind.guards_a_boundary());
        let installs = version.map_or_else(
            || "it with no version".to_owned(),
            |version| format!("version {version}"),
        );
        assert!(
            error.to_string().ends_with(&format!(
                "Gro's library imports the package 'dep', a peer dependency of Gro's at ^1, and \
                 the project installs {installs} at /p/node_modules/dep. Fix: npm i -D 'dep@^1'"
            )),
            "{error}"
        );
    }
    // a range the build would have refused, in a copy made by hand
    let (tree, roots) = project_with("odd", Some("1.2.3"));
    assert_eq!(
        from_embedded(&tree, &roots, "index.ts", "odd/x.js")
            .unwrap_err()
            .kind,
        ResolveErrorKind::GroPeer {
            name: Box::from("odd"),
            fault: Box::new(PeerFault::UnreadRange(Box::from("~1.2.3"))),
        }
    );
}
