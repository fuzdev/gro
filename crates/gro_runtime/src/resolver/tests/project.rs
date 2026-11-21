use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::resolver::test_tree::MemoryTree;
use crate::resolver::{
    InstalledPackage, ResolveErrorKind, read_installed_package, read_project_package,
};

/// A project at `/w/p` declaring packages every way a spec can be set or
/// not, with installs plain, linked, scoped, unversioned, and missing a
/// `package.json`.
fn tree() -> MemoryTree {
    MemoryTree {
        files: BTreeMap::from([
            (
                "/w/p/package.json",
                r#"{
                    "name": "p",
                    "dependencies": {"a": "^1.0.0", "empty": "", "zero": 0, "null": null, "no": false},
                    "devDependencies": {"@s/b": "*", "one": 1, "yes": true, "list": []},
                    "peerDependencies": {"peer": "^1.0.0"}
                }"#,
            ),
            (
                "/w/p/node_modules/a/package.json",
                r#"{"version": "1.2.3"}"#,
            ),
            (
                "/w/p/node_modules/@s/b/package.json",
                r#"{"version": "0.1.0-next.1"}"#,
            ),
            ("/w/p/node_modules/bare/package.json", r#"{"name": "bare"}"#),
            ("/w/p/node_modules/odd/package.json", r#"{"version": 3}"#),
            ("/w/linked/package.json", r#"{"version": "2.0.0"}"#),
            ("/w/m/package.json", r#"{"type": "module"}"#),
            ("/w/bad/package.json", "{"),
        ]),
        dirs: vec!["/w/p/node_modules/nojson", "/w/open/node_modules/c"],
        links: BTreeMap::from([("/w/p/node_modules/linked", "/w/linked")]),
        loops: vec![],
        world_writable: vec!["/w/open/node_modules"],
    }
}

#[test]
fn a_project_declares_what_its_direct_dependencies_set() {
    let package = read_project_package(&tree(), Path::new("/w/p"))
        .unwrap()
        .unwrap();
    assert_eq!(package.path, Path::new("/w/p/package.json"));
    // JS's truthiness, as `package_json_has_dependency` tests a spec, and
    // never a peer
    assert_eq!(
        package
            .declared
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["@s/b", "a", "list", "one", "yes"]
    );
    assert!(package.declares("a") && !package.declares("empty") && !package.declares("peer"));
}

#[test]
fn a_project_s_package_json_is_read_as_resolution_reads_it() {
    let tree = tree();
    assert_eq!(read_project_package(&tree, Path::new("/w/none")), Ok(None));
    let marker = read_project_package(&tree, Path::new("/w/m"))
        .unwrap()
        .unwrap();
    assert!(marker.declared.is_empty());
    assert!(matches!(
        read_project_package(&tree, Path::new("/w/bad")),
        Err(ResolveErrorKind::InvalidPackageJson { .. })
    ));
}

#[test]
fn an_installed_package_s_version_is_read_from_the_project_s_node_modules() {
    let tree = tree();
    let read = |name| read_installed_package(&tree, Path::new("/w/p"), name);
    let installed = |root: &str, version: Option<&str>| {
        Ok(Some(InstalledPackage {
            root: PathBuf::from(root),
            version: version.map(str::to_owned),
        }))
    };
    assert_eq!(read("a"), installed("/w/p/node_modules/a", Some("1.2.3")));
    assert_eq!(
        read("@s/b"),
        installed("/w/p/node_modules/@s/b", Some("0.1.0-next.1"))
    );
    // a linked install by its real root
    assert_eq!(read("linked"), installed("/w/linked", Some("2.0.0")));
    // a version that's absent or not a string is none
    assert_eq!(read("bare"), installed("/w/p/node_modules/bare", None));
    assert_eq!(read("odd"), installed("/w/p/node_modules/odd", None));
    assert_eq!(read("missing"), Ok(None));
    assert_eq!(
        read("nojson"),
        Err(ResolveErrorKind::NoPackageJson(PathBuf::from(
            "/w/p/node_modules/nojson"
        )))
    );
}

#[test]
fn an_installed_package_is_named_alone_and_never_read_through_a_world_writable_directory() {
    let tree = tree();
    for name in ["a/sub", "@s", "@s/b/c", "./a", ""] {
        assert_eq!(
            read_installed_package(&tree, Path::new("/w/p"), name),
            Err(ResolveErrorKind::InvalidPackageName),
            "{name}"
        );
    }
    assert_eq!(
        read_installed_package(&tree, Path::new("/w/open"), "c"),
        Err(ResolveErrorKind::WorldWritablePackage {
            name: "c".to_owned(),
            dir: PathBuf::from("/w/open/node_modules"),
        })
    );
}
