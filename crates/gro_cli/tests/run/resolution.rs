//! Import resolution: package exports and refusals, symlinks, `type: module`,
//! world-writable directories, `node_modules`, and imports staying in their root.

use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc::RecvTimeoutError;

use crate::support;
use crate::support::{Scratch, fgro_command};

use crate::{REFUSALS_FAIL, fixtures, fixtures_url, run, run_in, temp_fixtures};

/// The fixtures' own `package.json` isn't a type marker (it has a `"name"`),
/// so it ends the search, and a package isn't looked up past it, in gro's
/// own `node_modules`, which has `svelte`.
#[test]
fn a_package_is_not_found_past_the_project_root() {
    let run = run("import_bare.js");
    assert_eq!(run.code, 1);
    let fixtures = fixtures().canonicalize().unwrap();
    let fixtures = fixtures.display();
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'svelte' from {}/import_bare.js: no package 'svelte' is \
             installed: no node_modules/svelte directory from the importer up to \
             {fixtures}/package.json, which ends the search, as every package.json outside \
             node_modules does except one holding nothing but \"type\" and \
             \"sideEffects\", and the node_modules beside {}/package.json is not searched (npm \
             and yarn workspaces, which install at their root, aren't supported yet). Fix: \
             install it into {fixtures}/node_modules\n",
            fixtures_url(),
            // gro's own, the repo root
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .unwrap()
                .display()
        )
    );
}

/// Refusals of what isn't there (no Node compat, no such package, no
/// extension, a strip's refusal), of a feature that isn't (an attribute's
/// key, a module type), and `import.meta.resolve`'s, which imports nothing,
/// are catchable and, caught, print nothing and leave the run's exit 0, so
/// probing for an optional dependency or a feature doesn't fail a run.
#[test]
fn dynamic_import_refusals_are_catchable() {
    let run = run("import_refused_dynamic.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 8, "{}", run.stdout);
    // `deno_core` rethrows every load failure as a `TypeError`, a strip's included
    for (line, (start, reason)) in lines.iter().zip([
        (
            "true \"TypeError: cannot resolve '",
            "there is no Node compat",
        ),
        (
            "true \"TypeError: cannot resolve '",
            "no package 'svelte' is installed",
        ),
        (
            "true \"TypeError: cannot resolve '",
            "the path has no extension",
        ),
        (
            "true \"TypeError: cannot strip an enum. Fix: ",
            "/enum.ts:2:1\"",
        ),
        (
            "true \"TypeError: cannot import './sibling.js' from ",
            "the import attribute 'lazy' is not supported",
        ),
        (
            "true \"TypeError: cannot load ",
            "import attributes are not supported yet",
        ),
        (
            "true \"TypeError: cannot resolve '@fuzdev/gro/args.js' from ",
            "it is the .js spelling of the builtin module",
        ),
        (
            "true \"TypeError: cannot resolve 'ext:core/ops' from ",
            "the scheme 'ext:' is not supported",
        ),
    ]) {
        assert!(line.starts_with(start), "{line}");
        assert!(line.contains(reason), "{line}");
    }
}

/// `ext:` modules user code once reached or that carry the most authority:
/// `deno_core`'s ops and `core`, and `deno_web`'s lazy ES modules, which
/// `deno_core` falls back to when the loader refuses their names unless the
/// runtime declares none. `gro_runtime`'s own tests cover every `ext:`
/// specifier the build knows; these run against the built binary, a snapshot
/// build's included.
const EXT_SPECIFIERS: [&str; 4] = [
    "ext:core/ops",
    "ext:core/mod.js",
    "ext:deno_web/locks.js",
    "ext:deno_web/webtransport.js",
];

#[test]
fn an_ext_specifier_is_refused_to_a_static_import() {
    for (i, specifier) in EXT_SPECIFIERS.iter().enumerate() {
        let source = format!("import * as m from {specifier:?};\nconsole.log('loaded', m);\n");
        let fixtures = temp_fixtures(&format!("ext_static_{i}"), &[("main.js", &source)]);
        let run = run_in(&fixtures, "main.js");
        assert_eq!(run.code, 1, "{specifier}: {}{}", run.stdout, run.stderr);
        assert_eq!(run.stdout, "", "{specifier}");
        assert!(
            run.stderr.starts_with(&format!(
                "error: cannot resolve '{specifier}' from file://{}/main.js: the scheme 'ext:' \
                 is not supported",
                fixtures.0.display()
            )),
            "{specifier}: {}",
            run.stderr
        );
    }
}

#[test]
fn an_ext_specifier_is_refused_to_import_and_import_meta_resolve() {
    // Rust's debug form of the strings is a JS array literal of them
    let source = format!(
        "for (const specifier of {EXT_SPECIFIERS:?}) {{
\tfor (const [form, attempt] of [
\t\t['import()', () => import(specifier)],
\t\t['import.meta.resolve', () => import.meta.resolve(specifier)],
\t]) {{
\t\ttry {{
\t\t\tconsole.log(form, 'resolved', specifier, await attempt());
\t\t}} catch (error) {{
\t\t\tconsole.log(form, error instanceof TypeError, error.message.replace(import.meta.url, '<main>'));
\t\t}}
\t}}
}}
"
    );
    let fixtures = temp_fixtures("ext_dynamic", &[("main.js", &source)]);
    let run = run_in(&fixtures, "main.js");
    // each `import()`'s refusal fails the run, caught as it is, and is listed
    assert_eq!(run.code, 1, "{}", run.stderr);
    let listed: Vec<String> = EXT_SPECIFIERS
        .iter()
        .map(|specifier| {
            format!(
                "error: import() refused: cannot resolve '{specifier}' from \
                 file://{}/main.js: the scheme 'ext:' is not supported; only files on disk \
                 (file: URLs) load",
                fixtures.0.display()
            )
        })
        .chain([REFUSALS_FAIL.trim_end().to_owned()])
        .collect();
    assert_eq!(run.stderr.lines().collect::<Vec<_>>(), listed);
    let expected: String = EXT_SPECIFIERS
        .iter()
        .flat_map(|specifier| {
            ["import()", "import.meta.resolve"].map(|form| {
                format!(
                    "{form} true cannot resolve '{specifier}' from <main>: the scheme 'ext:' \
                         is not supported; only files on disk (file: URLs) load\n"
                )
            })
        })
        .collect();
    assert_eq!(run.stdout, expected);
}

/// An editor saves a file by writing a new one beside it and renaming it
/// over the old. A module loaded before that is served from the module map
/// after it, never read again: a dynamic `import()` of it runs, and so does
/// one of a new module that imports it, both with the first file's code.
/// The run waits for the rename by importing `ready.js` until it exists.
#[test]
fn a_loaded_module_renamed_over_is_served_from_the_module_map() {
    let main = "const first = await import('./a.js');
console.log('loaded', first.a);
let ready = false;
for (let i = 0; i < 3000 && !ready; i++) {
\ttry {
\t\tawait import('./ready.js');
\t\tready = true;
\t} catch {
\t\tawait new Promise((resolve) => setTimeout(resolve, 10));
\t}
}
const again = await import('./a.js');
const {b} = await import('./b.js');
console.log(ready, again.a, b);
";
    let fixtures = temp_fixtures(
        "renamed_over",
        &[
            ("main.js", main),
            ("a.js", "export const a = 1;\n"),
            (
                "b.js",
                "import {a} from './a.js';\nexport const b = a + 1;\n",
            ),
        ],
    );
    let mut child = support::KillOnDrop(
        fgro_command(&["run", "main.js"])
            .current_dir(&fixtures.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = support::lines(child.stdout.take().unwrap());
    let stderr = support::drain(child.stderr.take().unwrap());
    let line = stdout.recv_timeout(support::DEADLINE);
    assert_eq!(line.as_deref(), Ok("loaded 1\n"));
    // made before the rename, so it can't reuse the replaced file's inode
    let replacement = fixtures.0.join("a.js.replacement");
    std::fs::write(&replacement, "export const a = 2;\n").unwrap();
    std::fs::rename(replacement, fixtures.0.join("a.js")).unwrap();
    std::fs::write(fixtures.0.join("ready.js"), "export {};\n").unwrap();
    // the run ends by itself: its wait for `ready.js` is bounded
    let mut rest = String::new();
    // the channel closes with the pipe, as the run exits
    loop {
        match stdout.recv_timeout(support::DEADLINE) {
            Ok(line) => rest.push_str(&line),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => panic!("the run didn't end: {rest}"),
        }
    }
    let status = child.wait().unwrap();
    let stderr = String::from_utf8(stderr.join().unwrap()).unwrap();
    assert_eq!(rest, "true 1 2\n", "{stderr}");
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_symlinked_module_resolves_its_imports_from_its_real_location() {
    let run = run("import_link.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "neighbor via true\n");
}

#[test]
fn a_directory_named_like_a_module_is_refused() {
    let run = run("import_dir.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve './dir.js' from {}/import_dir.js: it is not a regular file (a \
             directory or a special file); name a module file\n",
            fixtures_url()
        )
    );
}

/// Packages in `node_modules` resolve through their `"exports"`: a scoped
/// package by the `gro` condition, its TypeScript stripped there too, and
/// from under a type marker `package.json`; a linked package, whose own
/// dependency resolves from where it really lives; a nested layout's
/// dependency, beside it; and the project's own package by its name. A
/// linked package's imports resolve from its real path, so one without the
/// dependency installed beside it doesn't reach the project's.
#[test]
fn packages_resolve_through_their_exports() {
    let nested = "node_modules/.deno/nested@1.0.0/node_modules";
    let fixtures = temp_fixtures(
        "packages",
        &[
            (
                "package.json",
                r#"{"name": "app", "type": "module", "exports": {"./self.js": "./self.js"}}"#,
            ),
            ("self.js", "export default 'self-reference';\n"),
            (
                "main.js",
                "import a from '@scope/a';\n\
                 import b from '@scope/a/b.ts';\n\
                 import linked from 'linked';\n\
                 import nested from 'nested';\n\
                 import self from 'app/self.js';\n\
                 import marker from './dist/esm/marker.js';\n\
                 console.log([a, b, linked, nested, self, marker].join('\\n'));\n",
            ),
            ("dist/esm/package.json", r#"{"type": "module"}"#),
            (
                "dist/esm/marker.js",
                "import a from '@scope/a';\nexport default `past a marker, ${a}`;\n",
            ),
            ("lonely.js", "import 'lonely';\n"),
            (
                "lonely_checkout/package.json",
                r#"{"name": "lonely", "type": "module", "exports": "./i.js"}"#,
            ),
            ("lonely_checkout/i.js", "import 'dep';\n"),
            ("node_modules/dep/package.json", r#"{"exports": "./i.mjs"}"#),
            (
                "node_modules/dep/i.mjs",
                "export default 'the project dep';\n",
            ),
            (
                "node_modules/@scope/a/package.json",
                r#"{"type": "module", "exports": {".": {"gro": "./gro.js", "default": "./default.js"}, "./*.ts": "./src/*.ts"}}"#,
            ),
            (
                "node_modules/@scope/a/gro.js",
                "export default 'gro condition';\n",
            ),
            (
                "node_modules/@scope/a/src/b.ts",
                "const b: string = 'stripped in node_modules';\nexport default b;\n",
            ),
            (
                "linked_checkout/package.json",
                r#"{"type": "module", "exports": "./i.js"}"#,
            ),
            (
                "linked_checkout/i.js",
                "import dep from 'dep';\nexport default `linked, ${dep}`;\n",
            ),
            (
                "linked_checkout/node_modules/dep/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            (
                "linked_checkout/node_modules/dep/i.mjs",
                "export default 'its own dependency';\n",
            ),
            (
                &format!("{nested}/nested/package.json"),
                r#"{"exports": "./i.mjs"}"#,
            ),
            (
                &format!("{nested}/nested/i.mjs"),
                "import dep from 'nested_dep';\nexport default `nested, ${dep}`;\n",
            ),
            (
                &format!("{nested}/nested_dep/package.json"),
                r#"{"exports": "./i.mjs"}"#,
            ),
            (
                &format!("{nested}/nested_dep/i.mjs"),
                "export default 'its dependency beside it';\n",
            ),
        ],
    );
    let root = &fixtures.0;
    std::os::unix::fs::symlink(
        root.join("linked_checkout"),
        root.join("node_modules/linked"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        root.join(nested).join("nested"),
        root.join("node_modules/nested"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        root.join("lonely_checkout"),
        root.join("node_modules/lonely"),
    )
    .unwrap();
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "gro condition\nstripped in node_modules\nlinked, its own dependency\n\
         nested, its dependency beside it\nself-reference\npast a marker, gro condition\n"
    );
    // from the symlink's path, the search would reach the project's node_modules/dep
    let run = run_in(&fixtures, "lonely.js");
    assert_eq!(run.code, 1);
    let root = root.display();
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'dep' from file://{root}/lonely_checkout/i.js: no package \
             'dep' is installed: no node_modules/dep directory from the importer up to \
             {root}/lonely_checkout/package.json, which ends the search, as every package.json \
             outside node_modules does except one holding nothing but \"type\" and \
             \"sideEffects\", and the node_modules beside {root}/package.json is not searched \
             (npm and yarn workspaces, which install at their root, aren't supported yet). Fix: \
             install it into {root}/lonely_checkout/node_modules\n"
        )
    );
}

#[test]
fn package_refusals_name_the_reason_and_a_fix() {
    let fixtures = temp_fixtures(
        "package_refusals",
        &[
            (
                "package.json",
                r#"{"name": "package_refusals", "type": "module"}"#,
            ),
            ("main_only.js", "import 'main_only';\n"),
            (
                "node_modules/main_only/package.json",
                r#"{"main": "./i.js"}"#,
            ),
            ("node_modules/main_only/i.js", "export {};\n"),
            ("commonjs.js", "import 'commonjs';\n"),
            (
                "node_modules/commonjs/package.json",
                r#"{"exports": "./i.js"}"#,
            ),
            ("node_modules/commonjs/i.js", "module.exports = 1;\n"),
        ],
    );
    let root = fixtures.0.display();
    let run = run_in(&fixtures, "main_only.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'main_only' from file://{root}/main_only.js: \
             {root}/node_modules/main_only/package.json has no \"exports\" field, and only \
             \"exports\" is read (not \"main\" or \"module\"). Fix: use a version with an ESM \
             build, vendor an ESM bundle of it into the project, or run it under Node\n"
        )
    );
    let run = run_in(&fixtures, "commonjs.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'commonjs' from file://{root}/commonjs.js: the package \
             exports it as {root}/node_modules/commonjs/i.js, which is CommonJS (no \"type\": \
             \"module\", not .mjs), and only ES modules load. Fix: use a version with an ESM \
             build, vendor an ESM bundle of it into the project, or run it under Node\n"
        )
    );
}

/// A package linked to itself is a symlink loop, named as one by the OS's
/// `ELOOP`.
#[test]
fn a_symlink_loop_is_named() {
    let fixtures = temp_fixtures(
        "symlink_loop",
        &[
            (
                "package.json",
                r#"{"name": "symlink_loop", "type": "module"}"#,
            ),
            ("main.js", "import 'looped';\n"),
        ],
    );
    let root = &fixtures.0;
    std::fs::create_dir_all(root.join("node_modules")).unwrap();
    std::os::unix::fs::symlink(
        root.join("node_modules/looped"),
        root.join("node_modules/looped"),
    )
    .unwrap();
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 1);
    let root = root.display();
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'looped' from file://{root}/main.js: \
             {root}/node_modules/looped leads into a symlink loop, so it names no file or \
             directory. Fix: remove the looping link, or point it at what it should name (for a \
             package, reinstall it)\n"
        )
    );
}

/// An entry that fails to resolve on a symlink loop, an I/O error rather
/// than an entry that isn't there, exits 1, not 2, naming the loop.
#[test]
fn an_entry_in_a_symlink_loop_exits_1() {
    let fixtures = temp_fixtures("entry_loop", &[]);
    let root = &fixtures.0;
    std::os::unix::fs::symlink("b.js", root.join("a.js")).unwrap();
    std::os::unix::fs::symlink("a.js", root.join("b.js")).unwrap();
    let run = run_in(&fixtures, "a.js");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let root = root.display();
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'a.js' from {root}: {root}/a.js leads into a symlink loop, so \
             it names no file or directory. Fix: remove the looping link, or point it at what it \
             should name (for a package, reinstall it)\n"
        )
    );
}

/// A `.js` file is an ES module only under `"type": "module"`, where Node
/// would guess from the code; `.mjs` needs no `package.json`.
#[test]
fn a_js_module_needs_type_module() {
    let fixtures = Scratch::in_system_temp(
        "no_package_json",
        &[
            ("a.js", "console.log('a');\n"),
            ("b.mjs", "console.log('b');\n"),
            ("commonjs/package.json", "{}\n"),
            ("commonjs/c.js", "console.log('c');\n"),
        ],
    );
    // the search stops at the temp dir, world-writable
    let run = run_in(&fixtures, "a.js");
    assert_eq!(run.code, 2);
    let reason = format!(
        "and the search for one stopped at {}, which is world-writable, so nothing in it or \
         above it is read (the format is never guessed from the code). Fix: rename the file to \
         .mjs, or move it under a package.json with \"type\": \"module\" in a directory that \
         isn't world-writable",
        support::system_temp().display()
    );
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'a.js' from {}: a .js file is an ES module only under a \
             package.json with \"type\": \"module\", {reason}\n",
            fixtures.0.display()
        )
    );
    let run = run_in(&fixtures, "commonjs/c.js");
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains(&format!(
            "which {}/commonjs/package.json does not set",
            fixtures.0.display()
        )),
        "{}",
        run.stderr
    );
    let run = run_in(&fixtures, "b.mjs");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "b\n");
}

/// A world-writable directory ends every walk up, and no `package.json` or
/// `node_modules` in it is read:
/// its `package.json` doesn't bound a project below it or name a
/// self-reference, and its `node_modules` isn't searched. A module directly
/// in it still runs, with relative imports only.
#[test]
fn a_world_writable_directory_ends_every_walk() {
    let fixtures = temp_fixtures(
        "world_writable",
        &[
            (
                "shared/package.json",
                r#"{"name": "shared", "type": "module", "exports": {"./evil.ts": "./evil.ts"}}"#,
            ),
            ("shared/evil.ts", "console.log('evil');\n"),
            (
                "shared/node_modules/foo/package.json",
                r#"{"exports": "./i.mjs"}"#,
            ),
            ("shared/node_modules/foo/i.mjs", "console.log('planted');\n"),
            ("shared/loose/main.ts", "import 'foo';\n"),
            ("shared/loose/self.ts", "import 'shared/evil.ts';\n"),
            ("shared/direct.ts", "import './sibling.mjs';\n"),
            ("shared/sibling.mjs", "console.log('relative');\n"),
        ],
    );
    fixtures.set_mode("shared", 0o1777);
    let root = fixtures.0.display();
    let no_project_root = |specifier: &str, file: &str| {
        format!(
            "error: cannot resolve '{specifier}' from file://{root}/shared/loose/{file}: no \
             project root is above {root}/shared/loose: a bare specifier resolves only under a \
             package.json outside node_modules that holds more than \"type\" and \
             \"sideEffects\", and the search for one stopped at {root}/shared, which is \
             world-writable, so nothing in it or above it is read. Fix: add a package.json at \
             the project root, with the package installed in the node_modules beside it\n"
        )
    };
    let run = run_in(&fixtures, "shared/loose/main.ts");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, no_project_root("foo", "main.ts"));
    let run = run_in(&fixtures, "shared/loose/self.ts");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, no_project_root("shared/evil.ts", "self.ts"));
    let run = run_in(&fixtures, "shared/direct.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "relative\n");
}

/// A package installed through a world-writable directory is refused, here
/// a `node_modules` any user could plant a package in, though the project
/// above it is sound.
#[test]
fn a_world_writable_node_modules_is_refused() {
    let fixtures = temp_fixtures(
        "world_writable_node_modules",
        &[
            ("package.json", r#"{"name": "proj", "type": "module"}"#),
            ("node_modules/foo/package.json", r#"{"exports": "./i.mjs"}"#),
            ("node_modules/foo/i.mjs", "console.log('planted');\n"),
            ("main.ts", "import 'foo';\n"),
        ],
    );
    fixtures.set_mode("node_modules", 0o777);
    let run = run_in(&fixtures, "main.ts");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let root = fixtures.0.display();
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'foo' from file://{root}/main.ts: package 'foo' is found \
             through {root}/node_modules, which is world-writable, so nothing in it is read: any \
             user can plant or replace a package there. Fix: remove the directory's other-write \
             bit (chmod o-w), then reinstall the package\n"
        )
    );
}

/// A relative or absolute specifier resolves only inside its importer's
/// root: the project's, never into `node_modules` below it or into a
/// package admitted inside it, and a package's own, never out of it. Every
/// resolution is checked by its real path, so an `"exports"` target or a
/// self-reference symlinked out of its package is refused too.
#[test]
fn imports_stay_inside_their_root() {
    let fixtures = temp_fixtures(
        "containment",
        &[
            ("package.json", r#"{"name": "proj", "type": "module"}"#),
            (
                "node_modules/bar/package.json",
                r#"{"type": "module", "exports": "./i.mjs"}"#,
            ),
            ("node_modules/bar/i.mjs", "export default 'bar';\n"),
            (
                "node_modules/bar/internal/secret.mjs",
                "console.log('secret');\n",
            ),
            (
                "node_modules/foo/package.json",
                r#"{"name": "foo", "type": "module", "exports": {".": "./i.mjs", "./out.mjs": "./out.mjs", "./self.mjs": "./self.mjs"}}"#,
            ),
            (
                "node_modules/foo/i.mjs",
                "import '../bar/internal/secret.mjs';\n",
            ),
            ("node_modules/foo/self.mjs", "import 'foo/out.mjs';\n"),
            (
                "packages/lnk/package.json",
                r#"{"name": "lnk", "type": "module", "exports": {"./x.ts": "./x.ts"}}"#,
            ),
            ("packages/lnk/x.ts", "console.log('x');\n"),
            ("elsewhere/evil.mjs", "console.log('evil');\n"),
            (
                "into_node_modules.ts",
                "import './node_modules/bar/internal/secret.mjs';\n",
            ),
            ("sibling.ts", "import 'foo';\n"),
            (
                "relative_first.ts",
                "import './packages/lnk/x.ts';\nimport 'lnk/x.ts';\n",
            ),
            (
                "name_first.ts",
                "import 'lnk/x.ts';\nimport './packages/lnk/x.ts';\n",
            ),
            ("export_out.ts", "import 'foo/out.mjs';\n"),
            ("self_out.ts", "import 'foo/self.mjs';\n"),
            (
                "meta_resolve_admits_nothing.ts",
                "import './packages/lnk/x.ts';\nconsole.log(import.meta.resolve('lnk/x.ts'));\n",
            ),
            (
                "meta_resolve.ts",
                "try {\n  await import(import.meta.resolve('bar'));\n} catch (error) {\n  \
                 console.log(String(error));\n}\n",
            ),
        ],
    );
    let root = &fixtures.0;
    std::os::unix::fs::symlink(root.join("packages/lnk"), root.join("node_modules/lnk")).unwrap();
    // the target is in the project, but outside `foo`
    std::os::unix::fs::symlink(
        root.join("elsewhere/evil.mjs"),
        root.join("node_modules/foo/out.mjs"),
    )
    .unwrap();
    let root = root.display();
    let refused = |file: &str, stderr: String| {
        let run = run_in(&fixtures, file);
        assert_eq!(run.code, 1, "{file}: {}", run.stderr);
        assert_eq!(run.stdout, "", "{file}");
        assert_eq!(run.stderr, stderr, "{file}");
    };
    refused(
        "into_node_modules.ts",
        format!(
            "error: cannot resolve './node_modules/bar/internal/secret.mjs' from \
             file://{root}/into_node_modules.ts: it resolves to \
             {root}/node_modules/bar/internal/secret.mjs, inside node_modules below {root}, which \
             only a bare specifier reaches, through the package's \"exports\". Fix: import it by \
             name\n"
        ),
    );
    refused(
        "sibling.ts",
        format!(
            "error: cannot resolve '../bar/internal/secret.mjs' from \
             file://{root}/node_modules/foo/i.mjs: it resolves to \
             {root}/node_modules/bar/internal/secret.mjs, outside package 'foo' at \
             {root}/node_modules/foo, and a package's relative and absolute specifiers resolve \
             only inside it, so the package reaches outside itself. Fix: use a version of the \
             package that doesn't\n"
        ),
    );
    refused(
        "relative_first.ts",
        format!(
            "error: cannot resolve 'lnk/x.ts' from file://{root}/relative_first.ts: \
             {root}/packages/lnk/x.ts was resolved inside the project at {root}, and package \
             'lnk' at {root}/packages/lnk was then admitted around it, so its origin would depend \
             on the import order. Fix: import it by name ('lnk' or a subpath it exports) \
             everywhere, not by a relative path\n"
        ),
    );
    refused(
        "name_first.ts",
        format!(
            "error: cannot resolve './packages/lnk/x.ts' from file://{root}/name_first.ts: it \
             resolves to {root}/packages/lnk/x.ts, inside package 'lnk' at {root}/packages/lnk, \
             which only a bare specifier reaches, through its \"exports\". Fix: import it by name \
             ('lnk' or a subpath it exports)\n"
        ),
    );
    let out = |importer: &str| {
        format!(
            "error: cannot resolve 'foo/out.mjs' from file://{root}/{importer}: the package \
             exports it as {root}/node_modules/foo/out.mjs, which resolves to \
             {root}/elsewhere/evil.mjs, outside {root}/node_modules/foo, and an \"exports\" \
             target stays inside its package. Fix: use a version of the package whose \
             \"exports\" stay inside it\n"
        )
    };
    refused("export_out.ts", out("export_out.ts"));
    refused("self_out.ts", out("node_modules/foo/self.mjs"));
    // `import.meta.resolve` admits nothing: admitting `lnk` around the file the project
    // already holds would be a conflict
    let run = run_in(&fixtures, "meta_resolve_admits_nothing.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, format!("x\nfile://{root}/packages/lnk/x.ts\n"));
    // so what it returns for an installed package is a path into node_modules,
    // whose `import()` a rule refuses, which fails the run though it's caught
    let run = run_in(&fixtures, "meta_resolve.ts");
    assert_eq!(run.code, 1, "{}", run.stderr);
    let refusal = format!(
        "cannot resolve 'file://{root}/node_modules/bar/i.mjs' from \
         file://{root}/meta_resolve.ts: it resolves to {root}/node_modules/bar/i.mjs, inside \
         node_modules below {root}, which only a bare specifier reaches, through the package's \
         \"exports\". Fix: import it by name\n"
    );
    assert_eq!(run.stdout, format!("TypeError: {refusal}"));
    assert_eq!(
        run.stderr,
        format!("error: import() refused: {refusal}{REFUSALS_FAIL}")
    );
}

/// An entry inside `node_modules` runs as the package it is in, so its
/// relative imports stay inside that package.
#[test]
fn an_entry_inside_node_modules_runs_as_its_package() {
    let fixtures = temp_fixtures(
        "entry_in_node_modules",
        &[
            ("package.json", r#"{"name": "proj", "type": "module"}"#),
            ("main.mjs", "console.log('project');\n"),
            (
                "node_modules/foo/package.json",
                r#"{"name": "foo", "type": "module", "exports": {"./own.mjs": "./own.mjs"}}"#,
            ),
            ("node_modules/foo/own.mjs", "console.log('own');\n"),
            (
                "node_modules/foo/cli.mjs",
                "import './own.mjs';\nimport 'foo/own.mjs';\n",
            ),
            ("node_modules/foo/up.mjs", "import '../../main.mjs';\n"),
            ("node_modules/.bin/real.mjs", "console.log('real');\n"),
        ],
    );
    let run = run_in(&fixtures, "node_modules/foo/cli.mjs");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "own\n");
    let run = run_in(&fixtures, "node_modules/foo/up.mjs");
    assert_eq!(run.code, 1);
    let root = fixtures.0.display();
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve '../../main.mjs' from file://{root}/node_modules/foo/up.mjs: \
             it resolves to {root}/main.mjs, outside package 'foo' at {root}/node_modules/foo, \
             and a package's relative and absolute specifiers resolve only inside it, so the \
             package reaches outside itself. Fix: use a version of the package that doesn't\n"
        )
    );
    // a file in node_modules but in no package has no origin: a usage error
    let run = run_in(&fixtures, "node_modules/.bin/real.mjs");
    assert_eq!(run.code, 2);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'node_modules/.bin/real.mjs' from {root}: it resolves to \
             {root}/node_modules/.bin/real.mjs, inside node_modules but in no package, so it has \
             no origin. Fix: run a file inside a package (node_modules/<name>/...)\n"
        )
    );
}

/// A `node_modules` planted above a directory with no project root is never
/// searched: the walk for the bound comes first, and with none, a bare
/// specifier other than a self-reference is refused.
#[test]
fn a_node_modules_above_a_package_less_dir_is_never_searched() {
    let fixtures = Scratch::in_system_temp(
        "planted_above",
        &[
            ("node_modules/foo/package.json", r#"{"exports": "./i.mjs"}"#),
            ("node_modules/foo/i.mjs", "console.log('planted');\n"),
            ("x/zap.ts", "import 'foo';\n"),
        ],
    );
    // the search stops at the temp dir, world-writable
    let run = run_in(&fixtures, "x/zap.ts");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let root = fixtures.0.display();
    let end = format!(
        "stopped at {}, which is world-writable, so nothing in it or above it is read",
        support::system_temp().display()
    );
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'foo' from file://{root}/x/zap.ts: no project root is above \
             {root}/x: a bare specifier resolves only under a package.json outside node_modules \
             that holds more than \"type\" and \"sideEffects\", and the search for one {end}. \
             Fix: add a package.json at the project root, with the package installed in the \
             node_modules beside it\n"
        )
    );
}
