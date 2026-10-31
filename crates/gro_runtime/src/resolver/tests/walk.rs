use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use url::Url;

use super::{MODULE, from_entry, package, packages, roots};
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{
    ConsultedFile, DiskTree, Entry, FileId, FileRole, MAX_PACKAGE_JSON_SIZE, PackageSearchBound,
    ResolveErrorKind, SourceTree, resolve, resolve_entry,
};

/// The search ends at the first `package.json` outside `node_modules`
/// that isn't a type marker, one holding only `"type"` and
/// `"sideEffects"`, which is walked past.
#[test]
fn the_search_ends_at_a_package_json_that_is_not_a_type_marker() {
    let tree = packages();
    let from = |importer: &str, specifier: &str| {
        let importer = Url::parse(importer).unwrap();
        from_entry(&tree, specifier, &importer)
    };
    let t = "file:///w/proj/node_modules/t/main.js";
    for importer in [
        "file:///w/proj/dist/esm/x.js",
        "file:///w/proj/scripts/x.mjs",
        "file:///w/proj/bom/x.js",
        "file:///w/proj/side/x.js",
        "file:///w/proj/empty/x.mjs",
    ] {
        assert_eq!(from(importer, "t").unwrap().url.as_str(), t, "{importer}");
    }
    // a nameless package.json that isn't a marker ends it, as an `npm i` root's does
    let bound = |dir: &str| {
        Box::new(PackageSearchBound {
            dir: PathBuf::from(dir),
            package_json: PathBuf::from(format!("{dir}/package.json")),
            above: Some(PathBuf::from("/w/proj/package.json")),
        })
    };
    assert_eq!(
        from("file:///w/proj/deps/x.mjs", "t").unwrap_err().kind,
        ResolveErrorKind::PackageNotFound {
            name: "t".to_owned(),
            bound: bound("/w/proj/deps"),
        }
    );
    let error = from("file:///w/proj/imports/x.js", "t").unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::PackageNotFound {
            name: "t".to_owned(),
            bound: bound("/w/proj/imports"),
        }
    );
    assert_eq!(
        error.to_string(),
        "cannot resolve 't' from file:///w/proj/imports/x.js: no package 't' is installed: \
         no node_modules/t directory from the importer up to /w/proj/imports/package.json, \
         which ends the search, as every package.json outside node_modules does except one \
         holding nothing but \"type\" and \"sideEffects\", and the node_modules beside \
         /w/proj/package.json is not searched (npm and yarn workspaces, which install at \
         their root, aren't supported). Fix: install it into /w/proj/imports/node_modules"
    );
    // the marker is the scope, read once and consulted in both roles, and the
    // walk reads every package.json up to the bound before any node_modules,
    // even when the package is installed below it
    assert_eq!(
        from("file:///w/proj/dist/esm/x.js", "t")
            .unwrap()
            .files_consulted[..4],
        [
            ConsultedFile {
                path: PathBuf::from("/w/proj/dist/esm/package.json"),
                role: FileRole::PackageScope
            },
            ConsultedFile {
                path: PathBuf::from("/w/proj/dist/esm/package.json"),
                role: FileRole::SearchBound
            },
            ConsultedFile {
                path: PathBuf::from("/w/proj/dist/package.json"),
                role: FileRole::SearchBound
            },
            ConsultedFile {
                path: PathBuf::from("/w/proj/package.json"),
                role: FileRole::SearchBound
            },
        ]
    );
    // a named nested project ends it, with the workspace's root above unsearched
    assert_eq!(
        from("file:///w/proj/inner/x.js", "t").unwrap_err().kind,
        ResolveErrorKind::PackageNotFound {
            name: "t".to_owned(),
            bound: Box::new(PackageSearchBound {
                dir: PathBuf::from("/w/proj/inner"),
                package_json: PathBuf::from("/w/proj/inner/package.json"),
                above: Some(PathBuf::from("/w/proj/package.json")),
            }),
        }
    );
    // a package.json on the search that isn't JSON is an error, not passed over
    assert!(matches!(
        from("file:///w/proj/broken/inner/x.mjs", "t").unwrap_err().kind,
        ResolveErrorKind::InvalidPackageJson { path, .. }
            if path == Path::new("/w/proj/broken/package.json")
    ));
    // and so is one whose "name" isn't a string, which Node never reads here
    assert_eq!(
        from("file:///w/proj/name_bool/inner/x.mjs", "t")
            .unwrap_err()
            .kind,
        ResolveErrorKind::InvalidPackageJson {
            path: PathBuf::from("/w/proj/name_bool/package.json"),
            reason: "its \"name\" is not a string".to_owned(),
        }
    );
    // as the importer's scope, read for a self-reference, as Node reads it
    assert_eq!(
        from("file:///w/proj/name_number/x.mjs", "t")
            .unwrap_err()
            .kind,
        ResolveErrorKind::InvalidPackageJson {
            path: PathBuf::from("/w/proj/name_number/package.json"),
            reason: "its \"name\" is not a string".to_owned(),
        }
    );
}

/// A tree that records the path of each read.
struct CountingTree {
    tree: MemoryTree,
    reads: std::cell::RefCell<Vec<PathBuf>>,
}

impl SourceTree for CountingTree {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        self.tree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        self.reads.borrow_mut().push(path.to_owned());
        self.tree.read(path, id)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        self.tree.is_world_writable(dir)
    }
}

/// The importer's scope, read for a self-reference, isn't read again
/// when the search passes its directory.
#[test]
fn the_scope_is_read_once() {
    let tree = CountingTree {
        tree: packages(),
        reads: std::cell::RefCell::default(),
    };
    let importer = Url::parse("file:///w/proj/dist/esm/x.js").unwrap();
    resolve(&tree, "t", &importer, &roots(&tree.tree, &importer)).unwrap();
    let reads = tree.reads.borrow();
    let scope_reads = reads
        .iter()
        .filter(|path| *path == Path::new("/w/proj/dist/esm/package.json"))
        .count();
    assert_eq!(scope_reads, 1, "{reads:?}");
    // the search passes the next marker up, reading it
    assert!(
        reads.contains(&PathBuf::from("/w/proj/dist/package.json")),
        "{reads:?}"
    );
}

/// A `package.json` that is a FIFO when it is looked up is refused by its
/// type, not opened: a package's, and a module's scope.
#[test]
fn a_special_file_package_json_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = std::env::temp_dir().join(format!("gro_resolver_fifo_{}", std::process::id()));
    std::fs::create_dir_all(root.join("node_modules/fifo")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    // pinned, since a world-writable one would end the walks
    for dir in ["", "node_modules", "node_modules/fifo", "sub"] {
        std::fs::set_permissions(root.join(dir), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
        root.join("package.json"),
        r#"{"name": "r", "type": "module"}"#,
    )
    .unwrap();
    std::fs::write(root.join("main.mjs"), "").unwrap();
    std::fs::write(root.join("sub/a.js"), "").unwrap();
    for fifo in ["node_modules/fifo/package.json", "sub/package.json"] {
        let status = std::process::Command::new("mkfifo")
            .arg(root.join(fifo))
            .status()
            .expect("coreutils `mkfifo` is required for the special file test");
        assert!(status.success(), "mkfifo {fifo}");
    }
    let real_root = root.canonicalize().unwrap();
    let importer = Url::from_file_path(real_root.join("main.mjs")).unwrap();
    let from = |specifier| from_entry(&DiskTree, specifier, &importer);
    let package = from("fifo");
    let scope = from("./sub/a.js");
    // removed before any assertion, so a failure leaves no FIFO behind
    std::fs::remove_dir_all(&root).unwrap();
    let package = package.unwrap_err().kind;
    let scope = scope.unwrap_err().kind;
    let invalid = |path: &str| ResolveErrorKind::InvalidPackageJson {
        path: real_root.join(path),
        reason: "it is not a regular file (a FIFO, a socket, or a device)".to_owned(),
    };
    assert_eq!(package, invalid("node_modules/fifo/package.json"));
    assert_eq!(scope, invalid("sub/package.json"));
    assert_eq!(
        package.to_string(),
        format!(
            "{}/node_modules/fifo/package.json is not a valid package.json: it is not a \
             regular file (a FIFO, a socket, or a device). Fix: correct it, or reinstall the \
             package",
            real_root.display()
        )
    );
}

#[test]
fn package_json_reads() {
    // a byte order mark is skipped, as Node skips it
    assert_eq!(
        package("./bom/x.js").unwrap().url.as_str(),
        "file:///w/proj/bom/x.js"
    );
    // one too large is refused by its size
    let mut tree = packages();
    let large = "x".repeat(usize::try_from(MAX_PACKAGE_JSON_SIZE).unwrap() + 1);
    tree.files
        .insert("/w/proj/node_modules/large/package.json", large.leak());
    let importer = Url::parse("file:///w/proj/main.js").unwrap();
    let error = from_entry(&tree, "large", &importer).unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::InvalidPackageJson {
            path: PathBuf::from("/w/proj/node_modules/large/package.json"),
            reason: "it is larger than 16 MiB, the most a package.json may be".to_owned(),
        }
    );
}

/// A `node_modules` planted above a directory with no project root
/// never resolves: the bound is found before any `node_modules` is
/// looked up, and with none, a module gets no bare imports except a
/// self-reference, which resolves through its own package's `"exports"`.
#[test]
fn a_node_modules_above_a_package_less_dir_is_never_searched() {
    let tree = MemoryTree {
        files: BTreeMap::from([
            (
                "/up/node_modules/foo/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/up/node_modules/foo/i.mjs", ""),
            ("/up/x/zap.ts", ""),
            ("/up/x/a.ts", ""),
            // a marker is no bound
            ("/up/marked/package.json", MODULE),
            ("/up/marked/zap.ts", ""),
        ]),
        ..MemoryTree::default()
    };
    for dir in ["/up/x", "/up/marked"] {
        let importer = Url::from_file_path(format!("{dir}/zap.ts")).unwrap();
        assert_eq!(
            from_entry(&tree, "foo", &importer).unwrap_err().kind,
            ResolveErrorKind::NoProjectRoot {
                dir: PathBuf::from(dir),
                stopped_at: None,
            }
        );
    }
    // relative imports are unaffected
    let importer = Url::parse("file:///up/x/zap.ts").unwrap();
    assert!(from_entry(&tree, "./a.ts", &importer).is_ok());
}

/// A tree with a world-writable `/shared` holding everything a planted
/// project would: a `package.json` that would bound the search, its
/// `node_modules`, and a name a config's self-reference would match.
/// `/shared/proj` is a project below it, and `/shared/loose` none.
fn shared() -> MemoryTree {
    MemoryTree {
        files: BTreeMap::from([
            (
                "/shared/package.json",
                r#"{"name": "@fuzdev/zap", "type": "module", "exports": {"./zap.ts": "./evil.ts"}}"#,
            ),
            ("/shared/evil.ts", ""),
            (
                "/shared/node_modules/foo/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/shared/node_modules/foo/i.mjs", ""),
            ("/shared/loose/config.ts", ""),
            ("/shared/loose/a.mjs", ""),
            ("/shared/loose/b.js", ""),
            (
                "/shared/proj/package.json",
                r#"{"name": "proj", "type": "module"}"#,
            ),
            ("/shared/proj/main.ts", ""),
            (
                "/shared/proj/node_modules/bar/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/shared/proj/node_modules/bar/i.mjs", ""),
        ]),
        world_writable: vec!["/shared"],
        ..MemoryTree::default()
    }
}

/// A world-writable directory ends every walk up, and no `package.json`
/// or `node_modules` in it is read: not as a bound, a self-reference's scope, or a `.js` file's
/// `"type"` scope, and not its `node_modules`.
#[test]
fn a_world_writable_directory_ends_every_walk() {
    let tree = CountingTree {
        tree: shared(),
        reads: std::cell::RefCell::default(),
    };
    let loose = Url::parse("file:///shared/loose/config.ts").unwrap();
    let from = |specifier| from_entry(&tree, specifier, &loose);
    let no_project_root = ResolveErrorKind::NoProjectRoot {
        dir: PathBuf::from("/shared/loose"),
        stopped_at: Some(PathBuf::from("/shared")),
    };
    assert_eq!(from("foo").unwrap_err().kind, no_project_root);
    // the self-reference never reads the package.json that would name it
    assert_eq!(
        from("@fuzdev/zap/zap.ts").unwrap_err().kind,
        no_project_root
    );
    assert_eq!(
        from("./b.js").unwrap_err().kind,
        ResolveErrorKind::NoTypeScope {
            stopped_at: Some(PathBuf::from("/shared")),
        }
    );
    let resolution = from("./a.mjs").unwrap();
    assert_eq!(resolution.url.as_str(), "file:///shared/loose/a.mjs");
    assert_eq!(resolution.files_consulted, []);
    // a project below it resolves its own packages, reading nothing in it
    let proj = Url::parse("file:///shared/proj/main.ts").unwrap();
    let resolution = from_entry(&tree, "bar", &proj).unwrap();
    assert_eq!(
        resolution.url.as_str(),
        "file:///shared/proj/node_modules/bar/i.mjs"
    );
    // the bound's read is recorded in its role
    assert!(resolution.files_consulted.contains(&ConsultedFile {
        path: PathBuf::from("/shared/proj/package.json"),
        role: FileRole::SearchBound,
    }));
    assert!(
        resolution
            .files_consulted
            .iter()
            .all(|file| !file.path.starts_with("/shared/package.json")
                && !file.path.starts_with("/shared/node_modules")),
        "{:?}",
        resolution.files_consulted
    );
    // and the not-found message's walk above the bound stops there too
    assert_eq!(
        from_entry(&tree, "foo", &proj).unwrap_err().kind,
        ResolveErrorKind::PackageNotFound {
            name: "foo".to_owned(),
            bound: Box::new(PackageSearchBound {
                dir: PathBuf::from("/shared/proj"),
                package_json: PathBuf::from("/shared/proj/package.json"),
                above: None,
            }),
        }
    );
    let reads = tree.reads.borrow();
    assert!(
        reads
            .iter()
            .all(|path| !path.starts_with("/shared/package.json")
                && !path.starts_with("/shared/node_modules")),
        "{reads:?}"
    );
}

/// A module whose own directory is world-writable runs with relative
/// imports only: no bare specifier resolves, and a `.js` file has no
/// `"type"` scope, even with a marker beside it.
#[test]
fn a_world_writable_importer_dir_gets_relative_imports_only() {
    let tree = MemoryTree {
        files: BTreeMap::from([
            ("/tmp/package.json", r#"{"name": "x", "type": "module"}"#),
            ("/tmp/main.ts", ""),
            ("/tmp/a.ts", ""),
            ("/tmp/b.mjs", ""),
            ("/tmp/c.js", ""),
            (
                "/tmp/node_modules/foo/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/tmp/node_modules/foo/i.mjs", ""),
        ]),
        world_writable: vec!["/tmp"],
        ..MemoryTree::default()
    };
    let importer = Url::parse("file:///tmp/main.ts").unwrap();
    let from = |specifier| from_entry(&tree, specifier, &importer);
    assert_eq!(from("./a.ts").unwrap().url.as_str(), "file:///tmp/a.ts");
    assert_eq!(from("./b.mjs").unwrap().url.as_str(), "file:///tmp/b.mjs");
    let no_type_scope = ResolveErrorKind::NoTypeScope {
        stopped_at: Some(PathBuf::from("/tmp")),
    };
    assert_eq!(from("./c.js").unwrap_err().kind, no_type_scope);
    assert_eq!(
        resolve_entry(&tree, Path::new("c.js"), Path::new("/tmp"))
            .map(|(entry, _)| entry)
            .unwrap_err()
            .kind,
        no_type_scope
    );
    let error = from("foo").unwrap_err();
    assert_eq!(
        error.kind,
        ResolveErrorKind::NoProjectRoot {
            dir: PathBuf::from("/tmp"),
            stopped_at: Some(PathBuf::from("/tmp")),
        }
    );
    assert_eq!(
        error.to_string(),
        "cannot resolve 'foo' from file:///tmp/main.ts: no project root is above /tmp: a \
         bare specifier resolves only under a package.json outside node_modules that holds \
         more than \"type\" and \"sideEffects\", and the search for one stopped at /tmp, \
         which is world-writable, so nothing in it or above it is read. Fix: add a \
         package.json at the project root, with the package installed in the node_modules \
         beside it"
    );
    assert_eq!(
        from("./c.js").unwrap_err().kind.to_string(),
        "a .js file is an ES module only under a package.json with \"type\": \"module\", \
         and the search for one stopped at /tmp, which is world-writable, so nothing in it \
         or above it is read (the format is never guessed from the code). Fix: rename the \
         file to .mjs, or move it under a package.json with \"type\": \"module\" in a \
         directory that isn't world-writable"
    );
}

/// A project with packages installed under `/p/node_modules`: `foo`
/// with a `.mjs` main, a `.js` subpath, and a `dist/` subpath, and the
/// scoped `@s/bar`. The world-writable directories are `world_writable`.
fn installed(world_writable: Vec<&'static str>) -> CountingTree {
    CountingTree {
        tree: MemoryTree {
            files: BTreeMap::from([
                ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
                ("/p/main.ts", ""),
                (
                    "/p/node_modules/foo/package.json",
                    r#"{"type": "module", "exports": {".": "./i.mjs", "./x.js": "./x.js", "./d.mjs": "./dist/d.mjs"}}"#,
                ),
                ("/p/node_modules/foo/i.mjs", ""),
                ("/p/node_modules/foo/x.js", ""),
                ("/p/node_modules/foo/dist/d.mjs", ""),
                (
                    "/p/node_modules/@s/bar/package.json",
                    r#"{"exports": "./i.mjs"}"#,
                ),
                ("/p/node_modules/@s/bar/i.mjs", ""),
            ]),
            world_writable,
            ..MemoryTree::default()
        },
        reads: std::cell::RefCell::default(),
    }
}

/// A package the search finds through a world-writable directory is
/// refused, reading nothing in it: the `node_modules` it is installed
/// in, its scope's directory, its own root, or for a linked package the
/// real directories holding it.
#[test]
fn a_package_through_a_world_writable_dir_is_refused() {
    let importer = Url::parse("file:///p/main.ts").unwrap();
    let cases = [
        ("/p/node_modules", "foo", "foo"),
        ("/p/node_modules", "@s/bar", "@s/bar"),
        ("/p/node_modules/@s", "@s/bar", "@s/bar"),
        ("/p/node_modules/foo", "foo", "foo"),
        // the type-scope walk would stop at the root, but the package is refused first
        ("/p/node_modules/foo", "foo/x.js", "foo"),
        ("/p/node_modules/@s/bar", "@s/bar", "@s/bar"),
    ];
    for (dir, specifier, name) in cases {
        let tree = installed(vec![dir]);
        assert_eq!(
            from_entry(&tree, specifier, &importer).unwrap_err().kind,
            ResolveErrorKind::WorldWritablePackage {
                name: name.to_owned(),
                dir: PathBuf::from(dir),
            },
            "{dir} {specifier}"
        );
        let reads = tree.reads.borrow();
        assert!(
            reads.iter().all(|path| !path.starts_with(dir)),
            "{dir} {specifier} {reads:?}"
        );
    }
    // the outermost is named when several are
    let tree = installed(vec!["/p/node_modules", "/p/node_modules/foo"]);
    let error = from_entry(&tree, "foo", &importer).unwrap_err();
    assert_eq!(
        error.to_string(),
        "cannot resolve 'foo' from file:///p/main.ts: package 'foo' is found through \
         /p/node_modules, which is world-writable, so nothing in it is read: any user can \
         plant or replace a package there. Fix: remove the directory's other-write bit \
         (chmod o-w), then reinstall the package"
    );
    assert!(error.kind.is_specifier_error());
    // a module file below the root, in a world-writable directory, still loads
    let tree = installed(vec!["/p/node_modules/foo/dist"]);
    assert_eq!(
        from_entry(&tree, "foo/d.mjs", &importer)
            .unwrap()
            .url
            .as_str(),
        "file:///p/node_modules/foo/dist/d.mjs"
    );
    // a linked root is refused through the real directories holding it
    for (dir, specifier) in [
        ("/s/nm", "foo"),
        ("/s/nm", "@s/bar"),
        ("/s/nm/@s", "@s/bar"),
        ("/p/node_modules/.deno", "foo"),
        ("/p/node_modules/.deno/x", "@s/bar"),
    ] {
        let mut tree = installed(vec![dir]);
        let store = if dir.starts_with("/s/") {
            "/s/nm"
        } else {
            "/p/node_modules/.deno/x/node_modules"
        };
        for (from, to) in [
            ("/p/node_modules/foo", format!("{store}/foo")),
            ("/p/node_modules/@s/bar", format!("{store}/@s/bar")),
        ] {
            let to: &'static str = Box::leak(to.into_boxed_str());
            let moved: Vec<_> = tree
                .tree
                .files
                .keys()
                .copied()
                .filter(|path| path.starts_with(from))
                .collect();
            for path in moved {
                let content = tree.tree.files.remove(path).unwrap();
                let real: &'static str = Box::leak(path.replacen(from, to, 1).into_boxed_str());
                tree.tree.files.insert(real, content);
            }
            tree.tree.links.insert(from, to);
            tree.tree.dirs.push(from.rsplit_once('/').unwrap().0);
        }
        assert_eq!(
            from_entry(&tree, specifier, &importer).unwrap_err().kind,
            ResolveErrorKind::WorldWritablePackage {
                name: specifier.to_owned(),
                dir: PathBuf::from(dir),
            },
            "{dir} {specifier}"
        );
    }
}

/// A store's every directory between a linked root and what is already
/// checked is checked: a dependency installed beside its dependent in
/// the store, a store inside a symlinked `node_modules`, and a package
/// under a symlinked scope directory.
#[test]
fn a_package_in_a_store_is_refused_through_its_every_holder() {
    // `a` in Deno's store imports `b`, linked beside it to `b`'s own directory
    let store = |world_writable| MemoryTree {
        files: BTreeMap::from([
            ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
            (
                "/p/node_modules/.deno/a@1/node_modules/a/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/p/node_modules/.deno/a@1/node_modules/a/i.mjs", ""),
            (
                "/p/node_modules/.deno/b@1/node_modules/b/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/p/node_modules/.deno/b@1/node_modules/b/i.mjs", ""),
        ]),
        links: BTreeMap::from([(
            "/p/node_modules/.deno/a@1/node_modules/b",
            "/p/node_modules/.deno/b@1/node_modules/b",
        )]),
        world_writable,
        ..MemoryTree::default()
    };
    let importer = Url::parse("file:///p/node_modules/.deno/a@1/node_modules/a/i.mjs").unwrap();
    assert!(from_entry(&store(vec![]), "b", &importer).is_ok());
    for dir in [
        "/p/node_modules/.deno/b@1",
        "/p/node_modules/.deno/b@1/node_modules",
    ] {
        assert_eq!(
            from_entry(&store(vec![dir]), "b", &importer)
                .unwrap_err()
                .kind,
            ResolveErrorKind::WorldWritablePackage {
                name: "b".to_owned(),
                dir: PathBuf::from(dir)
            },
            "{dir}"
        );
    }
    // `/p/node_modules` links to `/q/nm`, whose store holds `foo`, linked
    // relatively, as Deno links it
    let linked = |world_writable| MemoryTree {
        files: BTreeMap::from([
            ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
            ("/p/main.ts", ""),
            (
                "/q/nm/.deno/foo@1/node_modules/foo/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/q/nm/.deno/foo@1/node_modules/foo/i.mjs", ""),
        ]),
        links: BTreeMap::from([
            ("/p/node_modules", "/q/nm"),
            ("/q/nm/foo", ".deno/foo@1/node_modules/foo"),
        ]),
        world_writable,
        ..MemoryTree::default()
    };
    let importer = Url::parse("file:///p/main.ts").unwrap();
    assert!(from_entry(&linked(vec![]), "foo", &importer).is_ok());
    for dir in [
        "/q",
        "/q/nm/.deno",
        "/q/nm/.deno/foo@1",
        "/q/nm/.deno/foo@1/node_modules",
    ] {
        assert_eq!(
            from_entry(&linked(vec![dir]), "foo", &importer)
                .unwrap_err()
                .kind,
            ResolveErrorKind::WorldWritablePackage {
                name: "foo".to_owned(),
                dir: PathBuf::from(dir)
            },
            "{dir}"
        );
    }
    // `/p/node_modules/@s` links to `/x/@s`, whose `bar` links to `/y/bar`
    let scoped = |world_writable| MemoryTree {
        files: BTreeMap::from([
            ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
            ("/p/main.ts", ""),
            ("/y/bar/package.json", r#"{"exports": "./i.mjs"}"#),
            ("/y/bar/i.mjs", ""),
        ]),
        dirs: vec!["/p/node_modules"],
        links: BTreeMap::from([("/p/node_modules/@s", "/x/@s"), ("/x/@s/bar", "/y/bar")]),
        world_writable,
        ..MemoryTree::default()
    };
    assert!(from_entry(&scoped(vec![]), "@s/bar", &importer).is_ok());
    for dir in ["/x", "/y"] {
        assert_eq!(
            from_entry(&scoped(vec![dir]), "@s/bar", &importer)
                .unwrap_err()
                .kind,
            ResolveErrorKind::WorldWritablePackage {
                name: "@s/bar".to_owned(),
                dir: PathBuf::from(dir)
            },
            "{dir}"
        );
    }
}

/// A world-writable directory inside `node_modules` ends the walk for
/// the project root, as one outside it does, before the search reaches
/// it.
#[test]
fn a_world_writable_node_modules_ends_the_walk_for_the_bound() {
    let tree = installed(vec!["/p/node_modules"]);
    let importer = Url::parse("file:///p/node_modules/foo/i.mjs").unwrap();
    assert_eq!(
        from_entry(&tree, "@s/bar", &importer).unwrap_err().kind,
        ResolveErrorKind::NoProjectRoot {
            dir: PathBuf::from("/p/node_modules/foo"),
            stopped_at: Some(PathBuf::from("/p/node_modules")),
        }
    );
}

/// A self-reference needs no project root: it reads only the importer's
/// own `package.json`, so it resolves through its own package's
/// `"exports"`, and nothing else bare does.
#[test]
fn a_self_reference_needs_no_project_root() {
    let tree = MemoryTree {
        files: BTreeMap::from([
            (
                "/x/node_modules/foo/package.json",
                r#"{"name": "foo", "exports": {"./a.mjs": "./a.mjs"}}"#,
            ),
            ("/x/node_modules/foo/a.mjs", ""),
            ("/x/node_modules/foo/b.mjs", ""),
            (
                "/x/node_modules/bar/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/x/node_modules/bar/i.mjs", ""),
        ]),
        ..MemoryTree::default()
    };
    let importer = Url::parse("file:///x/node_modules/foo/b.mjs").unwrap();
    let from = |specifier| from_entry(&tree, specifier, &importer);
    assert_eq!(
        from("foo/a.mjs").unwrap().url.as_str(),
        "file:///x/node_modules/foo/a.mjs"
    );
    assert_eq!(
        from("bar").unwrap_err().kind,
        ResolveErrorKind::NoProjectRoot {
            dir: PathBuf::from("/x/node_modules/foo"),
            stopped_at: None,
        }
    );
}

/// A tree whose reads of `package.json` files and mode checks of
/// directories fail, by path.
struct FailingTree {
    tree: MemoryTree,
    unreadable: &'static str,
    unstatable: &'static str,
}

impl SourceTree for FailingTree {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        self.tree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        if path == Path::new(self.unreadable) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        self.tree.read(path, id)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        if dir == Path::new(self.unstatable) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        self.tree.is_world_writable(dir)
    }
}

/// A failure to read on the walk names the path that failed.
#[test]
fn a_read_failure_names_its_path() {
    let tree = FailingTree {
        tree: packages(),
        unreadable: "/w/proj/dist/package.json",
        unstatable: "/w/proj/scripts",
    };
    let from = |importer: &str| {
        let importer = Url::parse(importer).unwrap();
        from_entry(&tree, "t", &importer).unwrap_err().kind
    };
    let error = from("file:///w/proj/dist/esm/x.js");
    assert_eq!(
        error,
        ResolveErrorKind::Io {
            path: PathBuf::from("/w/proj/dist/package.json"),
            error: "permission denied".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "failed to read /w/proj/dist/package.json: permission denied"
    );
    assert!(!error.is_specifier_error());
    assert_eq!(
        from("file:///w/proj/scripts/x.mjs"),
        ResolveErrorKind::Io {
            path: PathBuf::from("/w/proj/scripts"),
            error: "permission denied".to_owned(),
        }
    );
}

/// A failure to read a directory's mode on the walk for the project
/// root fails the import, inside `node_modules` too, rather than
/// passing the directory.
#[test]
fn a_mode_failure_on_the_walk_for_the_bound_fails() {
    let mut tree = installed(Vec::new()).tree;
    // a scope below the unreadable directory, so the scope's walk never reaches it
    tree.files
        .insert("/p/node_modules/foo/lib/package.json", MODULE);
    tree.files.insert("/p/node_modules/foo/lib/x.mjs", "");
    let tree = FailingTree {
        tree,
        unreadable: "",
        unstatable: "/p/node_modules/foo",
    };
    let importer = Url::parse("file:///p/node_modules/foo/lib/x.mjs").unwrap();
    assert_eq!(
        from_entry(&tree, "@s/bar", &importer).unwrap_err().kind,
        ResolveErrorKind::Io {
            path: PathBuf::from("/p/node_modules/foo"),
            error: "permission denied".to_owned(),
        }
    );
}

/// World-writable is the other-write bit alone: group-write doesn't
/// count, and the sticky bit changes nothing.
#[cfg(unix)]
#[test]
fn world_writable_is_the_other_write_bit() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = std::env::temp_dir().join(format!("gro_resolver_mode_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let modes = [0o755, 0o775, 0o770, 0o757, 0o777, 0o1777, 0o1775, 0o702];
    let writable = modes.map(|mode| {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
        DiskTree.is_world_writable(&dir).unwrap()
    });
    std::fs::remove_dir(&dir).unwrap();
    assert_eq!(
        writable,
        [false, false, false, true, true, true, false, true]
    );
}
