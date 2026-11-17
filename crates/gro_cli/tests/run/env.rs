//! `--allow-env`, `args`, and the observations: `cwd`, `pid`, and
//! `runtime_version`.

use crate::support;
use crate::support::{fgro_command, spawn};

use crate::{REFUSALS_FAIL, fixtures, run_in, run_with, runtime_version, temp_fixtures};

/// `--allow-env=` gives the project's modules `@fuzdev/gro/env.ts`, whose
/// root reads exactly the names granted, an unset one as `undefined`, from
/// the environment `fgro` started with; a name outside them throws
/// `EnvError` `not_granted`, naming it and no value, and so does one
/// outside a narrowed handle's set, each refusal failing the run though
/// the program caught it. Without the flag the import is refused when the
/// graph links. `-A` reads every variable.
#[test]
fn allow_env_grants_the_named_variables_and_no_others() {
    let fixtures = temp_fixtures(
        "env_granted",
        &[(
            "main.js",
            "import { env } from '@fuzdev/gro/env.ts';\n\
                 const read = (handle, name) => {\n\
                 \ttry {\n\t\treturn JSON.stringify(handle.get(name) ?? null);\n\
                 \t} catch (error) {\n\
                 \t\treturn `${error.constructor.name} ${error.message}`;\n\t}\n};\n\
                 for (const name of ['GRO_A', 'GRO_B', 'GRO_UNSET']) console.log(name, read(env, name));\n\
                 const narrowed = env.narrow(['GRO_A']);\n\
                 console.log('narrowed', read(narrowed, 'GRO_UNSET'));\n\
                 console.log(JSON.stringify(env.get_all()));\n",
        )],
    );
    let vars = [("GRO_A", "1"), ("GRO_B", "secret value"), ("OTHER", "x")];
    let run = run_with(
        &fixtures,
        &["--allow-env=GRO_A,GRO_UNSET"],
        "main.js",
        &vars,
    );
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "GRO_A \"1\"\nGRO_B EnvError not_granted: GRO_B\nGRO_UNSET null\n\
         narrowed EnvError not_granted: GRO_UNSET\n{\"GRO_A\":\"1\"}\n"
    );
    // the refusals fail the run, though caught, listed by name, no value;
    // the hint names the one the run's grant lacked, never the one the
    // program's own narrowing refused, which no flag grants
    assert_eq!(
        run.stderr,
        format!(
            "error: env refused: not_granted: GRO_B\nerror: env refused: not_granted: GRO_UNSET\n\
             {REFUSALS_FAIL}hint: add --allow-env=GRO_B to grant what env refused\n"
        )
    );
    // unconfined, the root refuses nothing, so no refusal has a hint
    let run = run_with(&fixtures, &["-A"], "main.js", &vars);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        format!("error: env refused: not_granted: GRO_UNSET\n{REFUSALS_FAIL}")
    );
    assert_eq!(
        run.stdout,
        "GRO_A \"1\"\nGRO_B \"secret value\"\nGRO_UNSET null\n\
         narrowed EnvError not_granted: GRO_UNSET\n\
         {\"GRO_A\":\"1\",\"GRO_B\":\"secret value\",\"OTHER\":\"x\"}\n"
    );
    let run = run_with(&fixtures, &[], "main.js", &vars);
    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
    assert!(
        run.stderr
            .contains("cannot import '@fuzdev/gro/env.ts' from file://")
            && run
                .stderr
                .contains("it needs the grant 'env', which this run doesn't grant"),
        "{}",
        run.stderr
    );
}

/// A program that asks `env` for each of `names`, catching each refusal.
fn refusing(names: &[&str]) -> String {
    format!(
        "import {{env}} from '@fuzdev/gro/env.ts';\n\
         for (const name of {}) {{\n\ttry {{\n\t\tenv.get(name);\n\t}} catch {{}}\n}}\n",
        serde_json::to_string(names).unwrap()
    )
}

/// A refused environment variable fails the run as a refused `import()`
/// does, on its paths: listed when it's caught in the job that calls
/// `exit(0)`, which exits 1; printed once, as the run's error, when it's
/// uncaught or a rejection nothing handles; its name escaped and cut as an
/// import's message is; and quiet, exiting 0, for an unset granted name, a
/// narrowing to a subset, and a name that is no name. Each ends with the
/// hint of the flag that grants the name, the uncaught one's too, but for
/// a name too long to keep, or one that isn't ASCII letters, digits, and
/// `_` (a comma, a glob, a control character, a space, a shell's
/// metacharacter, a lookalike letter), whose hint says it names only some
/// of what was refused, or is left out when it names nothing.
#[test]
fn a_refused_env_name_fails_the_run_on_the_import_rule_s_paths() {
    let import = "import {env} from '@fuzdev/gro/env.ts';\n";
    let long = "L".repeat(gro_runtime::MAX_LISTED_MESSAGE + 10);
    let hint_b = "hint: add --allow-env=B to grant what env refused\n";
    let cases: Vec<(&str, String, i32, String, String)> = vec![
        (
            "same_job_exit",
            format!(
                "{import}import {{exit}} from '@fuzdev/gro/exit.ts';\n\
                 try {{\n\tenv.get('B');\n}} catch {{}}\nexit(0);\n"
            ),
            1,
            "error: env refused: not_granted: B\n".to_owned(),
            format!("{REFUSALS_FAIL}{hint_b}"),
        ),
        (
            "uncaught",
            format!("{import}env.get('B');\n"),
            1,
            "error: Uncaught (in promise) EnvError: not_granted: B\n".to_owned(),
            hint_b.to_owned(),
        ),
        (
            "unhandled",
            format!("{import}Promise.resolve().then(() => env.get('B'));\n"),
            1,
            "error: Uncaught (in promise) EnvError: not_granted: B\n".to_owned(),
            hint_b.to_owned(),
        ),
        (
            "escaped",
            format!("{import}try {{\n\tenv.get('B\\u001b[31m\\u202e');\n}} catch {{}}\n"),
            1,
            "error: env refused: not_granted: B\\u{1b}[31m\\u{202e}\n".to_owned(),
            REFUSALS_FAIL.to_owned(),
        ),
        (
            "unpastable",
            refusing(&["$(touch x)", "`id`", "X;Y", "A B", "q'u", "F_1"]),
            1,
            "error: env refused: not_granted: $(touch x)\nerror: env refused: not_granted: `id`\n\
             error: env refused: not_granted: X;Y\nerror: env refused: not_granted: A B\n\
             error: env refused: not_granted: q'u\nerror: env refused: not_granted: F_1\n"
                .to_owned(),
            format!("{REFUSALS_FAIL}hint: add --allow-env=F_1 to grant some of what env refused\n"),
        ),
        (
            // no name the hint can show, so no hint line at all
            "unpastable_only",
            refusing(&["q\"u", "N\nL", "\u{415}", "~", "\u{1b}[2J"]),
            1,
            "error: env refused: not_granted: q\"u\nerror: env refused: not_granted: N\\u{a}L\n\
             error: env refused: not_granted: \u{415}\nerror: env refused: not_granted: ~\n\
             error: env refused: not_granted: \\u{1b}[2J\n"
                .to_owned(),
            REFUSALS_FAIL.to_owned(),
        ),
        (
            "long",
            format!("{import}try {{\n\tenv.get('{long}');\n}} catch {{}}\n"),
            1,
            format!(
                "error: env refused: not_granted: {}… ({} bytes)\n",
                &long[..gro_runtime::MAX_LISTED_MESSAGE - "not_granted: ".len()],
                long.len() + "not_granted: ".len()
            ),
            REFUSALS_FAIL.to_owned(),
        ),
        (
            "long_beside_a_short_one",
            format!(
                "{import}for (const name of ['{long}', 'C']) {{\n\ttry {{\n\t\tenv.get(name);\n\t}} catch {{}}\n}}\n"
            ),
            1,
            format!(
                "error: env refused: not_granted: {}… ({} bytes)\n\
                 error: env refused: not_granted: C\n",
                &long[..gro_runtime::MAX_LISTED_MESSAGE - "not_granted: ".len()],
                long.len() + "not_granted: ".len()
            ),
            format!("{REFUSALS_FAIL}hint: add --allow-env=C to grant some of what env refused\n"),
        ),
        (
            "unflaggable",
            format!(
                "{import}for (const name of ['B,C', 'D*', 'E']) {{\n\ttry {{\n\t\tenv.get(name);\n\t}} catch {{}}\n}}\n"
            ),
            1,
            "error: env refused: not_granted: B,C\nerror: env refused: not_granted: D*\n\
             error: env refused: not_granted: E\n"
                .to_owned(),
            format!("{REFUSALS_FAIL}hint: add --allow-env=E to grant some of what env refused\n"),
        ),
        (
            "unflaggable_only",
            format!("{import}try {{\n\tenv.get('B,C');\n}} catch {{}}\n"),
            1,
            "error: env refused: not_granted: B,C\n".to_owned(),
            REFUSALS_FAIL.to_owned(),
        ),
        (
            "quiet",
            format!(
                "{import}if (env.get('A') !== undefined) throw new Error('set');\n\
                 env.narrow(['A']).narrow([]);\n\
                 try {{\n\tenv.get('B=1');\n}} catch {{}}\n"
            ),
            0,
            String::new(),
            String::new(),
        ),
    ];
    for (name, source, code, stderr, tail) in cases {
        let fixtures = temp_fixtures(&format!("env_refused_{name}"), &[("main.js", &source)]);
        let run = run_with(&fixtures, &["--allow-env=A"], "main.js", &[("B", "secret")]);
        assert_eq!(run.code, code, "{name}: {}", run.stderr);
        assert!(run.stderr.starts_with(&stderr), "{name}: {}", run.stderr);
        assert!(run.stderr.ends_with(&tail), "{name}: {}", run.stderr);
        // each refusal printed once, listed or as the run's error
        assert_eq!(
            run.stderr.matches("not_granted").count(),
            stderr.matches("not_granted").count(),
            "{name}: {}",
            run.stderr
        );
        assert_eq!(
            run.stderr.matches("hint: ").count(),
            usize::from(tail.contains("hint: ")),
            "{name}: {}",
            run.stderr
        );
        assert!(!run.stderr.contains("secret"), "{name}: {}", run.stderr);
    }
}

/// A package's module is refused `env` though the project is granted it,
/// and imports it unconfined, `-A`.
#[test]
fn a_package_is_refused_env_unless_the_run_is_unconfined() {
    let fixtures = temp_fixtures(
        "env_package",
        &[
            ("package.json", r#"{"name": "app", "type": "module"}"#),
            (
                "node_modules/dep/package.json",
                r#"{"name": "dep", "type": "module", "exports": "./index.js"}"#,
            ),
            (
                "node_modules/dep/index.js",
                "export const load = () => import('@fuzdev/gro/env.ts');\n",
            ),
            (
                "main.js",
                "import { env } from '@fuzdev/gro/env.ts';\n\
                 import { load } from 'dep';\n\
                 const loaded = await load().catch((error) => error.message);\n\
                 console.log(typeof loaded === 'string' ? loaded : loaded.env === env);\n",
            ),
        ],
    );
    let run = run_with(&fixtures, &["--allow-env=GRO_A"], "main.js", &[]);
    assert_eq!(run.code, 1, "{}", run.stderr);
    let refusal = format!(
        "cannot import '@fuzdev/gro/env.ts' from file://{}/node_modules/dep/index.js: it needs \
         the grant 'env', which package 'dep' isn't granted: only the project's own modules \
         are granted host modules\n",
        fixtures.0.display()
    );
    assert_eq!(run.stdout, refusal);
    assert_eq!(
        run.stderr,
        format!("error: import() refused: {refusal}{REFUSALS_FAIL}")
    );
    let run = run_with(&fixtures, &["-A"], "main.js", &[]);
    assert_eq!(
        (run.code, run.stdout.as_str(), run.stderr.as_str()),
        (0, "true\n", "")
    );
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
    assert_eq!(
        run.stderr,
        format!("error: import() refused: {refusal}{REFUSALS_FAIL}")
    );
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
