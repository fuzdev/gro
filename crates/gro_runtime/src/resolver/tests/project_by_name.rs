//! A package's bare import that resolves into the project's own root is
//! refused, by the importer's origin, while the project's own bare imports
//! of itself and every package-to-package import stay allowed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use url::Url;

use super::{resolve, roots};
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{Origin, ResolveError, ResolveErrorKind, Root};

/// A project at `/p` named `app`, exporting its own modules, linked into
/// its own `node_modules` as `self` and as `app`; an installed `pkg` that
/// imports it, exports itself, and has a target symlinked into the
/// project; an installed `dep`; and `lib`, linked from `/linked/lib`, with
/// the project and `pkg` linked into its own `node_modules`.
fn project() -> MemoryTree {
    MemoryTree {
        files: BTreeMap::from([
            (
                "/p/package.json",
                r#"{"name": "app", "type": "module", "exports": {"./args.js": "./args.js", "./sub/x.js": "./sub/x.js"}}"#,
            ),
            ("/p/main.js", ""),
            ("/p/args.js", ""),
            ("/p/sub/x.js", ""),
            (
                "/p/node_modules/pkg/package.json",
                r#"{"name": "pkg", "version": "1.0.0", "type": "module", "exports": {"./hop.js": "./hop.js", "./into.js": "./into.js"}}"#,
            ),
            ("/p/node_modules/pkg/hop.js", ""),
            (
                "/p/node_modules/dep/package.json",
                r#"{"name": "dep", "type": "module", "exports": "./i.js"}"#,
            ),
            ("/p/node_modules/dep/i.js", ""),
            (
                "/linked/lib/package.json",
                r#"{"name": "lib", "type": "module", "exports": {"./x.js": "./x.js"}}"#,
            ),
            ("/linked/lib/x.js", ""),
        ]),
        links: BTreeMap::from([
            ("/p/node_modules/self", ".."),
            ("/p/node_modules/app", "/p"),
            ("/p/node_modules/lib", "/linked/lib"),
            ("/p/node_modules/pkg/into.js", "/p/args.js"),
            ("/linked/lib/node_modules/app", "/p"),
            ("/linked/lib/node_modules/pkg", "/p/node_modules/pkg"),
        ]),
        ..MemoryTree::default()
    }
}

fn url(path: &str) -> Url {
    Url::from_file_path(path).unwrap()
}

/// Resolves `specifier` from `importer` in a graph whose entry is `entry`,
/// with `admitted` admitted beside the entry's roots.
fn resolve_in(
    tree: &MemoryTree,
    entry: &str,
    admitted: &[Root],
    specifier: &str,
    importer: &str,
) -> Result<Root, ResolveError> {
    let mut roots = roots(tree, &url(entry));
    for root in admitted {
        roots.admit(root.clone());
    }
    resolve(tree, specifier, &url(importer), &roots).map(|resolution| resolution.root)
}

fn package_root(dir: &str, name: &str, version: Option<&str>, linked: bool) -> Root {
    Root {
        dir: PathBuf::from(dir),
        origin: Origin::Package {
            name: name.to_owned(),
            version: version.map(str::to_owned),
            linked,
        },
    }
}

fn pkg() -> Root {
    package_root("/p/node_modules/pkg", "pkg", Some("1.0.0"), false)
}

fn lib() -> Root {
    package_root("/linked/lib", "lib", None, true)
}

/// Asserts the refusal of `package`'s import of the project's module at
/// `path`, and its message's load-bearing words.
fn assert_refused(result: Result<Root, ResolveError>, package: &str, path: &str) {
    let error = result.unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::PackageImportsProject {
            package: Box::from(package),
            path: PathBuf::from(path),
            project: PathBuf::from("/p"),
        }
    );
    assert!(error.kind.is_refusal());
    let message = error.to_string();
    for words in [
        format!("package '{package}' imports it by name"),
        format!("it resolves to {path}, a module of the project at /p"),
        "a package can't import the project's own modules by name".to_owned(),
    ] {
        assert!(message.contains(&words), "{message}");
    }
    assert!(!message.contains("Fix:"), "{message}");
}

/// An installed package's module importing the project by a name its
/// `node_modules` links to the project's directory is refused, through
/// every name and subpath, whatever else the graph admitted first.
#[test]
fn an_installed_package_importing_the_project_by_name_is_refused() {
    let tree = project();
    for admitted in [&[pkg()][..], &[pkg(), lib()]] {
        for (specifier, path) in [
            ("self/args.js", "/p/args.js"),
            ("app/args.js", "/p/args.js"),
            ("self/sub/x.js", "/p/sub/x.js"),
        ] {
            assert_refused(
                resolve_in(
                    &tree,
                    "/p/main.js",
                    admitted,
                    specifier,
                    "/p/node_modules/pkg/hop.js",
                ),
                "pkg",
                path,
            );
        }
    }
}

/// A linked package's module importing the project through its own
/// `node_modules` is refused the same way: being linked launders nothing.
#[test]
fn a_linked_package_importing_the_project_by_name_is_refused() {
    assert_refused(
        resolve_in(
            &project(),
            "/p/main.js",
            &[lib()],
            "app/args.js",
            "/linked/lib/x.js",
        ),
        "lib",
        "/p/args.js",
    );
}

/// An entry inside `node_modules` runs as its package, so its bare import
/// of the project is a package's, refused, with no other import first:
/// the project's root is admitted with the entry's roots, before any
/// package's, so no order admits it as the package's.
#[test]
fn an_entry_inside_node_modules_importing_the_project_is_refused() {
    let tree = project();
    let entry = "/p/node_modules/pkg/hop.js";
    let roots = roots(&tree, &url(entry));
    assert_eq!(roots.project().origin, Origin::Project);
    assert_eq!(roots.get(Path::new("/p")), Some(roots.project()));
    assert_refused(
        resolve_in(&tree, entry, &[], "self/args.js", entry),
        "pkg",
        "/p/args.js",
    );
    // a package admitted at the project's directory keeps the project's origin
    let mut roots = roots;
    roots.admit(package_root("/p", "self", None, true));
    assert_eq!(roots.get(Path::new("/p")).unwrap().origin, Origin::Project);
}

/// The project's own bare imports of itself stay allowed: a self-reference
/// and a link to its own directory, from the entry and a module below it.
#[test]
fn the_project_importing_itself_by_name_is_allowed() {
    let tree = project();
    for importer in ["/p/main.js", "/p/sub/x.js"] {
        for specifier in ["app/args.js", "self/args.js"] {
            let root = resolve_in(&tree, "/p/main.js", &[pkg()], specifier, importer)
                .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(root.origin, Origin::Project, "{specifier} from {importer}");
        }
    }
}

/// A package's self-reference stays allowed, as does every
/// package-to-package import, installed and linked alike, either way.
#[test]
fn a_package_importing_itself_or_another_package_is_allowed() {
    let tree = project();
    let admitted = [pkg(), lib()];
    let cases = [
        ("pkg/hop.js", "/p/node_modules/pkg/hop.js", pkg()),
        (
            "dep",
            "/p/node_modules/pkg/hop.js",
            package_root("/p/node_modules/dep", "dep", None, false),
        ),
        ("lib/x.js", "/p/node_modules/pkg/hop.js", lib()),
        ("pkg/hop.js", "/linked/lib/x.js", pkg()),
    ];
    for (specifier, importer, expected) in cases {
        let root = resolve_in(&tree, "/p/main.js", &admitted, specifier, importer)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(root, expected, "{specifier} from {importer}");
    }
}

/// A package's `"exports"` target symlinked into the project is refused by
/// containment before this rule applies: its real path leaves the package,
/// from the project and from the package's self-reference alike.
#[test]
fn an_exports_target_symlinked_into_the_project_is_outside_its_package() {
    let tree = project();
    for importer in ["/p/main.js", "/p/node_modules/pkg/hop.js"] {
        let error = resolve_in(&tree, "/p/main.js", &[pkg()], "pkg/into.js", importer).unwrap_err();
        assert!(
            matches!(
                &error.kind,
                ResolveErrorKind::Target { error, .. }
                    if matches!(**error, ResolveErrorKind::OutsideRoot { .. })
            ),
            "{importer}: {error}"
        );
    }
    // and a relative specifier from the package stays in its root
    let error = resolve_in(
        &tree,
        "/p/main.js",
        &[pkg()],
        "../../args.js",
        "/p/node_modules/pkg/hop.js",
    )
    .unwrap_err();
    assert!(
        matches!(error.kind, ResolveErrorKind::OutsideRoot { .. }),
        "{error}"
    );
}

/// The refusal comes before the project's module is looked up, so a
/// package learns nothing of which project files exist: its import of one
/// the project exports but doesn't have is refused the same way.
#[test]
fn a_package_is_refused_before_the_project_module_is_looked_up() {
    let mut tree = project();
    tree.files.insert(
        "/p/package.json",
        r#"{"name": "app", "type": "module", "exports": {"./gone.js": "./gone.js"}}"#,
    );
    assert_refused(
        resolve_in(
            &tree,
            "/p/main.js",
            &[pkg()],
            "self/gone.js",
            "/p/node_modules/pkg/hop.js",
        ),
        "pkg",
        "/p/gone.js",
    );
}
