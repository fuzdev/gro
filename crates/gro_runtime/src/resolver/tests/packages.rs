use std::path::{Path, PathBuf};

use url::Url;

use super::{from_entry, package, packages};
use crate::resolver::{ConsultedFile, FileRole, Origin, PackageSearchBound, ResolveErrorKind};

#[test]
fn package_origins_and_consulted_files() {
    let resolution = package("t").unwrap();
    assert_eq!(
        resolution.origin,
        Origin::Package {
            name: "t".to_owned(),
            version: Some("2.3.4".to_owned()),
            linked: false
        }
    );
    let consulted = |files: &[(&str, FileRole)]| {
        files
            .iter()
            .map(|&(path, role)| ConsultedFile {
                path: PathBuf::from(path),
                role,
            })
            .collect::<Vec<_>>()
    };
    // the importer's scope (a self-reference's), the same file as the bound, the
    // package's exports, then the module's scope, the same file in another role
    assert_eq!(
        resolution.files_consulted,
        consulted(&[
            ("/w/proj/package.json", FileRole::PackageScope),
            ("/w/proj/package.json", FileRole::SearchBound),
            ("/w/proj/node_modules/t/package.json", FileRole::Exports),
            (
                "/w/proj/node_modules/t/package.json",
                FileRole::PackageScope
            ),
        ])
    );
    let resolution = package("linked").unwrap();
    assert_eq!(
        resolution.origin,
        Origin::Package {
            name: "linked".to_owned(),
            version: Some("0.1.0".to_owned()),
            linked: true
        }
    );
    // the real paths, not the symlink's
    assert_eq!(
        resolution.files_consulted[2].path,
        Path::new("/w/linked/package.json")
    );
    // a nested layout's real root is inside node_modules, so it isn't linked
    let resolution = package("nested").unwrap();
    assert_eq!(
        resolution.origin,
        Origin::Package {
            name: "nested".to_owned(),
            version: Some("1.0.0".to_owned()),
            linked: false
        }
    );
    // a self-reference keeps the importer's origin
    let resolution = package("proj/self.js").unwrap();
    assert_eq!(resolution.origin, Origin::Project);
    assert_eq!(
        resolution.files_consulted,
        consulted(&[
            ("/w/proj/package.json", FileRole::PackageScope),
            ("/w/proj/package.json", FileRole::Exports),
        ])
    );
}

#[test]
fn packages_resolve_their_own_imports_from_where_they_live() {
    let tree = packages();
    let from = |importer: &str, specifier: &str| {
        let importer = Url::parse(importer).unwrap();
        from_entry(&tree, specifier, &importer).map(|resolution| resolution.url)
    };
    // a linked package's dependency, from its own node_modules, past the project's
    assert_eq!(
        from("file:///w/linked/i.js", "linked_dep")
            .unwrap()
            .as_str(),
        "file:///w/linked/node_modules/linked_dep/i.mjs"
    );
    // a linked package is a project root of its own
    assert_eq!(
        from("file:///w/linked/i.js", "t").unwrap_err().kind,
        ResolveErrorKind::PackageNotFound {
            name: "t".to_owned(),
            bound: Box::new(PackageSearchBound {
                dir: PathBuf::from("/w/linked"),
                package_json: PathBuf::from("/w/linked/package.json"),
                above: Some(PathBuf::from("/w/package.json")),
            }),
        }
    );
    // inside node_modules, the search goes on past the package's root
    assert_eq!(
        from("file:///w/proj/node_modules/needs_hoisted/i.mjs", "hoisted")
            .unwrap()
            .as_str(),
        "file:///w/proj/node_modules/hoisted/i.mjs"
    );
    assert_eq!(
        from(
            "file:///w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested/i.mjs",
            "nested_dep"
        )
        .unwrap()
        .as_str(),
        "file:///w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested_dep/i.mjs"
    );
    // and still ends at the project root
    assert_eq!(
        from("file:///w/proj/node_modules/hoisted/i.mjs", "parent_only")
            .unwrap_err()
            .kind,
        ResolveErrorKind::PackageNotFound {
            name: "parent_only".to_owned(),
            bound: Box::new(PackageSearchBound {
                dir: PathBuf::from("/w/proj"),
                package_json: PathBuf::from("/w/proj/package.json"),
                above: Some(PathBuf::from("/w/package.json")),
            }),
        }
    );
    // outside any package.json, a .js file has no "type" scope, as in Node, which then
    // guesses its format
    assert_eq!(
        from("file:///loose/b.mjs", "./a.js").unwrap_err().kind,
        ResolveErrorKind::NoTypeScope { stopped_at: None }
    );
    // with no project root, no node_modules is searched
    assert_eq!(
        from("file:///loose/b.mjs", "t").unwrap_err().kind,
        ResolveErrorKind::NoProjectRoot {
            dir: PathBuf::from("/loose"),
            stopped_at: None,
        }
    );
}

#[test]
fn package_messages() {
    let message = |specifier| package(specifier).unwrap_err().to_string();
    assert_eq!(
        message("parent_only"),
        "no package 'parent_only' is installed: no node_modules/parent_only directory from \
         the importer up to /w/proj/package.json, which ends the search, as every \
         package.json outside node_modules does except one holding nothing but \"type\" and \
         \"sideEffects\", and the node_modules beside /w/package.json is not searched (npm \
         and yarn workspaces, which install at their root, aren't supported). Fix: install it \
         into /w/proj/node_modules"
    );
    let importer = Url::parse("file:///loose/b.mjs").unwrap();
    assert_eq!(
        from_entry(&packages(), "t", &importer)
            .unwrap_err()
            .kind
            .to_string(),
        "no project root is above /loose: a bare specifier resolves only under a \
         package.json outside node_modules that holds more than \"type\" and \
         \"sideEffects\", and the search for one reached the filesystem's root. Fix: add a \
         package.json at the project root, with the package installed in the node_modules \
         beside it"
    );
    assert_eq!(
        message("t/nested_unmatched"),
        "no condition matches for subpath './nested_unmatched': the package's conditions \
         [gro.browser, node] don't, and only gro, import, default are matched. Fix: add a \
         \"gro\" or \"import\" condition before the others"
    );
    assert!(message("t/empty_conditions").starts_with(
        "subpath './empty_conditions' is not exported by \
         /w/proj/node_modules/t/package.json (exports: ., ./up, "
    ));
    assert_eq!(
        message("looped"),
        "/w/proj/node_modules/looped leads into a symlink loop, so it names no file or \
         directory. Fix: remove the looping link, or point it at what it should name (for a \
         package, reinstall it)"
    );
    assert_eq!(
        message("t/dot_star/.?"),
        "the part of the subpath the pattern './dot_star/*' matches, '.?', has a '.', '..', \
         or 'node_modules' segment, percent-encoded or not, or takes the target out of the \
         package's directory, and a pattern never substitutes those. Fix: import the module \
         by its plain subpath"
    );
    assert_eq!(
        message("no_exports"),
        "/w/proj/node_modules/no_exports/package.json has no \"exports\" field, and only \
         \"exports\" is read (not \"main\" or \"module\"). Fix: use a version with an ESM \
         build, vendor an ESM bundle of it into the project, or run it under Node"
    );
    assert_eq!(
        message("@scope/pkg/y.mjs"),
        "subpath './y.mjs' is not exported by /w/proj/node_modules/@scope/pkg/package.json \
         (exports: ., ./x.mjs). Fix: import an exported subpath, or add one to the package's \
         \"exports\""
    );
    assert_eq!(
        message("t/unmatched"),
        "no condition matches for subpath './unmatched': the package's conditions [node, \
         require] don't, and only gro, import, default are matched. Fix: add a \"gro\" or \
         \"import\" condition before the others"
    );
    assert_eq!(
        message("t/up"),
        "the \"exports\" target \"../escape.js\" for './up' is invalid: a target must start \
         with './'. Fix: use a version of the package whose \"exports\" are valid"
    );
    assert_eq!(
        message("t/ts"),
        "the package exports it as /w/proj/node_modules/t/only_ts.js, which does not exist \
         ('/w/proj/node_modules/t/only_ts.ts' does). Fix: reinstall the package"
    );
    assert_eq!(
        message("cjs"),
        "the package exports it as /w/proj/node_modules/cjs/i.js, which is CommonJS (no \
         \"type\": \"module\", not .mjs), and only ES modules load. Fix: use a version with \
         an ESM build, vendor an ESM bundle of it into the project, or run it under Node"
    );
    assert_eq!(
        message("t/no_extension"),
        "the package exports it as /w/proj/node_modules/t/no_extension: the path has no \
         extension; extensions and directory indexes are never probed"
    );
    assert_eq!(
        message("./no_type/a.js"),
        "a .js file is an ES module only under \"type\": \"module\", which \
         /w/proj/no_type/package.json does not set, and CommonJS is not supported. Fix: add \
         \"type\": \"module\" to it, or rename the file to .mjs"
    );
    let loose = Url::parse("file:///loose/b.mjs").unwrap();
    assert!(
        from_entry(&packages(), "./a.js", &loose)
            .unwrap_err()
            .kind
            .to_string()
            .contains("Fix: rename the file to .mjs")
    );
    assert!(message("@scope").contains("not a valid package name"));
    assert!(message("no_package").contains("has no package.json"));
    assert!(message("empty_exports").contains("(its \"exports\" are empty)"));
}
