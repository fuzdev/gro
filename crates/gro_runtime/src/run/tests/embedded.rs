//! Tests for Gro's embedded library in a run: a specifier of Gro's package
//! loads the run's copy, never a package of that name, its modules load
//! as they are, unstripped, and hold no grant whatever the run grants.

use std::sync::LazyLock;

use super::builtins::{granted, run_files, run_linked_outcome};
use crate::embed::{Embed, EmbedContents, EmbedError, LoadEmbed, encode};
use crate::loader::graph::{FailureReason, Record};
use crate::resolver::{GroModuleFault, Origin, ResolveErrorKind, find_builtin};
use crate::test_support::never_exits;
use crate::{Authority, RunError, RuntimeOptions};

/// The modules of [`test_embed`]: one that imports a grant-free row and a
/// package, one that imports a granted row, one with a type left in it,
/// and one that imports by `import()`.
const MODULES: [(&str, &str); 7] = [
    (
        "attributed.ts",
        "import { cwd } from './cwd.ts' with { type: 'json' };\nexport default cwd;\n",
    ),
    (
        "dyn.ts",
        "export const load = (specifier, options) => import(specifier, options);\n",
    ),
    ("fs_paths.ts", "export const fs_paths = 'PATHS';\n"),
    (
        "granted.ts",
        "import { fs } from './fs.ts';\nexport default fs;\n",
    ),
    (
        "lib.ts",
        "import { cwd } from './cwd.ts';\n\
         import { util } from 'util_pkg/x.js';\n\
         export const where = import.meta.url;\n\
         export const via = util;\n\
         export const cwd_kind = typeof cwd;\n",
    ),
    ("typed.ts", "export const typed: number = 1;\n"),
    (
        "unstripped.ts",
        "export const kept = 'a'   /* types blanked here */;\n",
    ),
];

/// A copy of Gro's library for these tests, [`MODULES`], `invoke.ts` left
/// out, and `util_pkg` its one peer.
pub(super) fn test_embed() -> Result<Embed<'static>, EmbedError> {
    static BLOB: LazyLock<Vec<u8>> = LazyLock::new(|| {
        encode(&EmbedContents {
            modules: MODULES
                .iter()
                .map(|(path, code)| ((*path).to_owned(), (*code).to_owned()))
                .collect(),
            left_out: vec!["invoke.ts".to_owned()],
            peers: vec![("util_pkg".to_owned(), "^1".to_owned())],
        })
        .expect("the test embed encodes")
    });
    Embed::decode(&BLOB)
}

/// The test copy, as an embedder hands it in.
pub(super) const TEST_EMBED: Option<LoadEmbed> = Some(test_embed);

/// A planted `@fuzdev/gro` in `node_modules`, serving every module the copy
/// holds, and `util_pkg`, the copy's peer.
const PLANTED: [(&str, &str); 5] = [
    (
        "node_modules/@fuzdev/gro/package.json",
        r#"{"name": "@fuzdev/gro", "type": "module", "exports": {".": "./index.js", "./*.ts": "./*.js", "./*.js": "./*.js"}}"#,
    ),
    (
        "node_modules/@fuzdev/gro/lib.js",
        "export const where = 'PLANTED';\n",
    ),
    (
        "node_modules/@fuzdev/gro/invoke.js",
        "export const where = 'PLANTED';\n",
    ),
    (
        "node_modules/util_pkg/package.json",
        r#"{"name": "util_pkg", "version": "1.2.0", "type": "module", "exports": {"./x.js": "./x.js"}}"#,
    ),
    (
        "node_modules/util_pkg/x.js",
        "export const util = 'UTIL';\n",
    ),
];

/// Options granting `authority`, with the test copy.
fn options(authority: Authority) -> RuntimeOptions {
    RuntimeOptions {
        authority,
        embed: TEST_EMBED,
        exit_step: Some(never_exits),
        ..RuntimeOptions::default()
    }
}

/// The message of the error a run failed with.
fn uncaught(result: Result<(), RunError>) -> String {
    match result {
        Err(RunError::Uncaught(report)) => report.to_string(),
        other => panic!("not an uncaught error: {other:?}"),
    }
}

/// A specifier of Gro's package loads the copy's module, never the planted
/// package's, at its `gro:lib/` URL, its relative import of a grant-free
/// row the builtin and its package import from the project's
/// `node_modules`, recorded with its origin, confined or not.
#[test]
fn a_run_loads_gro_s_library_from_its_copy() {
    let main = "import { where, via, cwd_kind } from '@fuzdev/gro/lib.ts';\n\
                if (where !== 'gro:lib/lib.ts') throw new Error(`where: ${where}`);\n\
                if (via !== 'UTIL') throw new Error(`via: ${via}`);\n\
                if (cwd_kind !== 'function') throw new Error(`cwd: ${cwd_kind}`);\n\
                const resolved = import.meta.resolve('@fuzdev/gro/lib.ts');\n\
                if (resolved !== 'gro:lib/lib.ts') throw new Error(`resolved: ${resolved}`);\n\
                const { fs_paths } = await import('@fuzdev/gro/fs_paths.ts');\n\
                if (fs_paths !== 'PATHS') throw new Error(`fs_paths: ${fs_paths}`);\n";
    for (case, authority) in [
        ("confined", granted()),
        ("unconfined", Authority::Unconfined),
    ] {
        let mut files = PLANTED.to_vec();
        files.push(("main.mjs", main));
        let (result, graph) = run_files(&format!("embedded_{case}"), &files, options(authority));
        assert!(result.is_ok(), "{case}: {result:?}");
        let lib = url::Url::parse("gro:lib/lib.ts").unwrap();
        assert!(
            matches!(&graph.records()[&lib], Record::Embedded(module) if module.path == "lib.ts"),
            "{case}"
        );
        assert_eq!(graph.origin(&lib), Some(&Origin::Embedded));
        let cwd = url::Url::parse(find_builtin("@fuzdev/gro/cwd.ts").unwrap().url).unwrap();
        assert!(
            matches!(graph.records()[&cwd], Record::Builtin(_)),
            "{case}"
        );
        let util = graph
            .roots()
            .iter()
            .find(|root| root.dir.ends_with("node_modules/util_pkg"));
        assert!(
            util.is_some_and(
                |root| matches!(&root.origin, Origin::Package { name, .. } if name == "util_pkg")
            ),
            "{case}: {:?}",
            graph.roots()
        );
        // nothing of the planted package was read
        assert!(
            !graph
                .consulted()
                .keys()
                .any(|path| path.to_string_lossy().contains("@fuzdev/gro")),
            "{case}: {:?}",
            graph.consulted()
        );
    }
}

/// The copy's loads, by [`counting_embed`].
static LOADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The test copy, counting each load.
fn counting_embed() -> Result<Embed<'static>, EmbedError> {
    LOADS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    test_embed()
}

/// A run loads its copy only when a specifier of Gro's library needs it:
/// never for a host module of Gro's package, a reserved specifier, or
/// anything else, and once for every one that does.
#[test]
fn a_run_loads_its_copy_only_when_it_names_gro_s_library() {
    use std::sync::atomic::Ordering;
    let options = || RuntimeOptions {
        embed: Some(counting_embed),
        ..options(granted())
    };
    let main = "import { cwd } from '@fuzdev/gro/cwd.ts';
                import { util } from 'util_pkg/x.js';
                try { await import('@fuzdev/gro/net.ts'); } catch {}
                export default [cwd, util, import.meta.resolve('./main.mjs')];
";
    let mut files = PLANTED.to_vec();
    files.push(("main.mjs", main));
    let (result, _) = run_files("embedded_unloaded", &files, options());
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(LOADS.load(Ordering::SeqCst), 0);
    let main = "await import('@fuzdev/gro/fs_paths.ts');
await import('@fuzdev/gro/lib.ts');
";
    let mut files = PLANTED.to_vec();
    files.push(("main.mjs", main));
    let (result, _) = run_files("embedded_loaded", &files, options());
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(LOADS.load(Ordering::SeqCst), 1);
}

/// An embedded module's import of a granted row is refused for a project
/// granted it, and in an unconfined run: Gro's library holds no grant.
#[test]
fn gro_s_library_holds_no_grant() {
    let fs = find_builtin("@fuzdev/gro/fs.ts").unwrap();
    for (case, authority) in [
        ("confined", granted()),
        ("unconfined", Authority::Unconfined),
    ] {
        let files = [("main.mjs", "import '@fuzdev/gro/granted.ts';\n")];
        let (result, graph) = run_files(
            &format!("embedded_grant_{case}"),
            &files,
            options(authority),
        );
        let report = uncaught(result);
        assert!(
            report.contains(
                "cannot import './fs.ts' from gro:lib/granted.ts: it needs the grant 'fs', and \
                 Gro's library holds no grant, whatever the run grants"
            ),
            "{case}: {report}"
        );
        let reasons: Vec<_> = graph
            .failures()
            .values()
            .flatten()
            .map(|failure| &failure.reason)
            .collect();
        assert_eq!(
            reasons,
            [&FailureReason::NotGranted {
                builtin: fs,
                origin: Origin::Embedded,
            }],
            "{case}"
        );
    }
}

/// An embedded module's `import()` resolves by the same rules: its copy's
/// module by a relative specifier, and a granted row refused, which fails
/// the run though the program catches it.
#[test]
fn an_embedded_module_s_import_follows_the_same_rules() {
    let main = "import { load } from '@fuzdev/gro/dyn.ts';\n\
                const { kept } = await load('./unstripped.ts');\n\
                if (kept !== 'a') throw new Error(`kept: ${kept}`);\n\
                try { await load('./fs.ts'); throw new Error('reached fs'); }\n\
                catch (error) { if (!(error instanceof TypeError)) throw error; }\n";
    let files = [("main.mjs", main)];
    let (outcome, _) = run_linked_outcome("embedded_dynamic", &files, &[], options(granted()));
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let messages = outcome.refused.messages();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(
        messages[0].contains("cannot import './fs.ts' from gro:lib/dyn.ts"),
        "{messages:?}"
    );
}

/// The copy's code loads as it is: a module the copy holds with a type
/// left in it fails to parse, never stripped again.
#[test]
fn an_embedded_module_loads_unstripped() {
    let files = [("main.mjs", "import '@fuzdev/gro/typed.ts';\n")];
    let (result, _) = run_files("embedded_typed", &files, options(granted()));
    let report = uncaught(result);
    assert!(report.contains("SyntaxError"), "{report}");
    assert!(report.contains("gro:lib/typed.ts"), "{report}");
}

/// The copy's URLs are refused from source, as every scheme but `file:`
/// is, so only a specifier of Gro's reaches one, and the refusal fails the
/// run though the program catches it.
#[test]
fn a_gro_url_is_refused_from_source() {
    let main = "try { await import('gro:lib/fs_paths.ts'); throw new Error('reached'); }\n\
                catch (error) { if (!(error instanceof TypeError)) throw error; }\n";
    let (outcome, _) = run_linked_outcome(
        "embedded_scheme",
        &[("main.mjs", main)],
        &[],
        options(granted()),
    );
    assert!(outcome.result.is_ok(), "{:?}", outcome.result);
    let messages = outcome.refused.messages();
    assert!(
        messages.len() == 1 && messages[0].contains("the scheme 'gro:' is not supported"),
        "{messages:?}"
    );
}

/// A run with no copy refuses a specifier of Gro's package that isn't a
/// builtin, never loading the planted package, and one left out of the
/// copy is refused as Node-only, never the planted package's either.
#[test]
fn what_the_copy_doesn_t_serve_never_reaches_a_package() {
    for (case, embed, fault) in [
        ("none", None, None),
        ("left_out", TEST_EMBED, Some(GroModuleFault::NodeOnly)),
    ] {
        let specifier = if fault.is_some() {
            "@fuzdev/gro/invoke.ts"
        } else {
            "@fuzdev/gro/lib.ts"
        };
        let mut files = PLANTED.to_vec();
        let main = format!("import {{ where }} from '{specifier}';\nconsole.log(where);\n");
        files.push(("main.mjs", &main));
        let options = RuntimeOptions {
            embed,
            ..options(granted())
        };
        let (result, graph) = run_files(&format!("embedded_unserved_{case}"), &files, options);
        let report = uncaught(result);
        assert!(!report.contains("PLANTED"), "{case}: {report}");
        let reasons: Vec<_> = graph
            .failures()
            .values()
            .flatten()
            .map(|failure| failure.reason.clone())
            .collect();
        let expected = fault.map_or(ResolveErrorKind::NoGroLibrary(None), |fault| {
            ResolveErrorKind::GroModule {
                path: Box::from("invoke.ts"),
                fault,
            }
        });
        assert_eq!(reasons, [FailureReason::Resolve(expected)], "{case}");
    }
}

/// An embedded module's import with attributes is refused as the
/// project's is: of a grant-free row named by a relative specifier, as a
/// builtin taking none, and of a module of the copy, as a module type past
/// JS.
#[test]
fn an_embedded_module_s_attributes_are_refused() {
    let cwd = find_builtin("@fuzdev/gro/cwd.ts").unwrap();
    for (case, main, expected) in [
        (
            "row",
            "import '@fuzdev/gro/attributed.ts';\n",
            FailureReason::Load(crate::loader::graph::LoadFailure::BuiltinAttributes(cwd)),
        ),
        (
            "module",
            "import { load } from '@fuzdev/gro/dyn.ts';\nawait load('./unstripped.ts', { with: { type: 'json' } });\n",
            FailureReason::Load(crate::loader::graph::LoadFailure::ModuleType),
        ),
    ] {
        let (result, graph) = run_files(
            &format!("embedded_attributes_{case}"),
            &[("main.mjs", main)],
            options(granted()),
        );
        assert!(result.is_err(), "{case}");
        let reasons: Vec<_> = graph
            .failures()
            .values()
            .flatten()
            .map(|failure| failure.reason.clone())
            .collect();
        assert_eq!(reasons, [expected], "{case}");
    }
}
