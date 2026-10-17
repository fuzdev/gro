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

/// Runs `fgro` with `args` from the fixtures directory, with stdout and stderr
/// piped (never terminals), so console output is uncolored.
fn fgro(args: &[&str]) -> Output {
    output_within_deadline(
        Command::new(env!("CARGO_BIN_EXE_fgro"))
            .args(args)
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
        code: output.status.code().unwrap(),
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
fn typescript_import_is_refused() {
    let run = run("import_ts.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve './typed.ts' from {}/import_ts.js: TypeScript is not supported \
             yet; .js and .mjs modules load\n",
            fixtures_url()
        )
    );
}

#[test]
fn bare_import_is_refused() {
    let run = run("import_bare.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve 'svelte' from {}/import_bare.js: a bare specifier names a \
             package, and package resolution is not supported yet. Fix: import the file by a \
             relative path\n",
            fixtures_url()
        )
    );
}

#[test]
fn dynamic_import_refusals_are_catchable() {
    let run = run("import_refused_dynamic.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 4, "{}", run.stdout);
    for (line, reason) in lines.iter().zip([
        "there is no Node compat",
        "package resolution is not supported yet",
        "TypeScript is not supported yet",
        "the path has no extension",
    ]) {
        assert!(
            line.starts_with("true TypeError: cannot resolve '"),
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
fn typescript_entry_is_a_usage_error() {
    let run = run("typed.ts");
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("TypeScript is not supported yet"),
        "{}",
        run.stderr
    );
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
        Command::new(env!("CARGO_BIN_EXE_fgro"))
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
    let command = format!("'{}' run console.js {redirect}", env!("CARGO_BIN_EXE_fgro"));
    let mut script = Command::new("script");
    script
        .args(["-qec", &command, "/dev/null"])
        .current_dir(fixtures())
        .env_remove("NO_COLOR")
        .env_remove("CLAUDECODE")
        .envs(env.iter().copied());
    // fail loudly rather than skip: the color policy has no other test
    Command::new("script")
        .arg("--version")
        .output()
        .expect("util-linux `script` is required for the terminal color tests");
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
fn no_color_and_claudecode_turn_color_off_when_non_empty() {
    assert!(run_on_terminal(&[], "").contains('\x1b'));
    for env in [[("NO_COLOR", "1")], [("CLAUDECODE", "1")]] {
        let output = run_on_terminal(&env, "");
        assert!(!output.contains('\x1b'), "{env:?}: {output:?}");
    }
    // set but empty counts as unset
    for env in [[("NO_COLOR", "")], [("CLAUDECODE", "")]] {
        let output = run_on_terminal(&env, "");
        assert!(output.contains('\x1b'), "{env:?}: {output:?}");
    }
}
