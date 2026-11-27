//! Console output, the global names, the clock, the event loop and output that ends
//! whole, and a sibling import.

use std::process::Stdio;

use crate::support::{fgro_command, without_unboxed_note};
use crate::{REFUSALS_FAIL, fixtures, fixtures_url, run, run_in, temp_fixtures};

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

/// A console write whose reader has left (`EPIPE`, a run piped to `head`)
/// is dropped, and the run goes on to its own exit code, writing to the
/// other stream, stdout's or stderr's; a write that fails otherwise, to a
/// full disk, throws an `Error` naming the stream.
#[test]
fn a_console_write_is_dropped_when_its_reader_left_and_throws_otherwise() {
    let fixtures = temp_fixtures(
        "closed_stream",
        &[(
            "main.js",
            "import {set_exit_code} from '@fuzdev/gro/exit.ts';\n\
             for (let i = 0; i < 3; i++) console.log('out', i);\n\
             console.error('err');\nconsole.log('out', 3);\nconsole.error('err', 2);\n\
             set_exit_code(3);\n",
        )],
    );
    let closed = |stderr: bool| {
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let mut command = fgro_command(&["run", "main.js"]);
        command.current_dir(&fixtures.0).stdin(Stdio::null());
        if stderr {
            command.stdout(Stdio::piped()).stderr(writer);
        } else {
            command.stdout(writer).stderr(Stdio::piped());
        }
        let output = command.output().unwrap();
        let other = if stderr {
            output.stdout
        } else {
            without_unboxed_note(output.stderr)
        };
        (output.status.code(), String::from_utf8(other).unwrap())
    };
    assert_eq!(closed(false), (Some(3), "err\nerr 2\n".to_owned()));
    assert_eq!(
        closed(true),
        (Some(3), "out 0\nout 1\nout 2\nout 3\n".to_owned())
    );

    let mut command = fgro_command(&["run", "main.js"]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(
            std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        );
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let dir = format!("file://{}", fixtures.0.display());
    let stderr = String::from_utf8(without_unboxed_note(output.stderr)).unwrap();
    assert!(
        stderr.starts_with(
            "error: Uncaught (in promise) Error: failed to write to stdout: No space left on \
             device (os error 28)\n    at "
        ),
        "{stderr}"
    );
    assert!(
        stderr.ends_with(&format!("\n    at {dir}/main.js:2:37\n")),
        "{stderr}"
    );
}

/// What a run printed is whole when stdout is a file, as when it's a
/// pipe: the process exits without tearing its runtime down, and each of
/// the console's writes was made when it was called, none left for the
/// end.
#[test]
fn output_to_a_file_is_whole_when_the_run_ends() {
    let fixtures = temp_fixtures(
        "output_file",
        &[(
            "main.js",
            "for (let i = 0; i < 3000; i++) console.log('line', i);\n\
             setTimeout(() => console.log('last'), 1);\n",
        )],
    );
    let expected: String = (0..3000)
        .map(|i| format!("line {i}\n"))
        .chain(["last\n".to_owned()])
        .collect();
    let path = fixtures.0.join("out.txt");
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&path).unwrap());
    let output = command.output().unwrap();
    let stderr = String::from_utf8(without_unboxed_note(output.stderr)).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert_eq!(stderr, "");
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written.len(), expected.len());
    assert_eq!(written, expected);
    assert_eq!(run_in(&fixtures, "main.js").stdout, expected);
}

/// What a program printed before `exit` is whole, to a pipe and to a
/// file: the console writes when it's called, and the process ends with
/// nothing of its own left to write.
#[test]
fn output_before_exit_is_whole() {
    let fixtures = temp_fixtures(
        "exit_output",
        &[(
            "main.js",
            "import {exit} from '@fuzdev/gro/exit.ts';\n\
             for (let i = 0; i < 3000; i++) console.log('line', i);\n\
             console.error('to stderr');\nexit(4);\nconsole.log('after');\n",
        )],
    );
    let lines: Vec<String> = (0..3000).map(|i| format!("line {i}")).collect();
    let expected = format!("{}\n", lines.join("\n"));
    let path = fixtures.0.join("out.txt");
    let mut command = fgro_command(&["run", "main.js"]);
    command
        .current_dir(&fixtures.0)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&path).unwrap());
    let output = command.output().unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(
        String::from_utf8(without_unboxed_note(output.stderr)).unwrap(),
        "to stderr\n"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 4);
    assert_eq!(run.stdout, expected);
    assert_eq!(run.stderr, "to stderr\n");
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
    assert_eq!(
        run.stderr,
        format!("error: import() refused: {refusal}\n{REFUSALS_FAIL}")
    );
}
