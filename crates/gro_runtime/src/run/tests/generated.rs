//! Tests for a run from an entry the embedder generated: it starts at its
//! `gro:main` URL with the root it was given, the project's, imports what
//! that origin is granted, Gro's library from the run's copy, and project
//! files by their `file:` URLs, and names its URL in a stack trace.

use std::path::Path;

use super::builtins::granted;
use super::embedded::TEST_EMBED;
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};
use crate::test_support::{TestDir, never_exits};
use crate::{
    Authority, GENERATED_ENTRY_URL, GeneratedEntry, RunError, RuntimeOptions, STACK_SIZE,
    run_generated,
};

/// Runs `source` as a generated entry whose root is `root`, started in
/// `cwd`, granted `authority` with the test copy of Gro's library.
fn run(source: &str, root: &Path, cwd: &Path, authority: Authority) -> Result<(), RunError> {
    run_flagged(source, root, cwd, authority).0
}

/// Runs as [`run`] does, returning whether a `node:` specifier was refused
/// beside the result.
fn run_flagged(
    source: &str,
    root: &Path,
    cwd: &Path,
    authority: Authority,
) -> (Result<(), RunError>, bool) {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let options = RuntimeOptions {
        authority,
        embed: TEST_EMBED,
        exit_step: Some(never_exits),
        args: vec!["check".to_owned()],
        ..RuntimeOptions::default()
    };
    let entry = GeneratedEntry {
        source: source.to_owned(),
        root: root.to_owned(),
    };
    let cwd = cwd.to_owned();
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let outcome = run_generated(engine, &options, &entry, &cwd);
            (outcome.result, outcome.node_specifier_refused)
        })
        .unwrap()
        .join()
        .unwrap()
}

/// The message of the error a run failed with.
fn uncaught(result: Result<(), RunError>) -> String {
    match result {
        Err(RunError::Uncaught(report)) => report.to_string(),
        other => panic!("not an uncaught error: {other:?}"),
    }
}

/// A project at a fresh directory, with a module, a package Gro's copy
/// imports, and a sibling directory outside it.
fn project(name: &str) -> (TestDir, std::path::PathBuf) {
    let dir = TestDir::on_disk(&format!("generated_{name}"));
    let root = dir.path().join("p");
    for (file, content) in [
        ("p/package.json", r#"{"name": "p", "type": "module"}"#),
        ("p/mod.js", "export const value = 'MOD';\n"),
        ("p/sub/x.js", "export const x = 'X';\n"),
        (
            "p/node_modules/util_pkg/package.json",
            r#"{"name": "util_pkg", "version": "1.2.0", "type": "module", "exports": {"./x.js": "./x.js"}}"#,
        ),
        (
            "p/node_modules/util_pkg/x.js",
            "export const util = 'UTIL';\n",
        ),
        ("outside/y.js", "export const y = 'Y';\n"),
    ] {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    (dir, root)
}

fn file_url(path: &Path) -> String {
    url::Url::from_file_path(path).unwrap().to_string()
}

/// The entry is the project's: it imports a row its grants give, Gro's
/// library, and a project file by its `file:` URL through `import()`, its
/// URL `gro:main`, and the cwd host module returns the cwd it was given,
/// though its root is another directory.
#[test]
fn a_generated_entry_imports_as_the_project_s_module() {
    let (dir, root) = project("imports");
    let cwd = root.join("sub");
    let source = format!(
        "import {{ args }} from '@fuzdev/gro/args.ts';\n\
         import {{ cwd }} from '@fuzdev/gro/cwd.ts';\n\
         import {{ where, via }} from '@fuzdev/gro/lib.ts';\n\
         const {{ value }} = await import({mod_url:?});\n\
         if (value !== 'MOD') throw new Error(`value: ${{value}}`);\n\
         if (args.join() !== 'check') throw new Error(`args: ${{args}}`);\n\
         if (where !== 'gro:lib/lib.ts' || via !== 'UTIL') throw new Error(`lib: ${{where}} ${{via}}`);\n\
         if (import.meta.url !== {url:?}) throw new Error(`url: ${{import.meta.url}}`);\n\
         if (cwd() !== {cwd:?}) throw new Error(`cwd: ${{cwd()}}`);\n",
        mod_url = file_url(&root.join("mod.js")),
        url = GENERATED_ENTRY_URL,
        cwd = cwd.to_str().unwrap(),
    );
    let result = run(&source, &root, &cwd, granted());
    assert!(result.is_ok(), "{result:?}");
    drop(dir);
}

/// Its imports resolve inside the root it was given, which is its own: a
/// file outside it is refused, and so is a relative specifier, which has
/// no path to join; a row the run doesn't grant is refused as the
/// project's.
#[test]
fn a_generated_entry_resolves_inside_its_root() {
    let (dir, root) = project("root");
    let outside = format!(
        "await import({:?});\n",
        file_url(&dir.path().join("outside/y.js"))
    );
    let message = uncaught(run(&outside, &root, &root, granted()));
    assert!(message.contains("outside"), "{message}");
    let relative = uncaught(run("await import('./mod.js');\n", &root, &root, granted()));
    assert!(relative.contains("./mod.js"), "{relative}");
    // the same import, granted, from the project's root
    let inside = format!("await import({:?});\n", file_url(&root.join("sub/x.js")));
    assert!(run(&inside, &root, &root, granted()).is_ok());
    let ungranted = uncaught(run(
        "import { fs } from '@fuzdev/gro/fs.ts';\n",
        &root,
        &root,
        Authority::default(),
    ));
    assert!(
        ungranted.contains("which this run doesn't grant"),
        "{ungranted}"
    );
}

/// A throw in the entry names its URL in the stack trace.
#[test]
fn a_generated_entry_s_frames_name_its_url() {
    let (dir, root) = project("frames");
    let message = uncaught(run(
        "const fail = () => { throw new Error('boom'); };\nfail();\n",
        &root,
        &root,
        granted(),
    ));
    assert!(message.contains("boom"), "{message}");
    assert!(
        message.contains(&format!("{GENERATED_ENTRY_URL}:2")),
        "{message}"
    );
    drop(dir);
}

/// A `node:` specifier the resolver refuses is flagged on the outcome,
/// caught or not, and by `import.meta.resolve` too; a refusal of another
/// kind, or none, flags nothing.
#[test]
fn a_refused_node_specifier_is_flagged() {
    let (dir, root) = project("node_specifier");
    let flagged = |source: &str| run_flagged(source, &root, &root, granted()).1;
    assert!(flagged(
        "try { await import('node:fs'); } catch {}\nthrow new Error('later');\n"
    ));
    assert!(flagged("await import('node:path');\n"));
    assert!(flagged(
        "try { import.meta.resolve('node:os'); } catch {}\n"
    ));
    assert!(!flagged(
        "try { await import('npm:x'); } catch {}\nthrow new Error('x');\n"
    ));
    assert!(!flagged("try { await import('missing_pkg'); } catch {}\n"));
    assert!(!flagged("throw new Error('x');\n"));
    drop(dir);
}
