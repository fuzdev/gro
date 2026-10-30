//! `fgro run` end to end: the built binary on the scripts in
//! `tests/fixtures/run/`, run from that directory with a relative path.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test's setup fails by panicking"
)]
#![expect(clippy::panic, reason = "a hung child fails its test by panicking")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// How long a child may run before it's killed and its test fails, generous
/// for a debug build on a loaded machine.
const DEADLINE: Duration = Duration::from_secs(60);

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run")
}

/// The fixtures directory as a `file:` URL prefix, as error positions name it.
fn fixtures_url() -> String {
    format!("file://{}", fixtures().canonicalize().unwrap().display())
}

/// Runs `command` with stdout and stderr piped, killing it at the deadline so
/// a run that never ends fails its test instead of hanging the suite. The
/// fixtures print little, well under a pipe's buffer, so polling can't
/// deadlock on a full pipe.
fn output_within_deadline(command: &mut Command) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > DEADLINE {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("killed after {DEADLINE:?}: {command:?}\n{output:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.wait_with_output().unwrap()
}

/// The variables the host's color decision reads, all of them, so a test sets
/// what it needs and the harness's own environment (Claude Code's
/// `CLAUDECODE`, a CI's `FORCE_COLOR`) doesn't leak in.
const COLOR_VARS: [&str; 5] = [
    "FORCE_COLOR",
    "NO_COLOR",
    "NODE_DISABLE_COLORS",
    "CLAUDECODE",
    "TERM",
];

/// A command for `fgro` with every color variable cleared.
fn fgro_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fgro"));
    for name in COLOR_VARS {
        command.env_remove(name);
    }
    command
}

/// Runs `fgro` with `args` from the fixtures directory, with stdout and stderr
/// piped (never terminals) and no color variable set, so console output is
/// uncolored.
fn fgro(args: &[&str]) -> Output {
    fgro_with_env(args, &[])
}

/// [`fgro`] with `env` set.
fn fgro_with_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    output_within_deadline(
        fgro_command()
            .args(args)
            .envs(env.iter().copied())
            .current_dir(fixtures()),
    )
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(file: &str) -> Run {
    let output = fgro(&["run", file]);
    let run = Run {
        code: output
            .status
            .code()
            .expect("the run exits, not killed by a signal"),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    };
    // console output is uncolored when it isn't going to a terminal
    assert!(!run.stdout.contains('\x1b'), "{}", run.stdout);
    assert!(!run.stderr.contains('\x1b'), "{}", run.stderr);
    run
}

#[test]
fn console_writes_log_to_stdout_and_warn_and_error_to_stderr() {
    let run = run("console.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "log 1 { a: [ 1, \"two\" ] }\ninfo\n");
    assert_eq!(run.stderr, "warn\nerror Map(1) { \"k\" => 1 }\n");
}

// A top-level throw reads `Uncaught (in promise)` by design: evaluating a
// module settles a promise, and Deno prints the same.
#[test]
fn uncaught_error_exits_1_with_positions() {
    let run = run("throw.js");
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: boom\n    at fail ({url}/throw.js:2:8)\n    at {url}/throw.js:4:1\n"
        )
    );
}

#[test]
fn unhandled_rejection_exits_1() {
    let run = run("reject.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: rejected\n    at {}/reject.js:1:16\n",
            fixtures_url()
        )
    );
}

#[test]
fn error_in_a_timer_exits_1() {
    let run = run("timer_throw.js");
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.starts_with(&format!(
            "error: Uncaught TypeError: late\n    at {}/timer_throw.js:2:8\n",
            fixtures_url()
        )),
        "{}",
        run.stderr
    );
}

#[test]
fn host_internals_are_not_visible() {
    let run = run("globals.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout.trim_end(),
        concat!(
            r#"{"deno":"undefined","bootstrap":"undefined","deno_core":"undefined","#,
            r#""own":[],"enumerable":[],"fetch":"undefined","report_error":"undefined","#,
            r#""web":["function","function","function","function","function","function","object","function"]}"#,
        )
    );
}

/// Every global name user code sees, pinned in `global_names.txt`, one a line,
/// sorted: a `deno_core` or V8 upgrade that adds a global (V8's flags have
/// brought `Temporal` and `DisposableStack`) fails here rather than widening
/// the surface silently. To accept a change deliberately, write the fixture's
/// output over the file, from the fixtures directory:
/// `../../../../../target/debug/fgro run global_names.js > global_names.txt`.
#[test]
fn the_global_names_are_pinned() {
    let run = run("global_names.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let expected = std::fs::read_to_string(fixtures().join("global_names.txt")).unwrap();
    let names = |text: &str| -> std::collections::BTreeSet<String> {
        text.lines().map(str::to_owned).collect()
    };
    let (seen, pinned) = (names(&run.stdout), names(&expected));
    let added: Vec<&String> = seen.difference(&pinned).collect();
    let removed: Vec<&String> = pinned.difference(&seen).collect();
    assert!(
        added.is_empty() && removed.is_empty() && run.stdout == expected,
        "the global names differ from tests/fixtures/run/global_names.txt\n  added: {added:?}\n  removed: {removed:?}\n\
         update the file only if the change is deliberate (see this test's doc)"
    );
}

#[test]
fn web_globals_work() {
    let run = run("web_globals.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "url=file:///a/b?q=1#h\nsearch=a=1&b=x+y\npattern=7\nencoding=4\nbase64=gro\nclone=2\n\
         dom_exception=AbortError\nevent=pong\nabort=true\nblob=abc\nfile=f.txt\n\
         stream=streamed\ncompression=150\nperformance=true\n"
    );
}

/// The time origin is set per process, so two runs differ, and it agrees with
/// the wall clock: `timeOrigin + now()` is `Date.now()` within a second. A
/// time origin baked into a snapshot fails both.
#[test]
fn time_origin_is_per_process() {
    let origin_and_drift = || {
        let run = run("time_origin.js");
        assert_eq!(run.code, 0, "{}", run.stderr);
        let line = run.stdout.trim_end();
        let field = |name: &str| -> f64 {
            let start = line.find(&format!("\"{name}\":")).unwrap() + name.len() + 3;
            let end = line[start..].find([',', '}']).unwrap() + start;
            line[start..end].parse().unwrap()
        };
        (field("origin"), field("drift"))
    };
    let (first, first_drift) = origin_and_drift();
    let (second, second_drift) = origin_and_drift();
    assert!(first < second, "{first} then {second}");
    for drift in [first_drift, second_drift] {
        assert!(drift.abs() < 1000.0, "drift {drift} ms");
    }
}

#[test]
fn static_import_of_a_sibling() {
    let run = run("import_static.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "static sibling\n");
}

#[test]
fn dynamic_import_of_a_sibling() {
    let run = run("import_dynamic.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "dynamic sibling true\n");
}

#[test]
fn typescript_import_runs_stripped() {
    let run = run("import_ts.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "typed\n");
}

/// Interfaces, generics, `as`, `satisfies`, and `import type` (of a module
/// that doesn't exist, which is never resolved) all blank away, and a `.mts`
/// import loads.
#[test]
fn typescript_entry_runs_its_type_only_constructs() {
    let run = run("entry.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "9 typed undefined\n");
}

/// Blanking keeps every character where it was, so V8's positions are the
/// source's: `new` follows a stripped annotation on its line.
#[test]
fn typescript_positions_are_the_source_positions() {
    let run = run("throw.ts");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: boom\n    at {}/throw.ts:2:22\n",
            fixtures_url()
        )
    );
}

/// Columns count UTF-16 units, V8's and the strip's alike.
#[test]
fn typescript_columns_count_utf16_units() {
    let run = run("throw_astral.ts");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: astral\n    at {}/throw_astral.ts:2:38\n",
            fixtures_url()
        )
    );
}

/// TypeScript with runtime semantics is refused, not transformed: a program
/// error (exit 1) naming the construct, the fix, and its position.
#[test]
fn an_enum_is_refused_with_its_position() {
    let run = run("enum.ts");
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot strip an enum. Fix: use a union of literal types, or an object with \
             'as const' (TypeScript's erasableSyntaxOnly option reports these)\n    at \
             {}/enum.ts:2:1\n",
            fixtures_url()
        )
    );
}

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
             and yarn workspaces, which install at their root, aren't supported). Fix: install it \
             into {fixtures}/node_modules\n",
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

#[test]
fn dynamic_import_refusals_are_catchable() {
    let run = run("import_refused_dynamic.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{}", run.stdout);
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
    ]) {
        assert!(line.starts_with(start), "{line}");
        assert!(line.contains(reason), "{line}");
    }
}

/// A directory of its own for `files`, so sources that must not pass the
/// format check (a parse error) or that it skips (`.tsx`, `.cts`) live
/// outside the fixtures. Removed on drop.
struct TempFixtures(PathBuf);

impl TempFixtures {
    /// The files, under a `package.json` with `"type": "module"` unless they
    /// have their own.
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let fixtures = Self::without_package_json(name, files);
        let package_json = fixtures.0.join("package.json");
        if !package_json.exists() {
            std::fs::write(package_json, "{\"type\": \"module\"}\n").unwrap();
        }
        fixtures
    }

    /// The files alone, each at its relative path. Every directory's mode
    /// is set to `0o755`, so a umask of 0 can't make one world-writable,
    /// which would end every walk up from the files at it.
    fn without_package_json(name: &str, files: &[(&str, &str)]) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("fgro_run_test_{}_{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (file, content) in files {
            let path = dir.join(file);
            for parent in path.ancestors().skip(1) {
                if parent == dir {
                    break;
                }
                std::fs::create_dir_all(parent).unwrap();
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self(dir.canonicalize().unwrap())
    }

    /// Sets the mode of the directory at `path`, relative to the fixtures.
    fn set_mode(&self, path: &str, mode: u32) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(self.0.join(path), std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// The first world-writable directory at or above the fixtures, where
    /// every walk up from them stops (`/tmp` is one), if any.
    ///
    /// # Panics
    ///
    /// Panics when a `package.json` is above the fixtures and below that
    /// directory, since the walks from the fixtures would read it, and the
    /// tests that expect them to find none would fail misleadingly.
    fn world_writable_ancestor(&self) -> Option<PathBuf> {
        use std::os::unix::fs::PermissionsExt as _;
        for dir in self.0.ancestors() {
            if std::fs::metadata(dir).unwrap().permissions().mode() & 0o002 != 0 {
                return Some(dir.to_owned());
            }
            let package_json = dir.join("package.json");
            assert!(
                dir == self.0 || !package_json.exists(),
                "TMPDIR is under a project: {} is above the temp dir, and the walks from the \
                 fixtures read it. Set TMPDIR outside one",
                package_json.display()
            );
        }
        None
    }

    fn run(&self, file: &str) -> Run {
        let output =
            output_within_deadline(fgro_command().args(["run", file]).current_dir(&self.0));
        Run {
            code: output
                .status
                .code()
                .expect("the run exits, not killed by a signal"),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }
}

impl Drop for TempFixtures {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An editor saves a file by writing a new one beside it and renaming it
/// over the old. A module loaded before that is served from the module map
/// after it, never read again: a dynamic `import()` of it runs, and so does
/// one of a new module that imports it, both with the first file's code.
/// The run waits for the rename by importing `ready.js` until it exists.
#[test]
fn a_loaded_module_renamed_over_is_served_from_the_module_map() {
    use std::io::{BufRead as _, Read as _};
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
    let fixtures = TempFixtures::new(
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
    let mut child = fgro_command()
        .args(["run", "main.js"])
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "loaded 1\n");
    // made before the rename, so it can't reuse the replaced file's inode
    let replacement = fixtures.0.join("a.js.replacement");
    std::fs::write(&replacement, "export const a = 2;\n").unwrap();
    std::fs::rename(replacement, fixtures.0.join("a.js")).unwrap();
    std::fs::write(fixtures.0.join("ready.js"), "export {};\n").unwrap();
    // the run ends by itself: its wait for `ready.js` is bounded
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    assert_eq!(rest, "true 1 2\n", "{stderr}");
    assert!(status.success(), "{stderr}");
}

#[test]
fn a_typescript_parse_error_has_its_position() {
    let fixtures = TempFixtures::new(
        "parse_error",
        &[
            ("parse_error.ts", "const a = 1;\nconst b: = 2;\n"),
            ("import_parse_error.js", "import './parse_error.ts';\n"),
        ],
    );
    let url = format!("file://{}", fixtures.0.display());
    for entry in ["parse_error.ts", "import_parse_error.js"] {
        let run = fixtures.run(entry);
        assert_eq!(run.code, 1, "{entry}");
        assert!(
            run.stderr
                .starts_with("error: cannot parse the TypeScript: "),
            "{}",
            run.stderr
        );
        assert!(
            run.stderr
                .ends_with(&format!("\n    at {url}/parse_error.ts:2:10\n")),
            "{}",
            run.stderr
        );
        assert_eq!(run.stderr.lines().count(), 2, "{}", run.stderr);
    }
}

/// `import {type T}` keeps its import, so the module's side effects run and
/// a missing one is an error, while `import type`, `export type {…} from`, and
/// `export type * from` drop entirely, as TypeScript's `verbatimModuleSyntax`
/// has it.
#[test]
fn type_only_imports_drop_and_inline_type_specifiers_keep_the_import() {
    let fixtures = TempFixtures::new(
        "type_imports",
        &[
            (
                "side.ts",
                "export type T = number;\nconsole.log('side effect');\n",
            ),
            (
                "keeps.ts",
                "import { type T } from './side.ts';\nconst a: T = 1;\nconsole.log('kept', a);\n",
            ),
            (
                "keeps_missing.ts",
                "import { type T } from './missing.ts';\nlet a: T;\nconsole.log('ran');\n",
            ),
            (
                "drops.ts",
                "import type { T } from './missing.ts';\n\
                 export type { U } from './missing.ts';\n\
                 export type * from './missing.ts';\n\
                 export type * as types from './missing.ts';\n\
                 let a: T;\nconsole.log('dropped');\n",
            ),
        ],
    );
    let run = fixtures.run("keeps.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "side effect\nkept 1\n");

    let run = fixtures.run("keeps_missing.ts");
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("error: cannot resolve './missing.ts' from ")
            && run.stderr.contains("no such file"),
        "{}",
        run.stderr
    );

    let run = fixtures.run("drops.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "dropped\n");
}

/// `n` levels of nested parens around a `1`, logged.
fn nested_parens(n: usize) -> String {
    format!(
        "const x = {}1{};\nconsole.log('ok', x);\n",
        "(".repeat(n),
        ")".repeat(n)
    )
}

/// The strip recurses with the nesting, on `fgro`'s own 32 MiB thread rather
/// than the main thread, so a `.ts` module nested past the main thread's
/// ceiling (8 MiB, or a raised `ulimit -s` of 16 MiB) runs as the same `.js`
/// does instead of aborting on a stack overflow: in a debug build V8 runs it,
/// in a release build V8's own depth limit refuses it with a `RangeError`
/// first. The depths sit past the main thread's measured ceiling for each
/// profile and within the reservation's.
#[test]
fn deep_typescript_nesting_runs_as_the_same_js_does() {
    let depth = if cfg!(debug_assertions) { 1200 } else { 30_000 };
    let source = nested_parens(depth);
    let fixtures = TempFixtures::new("deep", &[("deep.ts", &source), ("deep.js", &source)]);
    let ts = fixtures.run("deep.ts");
    let js = fixtures.run("deep.js");
    assert!(
        ts.stderr.is_empty()
            || ts
                .stderr
                .contains("RangeError: Maximum call stack size exceeded"),
        "{}",
        ts.stderr
    );
    assert_eq!(ts.code, js.code, "{}\n{}", ts.stderr, js.stderr);
    assert_eq!(ts.stdout, js.stdout);
    assert_eq!(ts.stderr, js.stderr.replace("deep.js", "deep.ts"));
    if cfg!(debug_assertions) {
        assert_eq!(ts.code, 0, "{}", ts.stderr);
        assert_eq!(ts.stdout, "ok 1\n");
    }
}

/// V8 keeps its own checked limit on the sized thread, so unbounded JS
/// recursion is a `RangeError` the program can catch, and an uncaught one an
/// error exit, not a crash.
#[test]
fn deep_js_recursion_is_a_catchable_range_error() {
    let run = run("deep_recursion.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "true Maximum call stack size exceeded\n");
    assert!(
        run.stderr.starts_with(
            "error: Uncaught (in promise) RangeError: Maximum call stack size exceeded\n"
        ),
        "{}",
        run.stderr
    );
}

/// The TypeScript that can't load by stripping is refused at resolution, each
/// kind with its own reason: a usage error as an entry, a catchable one as an
/// import.
#[test]
fn unloadable_typescript_is_refused_by_kind() {
    let refused = [
        (
            "common.cts",
            "a .cts file is CommonJS, and only ES modules load",
        ),
        ("view.tsx", "JSX is not supported"),
        ("types.d.ts", "a declaration file holds only types"),
        ("types.d.mts", "a declaration file holds only types"),
        ("types.d.cts", "a declaration file holds only types"),
        ("state.svelte.ts", "needs the Svelte compiler"),
        ("state.svelte.js", "needs the Svelte compiler"),
        ("state.svelte.test.ts", "needs the Svelte compiler"),
    ];
    let main = format!(
        "for (const specifier of {:?}) {{\n\
         \ttry {{ await import('./' + specifier); }}\n\
         \tcatch (error) {{ console.log(String(error)); }}\n}}\n",
        refused.map(|(file, _)| file)
    );
    let mut files: Vec<(&str, &str)> = refused
        .iter()
        .map(|&(file, _)| (file, "export {};\n"))
        .collect();
    files.push(("main.js", &main));
    let fixtures = TempFixtures::new("refused", &files);

    for (file, reason) in refused {
        let run = fixtures.run(file);
        assert_eq!(run.code, 2, "{file}: {}", run.stderr);
        assert!(
            run.stderr
                .starts_with(&format!("error: cannot resolve '{file}' from ")),
            "{}",
            run.stderr
        );
        assert!(run.stderr.contains(reason), "{file}: {}", run.stderr);
    }
    let run = fixtures.run("main.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), refused.len(), "{}", run.stdout);
    for (line, (file, reason)) in lines.iter().zip(refused) {
        assert!(
            line.starts_with(&format!("TypeError: cannot resolve './{file}' from ")),
            "{line}"
        );
        assert!(line.contains(reason), "{line}");
    }
}

#[test]
fn syntax_error_in_the_entry_has_its_position() {
    let run = run("syntax_error.mjs");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught SyntaxError: Unexpected token ';'\n    at {}/syntax_error.mjs:2:11\n",
            fixtures_url()
        )
    );
}

#[test]
fn syntax_error_in_an_import_has_the_importee_position() {
    let run = run("import_syntax_error.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught SyntaxError: Unexpected token ';'\n    at {}/syntax_error.mjs:2:11\n",
            fixtures_url()
        )
    );
}

#[test]
fn causes_print_by_name_and_message() {
    let run = run("cause.js");
    assert_eq!(run.code, 1);
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: outer\n    at {url}/cause.js:1:7\n\
             Caused by: TypeError: inner\n    at {url}/cause.js:1:35\n"
        )
    );
}

#[test]
fn a_cyclic_cause_chain_ends_at_the_repeat() {
    let run = run("cause_cycle.js");
    assert_eq!(run.code, 1);
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: b\n    at {url}/cause_cycle.js:2:11\n\
             Caused by: Error: a\n    at {url}/cause_cycle.js:1:11\n\
             Caused by: Error: b\n    at {url}/cause_cycle.js:2:11\n"
        )
    );
}

#[test]
fn errors_made_at_one_site_all_print() {
    let run = run("cause_retry.js");
    assert_eq!(run.code, 1);
    let url = fixtures_url();
    let frame = format!("\n    at {url}/cause_retry.js:2:37");
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: retry failed{frame}\n\
             Caused by: Error: retry failed{frame}\n\
             Caused by: Error: retry failed{frame}\n"
        )
    );
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
    let fixtures = TempFixtures::new(
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
    let run = fixtures.run("main.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "gro condition\nstripped in node_modules\nlinked, its own dependency\n\
         nested, its dependency beside it\nself-reference\npast a marker, gro condition\n"
    );
    // from the symlink's path, the search would reach the project's node_modules/dep
    let run = fixtures.run("lonely.js");
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
             (npm and yarn workspaces, which install at their root, aren't supported). Fix: \
             install it into {root}/lonely_checkout/node_modules\n"
        )
    );
}

#[test]
fn package_refusals_name_the_reason_and_a_fix() {
    let fixtures = TempFixtures::new(
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
    let run = fixtures.run("main_only.js");
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
    let run = fixtures.run("commonjs.js");
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
    let fixtures = TempFixtures::new(
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
    let run = fixtures.run("main.js");
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

/// A `.js` file is an ES module only under `"type": "module"`, where Node
/// would guess from the code; `.mjs` needs no `package.json`.
#[test]
fn a_js_module_needs_type_module() {
    let fixtures = TempFixtures::without_package_json(
        "no_package_json",
        &[
            ("a.js", "console.log('a');\n"),
            ("b.mjs", "console.log('b');\n"),
            ("commonjs/package.json", "{}\n"),
            ("commonjs/c.js", "console.log('c');\n"),
        ],
    );
    // where a temp dir is under `/tmp`, the search stops there
    let stopped_at = fixtures.world_writable_ancestor();
    let run = fixtures.run("a.js");
    assert_eq!(run.code, 2);
    let reason = stopped_at.map_or_else(
        || {
            "and none is above it (the format is never guessed from the code). Fix: rename the \
             file to .mjs, or add a package.json with \"type\": \"module\""
                .to_owned()
        },
        |stopped_at| {
            format!(
                "and the search for one stopped at {}, which is world-writable, so nothing in it \
                 or above it is read (the format is never guessed from the code). Fix: rename the \
                 file to .mjs, or move it under a package.json with \"type\": \"module\" in a \
                 directory that isn't world-writable",
                stopped_at.display()
            )
        },
    );
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'a.js' from {}: a .js file is an ES module only under a \
             package.json with \"type\": \"module\", {reason}\n",
            fixtures.0.display()
        )
    );
    let run = fixtures.run("commonjs/c.js");
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains(&format!(
            "which {}/commonjs/package.json does not set",
            fixtures.0.display()
        )),
        "{}",
        run.stderr
    );
    let run = fixtures.run("b.mjs");
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
    let fixtures = TempFixtures::new(
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
    let run = fixtures.run("shared/loose/main.ts");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, no_project_root("foo", "main.ts"));
    let run = fixtures.run("shared/loose/self.ts");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, no_project_root("shared/evil.ts", "self.ts"));
    let run = fixtures.run("shared/direct.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "relative\n");
}

/// A package installed through a world-writable directory is refused, here
/// a `node_modules` any user could plant a package in, though the project
/// above it is sound.
#[test]
fn a_world_writable_node_modules_is_refused() {
    let fixtures = TempFixtures::new(
        "world_writable_node_modules",
        &[
            ("package.json", r#"{"name": "proj", "type": "module"}"#),
            ("node_modules/foo/package.json", r#"{"exports": "./i.mjs"}"#),
            ("node_modules/foo/i.mjs", "console.log('planted');\n"),
            ("main.ts", "import 'foo';\n"),
        ],
    );
    fixtures.set_mode("node_modules", 0o777);
    let run = fixtures.run("main.ts");
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
    let fixtures = TempFixtures::new(
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
        let run = fixtures.run(file);
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
    let run = fixtures.run("meta_resolve_admits_nothing.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, format!("x\nfile://{root}/packages/lnk/x.ts\n"));
    // so what it returns for an installed package is a path into node_modules
    let run = fixtures.run("meta_resolve.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        format!(
            "TypeError: cannot resolve 'file://{root}/node_modules/bar/i.mjs' from \
             file://{root}/meta_resolve.ts: it resolves to {root}/node_modules/bar/i.mjs, inside \
             node_modules below {root}, which only a bare specifier reaches, through the \
             package's \"exports\". Fix: import it by name\n"
        )
    );
}

/// An entry inside `node_modules` runs as the package it is in, so its
/// relative imports stay inside that package.
#[test]
fn an_entry_inside_node_modules_runs_as_its_package() {
    let fixtures = TempFixtures::new(
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
    let run = fixtures.run("node_modules/foo/cli.mjs");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "own\n");
    let run = fixtures.run("node_modules/foo/up.mjs");
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
    let run = fixtures.run("node_modules/.bin/real.mjs");
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
    let fixtures = TempFixtures::without_package_json(
        "planted_above",
        &[
            ("node_modules/foo/package.json", r#"{"exports": "./i.mjs"}"#),
            ("node_modules/foo/i.mjs", "console.log('planted');\n"),
            ("x/zap.ts", "import 'foo';\n"),
        ],
    );
    // where a temp dir is under `/tmp`, the search stops there
    let stopped_at = fixtures.world_writable_ancestor();
    let run = fixtures.run("x/zap.ts");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    let root = fixtures.0.display();
    let end = stopped_at.map_or_else(
        || "reached the filesystem's root".to_owned(),
        |stopped_at| {
            format!(
                "stopped at {}, which is world-writable, so nothing in it or above it is read",
                stopped_at.display()
            )
        },
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

#[test]
fn top_level_await() {
    let run = run("top_level_await.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "awaited 7\n");
}

#[test]
fn event_loop_drains_timers() {
    let run = run("timers.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "sync\nmicrotask\ntick 1\ntick 2\ntick 3\ntimeout\n"
    );
}

#[test]
fn missing_entry_is_a_usage_error() {
    let run = run("missing.js");
    assert_eq!(run.code, 2);
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'missing.js' from {}: no such file\n",
            fixtures().display()
        )
    );
}

#[test]
fn typescript_entry_runs() {
    let run = run("smoke.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.ends_with("\nstripped\n"), "{}", run.stdout);
}

/// The bench records this line, so it must say which build ran: the plain one
/// under `cargo test`, with no commit, the snapshot one under
/// `cargo xtask build --verify`, with the commit the xtask embedded (it builds
/// these tests with the same `GRO_COMMIT`).
#[test]
fn version_names_the_commit_v8_and_the_snapshot() {
    let output = fgro(&["--version"]);
    assert!(output.status.success(), "{output:?}");
    let line = String::from_utf8(output.stdout).unwrap();
    let commit = option_env!("GRO_COMMIT")
        .filter(|value| cfg!(feature = "snapshot") && !value.is_empty())
        .unwrap_or("unknown");
    assert!(
        line.starts_with(&format!(
            "fgro {} (commit {commit}, V8 ",
            env!("CARGO_PKG_VERSION")
        )),
        "{line}"
    );
    let expected = if cfg!(feature = "snapshot") {
        ", snapshot)\n"
    } else {
        ", no snapshot)\n"
    };
    assert!(line.ends_with(expected), "{line}");
}

#[test]
fn usage_errors_exit_2() {
    for args in [&[][..], &["run"], &["--unknown"], &["run", "a.js", "b.js"]] {
        assert_eq!(fgro(args).status.code(), Some(2), "{args:?}");
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_is_a_usage_error() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let output = output_within_deadline(
        fgro_command()
            .arg("run")
            .arg(OsStr::from_bytes(b"\xff.js"))
            .current_dir(fixtures()),
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not valid UTF-8"));
}

/// Runs `console.js` on a pseudo-terminal through util-linux's `script`, with
/// `env` set and `redirect` (a shell redirection, or empty) applied inside the
/// terminal, returning what reached the terminal.
#[cfg(target_os = "linux")]
fn run_on_terminal(env: &[(&str, &str)], redirect: &str) -> String {
    // probed once per test binary, not once per call
    static SCRIPT_PROBED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    let command = format!("'{}' run console.js {redirect}", env!("CARGO_BIN_EXE_fgro"));
    let mut script = Command::new("script");
    script
        .args(["-qec", &command, "/dev/null"])
        .current_dir(fixtures());
    for name in COLOR_VARS {
        script.env_remove(name);
    }
    // a terminal that colors, unless `env` says otherwise
    script.env("TERM", "xterm").envs(env.iter().copied());
    // fail loudly rather than skip: the color policy has no other test
    SCRIPT_PROBED.get_or_init(|| {
        Command::new("script")
            .arg("--version")
            .output()
            .expect("util-linux `script` is required for the terminal color tests");
    });
    let output = output_within_deadline(&mut script);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// A file for one stream's output, unique to the calling test.
#[cfg(target_os = "linux")]
fn stream_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("fgro_run_test_{}_{name}", std::process::id()))
}

#[cfg(target_os = "linux")]
#[test]
fn color_is_decided_per_stream() {
    // stdout on the terminal, stderr to a file
    let stderr_file = stream_file("stderr");
    let terminal = run_on_terminal(&[], &format!("2>'{}'", stderr_file.display()));
    let stderr = std::fs::read_to_string(&stderr_file).unwrap();
    std::fs::remove_file(&stderr_file).unwrap();
    assert!(terminal.contains("log \x1b["), "{terminal:?}");
    assert_eq!(stderr, "warn\nerror Map(1) { \"k\" => 1 }\n");

    // stderr on the terminal, stdout to a file
    let stdout_file = stream_file("stdout");
    let terminal = run_on_terminal(&[], &format!(">'{}'", stdout_file.display()));
    let stdout = std::fs::read_to_string(&stdout_file).unwrap();
    std::fs::remove_file(&stdout_file).unwrap();
    assert!(terminal.contains("error Map(1) { \x1b["), "{terminal:?}");
    assert_eq!(stdout, "log 1 { a: [ 1, \"two\" ] }\ninfo\n");
}

#[cfg(target_os = "linux")]
#[test]
fn the_off_vars_turn_color_off_when_non_empty() {
    assert!(run_on_terminal(&[], "").contains('\x1b'));
    for env in [
        [("NO_COLOR", "1")],
        [("NODE_DISABLE_COLORS", "1")],
        [("CLAUDECODE", "1")],
    ] {
        let output = run_on_terminal(&env, "");
        assert!(!output.contains('\x1b'), "{env:?}: {output:?}");
    }
    // set but empty counts as unset
    for env in [
        [("NO_COLOR", "")],
        [("NODE_DISABLE_COLORS", "")],
        [("CLAUDECODE", "")],
    ] {
        let output = run_on_terminal(&env, "");
        assert!(output.contains('\x1b'), "{env:?}: {output:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn term_of_dumb_turns_color_off() {
    let output = run_on_terminal(&[("TERM", "dumb")], "");
    assert!(!output.contains('\x1b'), "{output:?}");
}

#[test]
fn force_color_colors_a_pipe_and_its_other_values_do_not() {
    let output = fgro_with_env(&["run", "console.js"], &[("FORCE_COLOR", "1")]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains("log \x1b["), "{stdout:?}");
    assert!(stderr.contains("error Map(1) { \x1b["), "{stderr:?}");

    // it wins over the variables that turn color off
    let output = fgro_with_env(
        &["run", "console.js"],
        &[("FORCE_COLOR", ""), ("NO_COLOR", "1"), ("CLAUDECODE", "1")],
    );
    assert!(String::from_utf8(output.stdout).unwrap().contains('\x1b'));

    let output = fgro_with_env(&["run", "console.js"], &[("FORCE_COLOR", "0")]);
    assert!(!String::from_utf8(output.stdout).unwrap().contains('\x1b'));
    assert!(!String::from_utf8(output.stderr).unwrap().contains('\x1b'));
}

#[cfg(target_os = "linux")]
#[test]
fn force_color_of_another_value_turns_a_terminal_off() {
    let output = run_on_terminal(&[("FORCE_COLOR", "0")], "");
    assert!(!output.contains('\x1b'), "{output:?}");
}
