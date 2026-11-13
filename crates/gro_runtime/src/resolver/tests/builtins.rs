//! The builtin table: its rows, and a bare specifier it names resolving to
//! the builtin ahead of any package, reading nothing.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use url::Url;

use super::{kind, resolve_any, roots};
use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{
    BUILTINS, Capability, Entry, FileId, Origin, ResolveErrorKind, Resolved, Root, SourceTree,
    find_builtin,
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
/// specifier like any other.
#[test]
fn near_misses_are_packages() {
    let tree = shadowed();
    let main = Url::parse("file:///p/main.js").unwrap();
    let roots = roots(&tree, &main);
    // a self-reference, so a package lookup, refused for what the project doesn't export
    assert!(matches!(
        resolve_any(&tree, "@fuzdev/gro/process.js", &main, &roots),
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
