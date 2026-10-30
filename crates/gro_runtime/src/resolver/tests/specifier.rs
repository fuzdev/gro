use std::path::{Path, PathBuf};

use url::Url;

use super::{from_entry, importer, kind, ok, tree};
use crate::resolver::{
    ConsultedFile, FileRole, ModuleFormat, Origin, ResolveErrorKind, Root, Roots, module_format,
    resolve, resolve_entry,
};

#[test]
fn relative() {
    assert_eq!(ok("./a.js"), "file:///p/a.js");
    assert_eq!(ok("./b.mjs"), "file:///p/b.mjs");
    assert_eq!(ok("./lib/c.js"), "file:///p/lib/c.js");
    assert_eq!(ok("./lib/../a.js"), "file:///p/a.js");
}

/// `..` resolves inside the project root, and past it is refused.
#[test]
fn parent() {
    let importer = Url::parse("file:///p/lib/c.js").unwrap();
    let resolution = from_entry(&tree(), "../a.js", &importer).unwrap();
    assert_eq!(resolution.url.as_str(), "file:///p/a.js");
    // `..` past the filesystem's root stays at it, as URL parsing does, outside `/p`
    assert_eq!(
        from_entry(&tree(), "../../../abs.js", &importer)
            .unwrap_err()
            .kind,
        ResolveErrorKind::OutsideRoot {
            path: PathBuf::from("/abs.js"),
            root: PathBuf::from("/p"),
            package: None,
        }
    );
}

/// An absolute specifier resolves inside the project root, and outside
/// it is refused.
#[test]
fn absolute() {
    assert_eq!(ok("/p/a.js"), "file:///p/a.js");
    assert_eq!(ok("file:///p/a.js"), "file:///p/a.js");
    let outside = ResolveErrorKind::OutsideRoot {
        path: PathBuf::from("/abs.js"),
        root: PathBuf::from("/p"),
        package: None,
    };
    assert_eq!(kind("/abs.js"), outside);
    assert_eq!(kind("file:///abs.js"), outside);
}

#[test]
fn record() {
    let resolution = from_entry(&tree(), "./a.js", &importer()).unwrap();
    assert_eq!(resolution.origin, Origin::Project);
    assert_eq!(resolution.root, Path::new("/p"));
    // a `.js` file's scope, read for its `"type"`
    assert_eq!(
        resolution.files_consulted,
        [ConsultedFile {
            path: PathBuf::from("/p/package.json"),
            role: FileRole::PackageScope
        }]
    );
    let resolution = from_entry(&tree(), "./typed.ts", &importer()).unwrap();
    assert!(resolution.files_consulted.is_empty());
    // a relative specifier resolves inside the importer's root, with its origin
    let origin = Origin::Package {
        name: "x".to_owned(),
        version: None,
        linked: true,
    };
    let mut roots = Roots::new(PathBuf::from("/"));
    roots.admit(Root {
        dir: PathBuf::from("/p"),
        origin: origin.clone(),
    });
    let resolution = resolve(&tree(), "./a.js", &importer(), &roots).unwrap();
    assert_eq!(resolution.origin, origin);
    assert_eq!(resolution.root, Path::new("/p"));
}

#[test]
fn query_and_fragment_are_kept() {
    assert_eq!(ok("./a.js?v=1"), "file:///p/a.js?v=1");
    assert_eq!(ok("./a.js#x"), "file:///p/a.js#x");
    assert_eq!(ok("./a.js?v=1#x"), "file:///p/a.js?v=1#x");
}

/// A symlink resolves to its real path, which must be inside the root.
#[test]
fn symlinks_resolve_to_the_real_path() {
    assert_eq!(ok("./link.js"), "file:///p/real/linked.js");
    assert_eq!(ok("./link.js?q"), "file:///p/real/linked.js?q");
    assert_eq!(
        kind("./link_out.js"),
        ResolveErrorKind::OutsideRoot {
            path: PathBuf::from("/real/linked.js"),
            root: PathBuf::from("/p"),
            package: None,
        }
    );
}

#[test]
fn missing_extension_is_refused() {
    assert_eq!(
        kind("./a"),
        ResolveErrorKind::NoExtension {
            suggestion: Some("./a.js".to_owned())
        }
    );
    // the suggestion prefers `.ts` and keeps the query
    assert_eq!(
        kind("./schema?x"),
        ResolveErrorKind::NoExtension {
            suggestion: Some("./schema.ts?x".to_owned())
        }
    );
    assert_eq!(
        kind("./only_mts"),
        ResolveErrorKind::NoExtension {
            suggestion: Some("./only_mts.mts".to_owned())
        }
    );
    assert_eq!(
        kind("./nothing"),
        ResolveErrorKind::NoExtension { suggestion: None }
    );
    // a directory never resolves to an index
    for specifier in ["./lib", "./lib/", "./dir.js/", ".", ".."] {
        assert_eq!(
            kind(specifier),
            ResolveErrorKind::NoExtension { suggestion: None },
            "{specifier}"
        );
    }
}

#[test]
fn missing_file() {
    assert_eq!(
        kind("./missing.js"),
        ResolveErrorKind::NotFound { ts_sibling: None }
    );
    assert_eq!(
        kind("./only_ts.js"),
        ResolveErrorKind::NotFound {
            ts_sibling: Some("./only_ts.ts".to_owned())
        }
    );
    assert_eq!(
        kind("./only_mts.mjs?x"),
        ResolveErrorKind::NotFound {
            ts_sibling: Some("./only_mts.mts?x".to_owned())
        }
    );
    // only the module extension's own TypeScript twin is hinted
    assert_eq!(
        kind("./only_mts.js"),
        ResolveErrorKind::NotFound { ts_sibling: None }
    );
    assert_eq!(kind("./dir.js"), ResolveErrorKind::NotAFile);
}

#[test]
fn encoded_separators_and_nul_are_refused() {
    for specifier in [
        "./..%2Fa.js",
        "./lib%2fc.js",
        "./lib%5Cc.js",
        "./%00.js",
        "file:///p/lib%2Fc.js",
    ] {
        assert!(
            matches!(kind(specifier), ResolveErrorKind::InvalidUrl(_)),
            "{specifier}"
        );
    }
    // the query isn't part of the path
    assert_eq!(ok("./a.js?%2F"), "file:///p/a.js?%2F");
}

#[test]
fn extensions() {
    assert_eq!(ok("./typed.ts"), "file:///p/typed.ts");
    assert_eq!(ok("./typed.mts"), "file:///p/typed.mts");
    assert_eq!(kind("./common.cts"), ResolveErrorKind::CommonJs);
    assert_eq!(kind("./view.tsx"), ResolveErrorKind::Jsx);
    for specifier in ["./types.d.ts", "./types.d.mts", "./types.d.cts"] {
        assert_eq!(
            kind(specifier),
            ResolveErrorKind::Declaration,
            "{specifier}"
        );
    }
    for specifier in [
        "./state.svelte.ts",
        "./state.svelte.js",
        "./state.svelte.test.ts",
    ] {
        assert_eq!(
            kind(specifier),
            ResolveErrorKind::SvelteModule,
            "{specifier}"
        );
    }
    assert_eq!(
        kind("./data.json"),
        ResolveErrorKind::UnsupportedExtension("json".to_owned())
    );
    // the real file's name decides, not the specifier's
    assert_eq!(ok("./link_to_ts.js"), "file:///p/typed.ts");
}

#[test]
fn formats() {
    let format = |name: &str| module_format(Path::new(name));
    assert_eq!(format("/p/a.js"), Ok(ModuleFormat::JavaScript));
    assert_eq!(format("/p/a.mjs"), Ok(ModuleFormat::JavaScript));
    assert_eq!(format("/p/a.ts"), Ok(ModuleFormat::TypeScript));
    assert_eq!(format("/p/a.mts"), Ok(ModuleFormat::TypeScript));
    // only the compound suffix is a declaration or a Svelte module
    assert_eq!(format("/p/d.ts"), Ok(ModuleFormat::TypeScript));
    assert_eq!(format("/p/svelte.ts"), Ok(ModuleFormat::TypeScript));
    // vite-plugin-svelte's default rule: `.svelte.`, any segments, then `js` or `ts`
    for name in [
        "/p/a.svelte.test.ts",
        "/p/a.svelte.test.js",
        "/p/a.svelte.x.y.ts",
        "/p/.svelte.ts",
        "/p/a.svelte.svelte.ts",
    ] {
        assert_eq!(format(name), Err(ResolveErrorKind::SvelteModule), "{name}");
    }
    // outside it, so plain TypeScript
    assert_eq!(format("/p/a.svelte.mts"), Ok(ModuleFormat::TypeScript));
    assert_eq!(format("/p/a.svelte.test.mts"), Ok(ModuleFormat::TypeScript));
    assert_eq!(format("/p/a.svelte..ts"), Ok(ModuleFormat::TypeScript));
    assert_eq!(format("/p/a.svelte_x.ts"), Ok(ModuleFormat::TypeScript));
    assert_eq!(format("/p/a.d.ts"), Err(ResolveErrorKind::Declaration));
    assert_eq!(
        format("/p/a.svelte.js"),
        Err(ResolveErrorKind::SvelteModule)
    );
    assert_eq!(
        format("/p/a.svelte"),
        Err(ResolveErrorKind::UnsupportedExtension("svelte".to_owned()))
    );
}

#[test]
fn schemes() {
    assert_eq!(kind("node:fs"), ResolveErrorKind::NodeBuiltin);
    for specifier in ["npm:svelte", "jsr:@std/toml@^1"] {
        assert_eq!(kind(specifier), ResolveErrorKind::Registry, "{specifier}");
    }
    for specifier in [
        "http://example.com/a.js",
        "https://example.com/a.js",
        "data:text/javascript,export default 1",
    ] {
        assert_eq!(kind(specifier), ResolveErrorKind::Remote, "{specifier}");
    }
    assert_eq!(
        kind("ext:deno_web/00_url.js"),
        ResolveErrorKind::UnsupportedScheme("ext".to_owned())
    );
    assert_eq!(
        kind("blob:abc"),
        ResolveErrorKind::UnsupportedScheme("blob".to_owned())
    );
}

#[test]
fn package_imports_and_empty() {
    assert_eq!(kind("#lib/a.js"), ResolveErrorKind::PackageImport);
    assert_eq!(kind(""), ResolveErrorKind::Empty);
}

#[test]
fn entry() {
    let cwd = Path::new("/p");
    let entry = |path| resolve_entry(&tree(), Path::new(path), cwd).map(|(entry, _)| entry);
    assert_eq!(entry("main.js").unwrap().url.as_str(), "file:///p/main.js");
    assert_eq!(
        entry("lib/c.js").unwrap().url.as_str(),
        "file:///p/lib/c.js"
    );
    assert_eq!(entry("../abs.js").unwrap().url.as_str(), "file:///abs.js");
    assert_eq!(
        entry("./lib/../a.js").unwrap().url.as_str(),
        "file:///p/a.js"
    );
    assert_eq!(entry("/abs.js").unwrap().url.as_str(), "file:///abs.js");
    assert_eq!(
        entry("typed.ts").unwrap().url.as_str(),
        "file:///p/typed.ts"
    );
    assert_eq!(entry("main.js").unwrap().origin, Origin::Project);
    // the entry defines the root, its project root, so outside a project is no refusal
    assert_eq!(entry("main.js").unwrap().root, Path::new("/p"));
    assert_eq!(entry("../abs.js").unwrap().root, Path::new("/"));
    assert_eq!(entry("view.tsx").unwrap_err().kind, ResolveErrorKind::Jsx);
    let error = entry("missing.js").unwrap_err();
    assert_eq!(error.kind, ResolveErrorKind::NotFound { ts_sibling: None });
    assert_eq!(
        error.to_string(),
        "cannot resolve 'missing.js' from /p: no such file"
    );
    // a path, not a URL: `?`, `#`, and `%` are file name characters
    assert_eq!(
        entry("a.js?x").unwrap_err().kind,
        ResolveErrorKind::NotFound { ts_sibling: None }
    );
    assert_eq!(
        entry("lib%2Fc.js").unwrap_err().kind,
        ResolveErrorKind::NotFound { ts_sibling: None }
    );
}

#[test]
fn messages() {
    let message = |specifier| {
        from_entry(&tree(), specifier, &importer())
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        message("node:fs"),
        "cannot resolve 'node:fs' from file:///p/main.js: a node: specifier names a Node \
         built-in, and there is no Node compat. Fix: import the fuz_host equivalent, or run \
         this under Node"
    );
    assert!(message("https://example.com/a.js").contains("only files on disk load"));
    assert!(!message("https://example.com/a.js").contains("node_modules"));
    assert!(message("jsr:@std/toml").contains("@jsr/<scope>__<name>"));
    assert_eq!(
        message(""),
        "cannot resolve '' from file:///p/main.js: the specifier is empty"
    );
    assert!(message("./dir.js").contains("not a regular file"));
    assert_eq!(
        message("./only_ts.js"),
        "cannot resolve './only_ts.js' from file:///p/main.js: no such file, but \
         './only_ts.ts' exists; write the real extension"
    );
    assert_eq!(
        message("./common.cts"),
        "cannot resolve './common.cts' from file:///p/main.js: a .cts file is CommonJS, and \
         only ES modules load. Fix: rename it to .ts or .mts and use import and export"
    );
    assert!(message("./view.tsx").contains("JSX is not supported"));
    assert!(message("./types.d.ts").contains("a declaration file holds only types"));
    assert!(message("./state.svelte.ts").contains("needs the Svelte compiler"));
    assert!(message("./data.json").ends_with(".js, .mjs, .ts, and .mts modules load"));
    assert_eq!(
        message("./..%2Fa.js"),
        "cannot resolve './..%2Fa.js' from file:///p/main.js: not a valid file URL: the path \
         /p/..%2Fa.js has an encoded '/' or '\\' (%2F or %5C), which never names a separator"
    );
}
