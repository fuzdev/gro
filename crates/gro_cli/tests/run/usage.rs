//! The command line: the entry, the version line, usage errors, the script's
//! arguments, and a closed stdout.

use std::process::Stdio;

use gro_runtime::BINARY_NAME;

use crate::support::{fgro_command, spawn, version};
use crate::{commit, fgro, fixtures, run};

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

/// The bench records this line, so it must say which build ran: the plain
/// one with Cargo's version and no commit, the snapshot one with
/// `package.json`'s version and [`commit`].
#[test]
fn version_names_the_version_commit_v8_and_the_snapshot() {
    let output = fgro(&["--version"]);
    assert!(output.status.success(), "{output:?}");
    let line = String::from_utf8(output.stdout).unwrap();
    let commit = commit().unwrap_or("unknown");
    assert!(
        line.starts_with(&format!(
            "{BINARY_NAME} {} (commit {commit}, V8 ",
            version()
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
/// refused, not skipped. An argument it names is printed up to its first
/// `=`, escaped, so a value after one never reaches the terminal, nor do
/// its raw control characters.
#[test]
fn usage_errors_exit_2() {
    use std::os::unix::ffi::OsStrExt as _;
    for args in [
        &["run"][..],
        &["run", "--"],
        &["--version", "--unknown"],
        &["run", "--x", "console.js"],
        &["run", "--allow-env", "console.js"],
        &["run", "--allow-env=", "console.js"],
        &["run", "--allow-env=A,", "console.js"],
        &["run", "--allow-env=A*", "console.js"],
        &["run", "-A", "--allow-env=TOKEN=hunter2", "console.js"],
        &["run", "--allow-read", "console.js"],
        &["run", "--allow-write", "console.js"],
        &["run", "-A", "--allow-write", "console.js"],
        &["run", "--allow-read=", "console.js"],
        &["run", "--allow-write=a,", "console.js"],
        &["run", "--allow-all", "console.js"],
        &["run", "-A"],
        &["run", "--allow_env=TOKEN=hunter2", "console.js"],
        &["run", "-A=hunter2", "console.js"],
        &["--allow-env=TOKEN=hunter2", "run", "console.js"],
    ] {
        let output = fgro(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        // what follows a `=` in a name can be a value, and is never echoed
        assert!(!stderr.contains("hunter2"), "{args:?}: {stderr}");
    }
    // an argument that isn't UTF-8 is named up to its first `=`, escaped
    let mut command = fgro_command(&["run"]);
    command.arg(std::ffi::OsStr::from_bytes(
        b"--allow-env=TOKEN=hunter2\x1b[2J\xff",
    ));
    command.arg("console.js").current_dir(fixtures());
    let output = spawn(command, b"");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: an argument is not valid UTF-8: --allow-env=…\n"
    );
    let output = fgro(&["run", "--allow-env", "console.js"]);
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "error: --allow-env takes the names it grants, as --allow-env=A,B\n\
             Run {BINARY_NAME} run --help for more information.\n"
        )
    );
}

/// `--allow-run-write=` under `fgro run`: refused without `--allow-run=`,
/// and for a program no `--allow-run=` lists, before anything runs; given
/// one, its directories are checked, the run otherwise unchanged, with no
/// note of its own, since the program's box holds it to them
/// (`run_boxed.rs`); under `-A` it's checked and says nothing.
#[test]
fn run_checks_a_program_s_write_flag() {
    let stderr = |args: &[&str]| {
        let output = fgro(args);
        (
            output.status.code(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };
    let help = format!("Run {BINARY_NAME} run --help for more information.\n");
    assert_eq!(
        stderr(&["run", "--allow-run-write=git:.", "console.js"]),
        (
            Some(2),
            String::new(),
            format!(
                "error: --allow-run-write lets a program --allow-run grants write, and no \
                 --allow-run grants one\n{help}"
            )
        )
    );
    assert_eq!(
        stderr(&[
            "run",
            "--allow-run=git",
            "--allow-run-write=x\u{1b}:secret",
            "console.js"
        ]),
        (
            Some(2),
            String::new(),
            format!(
                "error: --allow-run-write=… names x\\u{{1b}}, which no --allow-run= grants: list \
                 x\\u{{1b}} there too\n{help}"
            )
        )
    );
    let (code, _, missing) = stderr(&[
        "run",
        "--allow-run=git",
        "--allow-run-write=git:missing",
        "console.js",
    ]);
    assert_eq!(code, Some(2));
    assert!(
        missing.starts_with("error: a --allow-run-write directory doesn't exist: "),
        "{missing}"
    );
    let (code, stdout, noted) = stderr(&[
        "run",
        "--allow-run=git",
        "--allow-run-write=git:.",
        "console.js",
    ]);
    assert_eq!(code, Some(0), "{noted}");
    assert!(stdout.starts_with("log 1"), "{stdout}");
    assert!(!noted.contains("note:"), "{noted}");
    let (code, _, all) = stderr(&[
        "run",
        "-A",
        "--allow-run=git",
        "--allow-run-write=git:.",
        "console.js",
    ]);
    assert_eq!(code, Some(0), "{all}");
    assert!(!all.contains("note:"), "{all}");
}

/// An argument that isn't UTF-8 is refused before anything runs, the file or
/// one of the script's.
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
