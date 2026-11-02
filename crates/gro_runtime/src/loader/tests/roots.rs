//! Tests for roots: conflicts, each record's root, re-resolving, and
//! `import.meta.resolve`.

use super::*;

/// A file reached both by a relative specifier and through its package
/// is refused in either order, never loaded under the first one's root.
#[test]
fn a_root_conflict_is_refused_in_either_order() {
    let tree = tree();
    // the package first: the relative specifier is refused by the resolver
    let loader = new_loader(&tree);
    loader
        .resolve_reached(&tree, "lnk/x.ts", MAIN, STATIC)
        .unwrap();
    let error = loader
        .resolve_reached(&tree, "./packages/lnk/x.ts", MAIN, STATIC)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(
            "cannot resolve './packages/lnk/x.ts' from file:///p/main.ts: it resolves to \
             /p/packages/lnk/x.ts, inside package 'lnk' at /p/packages/lnk, which only a \
             bare specifier reaches"
        ),
        "{error}"
    );
    // the relative specifier first: admitting the package is the conflict
    let loader = new_loader(&tree);
    loader
        .resolve_reached(&tree, "./packages/lnk/x.ts", MAIN, STATIC)
        .unwrap();
    let before = state(&loader);
    let error = loader
        .resolve_reached(&tree, "lnk/x.ts", MAIN, STATIC)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(
            "cannot resolve 'lnk/x.ts' from file:///p/main.ts: /p/packages/lnk/x.ts was \
             resolved inside the project at /p, and package 'lnk' at /p/packages/lnk was \
             then admitted around it, so its origin would depend on the import order. Fix: \
             import it by name ('lnk' or a subpath it exports) everywhere, not by a relative \
             path"
        ),
        "{error}"
    );
    // nothing was admitted, so it fails the same way again
    assert_eq!(state(&loader), before);
    assert!(
        loader
            .resolve_reached(&tree, "lnk/x.ts", MAIN, STATIC)
            .is_err()
    );
}

/// Each record's root is the one its path is inside, which the loader
/// relies on to resolve from the importer's record without walking its
/// path: across project, installed, in-tree linked, and out-of-tree
/// linked roots, their modules' own imports, and a query.
#[test]
fn a_record_s_root_is_the_one_its_path_is_inside() {
    let tree = MemoryTree {
        files: BTreeMap::from([
            ("/p/package.json", r#"{"name": "proj", "type": "module"}"#),
            ("/p/main.ts", ""),
            ("/p/a.ts", ""),
            (
                "/p/packages/lnk/package.json",
                r#"{"name": "lnk", "type": "module", "exports": {"./x.ts": "./x.ts"}}"#,
            ),
            ("/p/packages/lnk/x.ts", ""),
            ("/p/packages/lnk/y.ts", ""),
            (
                "/p/node_modules/bar/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("/p/node_modules/bar/i.mjs", ""),
            ("/p/node_modules/bar/sub/j.mjs", ""),
            (
                "/q/far/package.json",
                r#"{"name": "far", "type": "module", "exports": "./f.ts"}"#,
            ),
            ("/q/far/f.ts", ""),
            ("/q/far/g.ts", ""),
        ]),
        links: BTreeMap::from([
            ("/p/node_modules/lnk", "/p/packages/lnk"),
            ("/p/node_modules/far", "/q/far"),
        ]),
        ..MemoryTree::default()
    };
    let loader = new_loader(&tree);
    for (specifier, referrer) in [
        ("./a.ts?v=1", MAIN),
        ("bar", MAIN),
        ("./sub/j.mjs", "file:///p/node_modules/bar/i.mjs"),
        ("lnk/x.ts", MAIN),
        ("./y.ts", "file:///p/packages/lnk/x.ts"),
        ("far", MAIN),
        ("./g.ts", "file:///q/far/f.ts"),
    ] {
        loader
            .resolve_reached(&tree, specifier, referrer, STATIC)
            .unwrap_or_else(|error| panic!("{specifier} from {referrer}: {error}"));
    }
    let (roots, records) = state(&loader);
    assert_eq!(records.len(), 8);
    for (url, record) in &records {
        let path = url.to_file_path().unwrap();
        assert_eq!(roots.containing(&path), Some(&record.root), "{url}");
    }
}

/// `deno_core` can resolve a specifier twice: the same URL inside the
/// same root changes nothing.
#[test]
fn resolving_twice_is_no_change() {
    let tree = tree();
    let loader = new_loader(&tree);
    let first = loader.resolve_reached(&tree, "bar", MAIN, STATIC).unwrap();
    let after_first = state(&loader);
    let second = loader.resolve_reached(&tree, "bar", MAIN, STATIC).unwrap();
    assert_eq!(first, second);
    assert_eq!(state(&loader), after_first);
    assert_eq!(after_first.0.iter().count(), 2);
    assert_eq!(after_first.1.len(), 2);
}

/// `import.meta.resolve` resolves as an import does but admits and
/// records nothing, so importing what it returned for a package is a
/// path into `node_modules`, refused.
#[test]
fn import_meta_resolve_changes_nothing() {
    let tree = tree();
    let loader = new_loader(&tree);
    let before = state(&loader);
    let url = loader
        .resolve_reached(&tree, "bar", MAIN, Reached::MetaResolve)
        .unwrap();
    assert_eq!(url.as_str(), "file:///p/node_modules/bar/i.mjs");
    assert_eq!(state(&loader), before);
    let error = loader
        .resolve_reached(&tree, url.as_str(), MAIN, STATIC)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("inside node_modules below /p") && error.contains("import it by name"),
        "{error}"
    );
    // refused as an import is
    assert!(
        loader
            .resolve_reached(
                &tree,
                "./node_modules/bar/i.mjs",
                MAIN,
                Reached::MetaResolve
            )
            .is_err()
    );
    // and an importer must have been resolved here
    let error = loader
        .resolve_reached(&tree, "./main.ts", url.as_str(), Reached::MetaResolve)
        .unwrap_err()
        .to_string();
    assert!(error.contains("was never resolved"), "{error}");
}
