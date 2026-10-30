use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use url::Url;

use super::{from_entry, roots};
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{
    ConsultedFile, FileRole, Origin, ResolveErrorKind, Root, Roots, resolve, resolve_entry,
};

/// A project at `/p` with `bar` and `foo` installed, `foo` with a
/// dependency of its own and two `"exports"` targets symlinked out of
/// it, `lnk` linked to a checkout inside the project, and `src` a
/// project of its own whose `package.json` is a symlink to `/other`'s.
fn contained() -> MemoryTree {
    MemoryTree {
        files: BTreeMap::from([
            ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
            ("/p/main.ts", ""),
            ("/p/sub/x.ts", ""),
            (
                "/p/node_modules/bar/package.json",
                r#"{"type": "module", "exports": "./i.mjs"}"#,
            ),
            ("/p/node_modules/bar/i.mjs", ""),
            ("/p/node_modules/bar/internal/secret.mjs", ""),
            (
                "/p/node_modules/foo/package.json",
                r#"{"name": "foo", "version": "1.0.0", "type": "module", "exports": {".": "./i.mjs", "./own.mjs": "./own.mjs", "./out.mjs": "./out.mjs", "./nm.mjs": "./nm.mjs"}}"#,
            ),
            ("/p/node_modules/foo/i.mjs", ""),
            ("/p/node_modules/foo/own.mjs", ""),
            (
                "/p/node_modules/foo/node_modules/baz/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/p/node_modules/foo/node_modules/baz/i.mjs", ""),
            ("/p/node_modules/.bin/x.mjs", ""),
            ("/elsewhere/evil.mjs", ""),
            (
                "/p/packages/lnk/package.json",
                r#"{"name": "lnk", "type": "module", "exports": {"./x.ts": "./x.ts"}}"#,
            ),
            ("/p/packages/lnk/x.ts", ""),
            (
                "/other/package.json",
                r#"{"name": "selfy", "type": "module", "exports": {"./evil.mjs": "./evil.mjs"}}"#,
            ),
            ("/other/evil.mjs", ""),
            ("/p/src/main.ts", ""),
            ("/p/src/evil.mjs", ""),
        ]),
        links: BTreeMap::from([
            ("/p/node_modules/foo/out.mjs", "/elsewhere/evil.mjs"),
            (
                "/p/node_modules/foo/nm.mjs",
                "/p/node_modules/foo/node_modules/baz/i.mjs",
            ),
            ("/p/node_modules/lnk", "/p/packages/lnk"),
            ("/p/node_modules/alias", "/p/node_modules/bar"),
            ("/p/src/package.json", "/other/package.json"),
        ]),
        ..MemoryTree::default()
    }
}

fn url(path: &str) -> Url {
    Url::from_file_path(path).unwrap()
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

/// A relative or absolute specifier never reaches into `node_modules`
/// below its importer's root, as written or by its real path, which
/// would bypass the package's `"exports"`: Node loads all of these.
#[test]
fn a_relative_specifier_never_reaches_into_node_modules() {
    let tree = contained();
    let main = url("/p/main.ts");
    let into = |path: &str| ResolveErrorKind::IntoNodeModules {
        path: PathBuf::from(path),
        root: PathBuf::from("/p"),
    };
    for specifier in [
        "./node_modules/bar/internal/secret.mjs",
        "/p/node_modules/bar/internal/secret.mjs",
        "./sub/../node_modules/bar/internal/secret.mjs",
    ] {
        assert_eq!(
            from_entry(&tree, specifier, &main).unwrap_err().kind,
            into("/p/node_modules/bar/internal/secret.mjs"),
            "{specifier}"
        );
    }
    // as `import(import.meta.resolve('bar'))` would import it
    assert_eq!(
        from_entry(&tree, "file:///p/node_modules/bar/i.mjs", &main)
            .unwrap_err()
            .kind,
        into("/p/node_modules/bar/i.mjs")
    );
    // as written: an in-tree checkout's real path has no node_modules in it
    assert_eq!(
        from_entry(&tree, "./node_modules/lnk/x.ts", &main)
            .unwrap_err()
            .kind,
        into("/p/node_modules/lnk/x.ts")
    );
    // a package's own relative imports stay out of its node_modules too
    let foo = url("/p/node_modules/foo/i.mjs");
    assert_eq!(
        from_entry(&tree, "./node_modules/baz/i.mjs", &foo)
            .unwrap_err()
            .kind,
        ResolveErrorKind::IntoNodeModules {
            path: PathBuf::from("/p/node_modules/foo/node_modules/baz/i.mjs"),
            root: PathBuf::from("/p/node_modules/foo"),
        }
    );
    // a segment past where the path leaves the root is outside it, not below it
    let error = from_entry(&tree, "../other/node_modules/z/i.mjs", &main).unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::OutsideRoot {
            path: PathBuf::from("/other/node_modules/z/i.mjs"),
            root: PathBuf::from("/p"),
            package: None,
        }
    );
    assert_eq!(
        error.kind.to_string(),
        "it resolves to /other/node_modules/z/i.mjs, outside /p, and a relative or absolute \
         specifier resolves only inside its importer's root, the project's directory or a \
         package's own. Fix: move the module inside /p, or install it as a package and \
         import it by name"
    );
    let error = from_entry(&tree, "./node_modules/bar/internal/secret.mjs", &main).unwrap_err();
    assert!(error.kind.is_specifier_error());
    assert_eq!(
        error.to_string(),
        "cannot resolve './node_modules/bar/internal/secret.mjs' from file:///p/main.ts: it \
         resolves to /p/node_modules/bar/internal/secret.mjs, inside node_modules below /p, \
         which only a bare specifier reaches, through the package's \"exports\". Fix: \
         import it by name"
    );
}

/// A package's relative imports stay inside its real root, and resolve
/// with its origin.
#[test]
fn a_package_stays_inside_its_root() {
    let tree = contained();
    let foo = url("/p/node_modules/foo/i.mjs");
    let resolution = from_entry(&tree, "./own.mjs", &foo).unwrap();
    assert_eq!(resolution.root, Path::new("/p/node_modules/foo"));
    assert_eq!(
        resolution.origin,
        package_root("/p/node_modules/foo", "foo", Some("1.0.0"), false).origin
    );
    // a sibling package, whose exports `../bar/` bypasses
    let error = from_entry(&tree, "../bar/internal/secret.mjs", &foo).unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::OutsideRoot {
            path: PathBuf::from("/p/node_modules/bar/internal/secret.mjs"),
            root: PathBuf::from("/p/node_modules/foo"),
            package: Some("foo".into()),
        }
    );
    assert_eq!(
        error.to_string(),
        "cannot resolve '../bar/internal/secret.mjs' from file:///p/node_modules/foo/i.mjs: \
         it resolves to /p/node_modules/bar/internal/secret.mjs, outside package 'foo' at \
         /p/node_modules/foo, and a package's relative and absolute specifiers resolve only \
         inside it, so the package reaches outside itself. Fix: use a version of the \
         package that doesn't"
    );
    // and the project's files
    assert!(matches!(
        from_entry(&tree, "../../main.ts", &foo).unwrap_err().kind,
        ResolveErrorKind::OutsideRoot { .. }
    ));
}

/// A relative specifier never reaches into a package root admitted
/// inside its importer's: an in-tree checkout linked into
/// `node_modules` is reached only by its name, once admitted.
#[test]
fn a_relative_specifier_never_reaches_into_an_admitted_package() {
    let tree = contained();
    let main = url("/p/main.ts");
    let mut roots = roots(&tree, &main);
    // before `lnk` is admitted, the file is the project's: the loader refuses
    // the admission that would follow
    let resolution = resolve(&tree, "./packages/lnk/x.ts", &main, &roots).unwrap();
    assert_eq!(resolution.origin, Origin::Project);
    let resolution = resolve(&tree, "lnk/x.ts", &main, &roots).unwrap();
    let lnk = package_root("/p/packages/lnk", "lnk", None, true);
    assert_eq!(resolution.url.as_str(), "file:///p/packages/lnk/x.ts");
    assert_eq!(resolution.root, lnk.dir);
    assert_eq!(resolution.origin, lnk.origin);
    roots.admit(lnk.clone());
    let error = resolve(&tree, "./packages/lnk/x.ts", &main, &roots).unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::IntoPackageRoot {
            path: PathBuf::from("/p/packages/lnk/x.ts"),
            root: Box::new(lnk),
        }
    );
    assert_eq!(
        error.kind.to_string(),
        "it resolves to /p/packages/lnk/x.ts, inside package 'lnk' at /p/packages/lnk, \
         which only a bare specifier reaches, through its \"exports\". Fix: import it by \
         name ('lnk' or a subpath it exports)"
    );
    // the package's own imports resolve inside it, with its origin
    let x = url("/p/packages/lnk/x.ts");
    let resolution = resolve(&tree, "./x.ts", &x, &roots).unwrap();
    assert_eq!(resolution.root, Path::new("/p/packages/lnk"));
    assert!(matches!(resolution.origin, Origin::Package { name, .. } if name == "lnk"));
    // a package never reaches into the project's root nested in its own
    let mut roots = Roots::new(PathBuf::from("/p/sub"));
    roots.admit(package_root("/p", "outer", None, true));
    assert_eq!(
        resolve(&tree, "./sub/x.ts", &main, &roots)
            .unwrap_err()
            .kind
            .to_string(),
        "it resolves to /p/sub/x.ts, inside the project at /p/sub, which a package's \
         modules never reach"
    );
}

/// Every resolution's real path is checked against its root: an
/// `"exports"` target symlinked out of its package, or into its
/// `node_modules`, is refused.
#[test]
fn an_exports_target_stays_inside_its_package() {
    let tree = contained();
    let main = url("/p/main.ts");
    let target = |specifier, path: &str, error| {
        assert_eq!(
            from_entry(&tree, specifier, &main).unwrap_err().kind,
            ResolveErrorKind::Target {
                path: PathBuf::from(path),
                error: Box::new(error),
            },
            "{specifier}"
        );
    };
    target(
        "foo/out.mjs",
        "/p/node_modules/foo/out.mjs",
        ResolveErrorKind::OutsideRoot {
            path: PathBuf::from("/elsewhere/evil.mjs"),
            root: PathBuf::from("/p/node_modules/foo"),
            package: Some("foo".into()),
        },
    );
    target(
        "foo/nm.mjs",
        "/p/node_modules/foo/nm.mjs",
        ResolveErrorKind::IntoNodeModules {
            path: PathBuf::from("/p/node_modules/foo/node_modules/baz/i.mjs"),
            root: PathBuf::from("/p/node_modules/foo"),
        },
    );
    assert_eq!(
        from_entry(&tree, "foo/out.mjs", &main)
            .unwrap_err()
            .kind
            .to_string(),
        "the package exports it as /p/node_modules/foo/out.mjs, which resolves to \
         /elsewhere/evil.mjs, outside /p/node_modules/foo, and an \"exports\" target stays \
         inside its package. Fix: use a version of the package whose \"exports\" stay \
         inside it"
    );
    // a package's root is its own, whatever the project around it holds
    let resolution = from_entry(&tree, "foo", &main).unwrap();
    assert_eq!(resolution.root, Path::new("/p/node_modules/foo"));
}

/// A self-reference resolves from the directory its `package.json` was
/// found in, not the file's real path, and stays inside both that
/// directory and the importer's root.
#[test]
fn a_self_reference_stays_inside_its_scope_and_root() {
    let tree = contained();
    // `src/package.json` is a symlink to `/other`'s, which used to move the root there
    let resolution = from_entry(&tree, "selfy/evil.mjs", &url("/p/src/main.ts")).unwrap();
    assert_eq!(resolution.url.as_str(), "file:///p/src/evil.mjs");
    assert_eq!(resolution.root, Path::new("/p/src"));
    // a package's self-reference to a target symlinked out of it
    assert_eq!(
        from_entry(&tree, "foo/out.mjs", &url("/p/node_modules/foo/own.mjs"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::Target {
            path: PathBuf::from("/p/node_modules/foo/out.mjs"),
            error: Box::new(ResolveErrorKind::OutsideRoot {
                path: PathBuf::from("/elsewhere/evil.mjs"),
                root: PathBuf::from("/p/node_modules/foo"),
                package: Some("foo".into()),
            }),
        }
    );
    // the same with no project root above the package
    let tree = MemoryTree {
        files: BTreeMap::from([
            (
                "/x/node_modules/foo/package.json",
                r#"{"name": "foo", "exports": {"./a.mjs": "./a.mjs"}}"#,
            ),
            ("/x/node_modules/foo/b.mjs", ""),
            ("/elsewhere/evil.mjs", ""),
        ]),
        links: BTreeMap::from([("/x/node_modules/foo/a.mjs", "/elsewhere/evil.mjs")]),
        ..MemoryTree::default()
    };
    assert!(matches!(
        from_entry(&tree, "foo/a.mjs", &url("/x/node_modules/foo/b.mjs"))
            .unwrap_err()
            .kind,
        ResolveErrorKind::Target { error, .. }
            if matches!(*error, ResolveErrorKind::OutsideRoot { .. })
    ));
    // inside its package's directory but outside the importer's root
    let tree = contained();
    let roots = Roots::new(PathBuf::from("/p/sub"));
    let mut files = tree.files;
    files.insert(
        "/p/package.json",
        r#"{"name": "proj", "type": "module", "exports": {"./main.ts": "./main.ts"}}"#,
    );
    let tree = MemoryTree {
        files,
        ..contained()
    };
    assert_eq!(
        resolve(&tree, "proj/main.ts", &url("/p/sub/x.ts"), &roots)
            .unwrap_err()
            .kind,
        ResolveErrorKind::Target {
            path: PathBuf::from("/p/main.ts"),
            error: Box::new(ResolveErrorKind::OutsideRoot {
                path: PathBuf::from("/p/main.ts"),
                root: PathBuf::from("/p/sub"),
                package: None,
            }),
        }
    );
}

/// A self-reference's target stays inside its package's directory even
/// when the importer's root around it holds the target.
#[test]
fn a_self_reference_stays_inside_its_package_directory() {
    let mut files = contained().files;
    files.insert(
        "/p/sub/package.json",
        r#"{"name": "subpkg", "exports": {"./a.mjs": "./a.mjs"}}"#,
    );
    let mut links = contained().links;
    links.insert("/p/sub/a.mjs", "/p/main.ts");
    let tree = MemoryTree {
        files,
        links,
        ..MemoryTree::default()
    };
    let main = url("/p/main.ts");
    assert_eq!(
        resolve(
            &tree,
            "subpkg/a.mjs",
            &url("/p/sub/x.ts"),
            &roots(&tree, &main)
        )
        .unwrap_err()
        .kind,
        ResolveErrorKind::Target {
            path: PathBuf::from("/p/sub/a.mjs"),
            error: Box::new(ResolveErrorKind::OutsideRoot {
                path: PathBuf::from("/p/main.ts"),
                root: PathBuf::from("/p/sub"),
                package: None,
            }),
        }
    );
    // a nested package.json inside an installed package names its own
    // directory, not the installed package
    let mut files = contained().files;
    files.insert(
        "/p/node_modules/foo/sub/package.json",
        r#"{"name": "subpkg", "exports": {"./a.mjs": "./a.mjs"}}"#,
    );
    files.insert("/p/node_modules/foo/sub/x.mjs", "");
    let mut links = contained().links;
    links.insert(
        "/p/node_modules/foo/sub/a.mjs",
        "/p/node_modules/foo/own.mjs",
    );
    let tree = MemoryTree {
        files,
        links,
        ..MemoryTree::default()
    };
    let importer = url("/p/node_modules/foo/sub/x.mjs");
    assert_eq!(
        resolve(&tree, "subpkg/a.mjs", &importer, &roots(&tree, &importer))
            .unwrap_err()
            .kind,
        ResolveErrorKind::Target {
            path: PathBuf::from("/p/node_modules/foo/sub/a.mjs"),
            error: Box::new(ResolveErrorKind::OutsideRoot {
                path: PathBuf::from("/p/node_modules/foo/own.mjs"),
                root: PathBuf::from("/p/node_modules/foo/sub"),
                package: None,
            }),
        }
    );
}

/// An entry inside `node_modules` runs as the package it is in, with
/// the project root at the directory holding the first segment.
#[test]
fn an_entry_inside_node_modules_runs_as_its_package() {
    let tree = contained();
    let (entry, roots) =
        resolve_entry(&tree, Path::new("node_modules/foo/i.mjs"), Path::new("/p")).unwrap();
    let foo = package_root("/p/node_modules/foo", "foo", Some("1.0.0"), false);
    assert_eq!(entry.root, foo.dir);
    assert_eq!(entry.origin, foo.origin);
    // its package's, read for its version, and no walk for a project root
    assert_eq!(
        entry.files_consulted,
        [ConsultedFile {
            path: PathBuf::from("/p/node_modules/foo/package.json"),
            role: FileRole::Version,
        }]
    );
    // a project's entry reads the walk for its project root
    let (project_entry, _) = resolve_entry(&tree, Path::new("main.ts"), Path::new("/p")).unwrap();
    assert_eq!(
        project_entry.files_consulted,
        [ConsultedFile {
            path: PathBuf::from("/p/package.json"),
            role: FileRole::SearchBound,
        }]
    );
    assert_eq!(roots.project().dir, Path::new("/p"));
    assert_eq!(roots.iter().collect::<Vec<_>>(), [roots.project(), &foo]);
    assert!(matches!(
        resolve(&tree, "../../main.ts", &entry.url, &roots)
            .unwrap_err()
            .kind,
        ResolveErrorKind::OutsideRoot { .. }
    ));
    // a store's nested layout runs as the package after the last segment
    let mut files = contained().files;
    files.insert(
        "/p/node_modules/.deno/q@1/node_modules/q/package.json",
        r#"{"exports": "./i.mjs"}"#,
    );
    files.insert("/p/node_modules/.deno/q@1/node_modules/q/i.mjs", "");
    let tree = MemoryTree {
        files,
        ..MemoryTree::default()
    };
    let (entry, roots) = resolve_entry(
        &tree,
        Path::new("/p/node_modules/.deno/q@1/node_modules/q/i.mjs"),
        Path::new("/"),
    )
    .unwrap();
    assert_eq!(
        entry.root,
        Path::new("/p/node_modules/.deno/q@1/node_modules/q")
    );
    assert_eq!(roots.project().dir, Path::new("/p"));
    // a file in node_modules but in no package
    for (path, real) in [
        ("node_modules/.bin/x.mjs", "/p/node_modules/.bin/x.mjs"),
        ("node_modules/loose.mjs", "/p/node_modules/loose.mjs"),
        ("node_modules/@scope/x.mjs", "/p/node_modules/@scope/x.mjs"),
    ] {
        let mut files = contained().files;
        files.insert(real, "");
        let tree = MemoryTree {
            files,
            ..contained()
        };
        let error = resolve_entry(&tree, Path::new(path), Path::new("/p"))
            .map(|(entry, _)| entry)
            .unwrap_err();
        assert_eq!(
            error.kind,
            ResolveErrorKind::InNoPackage(PathBuf::from(real)),
            "{path}"
        );
        assert!(error.kind.is_specifier_error());
        assert_eq!(
            error.to_string(),
            format!(
                "cannot resolve '{path}' from /p: it resolves to {real}, inside \
                 node_modules but in no package, so it has no origin. Fix: run a file inside \
                 a package (node_modules/<name>/...)"
            )
        );
    }
}

/// Roots are keyed by real directory: a package reached under a second
/// name keeps the origin it was admitted with.
#[test]
fn a_root_reached_under_two_names_keeps_its_first_origin() {
    let tree = contained();
    let main = url("/p/main.ts");
    let mut roots = roots(&tree, &main);
    let alias = resolve(&tree, "alias", &main, &roots).unwrap();
    assert_eq!(alias.root, Path::new("/p/node_modules/bar"));
    assert_eq!(
        alias.origin,
        package_root("/p/node_modules/bar", "alias", None, false).origin
    );
    roots.admit(package_root("/p/node_modules/bar", "bar", None, false));
    assert_eq!(
        resolve(&tree, "alias", &main, &roots).unwrap().origin,
        package_root("/p/node_modules/bar", "bar", None, false).origin
    );
    // and a package linked to the project's own directory is the project
    let mut files = contained().files;
    files.insert(
        "/p/package.json",
        r#"{"name": "proj", "type": "module", "exports": {"./main.ts": "./main.ts"}}"#,
    );
    let mut links = contained().links;
    links.insert("/p/node_modules/proj", "/p");
    let tree = MemoryTree {
        files,
        links,
        ..MemoryTree::default()
    };
    let resolution = resolve(&tree, "proj/main.ts", &url("/p/sub/x.ts"), &roots).unwrap();
    assert_eq!(resolution.origin, Origin::Project);
}

/// A path's root is the innermost admitted one above it, by path
/// component, and none with a `node_modules` segment below that.
#[test]
fn a_path_is_inside_its_innermost_root() {
    let mut roots = Roots::new(PathBuf::from("/p"));
    roots.admit(package_root("/p/packages/a", "a", None, true));
    roots.admit(package_root("/p/node_modules/b", "b", None, false));
    let containing = |path: &str| roots.containing(Path::new(path)).map(|root| &root.dir);
    assert_eq!(containing("/p/x.ts"), Some(&PathBuf::from("/p")));
    assert_eq!(
        containing("/p/packages/a/x.ts"),
        Some(&PathBuf::from("/p/packages/a"))
    );
    // by component, not by string prefix
    assert_eq!(
        containing("/p/packages/ab/x.ts"),
        Some(&PathBuf::from("/p"))
    );
    assert_eq!(
        containing("/p/node_modules/b/x.ts"),
        Some(&PathBuf::from("/p/node_modules/b"))
    );
    assert_eq!(containing("/p/node_modules/c/x.ts"), None);
    assert_eq!(containing("/p/node_modules/b/node_modules/c/x.ts"), None);
    assert_eq!(containing("/q/x.ts"), None);
    // the segments above a root don't count
    let roots = Roots::new(PathBuf::from("/home/node_modules/proj"));
    assert!(
        roots
            .containing(Path::new("/home/node_modules/proj/x.ts"))
            .is_some()
    );
    // one directory is admitted once
    let mut roots = Roots::new(PathBuf::from("/p"));
    roots.admit(package_root("/p", "p", None, true));
    assert_eq!(roots.iter().count(), 1);
}

#[test]
fn a_root_conflict_names_both_roots() {
    let kind = ResolveErrorKind::RootConflict {
        path: PathBuf::from("/p/packages/lnk/x.ts"),
        recorded: Box::new(Root {
            dir: PathBuf::from("/p"),
            origin: Origin::Project,
        }),
        admitted: Box::new(package_root("/p/packages/lnk", "lnk", None, true)),
    };
    assert!(kind.is_specifier_error());
    assert_eq!(
        kind.to_string(),
        "/p/packages/lnk/x.ts was resolved inside the project at /p, and package 'lnk' at \
         /p/packages/lnk was then admitted around it, so its origin would depend on the \
         import order. Fix: import it by name ('lnk' or a subpath it exports) everywhere, \
         not by a relative path"
    );
    // one file held by two packages, the inner linked into the outer
    let kind = ResolveErrorKind::RootConflict {
        path: PathBuf::from("/p/node_modules/aa/vendor/v.mjs"),
        recorded: Box::new(package_root("/p/node_modules/aa", "aa", None, false)),
        admitted: Box::new(package_root("/p/node_modules/aa/vendor", "bb", None, false)),
    };
    assert_eq!(
        kind.to_string(),
        "/p/node_modules/aa/vendor/v.mjs was resolved inside package 'aa' at \
         /p/node_modules/aa, and package 'bb' at /p/node_modules/aa/vendor was then admitted \
         around it, so its origin would depend on the import order. Two packages hold it, \
         'bb' inside 'aa'. Fix: import it through only one of them, or install 'bb' on its \
         own rather than inside 'aa'"
    );
}
