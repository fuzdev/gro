//! A builtin through the loader: the grant check at link time, by the run's
//! authority and the importer's origin, and what the graph records.

use super::*;
use crate::resolver::{BUILTINS, Origin, ResolveErrorKind};
use crate::{Authority, Grants, ProcessGrant};

const PROCESS: &str = "@fuzdev/gro/process.ts";
const BAR: &str = "file:///p/node_modules/bar/i.mjs";

/// `fgro run`'s authority: `process` granted.
fn granted() -> Authority {
    Authority::Confined(Grants {
        process: Some(ProcessGrant {}),
    })
}

/// A loader of `tree`'s graph with `bar` admitted, granting as `authority`
/// says, none when unset.
fn loader(tree: &MemoryTree, authority: Option<Authority>) -> Loader {
    let loader = new_loader(tree);
    let loader = match authority {
        Some(authority) => loader.with_authority(authority),
        None => loader,
    };
    loader.resolve_reached(tree, "bar", MAIN, STATIC).unwrap();
    loader
}

fn bar_origin() -> Origin {
    Origin::Package {
        name: "bar".to_owned(),
        version: None,
        linked: false,
    }
}

/// Asserts `loader` refuses `process` to `importer`, reached as `reached`,
/// with a message ending in `why`, recording [`FailureReason::NotGranted`]
/// with `origin`.
fn assert_refused(loader: &Loader, importer: &str, reached: Reached, origin: &Origin, why: &str) {
    let tree = tree();
    let error = loader
        .resolve_reached(&tree, PROCESS, importer, reached)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        format!("cannot import '{PROCESS}' from {importer}: it needs the grant 'process', {why}")
    );
    let graph = graph(loader);
    assert_failed(
        &graph,
        PROCESS,
        &FailureReason::NotGranted {
            builtin: &BUILTINS[0],
            origin: origin.clone(),
        },
    );
}

/// Asserts `loader` recorded no builtin.
fn assert_no_builtin(loader: &Loader) {
    assert!(
        !graph(loader)
            .records()
            .values()
            .any(|record| matches!(record, Record::Builtin(_)))
    );
}

/// With no authority, as eval's loader, nothing is granted, to the project
/// or a package, by a static import or `import()`.
#[test]
fn no_authority_grants_nothing() {
    let tree = tree();
    let loader = loader(&tree, None);
    for reached in [STATIC, Reached::Import(Reach::Dynamic)] {
        assert_refused(
            &loader,
            MAIN,
            reached,
            &Origin::Project,
            "and eval grants none",
        );
        assert_refused(&loader, BAR, reached, &bar_origin(), "and eval grants none");
    }
    assert_no_builtin(&loader);
}

/// Granting nothing refuses the project; granting `process` grants it to
/// the project and still refuses the package, whichever imports first.
#[test]
fn the_project_is_granted_and_a_package_never() {
    let tree = tree();
    let empty = loader(&tree, Some(Authority::default()));
    assert_refused(
        &empty,
        MAIN,
        STATIC,
        &Origin::Project,
        "which this run doesn't grant",
    );
    assert_no_builtin(&empty);
    let package_refusal = "which package 'bar' isn't granted: only the project's own modules are granted host \
         modules";
    for project_first in [true, false] {
        let granted = loader(&tree, Some(granted()));
        if !project_first {
            assert_refused(&granted, BAR, STATIC, &bar_origin(), package_refusal);
            assert_no_builtin(&granted);
        }
        for reached in [STATIC, Reached::Import(Reach::Dynamic)] {
            let url = granted
                .resolve_reached(&tree, PROCESS, MAIN, reached)
                .unwrap();
            assert_eq!(url.as_str(), BUILTINS[0].url);
        }
        assert_refused(
            &granted,
            BAR,
            Reached::Import(Reach::Dynamic),
            &bar_origin(),
            package_refusal,
        );
        let graph = graph(&granted);
        let url = Url::parse(BUILTINS[0].url).unwrap();
        assert_eq!(graph.records()[&url], Record::Builtin(&BUILTINS[0]));
        assert_eq!(graph.roots().iter().count(), 2, "a builtin admits no root");
        let edges: Vec<_> = graph
            .edges()
            .iter()
            .filter(|(_, to)| **to == url)
            .map(|(edge, _)| (edge.importer.as_str(), edge.reach))
            .collect();
        assert_eq!(edges, [(MAIN, Reach::Static), (MAIN, Reach::Dynamic)]);
        // a builtin is served as its synthetic module, never loaded
        assert!(
            load_disk(&granted, &url)
                .unwrap_err()
                .contains("a builtin has no file to load")
        );
    }
}

/// `import.meta.resolve` returns the builtin's URL to any importer, granted
/// or not, recording nothing, since it imports nothing.
#[test]
fn import_meta_resolve_returns_the_url_to_anyone() {
    let tree = tree();
    for authority in [None, Some(Authority::default()), Some(granted())] {
        let loader = loader(&tree, authority);
        let before = graph(&loader);
        for importer in [MAIN, BAR] {
            let url = loader
                .resolve_reached(&tree, PROCESS, importer, Reached::MetaResolve)
                .unwrap();
            assert_eq!(url.as_str(), BUILTINS[0].url);
        }
        let after = graph(&loader);
        assert_eq!(after.records(), before.records());
        assert_eq!(after.edges(), before.edges());
        assert_eq!(after.failures(), before.failures());
    }
}

/// The URL itself, written in source, is refused by its scheme, granted or
/// not.
#[test]
fn the_builtin_url_is_refused_from_source() {
    let tree = tree();
    let loader = loader(&tree, Some(granted()));
    for reached in [
        STATIC,
        Reached::Import(Reach::Dynamic),
        Reached::MetaResolve,
    ] {
        loader
            .resolve_reached(&tree, BUILTINS[0].url, MAIN, reached)
            .unwrap_err();
    }
    assert_failed(
        &graph(&loader),
        BUILTINS[0].url,
        &FailureReason::Resolve(ResolveErrorKind::UnsupportedScheme("builtin".to_owned())),
    );
}
