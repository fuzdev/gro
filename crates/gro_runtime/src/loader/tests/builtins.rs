//! A builtin through the loader: the grant check at link time, by the run's
//! authority and the importer's origin, and what the graph records.

use super::super::resolve::resolve_builtin;
use super::*;
use crate::resolver::{BUILTINS, Builtin, Origin, ResolveErrorKind};
use crate::{ArgsGrant, Authority, Grants};

const ARGS: &str = "@fuzdev/gro/args.ts";
const BAR: &str = "file:///p/node_modules/bar/i.mjs";

/// `fgro run`'s authority: `args` granted.
fn granted() -> Authority {
    Authority::Confined(Grants {
        args: Some(ArgsGrant {}),
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

/// Asserts `loader` refuses `args` to `importer`, reached as `reached`,
/// with a message ending in `why`, recording [`FailureReason::NotGranted`]
/// with `origin`.
fn assert_refused(loader: &Loader, importer: &str, reached: Reached, origin: &Origin, why: &str) {
    let tree = tree();
    let error = loader
        .resolve_reached(&tree, ARGS, importer, reached)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        format!("cannot import '{ARGS}' from {importer}: it needs the grant 'args', {why}")
    );
    let graph = graph(loader);
    assert_failed(
        &graph,
        ARGS,
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
            "and eval imports no builtin module",
        );
        assert_refused(
            &loader,
            BAR,
            reached,
            &bar_origin(),
            "and eval imports no builtin module",
        );
    }
    assert_no_builtin(&loader);
}

/// Granting nothing refuses the project; granting `args` grants it to
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
            let url = granted.resolve_reached(&tree, ARGS, MAIN, reached).unwrap();
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
                .resolve_reached(&tree, ARGS, importer, Reached::MetaResolve)
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

/// A grant-free builtin of the test's own: the table has none yet, so the
/// check is driven past the table's lookup, from the resolved builtin on.
static GRANT_FREE: Builtin = Builtin {
    specifier: "@fuzdev/gro/grant_free.ts",
    url: "builtin:@fuzdev/gro/grant_free.ts",
    grant: None,
    exports: &[],
};

/// Links [`GRANT_FREE`] into `loader`'s graph, imported by `importer` as
/// `reach`, under `loader`'s authority.
fn link_grant_free(loader: &Loader, importer: &str, reach: Reach) -> Result<String, Refused> {
    let importer = Url::parse(importer).unwrap();
    resolve_builtin(
        &mut loader.graph.borrow_mut(),
        loader.authority.as_ref(),
        &GRANT_FREE,
        GRANT_FREE.specifier,
        &importer,
        Reached::Import(reach),
    )
    .map(|url| url.to_string())
}

/// With no authority, as eval's loader, a grant-free builtin is refused
/// too, the eval checked before the grant, with a message that names no
/// grant; a run's loader links it for the project and a package alike,
/// whatever the run grants.
#[test]
fn eval_refuses_a_grant_free_builtin_and_a_run_links_it() {
    let tree = tree();
    let eval = loader(&tree, None);
    for (importer, origin) in [(MAIN, Origin::Project), (BAR, bar_origin())] {
        for reach in [Reach::Static, Reach::Dynamic] {
            let Err(refused) = link_grant_free(&eval, importer, reach) else {
                panic!("linked under eval");
            };
            assert_eq!(
                refused.message,
                format!(
                    "cannot import '{}' from {importer}: eval imports no builtin module",
                    GRANT_FREE.specifier
                )
            );
            assert_eq!(
                *refused.reason,
                FailureReason::NotGranted {
                    builtin: &GRANT_FREE,
                    origin: origin.clone(),
                }
            );
        }
    }
    assert_no_builtin(&eval);
    for authority in [Authority::default(), granted()] {
        let loader = loader(&tree, Some(authority));
        for importer in [MAIN, BAR] {
            for reach in [Reach::Static, Reach::Dynamic] {
                let url = link_grant_free(&loader, importer, reach)
                    .unwrap_or_else(|refused| panic!("{}", refused.message));
                assert_eq!(url, GRANT_FREE.url);
            }
        }
        let url = Url::parse(GRANT_FREE.url).unwrap();
        assert_eq!(graph(&loader).records()[&url], Record::Builtin(&GRANT_FREE));
    }
}
