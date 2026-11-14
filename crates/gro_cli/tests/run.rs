//! `fgro run` end to end: the built binary on the scripts in
//! `tests/fixtures/run/`, run from that directory with a relative path.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "a test's setup fails by panicking"
)]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::RecvTimeoutError;

use support::{COLOR_VARS, Scratch, fgro_command, spawn, wrapped_fgro_command};

fn fixtures() -> PathBuf {
    support::fixtures("run")
}

fn fixtures_url() -> String {
    support::fixtures_url("run")
}

/// Runs `fgro` with `args` from the fixtures directory, with stdout and stderr
/// piped (never terminals) and no color variable set, so console output is
/// uncolored.
fn fgro(args: &[&str]) -> Output {
    fgro_with_env(args, &[])
}

/// [`fgro`] with `env` set.
fn fgro_with_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = fgro_command(args);
    command.envs(env.iter().copied()).current_dir(fixtures());
    spawn(command, b"")
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

/// `console.trace` writes what it's given and the stack to stderr, formatted
/// by an op of its own, which no other method calls.
#[test]
fn console_trace_writes_its_stack_to_stderr() {
    let run = run("console_trace.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!(
            "Trace: trace 1\n    at {}/console_trace.js:1:9\n",
            fixtures_url()
        )
    );
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
/// the surface silently. To accept a change deliberately, rerun it with
/// `UPDATE_FIXTURES=1`, which writes the fixture's output over the file
/// instead of asserting:
/// `UPDATE_FIXTURES=1 cargo test -p gro_cli --test run the_global_names_are_pinned`.
#[test]
fn the_global_names_are_pinned() {
    let run = run("global_names.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let path = fixtures().join("global_names.txt");
    if std::env::var_os("UPDATE_FIXTURES").is_some_and(|value| value == "1") {
        std::fs::write(&path, &run.stdout).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap();
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

/// Every web global's constructor and a method or two, the console's
/// methods after them, with the output pinned in `web_surface.txt`: the ops
/// and classes the globals' code calls are called through it, so one the
/// runtime didn't register fails here where it's called. An op a script
/// reads and no global reaches (a message port's, a stream resource's) is
/// held only to being a function, by `gro_runtime`'s tests.
#[test]
fn every_web_global_works() {
    let run = run("web_surface.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let expected = std::fs::read_to_string(fixtures().join("web_surface.txt")).unwrap();
    assert_eq!(run.stdout, expected);
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

/// The imports of a module an `import()` loads resolve with no JS frame on
/// the stack, which the check for code compiled from a string must survive
/// (reading a frame of an empty stack trace crashes the process); an
/// `import()` in such code is refused, a `TypeError` the program catches,
/// which fails the run, listed once on stderr, naming no importer.
#[test]
fn dynamic_import_of_a_module_with_imports_and_from_string_code() {
    let run = run("import_string_code.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    let refusal = "cannot import './sibling.js' from code compiled from a string (eval, a \
                   Function constructor, or a string passed to a timer): V8 can share such code \
                   between modules that compile the same string, so it has no importer of its \
                   own. Fix: call import() in a module and pass what it returns to the compiled \
                   code";
    assert_eq!(
        run.stdout,
        format!("static sibling\ntrue {refusal}\ntrue {refusal}\n")
    );
    assert_eq!(run.stderr, format!("error: import() refused: {refusal}\n"));
}

/// The line `fgro run` prints for the refused `import()` of `ext:core/ops`
/// by the module at `importer`.
fn ext_refused(importer: &str) -> String {
    format!(
        "error: import() refused: cannot resolve 'ext:core/ops' from {importer}: the scheme \
         'ext:' is not supported; only files on disk (file: URLs) load\n"
    )
}

/// An `import()` refused by rule fails the run though the program caught
/// it: exit 1, and the refusal on stderr in the binary's error voice,
/// uncolored, once however often it's retried, in the order each first
/// happened; probing for what isn't there, or for an attribute that isn't
/// supported, prints nothing, and stdout is the program's.
#[test]
fn a_caught_refusal_by_rule_fails_the_run_and_is_listed_once() {
    let run = run("import_refused_caught.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "done\n");
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: import() refused: cannot resolve '@fuzdev/gro/args.js' from \
             {url}/import_refused_caught.js: it is the .js spelling of the builtin module \
             '@fuzdev/gro/args.ts', reserved so it never resolves to a package. Fix: import \
             '@fuzdev/gro/args.ts'\n\
             error: import() refused: cannot import './sibling.js' from code compiled from a \
             string (eval, a Function constructor, or a string passed to a timer): V8 can share \
             such code between modules that compile the same string, so it has no importer of \
             its own. Fix: call import() in a module and pass what it returns to the compiled \
             code\n"
        )
    );
}

/// However a run ends after its program caught a refusal by rule, it
/// fails, exit 1, and the refusal is listed after whatever error the run
/// printed: the program running to completion, an uncaught error, an
/// unhandled rejection, a top-level `await` that never settles, and an
/// error thrown in a timer.
#[test]
fn a_caught_refusal_fails_the_run_however_it_ends() {
    // each error's first lines, `<main>` standing for the entry's URL
    for (name, rest, stdout, error) in [
        ("completes", "console.log('done');\n", "done\n", ""),
        (
            "throws",
            "throw new Error('x');\n",
            "",
            "error: Uncaught (in promise) Error: x\n    at <main>:4:7\n",
        ),
        (
            "rejects",
            "Promise.reject(new Error('x'));\n",
            "",
            "error: Uncaught (in promise) Error: x\n    at <main>:4:16\n",
        ),
        (
            "unsettled",
            "await new Promise(() => {});\n",
            "",
            "error: Top-level await promise never resolved\n    at <main>:4:1\n",
        ),
        (
            "timer_throws",
            "console.log('before');\nsetTimeout(() => {\n\tthrow new Error('late');\n}, 1);\n",
            "before\n",
            "error: Uncaught Error: late\n    at <main>:6:8\n",
        ),
    ] {
        let source = format!("try {{\n\tawait import('ext:core/ops');\n}} catch {{}}\n{rest}");
        let fixtures = temp_fixtures(&format!("refused_then_{name}"), &[("main.js", &source)]);
        let run = run_in(&fixtures, "main.js");
        assert_eq!(run.code, 1, "{name}: {}", run.stderr);
        assert_eq!(run.stdout, stdout, "{name}");
        let main = format!("file://{}/main.js", fixtures.0.display());
        let error = error.replace("<main>", &main);
        assert!(run.stderr.starts_with(&error), "{name}: {}", run.stderr);
        // after the error and its frames, the last line
        assert!(
            run.stderr.ends_with(&ext_refused(&main)),
            "{name}: {}",
            run.stderr
        );
        assert_eq!(
            run.stderr.matches("import() refused").count(),
            1,
            "{name}: {}",
            run.stderr
        );
    }
}

/// A refusal by rule nothing awaits fails the run all the same: one whose
/// rejection a promise chain handles, and one refused in a timer while the
/// event loop still has work, each listed once the loop drains; so does one
/// of a static import in a module an `import()` loads, listed under that
/// module.
#[test]
fn a_refusal_nothing_awaits_fails_the_run() {
    for (name, source, stdout) in [
        (
            "chain",
            "import('ext:core/ops').then(\n\t() => {},\n\t() => {}\n);\nconsole.log('done');\n",
            "done\n",
        ),
        (
            "timer",
            "setTimeout(async () => {\n\ttry {\n\t\tawait import('ext:core/ops');\n\t} catch {}\n}, 1);\n\
             setTimeout(() => console.log('later'), 30);\nconsole.log('first');\n",
            "first\nlater\n",
        ),
    ] {
        let fixtures = temp_fixtures(&format!("refused_unawaited_{name}"), &[("main.js", source)]);
        let run = run_in(&fixtures, "main.js");
        assert_eq!(run.code, 1, "{name}: {}", run.stderr);
        assert_eq!(run.stdout, stdout, "{name}");
        assert_eq!(
            run.stderr,
            ext_refused(&format!("file://{}/main.js", fixtures.0.display())),
            "{name}"
        );
    }
    let fixtures = temp_fixtures(
        "refused_in_a_loaded_module",
        &[
            (
                "main.js",
                "try {\n\tawait import('./dep.js');\n} catch {}\n",
            ),
            ("dep.js", "import 'ext:core/ops';\n"),
        ],
    );
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        ext_refused(&format!("file://{}/dep.js", fixtures.0.display()))
    );
}

/// Each rule's refusal of an `import()` the program catches fails the run,
/// exit 1, listed once: a path that leaves its root, the project's or a
/// package's, or enters `node_modules` or an admitted package's root, an
/// encoded separator or NUL, a remote or inline specifier, a host module's
/// reserved name, a builtin with attributes or that a package isn't
/// granted, a package's import of the project, an `"exports"` target or a
/// pattern's match that leaves its package, a package admitted around a
/// module already resolved, and a package found through a world-writable
/// directory. A missing file beside them leaves the run's exit 0.
#[test]
fn each_rule_s_refusal_of_a_caught_import_fails_the_run() {
    let caught = |import: &str| {
        format!("try {{\n\tawait import({import});\n}} catch {{}}\nconsole.log('done');\n")
    };
    // a package's module that catches its own refused `import()`
    let package_catches =
        |import: &str| format!("try {{\n\tawait import({import});\n}} catch {{}}\n");
    let through = |module: &str| format!("await import('{module}');\nconsole.log('done');\n");
    let cases: [(&str, String, &str); 15] = [
        ("outside_root.js", caught("'../outside.mjs'"), ", outside "),
        (
            "into_node_modules.js",
            caught("'./node_modules/bar/i.mjs'"),
            "inside node_modules below",
        ),
        (
            "encoded_separator.js",
            caught("'./a%2fb.js'"),
            "has an encoded '/' or '\\'",
        ),
        (
            "encoded_nul.js",
            caught("'./a%00b.js'"),
            "has an encoded NUL",
        ),
        (
            "remote.js",
            caught("'https://example.com/x.js'"),
            "it is a remote or inline specifier",
        ),
        (
            "inline.js",
            caught("'data:text/javascript,1'"),
            "it is a remote or inline specifier",
        ),
        (
            "reserved.js",
            caught("'@fuzdev/gro/fs.ts'"),
            "it names Gro's host module",
        ),
        (
            "builtin_attributes.js",
            caught("'@fuzdev/gro/args.ts', {with: {type: 'json'}}"),
            "a builtin module takes no import attributes",
        ),
        (
            "invalid_target.js",
            caught("'bad/up.mjs'"),
            "is invalid: a target must start with './'",
        ),
        (
            "invalid_subpath.js",
            caught("'pat/a/../ok.mjs'"),
            "has a '.', '..', or 'node_modules' segment",
        ),
        (
            "package_outside.js",
            through("foo/outside.mjs"),
            "outside package 'foo'",
        ),
        (
            "package_not_granted.js",
            through("foo/args.mjs"),
            "which package 'foo' isn't granted",
        ),
        (
            "package_imports_project.js",
            through("foo/project.mjs"),
            "a package can't import the project's own modules by name",
        ),
        (
            "into_package_root.js",
            format!(
                "await import('lnk/x.mjs');\n{}",
                caught("'./packages/lnk/x.mjs'")
            ),
            "inside package 'lnk'",
        ),
        (
            "root_conflict.js",
            format!(
                "await import('./packages/lnk/x.mjs');\n{}",
                caught("'lnk/x.mjs'")
            ),
            "was then admitted around it",
        ),
    ];
    let world_writable = caught("'bar'");
    let missing = caught("'./missing.js'");
    let outside = package_catches("'../bar/i.mjs'");
    let args = package_catches("'@fuzdev/gro/args.ts'");
    let project = package_catches("'proj/ok.js'");
    let mut files = vec![
        ("outside.mjs", "console.log('outside');\n"),
        (
            "proj/package.json",
            r#"{"name": "proj", "type": "module", "exports": {"./ok.js": "./ok.js"}}"#,
        ),
        ("proj/ok.js", "console.log('ok');\n"),
        ("proj/world_writable.js", world_writable.as_str()),
        ("proj/nothing_there.js", missing.as_str()),
        (
            "proj/node_modules/bar/package.json",
            r#"{"type": "module", "exports": "./i.mjs"}"#,
        ),
        ("proj/node_modules/bar/i.mjs", "console.log('bar');\n"),
        (
            "proj/node_modules/bad/package.json",
            r#"{"exports": {"./up.mjs": "../bar/i.mjs"}}"#,
        ),
        (
            "proj/node_modules/pat/package.json",
            r#"{"exports": {"./*": "./*"}}"#,
        ),
        ("proj/node_modules/pat/ok.mjs", "console.log('pat');\n"),
        (
            "proj/node_modules/foo/package.json",
            r#"{"name": "foo", "exports": {"./*": "./*"}}"#,
        ),
        ("proj/node_modules/foo/outside.mjs", outside.as_str()),
        ("proj/node_modules/foo/args.mjs", args.as_str()),
        ("proj/node_modules/foo/project.mjs", project.as_str()),
        (
            "proj/packages/lnk/package.json",
            r#"{"name": "lnk", "exports": {"./x.mjs": "./x.mjs"}}"#,
        ),
        ("proj/packages/lnk/x.mjs", "console.log('x');\n"),
    ];
    let entries: Vec<String> = cases
        .iter()
        .map(|(name, _, _)| format!("proj/{name}"))
        .collect();
    files.extend(
        entries
            .iter()
            .zip(&cases)
            .map(|(entry, (_, source, _))| (entry.as_str(), source.as_str())),
    );
    let fixtures = temp_fixtures("each_rule", &files);
    let root = &fixtures.0;
    std::os::unix::fs::symlink(
        root.join("proj/packages/lnk"),
        root.join("proj/node_modules/lnk"),
    )
    .unwrap();
    // the project linked into its own `node_modules`
    std::os::unix::fs::symlink(root.join("proj"), root.join("proj/node_modules/proj")).unwrap();
    let refused = |entry: &str, reason: &str| {
        let run = run_in(&fixtures, entry);
        assert_eq!(run.code, 1, "{entry}: {}", run.stderr);
        assert!(run.stdout.ends_with("done\n"), "{entry}: {}", run.stdout);
        assert!(
            run.stderr.starts_with("error: import() refused: cannot "),
            "{entry}: {}",
            run.stderr
        );
        assert_eq!(run.stderr.lines().count(), 1, "{entry}: {}", run.stderr);
        assert!(run.stderr.contains(reason), "{entry}: {}", run.stderr);
    };
    // nothing there to load, in the same tree
    let run = run_in(&fixtures, "proj/nothing_there.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "done\n");
    assert_eq!(run.stderr, "");
    for (entry, (_, _, reason)) in entries.iter().zip(&cases) {
        refused(entry, reason);
    }
    // last, since every package is found through it
    fixtures.set_mode("proj/node_modules", 0o777);
    refused("proj/world_writable.js", "which is world-writable");
}

/// A refusal by rule that is the run's own error prints once, as that
/// error, exit 1, with no listing after it: an `import()` awaited at the
/// top level and uncaught, one whose rejection nothing handles, and a
/// static import, which fails the run before any of it evaluates.
#[test]
fn a_refusal_that_is_the_run_s_error_prints_once() {
    for (name, source, error) in [
        (
            "awaited",
            "await import('ext:core/ops');\n",
            "error: Uncaught (in promise) TypeError: cannot resolve",
        ),
        (
            "unhandled",
            "import('ext:core/ops');\n",
            "error: Uncaught (in promise) TypeError: cannot resolve",
        ),
        (
            "static",
            "import 'ext:core/ops';\n",
            "error: cannot resolve",
        ),
    ] {
        let fixtures = temp_fixtures(&format!("refused_error_{name}"), &[("main.js", source)]);
        let run = run_in(&fixtures, "main.js");
        assert_eq!(run.code, 1, "{name}: {}", run.stderr);
        assert_eq!(run.stdout, "", "{name}");
        assert!(
            run.stderr.starts_with(&format!(
                "{error} 'ext:core/ops' from file://{}/main.js: the scheme 'ext:' is not \
                 supported; only files on disk (file: URLs) load\n",
                fixtures.0.display()
            )),
            "{name}: {}",
            run.stderr
        );
        assert_eq!(
            run.stderr.matches("'ext:core/ops'").count(),
            1,
            "{name}: {}",
            run.stderr
        );
        assert!(
            !run.stderr.contains("import() refused"),
            "{name}: {}",
            run.stderr
        );
    }
}

/// A listed refusal's specifier, which the program chose, prints with its
/// control and bidirectional formatting characters escaped, so it can't
/// start a line of its own, erase or conceal the lines around it, or
/// reorder them.
#[test]
fn a_listed_refusal_escapes_control_characters() {
    let run = run("import_refused_controls.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "done\n");
    let url = fixtures_url();
    let remote = "it is a remote or inline specifier; only files on disk load. Fix: save the \
                  module into the project and import it by a relative path";
    assert_eq!(
        run.stderr,
        format!(
            "error: import() refused: cannot resolve 'https://x/\\u{{d}}\\u{{1b}}[2K' from \
             {url}/import_refused_controls.js: {remote}\n\
             error: import() refused: cannot resolve 'ext:a\\u{{1b}}[8m' from \
             {url}/import_refused_controls.js: the scheme 'ext:' is not supported; only files on \
             disk (file: URLs) load\n\
             error: import() refused: cannot import 'z\\u{{a}}error: x\\u{{202e}}' from code \
             compiled from a string (eval, a Function constructor, or a string passed to a \
             timer): V8 can share such code between modules that compile the same string, so it \
             has no importer of its own. Fix: call import() in a module and pass what it returns \
             to the compiled code\n"
        )
    );
}

/// The first distinct refusals are listed, in the order they happened, and
/// one line counts how many more times an `import()` was refused, a retry
/// of an unlisted one included and a retry of a listed one not.
#[test]
fn refusals_past_the_listed_ones_are_counted_in_one_line() {
    let run = run("import_refused_flood.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "done\n");
    let url = fixtures_url();
    let expected: String = (0..gro_runtime::MAX_LISTED_REFUSALS)
        .map(|i| {
            format!(
                "error: import() refused: cannot resolve 'ext:{i}' from \
                 {url}/import_refused_flood.js: the scheme 'ext:' is not supported; only files \
                 on disk (file: URLs) load\n"
            )
        })
        .chain(["error: import() refused: 4 more times, not listed\n".to_owned()])
        .collect();
    assert_eq!(run.stderr, expected);
    // one past the listed
    let source = format!(
        "for (let i = 0; i <= {}; i++) {{\n\ttry {{\n\t\tawait import(`ext:${{i}}`);\n\t}} catch {{}}\n}}\n",
        gro_runtime::MAX_LISTED_REFUSALS
    );
    let fixtures = temp_fixtures("refused_one_more", &[("main.js", &source)]);
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    let lines: Vec<&str> = run.stderr.lines().collect();
    assert_eq!(
        lines.len(),
        gro_runtime::MAX_LISTED_REFUSALS + 1,
        "{}",
        run.stderr
    );
    assert_eq!(
        lines.last(),
        Some(&"error: import() refused: 1 more time, not listed")
    );
}

/// A long refusal is listed cut, marked with its message's whole length,
/// so a specifier as long as the program's heap holds prints as one short
/// line.
#[test]
fn a_long_refusal_is_listed_cut() {
    let source = "try {\n\tawait import('data:,' + 'a'.repeat(1 << 20));\n} catch {}\n";
    let fixtures = temp_fixtures("refused_long", &[("main.js", source)]);
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    let whole = format!(
        "cannot resolve 'data:,{}' from file://{}/main.js: it is a remote or inline specifier; \
         only files on disk load. Fix: save the module into the project and import it by a \
         relative path",
        "a".repeat(1 << 20),
        fixtures.0.display()
    );
    assert_eq!(
        run.stderr,
        format!(
            "error: import() refused: {}… ({} bytes)\n",
            &whole[..gro_runtime::MAX_LISTED_MESSAGE],
            whole.len()
        )
    );
}

/// A refusal the program didn't catch is the run's error, printed once,
/// and the one it caught before is listed after it; the run exits 1.
#[test]
fn an_uncaught_refusal_is_the_error_and_a_caught_one_is_listed() {
    let run = run("import_refused_uncaught.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) TypeError: cannot resolve '@fuzdev/gro/args.js' from \
             {url}/import_refused_uncaught.js: it is the .js spelling of the builtin module \
             '@fuzdev/gro/args.ts', reserved so it never resolves to a package. Fix: import \
             '@fuzdev/gro/args.ts'\n    at async {url}/import_refused_uncaught.js:5:1\n{}",
            ext_refused(&format!("{url}/import_refused_uncaught.js"))
        )
    );
}

/// A run killed by a signal after its program caught a refusal by rule
/// ends by the signal, never exit 0, and prints nothing.
#[test]
fn a_run_killed_by_a_signal_after_a_refusal_ends_by_the_signal() {
    use std::os::unix::process::ExitStatusExt as _;

    let source = "try {\n\tawait import('ext:core/ops');\n} catch {}\nconsole.log('refused');\n\
                  setInterval(() => {}, 1000);\n";
    let fixtures = temp_fixtures("refused_then_signal", &[("main.js", source)]);
    for (name, signal) in [("TERM", 15), ("INT", 2)] {
        let mut command = fgro_command(&["run", "main.js"]);
        command
            .current_dir(&fixtures.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = support::KillOnDrop(command.spawn().unwrap());
        let stdout = support::lines(child.stdout.take().unwrap());
        let stderr = support::drain(child.stderr.take().unwrap());
        assert_eq!(
            stdout.recv_timeout(support::DEADLINE).as_deref(),
            Ok("refused\n"),
            "{name}"
        );
        let killed = Command::new("kill")
            .args([format!("-{name}"), child.id().to_string()])
            .status()
            .unwrap();
        assert!(killed.success(), "{name}");
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(signal), "{name}: {status}");
        assert_eq!(status.code(), None, "{name}");
        assert_eq!(
            String::from_utf8(stderr.join().unwrap()).unwrap(),
            "",
            "{name}"
        );
    }
}

/// A run V8 aborts, out of memory under a data limit, after its program
/// caught a refusal by rule ends by a signal, never exit 0, with V8's
/// report and no listing.
#[test]
fn a_run_v8_aborts_after_a_refusal_ends_by_a_signal() {
    let source = "try {\n\tawait import('ext:core/ops');\n} catch {}\nconsole.log('refused');\n\
                  const held = [];\nfor (;;) held.push(new Array(1 << 16).fill(held.length));\n";
    let fixtures = temp_fixtures("refused_then_abort", &[("main.js", source)]);
    let mut prlimit = Command::new("prlimit");
    // far below what the harness's own cap lets a child reach
    prlimit.arg(format!("--data={0}:{0}", 768 << 20));
    let mut command = wrapped_fgro_command(prlimit, &["run", "main.js"]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), None, "{}: {stderr}", output.status);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "refused\n");
    assert!(stderr.contains("out of memory"), "{stderr}");
    assert!(!stderr.contains("import() refused"), "{stderr}");
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

/// A directory of its own for `files` (see [`Scratch`]), so sources that
/// must not pass the format check (a parse error) or that it skips (`.tsx`,
/// `.cts`) live outside the fixtures, under the system's temporary
/// directory, which ends every walk up from them; the files under a
/// `package.json` with `"type": "module"` unless they have their own.
fn temp_fixtures(name: &str, files: &[(&str, &str)]) -> Scratch {
    let fixtures = Scratch::in_system_temp(name, files);
    let package_json = fixtures.0.join("package.json");
    if !package_json.exists() {
        std::fs::write(package_json, "{\"type\": \"module\"}\n").unwrap();
    }
    fixtures
}

/// Runs `file` in `fixtures`.
fn run_in(fixtures: &Scratch, file: &str) -> Run {
    let mut command = fgro_command(&["run", file]);
    command.current_dir(&fixtures.0);
    let output = spawn(command, b"");
    Run {
        code: output
            .status
            .code()
            .expect("the run exits, not killed by a signal"),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
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
fn a_typescript_parse_error_has_its_position() {
    let fixtures = temp_fixtures(
        "parse_error",
        &[
            ("parse_error.ts", "const a = 1;\nconst b: = 2;\n"),
            ("import_parse_error.js", "import './parse_error.ts';\n"),
        ],
    );
    let url = format!("file://{}", fixtures.0.display());
    for entry in ["parse_error.ts", "import_parse_error.js"] {
        let run = run_in(&fixtures, entry);
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
    let fixtures = temp_fixtures(
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
    let run = run_in(&fixtures, "keeps.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "side effect\nkept 1\n");

    let run = run_in(&fixtures, "keeps_missing.ts");
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("error: cannot resolve './missing.ts' from ")
            && run.stderr.contains("no such file"),
        "{}",
        run.stderr
    );

    let run = run_in(&fixtures, "drops.ts");
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

/// `n` binary operators chained after a `1`, logged.
fn chained_operators(n: usize) -> String {
    format!("const x = 1{};\nconsole.log('ok', x);\n", " + 1".repeat(n))
}

/// A `.ts` module nested or chained past tsv's depth limits fails to load
/// with a message naming the limit, an error exit rather than a stack
/// overflow, though V8, whose limit is far deeper, runs the same `.js`.
#[test]
fn typescript_past_tsv_depth_limits_fails_to_load() {
    let nested = nested_parens(255);
    let chained = chained_operators(7000);
    let fixtures = temp_fixtures(
        "deep_limit",
        &[
            ("deep.ts", &nested),
            ("deep.js", &nested),
            ("chain.ts", &chained),
            ("chain.js", &chained),
        ],
    );
    let url = format!("file://{}", fixtures.0.display());
    let run = run_in(&fixtures, "deep.ts");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot strip the TypeScript: Nesting exceeds tsv's depth limit of 256 \
             wire levels\n    at {url}/deep.ts:1:266\n"
        )
    );
    let run = run_in(&fixtures, "chain.ts");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    // the column is where tsv's weighted count trips, so only the file is pinned
    assert!(
        run.stderr.starts_with(&format!(
            "error: cannot strip the TypeScript: Chain of operators, member accesses, calls, \
             assertions or type suffixes exceeds tsv's length limit\n    at {url}/chain.ts:1:"
        )),
        "{}",
        run.stderr
    );
    assert_eq!(run.stderr.lines().count(), 2, "{}", run.stderr);
    for (file, out) in [("deep.js", "ok 1\n"), ("chain.js", "ok 7001\n")] {
        let run = run_in(&fixtures, file);
        assert_eq!(run.code, 0, "{file}: {}", run.stderr);
        assert_eq!(run.stdout, out, "{file}");
    }
}

/// A module's source is capped as eval's is, since the loader is shared: one
/// at 16 MiB runs, and one a byte over fails to load naming the cap and the
/// file, before its syntax error is parsed.
#[test]
fn a_module_past_its_byte_cap_fails_to_load() {
    // `gro_runtime::resolver::MAX_MODULE_SIZE`, which gro_runtime pins; a
    // literal keeps V8 out of this test binary
    let cap: usize = 16 << 20;
    let padded = |first: &str, size: usize| {
        let mut source = format!("{first}\n//");
        source.push_str(&"x".repeat(size - source.len()));
        source
    };
    let at = padded("console.log('at the cap');", cap);
    let over = padded("console.log() 'over';", cap + 1);
    let fixtures = temp_fixtures("module_cap", &[("at.js", &at), ("over.js", &over)]);
    let run = run_in(&fixtures, "at.js");
    assert_eq!(
        (run.code, run.stdout.as_str()),
        (0, "at the cap\n"),
        "{}",
        run.stderr
    );
    let run = run_in(&fixtures, "over.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    let url = format!("file://{}/over.js", fixtures.0.display());
    assert!(
        run.stderr.starts_with(&format!(
            "error: cannot load {url}: the module is larger than 16 MiB, the cap on a module's \
             source"
        )),
        "{}",
        run.stderr
    );
}

/// Within tsv's depth limits, deep TypeScript strips and runs as the same JS
/// does, on `fgro`'s sized thread whatever the build profile.
#[test]
fn deep_typescript_within_the_limits_runs_as_the_same_js_does() {
    // the deepest parens the nesting limit admits, the statement taking a level
    let nested = nested_parens(254);
    let chained = chained_operators(5000);
    let fixtures = temp_fixtures(
        "deep",
        &[
            ("deep.ts", &nested),
            ("deep.js", &nested),
            ("chain.ts", &chained),
            ("chain.js", &chained),
        ],
    );
    for (name, out) in [("deep", "ok 1\n"), ("chain", "ok 5001\n")] {
        let ts = run_in(&fixtures, &format!("{name}.ts"));
        let js = run_in(&fixtures, &format!("{name}.js"));
        assert_eq!(ts.code, 0, "{name}: {}", ts.stderr);
        assert_eq!(js.code, 0, "{name}: {}", js.stderr);
        assert_eq!(ts.stdout, out, "{name}");
        assert_eq!(js.stdout, out, "{name}");
        assert!(ts.stderr.is_empty() && js.stderr.is_empty(), "{name}");
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
    let fixtures = temp_fixtures("refused", &files);

    for (file, reason) in refused {
        let run = run_in(&fixtures, file);
        assert_eq!(run.code, 2, "{file}: {}", run.stderr);
        assert!(
            run.stderr
                .starts_with(&format!("error: cannot resolve '{file}' from ")),
            "{}",
            run.stderr
        );
        assert!(run.stderr.contains(reason), "{file}: {}", run.stderr);
    }
    let run = run_in(&fixtures, "main.js");
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

/// The reports beyond a plain throw: an async rejection across modules, its
/// path holding a space and non-ASCII letters, printed decoded; a thrown
/// value that isn't an error, which has no frames; an error whose stack a
/// program's `Error.prepareStackTrace` formats with no frames in its text,
/// reported at its first frame, read from the call sites kept on it; a web
/// API's `DOMException`, built by the builder the bootstrap registers for its
/// class; and a throw in a module with an inline source map, at its position
/// in the code that ran.
#[test]
fn reports_carry_their_stacks() {
    let fixtures = temp_fixtures(
        "reports",
        &[
            (
                "sub d\u{ed}r/m.js",
                "export async function load() { await null; throw new TypeError('async boom'); }\n",
            ),
            (
                "async_nested.js",
                "import {load} from './sub d\u{ed}r/m.js';\nasync function main() { await load(); }\nmain();\n",
            ),
            ("non_error.js", "function f() { throw {code: 42}; }\nf();\n"),
            (
                "user_prepare.js",
                "Error.prepareStackTrace = (error, sites) => `custom ${sites.length}`;\n\
                 function f() { throw new Error('x'); }\nf();\n",
            ),
            ("atob.js", "function decode() { atob('!'); }\ndecode();\n"),
            (
                "mapped_lib.js",
                "export function fail() { throw new Error('mapped'); }\n\
                 //# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbIm9yaWdpbmFsLnRzIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiJBQVNJLDBCQUFBIn0=\n",
            ),
            (
                "mapped.js",
                "const {fail} = await import('./mapped_lib.js');\nfail();\n",
            ),
        ],
    );
    // the reports print a module's URL percent-decoded, so as its path
    let dir = format!("file://{}", fixtures.0.display());
    let cases = [
        (
            "async_nested.js",
            format!(
                "error: Uncaught (in promise) TypeError: async boom\n    at load ({dir}/sub d\u{ed}r/m.js:1:50)\n    at async main ({dir}/async_nested.js:2:25)\n"
            ),
        ),
        (
            "non_error.js",
            "error: Uncaught (in promise) #<Object>\n".to_owned(),
        ),
        (
            "user_prepare.js",
            format!("error: Uncaught (in promise) Error: x\n    at {dir}/user_prepare.js:2:22\n"),
        ),
        (
            "atob.js",
            format!(
                "error: Uncaught (in promise) InvalidCharacterError: Failed to decode base64\n    at atob (ext:deno_web/05_base64.js:21:12)\n    at decode ({dir}/atob.js:1:21)\n    at {dir}/atob.js:2:1\n"
            ),
        ),
        // the map would place `fail` at original.ts:10:5; fgro applies none
        (
            "mapped.js",
            format!(
                "error: Uncaught (in promise) Error: mapped\n    at fail ({dir}/mapped_lib.js:1:32)\n    at {dir}/mapped.js:2:1\n"
            ),
        ),
    ];
    for (file, expected) in cases {
        let run = run_in(&fixtures, file);
        assert_eq!(run.code, 1, "{file}");
        assert_eq!(run.stderr, expected, "{file}");
    }
}

/// No source map applies to the call sites a program's
/// `Error.prepareStackTrace` reads, even in a dynamically imported module's
/// top-level code, which runs before `deno_core` finishes that import's load.
#[test]
fn no_source_map_applies_in_a_dynamic_imports_top_level() {
    let fixtures = temp_fixtures(
        "dynamic_top_level",
        &[
            (
                "top.js",
                "Error.prepareStackTrace = (error, sites) => sites.map((site) => \
                 `${site.getLineNumber()}:${site.getColumnNumber()}`).join(','); \
                 console.log(new Error('top').stack); delete Error.prepareStackTrace;\n\
                 //# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbIm9yaWdpbmFsLnRzIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiJBQVNJLDBCQUFBIn0=\n",
            ),
            ("main.js", "await import('./top.js');\n"),
        ],
    );
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    // the map would place line 1 at original.ts:10:5
    assert_eq!(run.stdout, "1:141\n");
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
    assert_eq!(run.stderr, format!("error: import() refused: {refusal}"));
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

#[test]
fn top_level_await() {
    let run = run("top_level_await.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "awaited 7\n");
}

/// A top-level `await` on a promise nothing settles ends the run once the
/// event loop drains, exit 1, rather than hanging or exiting 0.
#[test]
fn an_unsettled_top_level_await_exits_1() {
    let run = run("top_level_await_unsettled.js");
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        format!(
            "error: Top-level await promise never resolved\n    at \
             {}/top_level_await_unsettled.js:3:1\n",
            fixtures_url()
        )
    );
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

/// The commit the binary under test was built from: none for the plain one
/// under `cargo test`, and for the snapshot one under
/// `cargo xtask build --verify`, the commit the xtask embedded (it builds
/// these tests with the same `GRO_COMMIT`).
fn commit() -> Option<&'static str> {
    option_env!("GRO_COMMIT").filter(|value| cfg!(feature = "snapshot") && !value.is_empty())
}

/// What a script run by the binary under test reads as `runtime_version`:
/// `fgro/<version>`, then `+` and [`commit`] when the build has one.
fn runtime_version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    commit().map_or_else(
        || format!("fgro/{version}"),
        |commit| format!("fgro/{version}+{commit}"),
    )
}

/// The bench records this line, so it must say which build ran: the plain
/// one with no commit, the snapshot one with [`commit`].
#[test]
fn version_names_the_commit_v8_and_the_snapshot() {
    let output = fgro(&["--version"]);
    assert!(output.status.success(), "{output:?}");
    let line = String::from_utf8(output.stdout).unwrap();
    let commit = commit().unwrap_or("unknown");
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

/// `--version` and `--help` into a pipe whose reader has closed fail as a
/// write, exit `1` with an error on stderr, never a panic: Rust ignores
/// `SIGPIPE`, so the write returns `EPIPE`.
#[test]
fn a_closed_stdout_fails_the_write_without_a_panic() {
    for args in [&["--version"][..], &["--help"], &["run", "--help"]] {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let mut command = fgro_command(args);
        command
            .current_dir(fixtures())
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(Stdio::piped());
        let output = command.output().unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {stderr}");
        assert!(
            stderr.starts_with("error: failed to write to stdout: "),
            "{args:?}: {stderr}"
        );
        assert_eq!(stderr.lines().count(), 1, "{args:?}: {stderr}");
    }
}

/// A usage error runs nothing: an option before an entry that exists is
/// refused, not skipped.
#[test]
fn usage_errors_exit_2() {
    for args in [
        &[][..],
        &["run"],
        &["run", "--"],
        &["--unknown"],
        &["run", "--x", "console.js"],
    ] {
        let output = fgro(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
    }
}

/// Every argument after the file is the script's, fgro's own spellings
/// included, so the run is the file's whatever follows it.
#[test]
fn arguments_after_the_file_are_the_script_s() {
    let alone = fgro(&["run", "console.js"]);
    assert_eq!(alone.status.code(), Some(0));
    for tail in [
        &["--help"][..],
        &["--", "--version", ""],
        &["help", "-A", "--allow-env=A", "b.js"],
    ] {
        let output = fgro(&[&["run", "console.js"][..], tail].concat());
        assert_eq!(output.status.code(), Some(0), "{tail:?}");
        assert_eq!(output.stdout, alone.stdout, "{tail:?}");
        assert_eq!(output.stderr, alone.stderr, "{tail:?}");
    }
}

/// The script reads its arguments as `args`, exactly as given: none,
/// several, ones spelled like flags, empty, and non-ASCII.
#[test]
fn the_script_reads_its_arguments_from_args() {
    for args in [
        &[][..],
        &["a", "b c", "d"],
        &["--help", "-A", "--", "--allow-env=A", "-"],
        &["", " ", ""],
        &["\u{e9}", "\u{1f600}", "\u{65e5}\u{672c}"],
    ] {
        let output = fgro(&[&["run", "args.js"][..], args].concat());
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{}\n", serde_json::to_string(args).unwrap()),
            "{args:?}"
        );
    }
}

/// A package's module is refused `args`, the message naming the package,
/// while the project imports it in the same run, which the refusal fails
/// though the project caught it.
#[test]
fn a_package_is_refused_args_while_the_project_imports_it() {
    let fixtures = temp_fixtures(
        "args_package",
        &[
            ("package.json", r#"{"name": "app", "type": "module"}"#),
            (
                "node_modules/dep/package.json",
                r#"{"name": "dep", "type": "module", "exports": "./index.js"}"#,
            ),
            (
                "node_modules/dep/index.js",
                "export const load = () => import('@fuzdev/gro/args.ts');\n",
            ),
            (
                "main.js",
                "import { args } from '@fuzdev/gro/args.ts';\n\
                 import { load } from 'dep';\n\
                 console.log(JSON.stringify(args));\n\
                 await load().catch((error) => console.log(error instanceof TypeError, error.message));\n\
                 console.log(import.meta.resolve('@fuzdev/gro/args.ts'));\n",
            ),
        ],
    );
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    let refusal = format!(
        "cannot import '@fuzdev/gro/args.ts' from file://{}/node_modules/dep/index.js: it needs \
         the grant 'args', which package 'dep' isn't granted: only the project's own modules \
         are granted host modules\n",
        fixtures.0.display()
    );
    assert_eq!(
        run.stdout,
        format!("[]\ntrue {refusal}builtin:@fuzdev/gro/args.ts\n")
    );
    assert_eq!(run.stderr, format!("error: import() refused: {refusal}"));
}

/// The script reads the observations with no grant: `cwd()` is the
/// directory `fgro` was started in, `pid` the id of the `fgro` process, and
/// `runtime_version` the binary's name and version, with the commit of a
/// build that has one.
#[test]
fn the_script_reads_the_observations() {
    let mut command = fgro_command(&["run", "observations.js"]);
    command.current_dir(fixtures());
    let (pid, output) = support::spawn_with_pid(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        printed,
        serde_json::json!({
            "cwd": fixtures().canonicalize().unwrap().to_str().unwrap(),
            "pid": pid,
            "runtime_version": runtime_version(),
        })
    );
}

/// A package's module reads the observations as the project does, by a
/// static import and `import()`, with nothing refused, and `cwd()` is the
/// directory the run was started in, not the entry's or the project's.
#[test]
fn a_package_reads_the_observations_and_cwd_is_the_start_directory() {
    let fixtures = temp_fixtures(
        "observations_package",
        &[
            ("package.json", r#"{"name": "app", "type": "module"}"#),
            (
                "node_modules/dep/package.json",
                r#"{"name": "dep", "type": "module", "exports": "./index.js"}"#,
            ),
            (
                "node_modules/dep/index.js",
                "import { cwd } from '@fuzdev/gro/cwd.ts';\n\
                 import { pid } from '@fuzdev/gro/pid.ts';\n\
                 import { runtime_version } from '@fuzdev/gro/runtime_version.ts';\n\
                 export const read = () => [cwd(), pid, runtime_version];\n\
                 export const load = async () => [\n\
                 \t(await import('@fuzdev/gro/cwd.ts')).cwd(),\n\
                 \t(await import('@fuzdev/gro/pid.ts')).pid,\n\
                 \t(await import('@fuzdev/gro/runtime_version.ts')).runtime_version\n\
                 ];\n",
            ),
            (
                "main.js",
                "import { cwd } from '@fuzdev/gro/cwd.ts';\n\
                 import { pid } from '@fuzdev/gro/pid.ts';\n\
                 import { runtime_version } from '@fuzdev/gro/runtime_version.ts';\n\
                 import { load, read } from 'dep';\n\
                 console.log(JSON.stringify([[cwd(), pid, runtime_version], read(), await load()]));\n",
            ),
            ("started/in/keep", ""),
        ],
    );
    let started_in = fixtures.0.join("started/in");
    let mut command = fgro_command(&["run", "../../main.js"]);
    command.current_dir(&started_in);
    let (pid, output) = support::spawn_with_pid(command, b"");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let expected = serde_json::json!([started_in.to_str().unwrap(), pid, runtime_version()]);
    assert_eq!(printed, serde_json::json!([expected, expected, expected]));
}

/// Started in a directory whose path isn't UTF-8, a run imports `cwd`, and
/// calling it throws an `Error` saying so, which uncaught fails the run;
/// a script that never calls it runs as anywhere.
#[cfg(unix)]
#[test]
fn cwd_throws_when_called_in_a_directory_that_is_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let fixtures = temp_fixtures(
        "observations_not_utf8",
        &[
            (
                "caught.js",
                "import { cwd } from '@fuzdev/gro/cwd.ts';\n\
                 try {\n\tconsole.log(cwd());\n} catch (error) {\n\
                 \tconsole.log(Object.getPrototypeOf(error) === Error.prototype, error.message);\n}\n",
            ),
            (
                "uncaught.js",
                "import { cwd } from '@fuzdev/gro/cwd.ts';\ncwd();\n",
            ),
            (
                "unused.js",
                "import { cwd } from '@fuzdev/gro/cwd.ts';\nconsole.log(typeof cwd);\n",
            ),
        ],
    );
    let started_in = fixtures.0.join(OsStr::from_bytes(b"not_utf8_\xff"));
    std::fs::create_dir(&started_in).unwrap();
    let run_file = |file: &str| {
        let mut command = fgro_command(&["run", &fixtures.path(file)]);
        command.current_dir(&started_in);
        let output = spawn(command, b"");
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    let message = "the working directory's path isn't UTF-8, so Gro can't name it";
    assert_eq!(
        run_file("caught.js"),
        (Some(0), format!("true {message}\n"), String::new())
    );
    assert_eq!(
        run_file("unused.js"),
        (Some(0), "function\n".to_owned(), String::new())
    );
    let (code, stdout, stderr) = run_file("uncaught.js");
    assert_eq!((code, stdout.as_str()), (Some(1), ""), "{stderr}");
    assert!(
        stderr.starts_with(&format!("error: Uncaught (in promise) Error: {message}\n")),
        "{stderr}"
    );
}

/// An argument that isn't UTF-8 is refused before anything runs, the file or
/// one of the script's.
#[cfg(unix)]
#[test]
fn non_utf8_argument_is_a_usage_error() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    for args in [&[&b"\xff.js"[..]][..], &[b"console.js", b"\xff"]] {
        let mut command = fgro_command(&["run"]);
        command
            .args(args.iter().map(|arg| OsStr::from_bytes(arg)))
            .current_dir(fixtures());
        let output = spawn(command, b"");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("not valid UTF-8"));
    }
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
    let output = spawn(script, b"");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn color_is_decided_per_stream() {
    let scratch = Scratch::new("streams", &[]);
    // stdout on the terminal, stderr to a file
    let stderr_file = scratch.0.join("stderr");
    let terminal = run_on_terminal(&[], &format!("2>'{}'", stderr_file.display()));
    let stderr = std::fs::read_to_string(&stderr_file).unwrap();
    assert!(terminal.contains("log \x1b["), "{terminal:?}");
    assert_eq!(stderr, "warn\nerror Map(1) { \"k\" => 1 }\n");

    // stderr on the terminal, stdout to a file
    let stdout_file = scratch.0.join("stdout");
    let terminal = run_on_terminal(&[], &format!(">'{}'", stdout_file.display()));
    let stdout = std::fs::read_to_string(&stdout_file).unwrap();
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
