//! The builtin table: its rows, and a bare specifier it names resolving to
//! the builtin ahead of any package, reading nothing.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use url::Url;

use super::{kind, resolve, resolve_any, roots};
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{
    BUILTINS, Capability, Entry, FileId, Origin, Reserved, ResolveErrorKind, Resolved, Root,
    SourceTree, UNBUILT_HOST_MODULES, find_builtin, find_reserved,
};

/// The table's rows, pinned: a row added, removed, or regranted fails here
/// and gets a look.
#[test]
fn the_table_is_pinned() {
    let rows: Vec<_> = BUILTINS
        .iter()
        .map(|builtin| {
            (
                builtin.specifier,
                builtin.url,
                builtin.grant,
                builtin.exports,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [(
            "@fuzdev/gro/process.ts",
            "builtin:@fuzdev/gro/process.ts",
            Some(Capability::Process),
            &["process"][..],
        )]
    );
}

/// Every row's specifier is unique and can be spliced into a JS string
/// literal and a URL as is, and its URL is the specifier in the `builtin:`
/// scheme, a URL the resolver refuses from source.
#[test]
fn every_row_is_well_formed() {
    for builtin in BUILTINS {
        let specifier = builtin.specifier;
        assert!(
            specifier
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"@/._-".contains(&byte)),
            "{specifier}"
        );
        assert_eq!(builtin.url, format!("builtin:{specifier}"));
        assert_eq!(
            find_builtin(specifier).map(|found| found.url),
            Some(builtin.url)
        );
        assert_eq!(
            kind(builtin.url),
            ResolveErrorKind::UnsupportedScheme("builtin".to_owned())
        );
        assert!(!builtin.exports.contains(&"default"), "{specifier}");
    }
}

/// A tree that records every call, of any kind.
struct Watched {
    tree: MemoryTree,
    calls: RefCell<Vec<PathBuf>>,
}

impl SourceTree for Watched {
    fn lookup(&self, path: &Path) -> io::Result<Option<Entry>> {
        self.calls.borrow_mut().push(path.to_owned());
        self.tree.lookup(path)
    }

    fn read(&self, path: &Path, id: FileId) -> io::Result<Vec<u8>> {
        self.calls.borrow_mut().push(path.to_owned());
        self.tree.read(path, id)
    }

    fn is_world_writable(&self, dir: &Path) -> io::Result<bool> {
        self.calls.borrow_mut().push(dir.to_owned());
        self.tree.is_world_writable(dir)
    }
}

/// A project named `@fuzdev/gro` exporting `./process.ts` itself, with the
/// npm package installed too and a package `dep` beside it, so a package
/// lookup of the builtin's specifier would succeed twice over.
fn shadowed() -> MemoryTree {
    MemoryTree {
        files: BTreeMap::from([
            (
                "/p/package.json",
                r#"{"name": "@fuzdev/gro", "type": "module", "exports": {"./process.ts": "./process.ts"}}"#,
            ),
            ("/p/main.js", ""),
            ("/p/process.ts", ""),
            (
                "/p/node_modules/@fuzdev/gro/package.json",
                r#"{"name": "@fuzdev/gro", "type": "module", "exports": {"./process.ts": "./process.ts", "./process.js": "./process.ts"}}"#,
            ),
            ("/p/node_modules/@fuzdev/gro/process.ts", ""),
            (
                "/p/node_modules/dep/package.json",
                r#"{"name": "dep", "type": "module"}"#,
            ),
            ("/p/node_modules/dep/index.js", ""),
        ]),
        ..MemoryTree::default()
    }
}

/// The table's specifier resolves to its builtin from the project and from
/// a package alike, ahead of a self-reference and an installed package of
/// that name, with no lookup or read of any file.
#[test]
fn a_table_hit_reads_nothing() {
    let tree = shadowed();
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
    for importer in ["file:///p/main.js", "file:///p/node_modules/dep/index.js"] {
        let importer = Url::parse(importer).unwrap();
        let resolved = resolve_any(&watched, "@fuzdev/gro/process.ts", &importer, &roots);
        assert_eq!(resolved, Ok(Resolved::Builtin(&BUILTINS[0])), "{importer}");
    }
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
}

/// Only the exact specifier is the builtin: another spelling is a package
/// specifier like any other, or one the table reserves.
#[test]
fn near_misses_are_packages() {
    let tree = shadowed();
    let main = Url::parse("file:///p/main.js").unwrap();
    let roots = roots(&tree, &main);
    // a self-reference, so a package lookup, refused for what the project doesn't export
    assert!(matches!(
        resolve_any(&tree, "@fuzdev/gro/process", &main, &roots),
        Err(error) if matches!(error.kind, ResolveErrorKind::NotExported { .. })
    ));
    for specifier in [
        "@fuzdev/gro/process.js",
        "@fuzdev/gro/process",
        "@fuzdev/gro/./process.ts",
        "@FUZDEV/gro/process.ts",
        "@fuzdev/gro/process.ts?x",
        "@fuzdev/gro/process.ts ",
    ] {
        assert!(
            !matches!(
                resolve_any(&tree, specifier, &main, &roots),
                Ok(Resolved::Builtin(_))
            ),
            "{specifier}"
        );
    }
}

/// Every specifier the table reserves, pinned: the `.js` spelling of each
/// row's, and both spellings of each host module not built yet. None is a
/// row's, and each unbuilt one is a host module of the npm package's.
#[test]
fn the_reserved_specifiers_are_pinned() {
    let process = &BUILTINS[0];
    let reserved: Vec<_> = [
        "@fuzdev/gro/process.js",
        "@fuzdev/gro/fs.ts",
        "@fuzdev/gro/fs.js",
        "@fuzdev/gro/spawn.ts",
        "@fuzdev/gro/spawn.js",
        "@fuzdev/gro/net.ts",
        "@fuzdev/gro/net.js",
        "@fuzdev/gro/watch.ts",
        "@fuzdev/gro/watch.js",
    ]
    .into_iter()
    .map(|specifier| (specifier, find_reserved(specifier)))
    .collect();
    assert_eq!(
        reserved,
        [
            (
                "@fuzdev/gro/process.js",
                Some(Reserved::JsSpelling(process))
            ),
            (
                "@fuzdev/gro/fs.ts",
                Some(Reserved::Unbuilt("@fuzdev/gro/fs"))
            ),
            (
                "@fuzdev/gro/fs.js",
                Some(Reserved::Unbuilt("@fuzdev/gro/fs"))
            ),
            (
                "@fuzdev/gro/spawn.ts",
                Some(Reserved::Unbuilt("@fuzdev/gro/spawn"))
            ),
            (
                "@fuzdev/gro/spawn.js",
                Some(Reserved::Unbuilt("@fuzdev/gro/spawn"))
            ),
            (
                "@fuzdev/gro/net.ts",
                Some(Reserved::Unbuilt("@fuzdev/gro/net"))
            ),
            (
                "@fuzdev/gro/net.js",
                Some(Reserved::Unbuilt("@fuzdev/gro/net"))
            ),
            (
                "@fuzdev/gro/watch.ts",
                Some(Reserved::Unbuilt("@fuzdev/gro/watch"))
            ),
            (
                "@fuzdev/gro/watch.js",
                Some(Reserved::Unbuilt("@fuzdev/gro/watch"))
            ),
        ]
    );
    for builtin in BUILTINS {
        assert_eq!(
            find_reserved(builtin.specifier),
            None,
            "{}",
            builtin.specifier
        );
    }
    // the grant-free error modules load from the package, and nothing else is reserved
    for specifier in [
        "@fuzdev/gro/fs_error.ts",
        "@fuzdev/gro/process_error.js",
        "@fuzdev/gro/fs_search.ts",
        "@fuzdev/gro/fs",
        "@fuzdev/gro/fs.mts",
        "@fuzdev/gro/process",
        "@fuzdev/gro/process.mjs",
        "@fuzdev/grox/fs.ts",
        "fs.ts",
    ] {
        assert_eq!(find_reserved(specifier), None, "{specifier}");
    }
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/lib");
    for unbuilt in UNBUILT_HOST_MODULES {
        let name = unbuilt.strip_prefix("@fuzdev/gro/").unwrap();
        assert!(lib.join(format!("{name}.ts")).is_file(), "{unbuilt}");
        assert_eq!(find_builtin(&format!("{unbuilt}.ts")), None, "{unbuilt}");
    }
}

/// A reserved specifier is refused ahead of any package, from the project
/// and from a package alike, with no lookup or read of any file: not the
/// project's self-reference of that name (a project named `@fuzdev/gro`,
/// as gro's own checkout is) nor the installed package that exports it.
#[test]
fn a_reserved_specifier_is_refused_reading_nothing() {
    let mut files = shadowed().files;
    files.insert(
        "/p/package.json",
        r#"{"name": "@fuzdev/gro", "type": "module", "exports": {"./*.ts": "./*.ts", "./*.js": "./*.ts"}}"#,
    );
    files.insert(
        "/p/node_modules/@fuzdev/gro/package.json",
        r#"{"name": "@fuzdev/gro", "type": "module", "exports": {"./*.ts": "./*.ts", "./*.js": "./*.ts"}}"#,
    );
    files.insert("/p/fs.ts", "");
    files.insert("/p/fs_error.ts", "");
    files.insert("/p/node_modules/@fuzdev/gro/fs.ts", "");
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
    // unreserved, the self-reference resolves the project's own file
    assert!(matches!(
        resolve_any(&tree, "@fuzdev/gro/fs_error.ts", &main, &roots),
        Ok(Resolved::Module(resolution)) if resolution.url.as_str() == "file:///p/fs_error.ts"
    ));
    let watched = Watched {
        tree,
        calls: RefCell::default(),
    };
    for importer in ["file:///p/main.js", "file:///p/node_modules/dep/index.js"] {
        let importer = Url::parse(importer).unwrap();
        for (specifier, reserved, message) in [
            (
                "@fuzdev/gro/process.js",
                Reserved::JsSpelling(&BUILTINS[0]),
                "it is the .js spelling of the builtin module '@fuzdev/gro/process.ts', reserved \
                 so it never resolves to a package. Fix: import '@fuzdev/gro/process.ts'",
            ),
            (
                "@fuzdev/gro/fs.ts",
                Reserved::Unbuilt("@fuzdev/gro/fs"),
                "it names Gro's host module '@fuzdev/gro/fs.ts', which isn't on fgro yet, and a \
                 host module's specifier never resolves to a package. Fix: run this under Node \
                 with gro",
            ),
            (
                "@fuzdev/gro/fs.js",
                Reserved::Unbuilt("@fuzdev/gro/fs"),
                "it names Gro's host module '@fuzdev/gro/fs.ts', which isn't on fgro yet, and a \
                 host module's specifier never resolves to a package. Fix: run this under Node \
                 with gro",
            ),
        ] {
            let error = resolve_any(&watched, specifier, &importer, &roots).unwrap_err();
            assert_eq!(
                error.kind,
                ResolveErrorKind::Reserved(reserved),
                "{importer}"
            );
            assert!(error.kind.is_refusal());
            assert_eq!(
                error.to_string(),
                format!("cannot resolve '{specifier}' from {importer}: {message}")
            );
        }
    }
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
}

/// A builtin's or reserved specifier respelled, as URL parsing and the
/// path's decoding read it, is reserved too: through the resolver's own
/// `"exports"` and decoding, each respelling reaches the file its plain
/// spelling does, in a package whose pattern (`"./*"`) keeps the whole
/// subpath, so none reaches the npm package. A spelling that names another
/// file, or that the package lookup refuses anyway, is no respelling.
#[test]
fn a_respelled_specifier_is_reserved() {
    let process = &BUILTINS[0];
    let fs = Reserved::Unbuilt("@fuzdev/gro/fs");
    let cases = [
        ("proces%73.ts", "process.ts", Reserved::Respelled(process)),
        ("process.ts?x", "process.ts", Reserved::Respelled(process)),
        ("process.ts#x", "process.ts", Reserved::Respelled(process)),
        ("pro\tcess.ts", "process.ts", Reserved::Respelled(process)),
        ("proces%73.js", "process.js", Reserved::JsSpelling(process)),
        (
            "process.js#.ts",
            "process.js",
            Reserved::JsSpelling(process),
        ),
        ("f%73.ts", "fs.ts", fs),
        ("f%73.js", "fs.js", fs),
        ("fs%2ets", "fs.ts", fs),
        ("fs.js#.ts", "fs.js", fs),
        ("fs.js?.ts", "fs.js", fs),
        ("fs.ts?x", "fs.ts", fs),
        ("f\ts.ts", "fs.ts", fs),
        ("fs.ts\n", "fs.ts", fs),
        ("fs.ts ", "fs.ts", fs),
        (
            "%77atch.ts",
            "watch.ts",
            Reserved::Unbuilt("@fuzdev/gro/watch"),
        ),
    ];
    let tree = MemoryTree {
        files: BTreeMap::from([
            ("/p/package.json", r#"{"name": "app", "type": "module"}"#),
            ("/p/main.js", ""),
            (
                "/p/node_modules/other/package.json",
                r#"{"name": "other", "type": "module", "exports": {"./*": "./*"}}"#,
            ),
            ("/p/node_modules/other/process.ts", ""),
            ("/p/node_modules/other/process.js", ""),
            ("/p/node_modules/other/fs.ts", ""),
            ("/p/node_modules/other/fs.js", ""),
            ("/p/node_modules/other/watch.ts", ""),
        ]),
        ..MemoryTree::default()
    };
    let main = Url::parse("file:///p/main.js").unwrap();
    let roots = roots(&tree, &main);
    for (respelled, plain, reserved) in cases {
        let specifier = format!("@fuzdev/gro/{respelled}");
        assert_eq!(find_reserved(&specifier), Some(reserved), "{specifier:?}");
        let resolution = resolve(&tree, &format!("other/{respelled}"), &main, &roots)
            .unwrap_or_else(|error| panic!("{respelled:?}: {error}"));
        assert_eq!(
            resolution.url.to_file_path().unwrap(),
            Path::new("/p/node_modules/other").join(plain),
            "{respelled:?}"
        );
    }
    assert_eq!(
        ResolveErrorKind::Reserved(Reserved::Respelled(process)).to_string(),
        "it respells the builtin module '@fuzdev/gro/process.ts', reserved so no spelling of it \
         resolves to a package. Fix: import '@fuzdev/gro/process.ts'"
    );
    for specifier in [
        "@fuzdev/gro/fs_paths.ts",
        "@fuzdev/gro/fs.ts.map",
        "@fuzdev/gro/fs_error.ts?x",
        "@fuzdev/gro/process_error.ts#x",
        "@fuzdev/gro/f%7.ts",
        "@fuzdev/gro/f%zzs.ts",
        "@fuzdev/gro/F%73.ts",
        "@fuzdev/gro/ fs.ts",
        "@fuzdev/gro/fs.ts/",
        "@fuzdev/gro/fs.ts%2f",
        "@fuzdev/gro/fs.ts%00",
        "@fuzdev/gro/x/../fs.ts",
        "@fuzdev/gro/x/%2e%2e/fs.ts",
        "@fuzdev/gro/x\\..\\fs.ts",
        "@fuzdev/grox/f%73.ts",
        "@fuzdev/gr%6f/fs.ts",
        "other/f%73.ts",
    ] {
        assert_eq!(find_reserved(specifier), None, "{specifier:?}");
    }
}

/// A respelled specifier is refused ahead of any package, from the project
/// and from a package alike, with no lookup or read of any file, while a
/// neighbor that isn't one still resolves through the package.
#[test]
fn a_respelled_specifier_is_refused_reading_nothing() {
    let mut files = shadowed().files;
    files.insert(
        "/p/node_modules/@fuzdev/gro/package.json",
        r#"{"name": "@fuzdev/gro", "type": "module", "exports": {"./*.ts": "./*.ts", "./*.js": "./*.ts"}}"#,
    );
    files.insert("/p/node_modules/@fuzdev/gro/fs.ts", "");
    files.insert("/p/node_modules/@fuzdev/gro/fs_paths.ts", "");
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
    for importer in ["file:///p/main.js", "file:///p/node_modules/dep/index.js"] {
        let importer = Url::parse(importer).unwrap();
        for (specifier, reserved) in [
            (
                "@fuzdev/gro/proces%73.ts",
                Reserved::Respelled(&BUILTINS[0]),
            ),
            (
                "@fuzdev/gro/process.js#.ts",
                Reserved::JsSpelling(&BUILTINS[0]),
            ),
            ("@fuzdev/gro/f%73.ts", Reserved::Unbuilt("@fuzdev/gro/fs")),
        ] {
            let error = resolve_any(&watched, specifier, &importer, &roots).unwrap_err();
            assert_eq!(
                error.kind,
                ResolveErrorKind::Reserved(reserved),
                "{specifier} from {importer}"
            );
        }
    }
    assert_eq!(*watched.calls.borrow(), Vec::<PathBuf>::new());
    // from the package, since the project is a self-reference of that name
    let dep = Url::parse("file:///p/node_modules/dep/index.js").unwrap();
    let resolution = resolve(&watched, "@fuzdev/gro/fs_paths.ts", &dep, &roots)
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        resolution.url.as_str(),
        "file:///p/node_modules/@fuzdev/gro/fs_paths.ts"
    );
}

/// Every spelling a package's `"exports"` turns into a table module's file
/// is reserved, through a `"./*"` package and through one shaped as gro's
/// own (`"./*.ts"` and `"./*.js"` onto `./dist/*.js`): each table subpath
/// with each character percent-encoded, or with one of the characters URL
/// parsing reads specially inserted at each position, is either reserved
/// or reaches no table module through either package.
#[test]
fn every_respelling_the_lookup_reads_as_a_table_module_is_reserved() {
    let stems = ["process", "fs", "spawn", "net", "watch"];
    let files = BTreeMap::from([
        ("/p/package.json", r#"{"name": "app", "type": "module"}"#),
        ("/p/main.js", ""),
        (
            "/p/node_modules/any/package.json",
            r#"{"name": "any", "type": "module", "exports": {"./*": "./*"}}"#,
        ),
        (
            "/p/node_modules/shaped/package.json",
            r#"{"name": "shaped", "type": "module", "exports": {"./*.ts": "./dist/*.js", "./*.js": "./dist/*.js"}}"#,
        ),
        ("/p/node_modules/any/process.ts", ""),
        ("/p/node_modules/any/process.js", ""),
        ("/p/node_modules/shaped/dist/process.js", ""),
        ("/p/node_modules/any/fs.ts", ""),
        ("/p/node_modules/any/fs.js", ""),
        ("/p/node_modules/shaped/dist/fs.js", ""),
        ("/p/node_modules/any/spawn.ts", ""),
        ("/p/node_modules/any/spawn.js", ""),
        ("/p/node_modules/shaped/dist/spawn.js", ""),
        ("/p/node_modules/any/net.ts", ""),
        ("/p/node_modules/any/net.js", ""),
        ("/p/node_modules/shaped/dist/net.js", ""),
        ("/p/node_modules/any/watch.ts", ""),
        ("/p/node_modules/any/watch.js", ""),
        ("/p/node_modules/shaped/dist/watch.js", ""),
    ]);
    let tree = MemoryTree {
        files,
        ..MemoryTree::default()
    };
    let main = Url::parse("file:///p/main.js").unwrap();
    let roots = roots(&tree, &main);
    let inserts = [
        "%2e", "%2E", "%2f", "%5c", "%25", "%20", "%09", "%0a", "%3f", "%23", "\t", "\n", "\r",
        "?", "#", "?.ts", "#.ts", "/", "\\", " ", "./", "x/../", "\u{0}", "\u{1}", "\u{c}",
    ];
    let mut rests = Vec::new();
    for stem in stems {
        for extension in ["ts", "js"] {
            let plain: Vec<char> = format!("{stem}.{extension}").chars().collect();
            for index in 0..plain.len() {
                let (before, after) = plain.split_at(index);
                let before: String = before.iter().collect();
                let rest: String = after[1..].iter().collect();
                let code = u32::from(after[0]);
                rests.push(format!("{before}%{code:02x}{rest}"));
                rests.push(format!("{before}%{code:02X}{rest}"));
            }
            for index in 0..=plain.len() {
                let (before, after) = plain.split_at(index);
                let before: String = before.iter().collect();
                let after: String = after.iter().collect();
                for insert in inserts {
                    rests.push(format!("{before}{insert}{after}"));
                }
            }
        }
    }
    for rest in rests {
        let reserved = find_reserved(&format!("@fuzdev/gro/{rest}")).is_some();
        for (package, dir) in [
            ("any", "/p/node_modules/any/"),
            ("shaped", "/p/node_modules/shaped/dist/"),
        ] {
            let Ok(resolution) = resolve(&tree, &format!("{package}/{rest}"), &main, &roots) else {
                continue;
            };
            let path = resolution.url.to_file_path().unwrap();
            let reached = path
                .strip_prefix(dir)
                .ok()
                .and_then(|name| name.file_stem()?.to_str())
                .is_some_and(|stem| stems.contains(&stem));
            assert!(
                !reached || reserved,
                "{rest:?} reaches {} through {package}",
                path.display()
            );
        }
    }
}
