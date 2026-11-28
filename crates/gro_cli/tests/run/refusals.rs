//! Refusals by rule: a caught refusal failing the run, the listing and its
//! limits, the exit code, and a run ended by a signal after a refusal.

use std::process::{Command, Stdio};

use crate::support;
use crate::support::{spawn, wrapped_fgro_command};

use crate::{REFUSALS_FAIL, fixtures_url, run, run_in, run_with, temp_fixtures};

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
             code\n{REFUSALS_FAIL}"
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
        for refused in [Refused::Import, Refused::Env] {
            let source = format!("{}{rest}", refused.prelude());
            let fixtures = temp_fixtures(
                &format!("refused_then_{name}_{refused:?}"),
                &[("main.js", &source)],
            );
            let run = run_with(&fixtures, &[REFUSED_ENV_GRANT], "main.js", &[]);
            assert_eq!(run.code, 1, "{name}: {}", run.stderr);
            assert_eq!(run.stdout, stdout, "{name}");
            let main = format!("file://{}/main.js", fixtures.0.display());
            let error = error.replace("<main>", &main);
            assert!(run.stderr.starts_with(&error), "{name}: {}", run.stderr);
            // after the error and its frames, the last line
            assert!(
                run.stderr.ends_with(&refused.line(&main)),
                "{name}: {}",
                run.stderr
            );
            assert_eq!(
                run.stderr.matches(" refused: ").count(),
                1,
                "{name}: {}",
                run.stderr
            );
        }
    }
}

/// What a test's program refused by rule before it went on: nothing, an
/// `import()`, or an environment variable outside its grant
/// ([`REFUSED_ENV_GRANT`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refused {
    None,
    Import,
    Env,
}

/// The grant the programs of [`Refused`] run with, which lacks the name
/// [`Refused::Env`]'s asks for.
const REFUSED_ENV_GRANT: &str = "--allow-env=GRO_GRANTED";

impl Refused {
    /// The statements that make the refusal, caught, at the program's top
    /// level, three lines of each kind.
    const fn prelude(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Import => "try {\n\tawait import('ext:core/ops');\n} catch {}\n",
            Self::Env => {
                "try {\n\t(await import('@fuzdev/gro/env.ts')).env.get('GRO_NOT_GRANTED');\n\
                 } catch {}\n"
            }
        }
    }

    /// The lines a run that made the refusal ends its stderr with, `main`
    /// the program's URL: the refusal listed, the line saying it fails the
    /// run, and for the name, which the run's grant lacks, the flag that
    /// grants it.
    fn line(self, main: &str) -> String {
        match self {
            Self::None => String::new(),
            Self::Import => format!("{}{REFUSALS_FAIL}", ext_refused(main)),
            Self::Env => format!(
                "error: env refused: not_granted: GRO_NOT_GRANTED\n{REFUSALS_FAIL}\
                 hint: add --allow-env=GRO_NOT_GRANTED to grant what env refused\n"
            ),
        }
    }
}

/// The exit-code table, row by row, each way a run ends, with and without
/// an `import()` or an environment variable refused by rule before it: a
/// program's code passes as it is, given to `exit` or set and then left to
/// the run's own end; a run that fails exits 1 whatever its program set;
/// and a refusal of either kind turns a 0 into 1 on every path, `exit(0)`
/// included, leaving any other code as it is, its line printed last, before
/// the process ends. `exit` ends the
/// run inside the call: nothing after it prints, no `finally` runs, and no
/// error then unwinding is reported.
#[test]
fn the_exit_code_is_the_program_s_or_the_failure_s_and_a_refusal_fails_a_0() {
    const IMPORT: &str = "import {exit, set_exit_code} from '@fuzdev/gro/exit.ts';\n";
    // the program after its imports, the code it exits with, the code it
    // exits with when it was refused an `import()`, its stdout, and the
    // start of its stderr, `<main>` standing for the entry's URL
    for (name, program, code, refused_code, stdout, error) in [
        // ran to completion
        ("completes", "console.log('done');\n", 0, 1, "done\n", ""),
        (
            "completes_set_0",
            "set_exit_code(0);\nconsole.log('done');\n",
            0,
            1,
            "done\n",
            "",
        ),
        (
            "completes_set_7",
            "set_exit_code(7);\nconsole.log('done');\n",
            7,
            7,
            "done\n",
            "",
        ),
        (
            "completes_set_1",
            "set_exit_code(2);\nset_exit_code(1);\n",
            1,
            1,
            "",
            "",
        ),
        (
            "completes_set_in_a_timer",
            "setTimeout(() => set_exit_code(255), 1);\n",
            255,
            255,
            "",
            "",
        ),
        // the program called `exit`
        (
            "exit_0",
            "console.log('before');\nexit(0);\nconsole.log('after');\n",
            0,
            1,
            "before\n",
            "",
        ),
        (
            "exit_nothing",
            "console.log('before');\nexit();\nconsole.log('after');\n",
            0,
            1,
            "before\n",
            "",
        ),
        ("exit_1", "exit(1);\n", 1, 1, "", ""),
        ("exit_2", "exit(2);\n", 2, 2, "", ""),
        ("exit_255", "exit(255);\n", 255, 255, "", ""),
        (
            "exit_set",
            "set_exit_code(9);\nexit();\nconsole.log('after');\n",
            9,
            9,
            "",
            "",
        ),
        (
            "exit_0_over_set",
            "set_exit_code(9);\nexit(0);\n",
            0,
            1,
            "",
            "",
        ),
        (
            "exit_in_finally",
            "try {\n\texit(5);\n} finally {\n\tconsole.log('finally');\n}\n",
            5,
            5,
            "",
            "",
        ),
        (
            "exit_while_unwinding",
            "try {\n\tthrow new Error('x');\n} finally {\n\texit(0);\n}\n",
            0,
            1,
            "",
            "",
        ),
        (
            "exit_in_a_timer",
            "setTimeout(() => exit(3), 1);\nsetTimeout(() => console.log('later'), 5);\n",
            3,
            3,
            "",
            "",
        ),
        // the run failed, whatever its program set
        (
            "throws_set_7",
            "set_exit_code(7);\nthrow new Error('x');\n",
            1,
            1,
            "",
            "error: Uncaught (in promise) Error: x\n    at <main>:",
        ),
        (
            "throws_set_0",
            "set_exit_code(0);\nthrow new Error('x');\n",
            1,
            1,
            "",
            "error: Uncaught (in promise) Error: x\n    at <main>:",
        ),
        (
            "rejects_set_7",
            "set_exit_code(7);\nPromise.reject(new Error('x'));\n",
            1,
            1,
            "",
            "error: Uncaught (in promise) Error: x\n    at <main>:",
        ),
        (
            "unsettled_set_7",
            "set_exit_code(7);\nawait new Promise(() => {});\n",
            1,
            1,
            "",
            "error: Top-level await promise never resolved\n    at <main>:",
        ),
        (
            "timer_throws_set_7",
            "set_exit_code(7);\nsetTimeout(() => {\n\tthrow new Error('late');\n}, 1);\n",
            1,
            1,
            "",
            "error: Uncaught Error: late\n    at <main>:",
        ),
        // a value that is no code ends nothing and sets nothing
        (
            "refused_values",
            "for (const call of [() => exit('1'), () => exit(256), () => set_exit_code(1.5)]) {\n\
             \ttry {\n\t\tcall();\n\t} catch (error) {\n\
             \t\tconsole.log(error instanceof TypeError, error.message);\n\t}\n}\n",
            0,
            1,
            "true an exit code is an integer from 0 to 255\n\
             true an exit code is an integer from 0 to 255\n\
             true an exit code is an integer from 0 to 255\n",
            "",
        ),
    ] {
        for (refused, expected) in [
            (Refused::None, code),
            (Refused::Import, refused_code),
            (Refused::Env, refused_code),
        ] {
            let source = format!("{IMPORT}{}{program}", refused.prelude());
            let name = format!("exit_table_{name}_{refused:?}");
            let fixtures = temp_fixtures(&name, &[("main.js", &source)]);
            let run = run_with(&fixtures, &[REFUSED_ENV_GRANT], "main.js", &[]);
            assert_eq!(run.code, expected, "{name}: {}", run.stderr);
            assert_eq!(run.stdout, stdout, "{name}");
            let main = format!("file://{}/main.js", fixtures.0.display());
            let error = error.replace("<main>", &main);
            assert!(run.stderr.starts_with(&error), "{name}: {}", run.stderr);
            if refused == Refused::None {
                if error.is_empty() {
                    assert_eq!(run.stderr, "", "{name}");
                }
                assert!(!run.stderr.contains(" refused"), "{name}: {}", run.stderr);
            } else {
                // the last line, after the run's error and its frames
                let line = refused.line(&main);
                assert!(run.stderr.ends_with(&line), "{name}: {}", run.stderr);
                assert_eq!(run.stderr.matches(" refused: ").count(), 1, "{name}");
            }
        }
    }
}

/// A refusal by rule after the program's last statement still counts when
/// `exit` ends the run: one an earlier job's `import()` met, whose
/// rejection nothing awaited, is listed before the process ends.
#[test]
fn a_refusal_nothing_awaited_is_listed_when_the_program_exits() {
    let source = "import {exit} from '@fuzdev/gro/exit.ts';\n\
                  import('ext:core/ops').catch(() => {});\n\
                  setTimeout(() => exit(0), 5);\n";
    let fixtures = temp_fixtures("exit_refused_unawaited", &[("main.js", source)]);
    let run = run_in(&fixtures, "main.js");
    let main = format!("file://{}/main.js", fixtures.0.display());
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stderr, format!("{}{REFUSALS_FAIL}", ext_refused(&main)));
}

/// A refusal of the specifier an `import()` names is made as the call is
/// made, so one in the job that then calls `exit(0)` still fails the run
/// and is listed.
#[test]
fn a_refusal_in_the_job_that_exits_is_listed() {
    let source = "import {exit} from '@fuzdev/gro/exit.ts';\n\
                  import('ext:core/ops').catch(() => {});\n\
                  exit(0);\n";
    let fixtures = temp_fixtures("exit_refused_same_job", &[("main.js", source)]);
    let run = run_in(&fixtures, "main.js");
    let main = format!("file://{}/main.js", fixtures.0.display());
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stderr, format!("{}{REFUSALS_FAIL}", ext_refused(&main)));
}

/// An `import()` whose graph is still loading when `exit` is called is
/// abandoned where it is: a refusal the graph would meet later is never
/// reached, so it neither fails the run nor prints. Called once the
/// `import()` has settled, `exit(0)` exits 1 with the refusal listed.
#[test]
fn an_import_still_loading_at_exit_is_abandoned_unrefused() {
    for (call, code) in [
        ("import('./loads.js').catch(() => {});\nexit(0);", 0),
        ("import('./loads.js').catch(() => exit(0));", 1),
    ] {
        let main = format!("import {{exit}} from '@fuzdev/gro/exit.ts';\n{call}\n");
        let files = [
            ("main.js", main.as_str()),
            ("loads.js", "import 'ext:core/ops';\n"),
        ];
        let fixtures = temp_fixtures("exit_import_loading", &files);
        let run = run_in(&fixtures, "main.js");
        assert_eq!(run.code, code, "{call}: {}", run.stderr);
        let loads = format!("file://{}/loads.js", fixtures.0.display());
        let listed = if code == 0 {
            String::new()
        } else {
            format!("{}{REFUSALS_FAIL}", ext_refused(&loads))
        };
        assert_eq!(run.stderr, listed, "{call}");
    }
}

/// A package's module is refused `exit`, the message naming the grant and
/// the package, so a dependency can't end the run on its own; the project
/// imports it, and a function it hands the package ends the run when the
/// package calls it, since a grant limits what code can import, not what
/// it's given.
#[test]
fn a_package_is_refused_exit_and_ends_the_run_only_when_handed_it() {
    let files = [
        ("package.json", r#"{"name": "app", "type": "module"}"#),
        (
            "node_modules/dep/package.json",
            r#"{"name": "dep", "type": "module", "exports": "./index.js"}"#,
        ),
        (
            "node_modules/dep/index.js",
            "export const load = () => import('@fuzdev/gro/exit.ts');\n\
             export const end = (exit) => exit(6);\n",
        ),
        (
            "refused.js",
            "import {load} from 'dep';\n\
             try {\n\tawait load();\n} catch (error) {\n\tconsole.log(error.message);\n}\n",
        ),
        (
            "handed.js",
            "import {exit} from '@fuzdev/gro/exit.ts';\nimport {end} from 'dep';\n\
             console.log('before');\nend(exit);\nconsole.log('after');\n",
        ),
    ];
    let fixtures = temp_fixtures("exit_package", &files);
    let dep = format!("file://{}/node_modules/dep/index.js", fixtures.0.display());
    let refusal = format!(
        "cannot import '@fuzdev/gro/exit.ts' from {dep}: it needs the grant 'exit', which \
         package 'dep' isn't granted: only the project's own modules are granted host modules"
    );
    let run = run_in(&fixtures, "refused.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, format!("{refusal}\n"));
    assert_eq!(
        run.stderr,
        format!("error: import() refused: {refusal}\n{REFUSALS_FAIL}")
    );
    let run = run_in(&fixtures, "handed.js");
    assert_eq!(run.code, 6, "{}", run.stderr);
    assert_eq!(run.stdout, "before\n");
    assert_eq!(run.stderr, "");
    // unconfined, the package imports it as the project does
    let run = run_with(&fixtures, &["-A"], "refused.js", &[]);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "", "")
    );
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
            format!(
                "{}{REFUSALS_FAIL}",
                ext_refused(&format!("file://{}/main.js", fixtures.0.display()))
            ),
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
        format!(
            "{}{REFUSALS_FAIL}",
            ext_refused(&format!("file://{}/dep.js", fixtures.0.display()))
        )
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
            caught("'@fuzdev/gro/net.ts'"),
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
        // the refusal, then the line saying it fails the run
        assert_eq!(run.stderr.lines().count(), 2, "{entry}: {}", run.stderr);
        assert!(
            run.stderr.ends_with(REFUSALS_FAIL),
            "{entry}: {}",
            run.stderr
        );
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

/// A listed refusal's specifier, which the program chose, prints escaped
/// (`escape_controls`), so it can't
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
             to the compiled code\n{REFUSALS_FAIL}"
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
        .chain([
            "error: 4 more refusals, not listed\n".to_owned(),
            REFUSALS_FAIL.to_owned(),
        ])
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
        gro_runtime::MAX_LISTED_REFUSALS + 2,
        "{}",
        run.stderr
    );
    assert_eq!(
        lines[lines.len() - 2..],
        [
            "error: 1 more refusal, not listed",
            REFUSALS_FAIL.trim_end()
        ]
    );
    // an env name's refusals and an import's share the list and the count,
    // each listing naming what refused it, and the hint names every name the
    // run's grant lacked, the counted ones' too
    let source = format!(
        "import {{ env }} from '@fuzdev/gro/env.ts';\n\
         for (let i = 0; i < {}; i++) {{\n\
         \ttry {{\n\t\tenv.get(`N${{i}}`);\n\t}} catch {{}}\n\
         \ttry {{\n\t\tawait import(`ext:${{i}}`);\n\t}} catch {{}}\n}}\n",
        gro_runtime::MAX_LISTED_REFUSALS
    );
    let fixtures = temp_fixtures("refused_both", &[("main.js", &source)]);
    let run = run_with(&fixtures, &["--allow-env=A"], "main.js", &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let main = format!("file://{}/main.js", fixtures.0.display());
    let expected: String = (0..gro_runtime::MAX_LISTED_REFUSALS / 2)
        .map(|i| {
            format!(
                "error: env refused: not_granted: N{i}\n\
                 error: import() refused: cannot resolve 'ext:{i}' from {main}: the scheme \
                 'ext:' is not supported; only files on disk (file: URLs) load\n"
            )
        })
        .chain([
            format!(
                "error: {} more refusals, not listed\n",
                gro_runtime::MAX_LISTED_REFUSALS
            ),
            REFUSALS_FAIL.to_owned(),
            format!(
                "hint: add --allow-env={} to grant what env refused\n",
                (0..gro_runtime::MAX_LISTED_REFUSALS)
                    .map(|i| format!("N{i}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ])
        .collect();
    assert_eq!(run.stderr, expected);
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
            "error: import() refused: {}… ({} bytes)\n{REFUSALS_FAIL}",
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
             '@fuzdev/gro/args.ts'\n    at async {url}/import_refused_uncaught.js:5:1\n{}\
             {REFUSALS_FAIL}",
            ext_refused(&format!("{url}/import_refused_uncaught.js"))
        )
    );
}

/// An uncaught `FsError` names no need, so when `fs` refused its path
/// before with another need, caught, both refusals stay listed, each with
/// its need, and the caught one isn't lost.
#[test]
fn a_caught_fs_refusal_of_the_uncaught_one_s_path_stays_listed() {
    let source = "import { fs } from '@fuzdev/gro/fs.ts';
try {
\tawait fs.project.write('out', 'x');
} catch {}
await fs.project.remove('out');
";
    let fixtures = temp_fixtures(
        "fs_refused_by_two_needs",
        &[("out/kept.txt", ""), ("main.js", source)],
    );
    let run = run_with(&fixtures, &["--allow-write=out"], "main.js", &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let dir = fixtures.0.display();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) FsError: not_granted: {dir}/out\n    \
             at file://{dir}/main.js:5:18\n\
             error: fs refused what the run grants: not_granted: {dir}/out\n\
             error: fs refused what no grant allows: not_granted: {dir}/out\n\
             {REFUSALS_FAIL}\
             hint: the run grants what fs refused: open the directory with fs.open_dir and use \
             that handle\n"
        )
    );
}

/// When the uncaught refusal is past the listed ones, counted, the listed
/// refusal of its class and message is another the program caught, so it
/// stays listed.
#[test]
fn a_caught_fs_refusal_stays_listed_when_the_uncaught_one_is_counted() {
    let source = format!(
        "import {{ fs }} from '@fuzdev/gro/fs.ts';
for (let i = 0; i < {}; i++) {{
\ttry {{
\t\tawait import('@fuzdev/gro/args.js?' + i);
\t}} catch {{}}
}}
try {{
\tawait fs.project.write('out', 'x');
}} catch {{}}
await fs.project.remove('out');
",
        gro_runtime::MAX_LISTED_REFUSALS - 1
    );
    let fixtures = temp_fixtures(
        "fs_refused_past_the_listed",
        &[("out/kept.txt", ""), ("main.js", &source)],
    );
    let run = run_with(&fixtures, &["--allow-write=out"], "main.js", &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let dir = fixtures.0.display();
    let imports = (0..gro_runtime::MAX_LISTED_REFUSALS - 1)
        .map(|i| {
            format!(
                "error: import() refused: cannot resolve '@fuzdev/gro/args.js?{i}' from \
                 file://{dir}/main.js: it is the .js spelling of the builtin module \
                 '@fuzdev/gro/args.ts', reserved so it never resolves to a package. Fix: import \
                 '@fuzdev/gro/args.ts'\n"
            )
        })
        .collect::<Vec<_>>()
        .concat();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) FsError: not_granted: {dir}/out\n    \
             at file://{dir}/main.js:10:18\n\
             {imports}\
             error: fs refused what the run grants: not_granted: {dir}/out\n\
             error: 1 more refusal, not listed\n\
             {REFUSALS_FAIL}\
             hint: the run grants what fs refused: open the directory with fs.open_dir and use \
             that handle\n"
        )
    );
}

/// A run killed by a signal after its program caught a refusal by rule
/// ends by the signal, never exit 0, and prints nothing. `fgro` keeps the
/// signal dispositions it inherits, and a run that inherits a signal ignored
/// (from a shell's `&` without job control, or `nohup`) runs on, so the
/// run starts under `env --default-signal`, both at their defaults however
/// the test was launched, and the wait is bounded.
#[test]
fn a_run_killed_by_a_signal_after_a_refusal_ends_by_the_signal() {
    use std::os::unix::process::ExitStatusExt as _;

    let source = "try {\n\tawait import('ext:core/ops');\n} catch {}\nconsole.log('refused');\n\
                  setInterval(() => {}, 1000);\n";
    let fixtures = temp_fixtures("refused_then_signal", &[("main.js", source)]);
    for (name, signal) in [("TERM", 15), ("INT", 2)] {
        let mut env = Command::new("env");
        env.arg("--default-signal=INT,TERM");
        let mut command = wrapped_fgro_command(env, &["run", "main.js"]);
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
        let status = support::wait_bounded(
            &format!("SIG{name} (a run that inherits it ignored never ends by it)"),
            &mut child,
        );
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

/// A project's `import()` of a host module the run's flags don't grant
/// fails the run though it's caught, and ends with the hint naming the
/// flag that grants it, once however often it's tried, naming no program;
/// granted, it imports.
#[test]
fn a_host_module_the_flags_dont_grant_names_its_flag() {
    let fixtures = temp_fixtures(
        "module_hint",
        &[(
            "main.js",
            "for (let i = 0; i < 2; i++) {\n\
             \ttry {\n\t\tawait import('@fuzdev/gro/spawn.ts');\n\t\tconsole.log('imported');\n\
             \t} catch {\n\t\tconsole.log('caught');\n\t}\n}\n",
        )],
    );
    let run = run_with(&fixtures, &[], "main.js", &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "caught\ncaught\n");
    assert_eq!(
        run.stderr,
        format!(
            "error: import() refused: cannot import '@fuzdev/gro/spawn.ts' from file://{}/main.js: \
             it needs the grant 'spawn', which this run doesn't grant\n{REFUSALS_FAIL}\
             hint: add --allow-run naming each program the run starts, or --allow-run-in naming \
             a directory holding them, to grant the import of '@fuzdev/gro/spawn.ts'\n",
            fixtures.0.display()
        )
    );
    let run = run_with(&fixtures, &["--allow-run=git"], "main.js", &[]);
    assert_eq!(
        (
            run.code,
            run.stdout.as_str(),
            support::past_network_note(&run.stderr).as_str()
        ),
        (0, "imported\nimported\n", "")
    );
}

/// An `fs` call refused `outside_root` for a path that leaves its handle,
/// by `..` or as an absolute path, where one root of the run already
/// grants the whole call, a copy's or a rename's both ends, ends with the
/// hint to open that directory, when it's the run's uncaught error,
/// however long the path; caught, it fails nothing and prints nothing, and
/// with no root granting the call there's no hint.
#[test]
fn an_uncaught_outside_root_a_root_grants_hints_at_open_dir() {
    let fixtures = temp_fixtures(
        "outside_granted",
        &[
            ("proj/package.json", r#"{"name": "proj", "type": "module"}"#),
            ("other/f.txt", "hi\n"),
            (
                "proj/climb.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.read_text('../other/f.txt');\n",
            ),
            (
                "proj/absolute.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.read_text(`${fs.project.path}/../other/f.txt`);\n",
            ),
            (
                "proj/write.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.write('../other/w.txt', 'x');\n",
            ),
            (
                "proj/copy.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.copy('../other/f.txt', 'c.txt');\n",
            ),
            (
                "proj/copy_other.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.copy('../other/f.txt', '../other/c.txt');\n",
            ),
            (
                "proj/rename.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.rename('../other/f.txt', 'r.txt');\n",
            ),
            (
                "proj/rename_other.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.rename('../other/f.txt', '../other/r.txt');\n",
            ),
            (
                "proj/dir.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 fs.project.dir('../other');\n",
            ),
            (
                "proj/long.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 await fs.project.read_text('../other/' + 'a'.repeat(2100));\n",
            ),
            (
                "proj/caught.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 try {\n\tawait fs.project.read_text('../other/f.txt');\n} catch (error) {\n\
                 \tconsole.log(error.kind);\n}\n",
            ),
            (
                "proj/caught_then.js",
                "import { fs } from '@fuzdev/gro/fs.ts';\n\
                 try {\n\tawait fs.project.read_text('../other/f.txt');\n} catch {}\n\
                 await fs.project.read_text('missing.txt');\n",
            ),
        ],
    );
    let proj = fixtures.0.join("proj");
    let run_here = |flags: &[&str], file: &str| {
        let mut command = support::fgro_command(&[&["run"][..], flags, &[file]].concat());
        command.env_clear().current_dir(&proj);
        crate::run_command(command)
    };
    let hint = "hint: the run grants what fs refused: open the directory with fs.open_dir and \
                use that handle\n";
    let run = run_here(&["--allow-read=../other"], "climb.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) FsError: outside_root: ../other/f.txt\n    at \
             file://{dir}/climb.js:2:18\nCaused by: Error: ../other/f.txt climbs above {dir}\n    \
             at file://{dir}/climb.js:2:18\n{hint}",
            dir = proj.display()
        )
    );
    for (flags, file, hinted) in [
        (&["--allow-read=../other"][..], "absolute.js", true),
        (&["--allow-read=.."][..], "climb.js", true),
        (&[][..], "climb.js", false),
        (&[][..], "absolute.js", false),
        (&["--allow-read=../other"][..], "write.js", false),
        (&["--allow-write=../other"][..], "write.js", true),
        (&["--allow-read=../other"][..], "caught_then.js", false),
        (&["--allow-read=.."][..], "dir.js", true),
        (&["--allow-read=.."][..], "long.js", true),
        // a copy needs one root reading its `from` and writing its `to`
        (&["--allow-read=.."][..], "copy.js", false),
        (
            &["--allow-read=../other", "--allow-write=."][..],
            "copy.js",
            false,
        ),
        (
            &["--allow-read=../other", "--allow-write=../other"][..],
            "copy_other.js",
            true,
        ),
        (&["--allow-write=../other"][..], "copy_other.js", false),
        // a rename, one writing both
        (&["--allow-read=.."][..], "rename.js", false),
        (
            &["--allow-write=../other", "--allow-write=."][..],
            "rename.js",
            false,
        ),
        (&["--allow-write=../other"][..], "rename_other.js", true),
    ] {
        let run = run_here(flags, file);
        assert_eq!(run.code, 1, "{flags:?} {file}: {}", run.stderr);
        assert_eq!(
            run.stderr.contains("FsError: outside_root: "),
            file != "caught_then.js",
            "{flags:?} {file}: {}",
            run.stderr
        );
        assert_eq!(
            run.stderr.ends_with(hint),
            hinted,
            "{flags:?} {file}: {}",
            run.stderr
        );
    }
    let run = run_here(&["--allow-read=../other"], "caught.js");
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "outside_root\n", "")
    );
}
