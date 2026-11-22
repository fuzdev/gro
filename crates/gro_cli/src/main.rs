//! `fgro`: gro on its own JS runtime, the binary's name while it's developed
//! beside the Node `gro`.
//!
//! - `fgro run <file> [<args...>]` runs an ES module (JS or TypeScript) from
//!   disk until its event loop drains, every argument after the file the
//!   script's (`cli::RunInvocation`); the reference is `src/docs/run.md`
//! - `fgro eval <module> [--driver -] [--export NAME] [--jit] [--timeout MS]
//!   [--graph]` evaluates a module to JSON data in a kernel box and prints
//!   one envelope on stdout; the reference is `src/docs/eval.md`
//! - `fgro --version` prints one line naming the version, the commit, the
//!   V8 version, and whether a snapshot is embedded (`version`)
//! - `fgro <command> [<args...>]`, for every other first argument and for
//!   none, runs a command of Gro's (`gro check`, the listing): its
//!   pre-pass (`verb::prepass`) decides on the main thread, before any
//!   thread starts, whether it runs on the binary from a generated entry,
//!   held to its closure's grants and the grant flags right after the
//!   command word, or on the Node Gro the project installed, which it execs
//!   after one line saying why (`verb::fallback`), or neither; the
//!   reference is `src/docs/runtime.md` (§Gro's workflow on the runtime)
//!
//! Every command but `eval` and a command of Gro's pre-pass runs on one
//! thread with [`gro_runtime::STACK_SIZE`] reserved, not on the main
//! thread, so a debug build stays inside the stack tsv's TypeScript strip
//! is sized for, whatever the platform's main thread has. `eval` keeps its
//! process at one thread until it is confined; the order of its steps is
//! `eval::confine_eval`'s.
//!
//! Exit codes, the same for `run` and `eval`: `0` success, `1` a run that
//! fails, `2` a usage error, an entry the resolver refuses included (one
//! that's missing, say), or under `eval` one that isn't a project's own.
//! An entry whose resolution fails reading the tree, on an I/O error or a
//! symlink loop, exits `1`: no change to the command line fixes it. A
//! program under `run` can exit with a code of its own, 0 to 255, by the
//! exit host module (`run::exit_code` has the rules). A command of Gro's
//! exits as `run` does on the binary, its code Gro's; with `2` when its
//! pre-pass refuses it (its grant flags, `GRO_FALLBACK`'s value, a working
//! directory with no project's `package.json`, a fallback
//! `GRO_FALLBACK=command` refuses, or one with no built Node Gro or no
//! `node` to run it), `1` when the grants table can't be read or `node`
//! fails to start; and with the Node Gro's code when it falls back.
//!
//! Features (the binary that ships never enables `phases` or `prices`):
//!
//! - `snapshot` embeds a startup snapshot and the embed of Gro's own library
//!   (`embed`), both read from `GRO_SNAPSHOT_DIR`, the commit from
//!   `GRO_COMMIT`, and the version from `GRO_VERSION`; `cargo xtask build`
//!   makes it in two stages, the first running the internal
//!   `fgro make-snapshot <dir>` and `fgro make-embed <src> <dir>`.
//!   Without it, `fgro` evaluates the runtime's sources at startup, reading
//!   the web extensions' JS from the build tree, embeds no library but
//!   makes its copy from the build tree's `src/` the first time a run needs
//!   it (`embed::load_embed`), and names Cargo's version
//! - `phases` writes a timestamp at each phase boundary to the file
//!   `GRO_PHASES_FILE` names (`gro_runtime::phases`), for
//!   `cargo xtask bench --phases`
//! - `prices` reads `GRO_PRICES_OFF`, eval's defaults to turn off
//!   (`gro_runtime::prices`), for the bench's prices suite
//!
//! Without its feature, no such variable is read.
//!
//! Linux-only, as `gro_runtime` and `gro_sys` are: built for another OS the
//! crate fails to compile.

#[cfg(not(target_os = "linux"))]
compile_error!("gro_cli is Linux-only, as the gro_runtime it embeds is");

mod cli;
mod color;
mod embed;
mod error;
mod eval;
mod harden;
mod print;
mod run;
mod snapshot;
mod thread;
mod verb;
mod version;

use std::ffi::OsString;

use gro_runtime::{BINARY_NAME, phases, prices};

use crate::cli::{Invocation, Task};
use crate::embed::make_embed;
use crate::error::CliError;
use crate::print::stdout_line;
use crate::snapshot::make_snapshot;
use crate::thread::run_on_sized_stack;

// the file cargo builds is the binary every message names, so neither is
// renamed without the other
const _: () = assert!(
    same_str(env!("CARGO_BIN_NAME"), BINARY_NAME),
    "the binary's `[[bin]]` name is `gro_runtime::BINARY_NAME`"
);

/// Whether `a` and `b` are the same string, at compile time.
const fn same_str(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The caller must change the arguments before re-running.
const EXIT_USAGE: u8 = 2;

/// The program failed.
const EXIT_FAILURE: u8 = 1;

/// A panic, Rust's own exit code for one, restated because the panicking
/// thread is joined rather than unwinding out of `main`. Only a debug build
/// reaches it: the release profile aborts on a panic.
const EXIT_PANIC: u8 = 101;

fn main() {
    phases::start();
    end(main_code());
}

/// Ends the process with `code`, after a phases build writes its file: the
/// one step every command exits by, a program's `exit` included, which
/// reaches it from inside its host op (`run::exit_now`). Eval's
/// out-of-memory backstop is the exception: it ends the process from V8's
/// out-of-memory callback without reaching this. Nothing a run or an eval
/// made is torn down before it: their runtimes were left as they were for
/// the OS to reclaim, which costs less than tearing them down.
///
/// It takes only a code and reaches no runtime. Console output is written
/// and flushed as it's printed, so nothing is buffered here today; a later
/// host module that buffers stdout or stderr must have it written before
/// the runtime is handed back and before `exit` calls the embedder's
/// step, or keep the buffer where this can reach it.
fn end(code: u8) -> ! {
    phases::finish();
    std::process::exit(i32::from(code))
}

/// [`main`]'s work, returning the exit code.
fn main_code() -> u8 {
    match invocation() {
        // eval's process must keep one thread until it is confined, so it
        // runs here and starts its own
        Ok(Invocation::Eval(args)) => eval::eval(args),
        // its pre-pass runs here, before any thread starts, and a fallback
        // execs the Node Gro from here
        Ok(Invocation::Verb(args)) => run::verb(&args),
        Ok(Invocation::Sized(task)) => run_on_sized_stack(&task),
        Err(error) => error.report(),
    }
}

/// Reads the process's arguments into an [`Invocation`], after a priced
/// build's switches, refused before anything runs when one is unknown
/// (nothing without the `prices` feature).
///
/// # Errors
///
/// Returns a usage error: an unknown switch, an argument that isn't UTF-8,
/// or one [`cli::parse`] refuses.
fn invocation() -> Result<Invocation, CliError> {
    prices::start()?;
    let args = utf8_args(std::env::args_os().skip(1))?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    cli::parse(&args)
}

/// `args` as strings, every one checked, the script's after `run`'s file
/// included.
///
/// # Errors
///
/// Returns [`CliError::NotUtf8`] naming the first that isn't UTF-8.
fn utf8_args(args: impl IntoIterator<Item = OsString>) -> Result<Vec<String>, CliError> {
    args.into_iter()
        .map(OsString::into_string)
        .collect::<Result<Vec<String>, OsString>>()
        .map_err(CliError::NotUtf8)
}

/// Runs `task`, returning the exit code and printing a failure to stderr.
fn run(task: &Task) -> u8 {
    let result = match task {
        Task::Version => stdout_line(version::version_line()).map_err(CliError::Stdout),
        Task::Help(text) => stdout_line(text.trim_end()).map_err(CliError::Stdout),
        Task::MakeSnapshot(dir) => make_snapshot(dir),
        Task::MakeEmbed(src, dir) => make_embed(src, dir),
        // each prints its own failure, before the imports it was refused
        Task::Run(invocation) => return run::run_module(invocation),
        Task::Verb(plan) => return run::run_verb(plan),
    };
    result.map_or_else(|error| error.report(), |()| 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exit code of `fgro` with `args`, eval's run as it would be: only
    /// eval's usage errors, which return before it touches the process.
    fn code(args: &[&str]) -> u8 {
        match cli::parse(args) {
            Ok(Invocation::Eval(args)) => eval::eval(args),
            Ok(Invocation::Sized(task)) => run(&task),
            Ok(Invocation::Verb(args)) => panic!("{args:?} is a command of gro's"),
            Err(error) => error.report(),
        }
    }

    /// Every first argument but the binary's own commands, version, help,
    /// and internal subcommands is a command of gro's, and so is none,
    /// every argument kept as written.
    #[test]
    fn every_other_first_argument_is_a_command_of_gro_s() {
        let verb = |args: &[&str]| match cli::parse(args) {
            Ok(Invocation::Verb(verb)) => verb,
            _ => panic!("{args:?} is a command of gro's"),
        };
        for args in [
            &[][..],
            &["check", "--allow-read=.", "--", "x"],
            &["-v"],
            &["--unknown"],
            &["--"],
            &["--", "eval", "a.js"],
            &["--allow-env=TOKEN=hunter2", "run", "a.js"],
            &["src/a.task.ts"],
            &["Run"],
            &["-A", "eval"],
        ] {
            assert_eq!(verb(args), args, "{args:?}");
        }
        for args in [
            &["run", "a.js"][..],
            &["eval", "a.js"],
            &["--version"],
            &["--help"],
            &["help"],
            &["make-snapshot", "dir"],
            &["make-embed", "src", "dir"],
        ] {
            assert!(
                !matches!(cli::parse(args), Ok(Invocation::Verb(_))),
                "{args:?}"
            );
        }
    }

    #[test]
    fn help_exits_0() {
        assert_eq!(code(&["--help"]), 0);
        assert_eq!(code(&["--version"]), 0);
        assert_eq!(code(&["run", "--help"]), 0);
        assert_eq!(code(&["eval", "--help"]), 0);
    }

    /// Every spelling argh parses as `eval` runs on the main thread, `--`
    /// before the subcommand included, and nothing else does.
    #[test]
    fn eval_alone_runs_on_the_main_thread() {
        let on_main = |args| matches!(cli::parse(args), Ok(Invocation::Eval(_)));
        for args in [&["eval", "a.js"][..], &["eval", "--jit", "a.js"]] {
            assert!(on_main(args), "{args:?}");
        }
        for args in [
            &["run", "a.js"][..],
            &["--version"],
            &["eval"],
            &["eval", "--help"],
            &["--version", "eval", "a.js"],
            &["make-snapshot", "eval"],
            &["make-embed", "eval", "a.js"],
            &["--", "eval", "a.js"],
            &[],
        ] {
            assert!(!on_main(args), "{args:?}");
        }
    }

    #[test]
    fn usage_error_exits_2() {
        assert_eq!(code(&["--version", "--unknown"]), EXIT_USAGE);
        assert_eq!(code(&["--help", "positional"]), EXIT_USAGE);
        assert_eq!(code(&["run"]), EXIT_USAGE);
        assert_eq!(code(&["run", "--"]), EXIT_USAGE);
        assert_eq!(code(&["run", "--x", "a.js"]), EXIT_USAGE);
        assert_eq!(code(&["run", "-", "a.js"]), EXIT_USAGE);
        assert_eq!(code(&["run", "--allow-env", "a.js"]), EXIT_USAGE);
        assert_eq!(code(&["run", "--allow-env=A=1", "a.js"]), EXIT_USAGE);
        assert_eq!(code(&["--version", "run", "a.js"]), EXIT_USAGE);
        assert_eq!(code(&["make-snapshot"]), EXIT_USAGE);
        assert_eq!(code(&["make-snapshot", "a", "b"]), EXIT_USAGE);
        assert_eq!(code(&["make-snapshot", "--help"]), EXIT_USAGE);
        assert_eq!(code(&["make-snapshot", "-x"]), EXIT_USAGE);
        assert_eq!(code(&["make-embed"]), EXIT_USAGE);
        assert_eq!(code(&["make-embed", "src"]), EXIT_USAGE);
        assert_eq!(code(&["make-embed", "src", "a", "b"]), EXIT_USAGE);
        assert_eq!(code(&["make-embed", "src", "--help"]), EXIT_USAGE);
        assert_eq!(code(&["make-embed", "-x", "dir"]), EXIT_USAGE);
        assert_eq!(code(&["eval"]), EXIT_USAGE);
        assert_eq!(code(&["eval", "a.js", "b.js"]), EXIT_USAGE);
        assert_eq!(code(&["eval", "a.js", "--driver", "d.js"]), EXIT_USAGE);
        assert_eq!(
            code(&["eval", "a.js", "--driver", "-", "--export", "x"]),
            EXIT_USAGE
        );
        assert_eq!(code(&["eval", "a.js", "--timeout", "0"]), EXIT_USAGE);
        assert_eq!(code(&["eval", "a.js", "--timeout", "-1"]), EXIT_USAGE);
        assert_eq!(code(&["eval", "a.js", "--timeout", "1s"]), EXIT_USAGE);
        assert_eq!(code(&["eval", "a.js", "--unknown"]), EXIT_USAGE);
    }

    /// The messages the parse's usage errors print, labeled or as written.
    #[test]
    fn usage_errors_print_their_messages() {
        let message = |args| {
            let error = cli::parse(args).err().unwrap_or_else(|| panic!("{args:?}"));
            (error.to_string(), error.exit_code())
        };
        assert_eq!(
            message(&["make-snapshot"]),
            (
                format!("usage: {BINARY_NAME} make-snapshot <dir>"),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["make-snapshot", "-x"]),
            (
                format!("usage: {BINARY_NAME} make-snapshot <dir> (it takes no flags)"),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["make-embed", "src"]),
            (
                format!("usage: {BINARY_NAME} make-embed <src> <dir>"),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["make-embed", "src", "--help"]),
            (
                format!("usage: {BINARY_NAME} make-embed <src> <dir> (it takes no flags)"),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["run", "--allow-net=.", "a.js"]),
            (
                format!(
                    "unknown option \"--allow-net=…\" before the file: the script's arguments \
                     go after it\nRun {BINARY_NAME} run --help for more information."
                ),
                EXIT_USAGE
            )
        );
        // an argument a usage error names is cut at its first `=`, which can
        // start a value, and escaped, whether `run` or argh refused it
        for args in [
            &["run", "--allow_env=TOKEN=hunter2", "a.js"][..],
            &["run", "-A=hunter2", "a.js"],
            &["run", "-\u{1b}[2J=hunter2", "a.js"],
            &["--version", "--allow-env=TOKEN=hunter2", "run", "a.js"],
            &["--version", "--\u{1b}[2J=hunter2", "run", "a.js"],
            &["eval", "a.js", "--x=hunter2"],
            // an argument that's part of another, which argh refuses
            &["eval", "--export", "=", "a.js", "TOKEN=hunter2"],
            &["eval", "--export", "x=", "a.js", "x=\u{1b}[2Jhunter2"],
            // one whose text is what the other prints as
            &["eval", "--export", "x=…", "a.js", "x=hunter2"],
        ] {
            let (text, code) = message(args);
            assert_eq!(code, EXIT_USAGE, "{args:?}");
            assert!(!text.contains("hunter2"), "{args:?}: {text}");
            assert!(!text.contains('\u{1b}'), "{args:?}: {text}");
            assert!(text.contains("=…"), "{args:?}: {text}");
        }
        assert_eq!(
            message(&["run", "-\u{1b}[2J", "a.js"]).0,
            format!(
                "unknown option \"-\\u{{1b}}[2J\" before the file: the script's arguments go \
                 after it\nRun {BINARY_NAME} run --help for more information."
            )
        );
        assert_eq!(
            message(&["run", "--allow-env", "a.js"]),
            (
                format!(
                    "--allow-env takes the names it grants, as --allow-env=A,B\nRun \
                     {BINARY_NAME} run --help for more information."
                ),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["run", "--allow-temp=secret", "a.js"]),
            (
                format!(
                    "--allow-temp takes no value: it grants fresh directories under the OS's \
                     temporary directory, as --allow-temp\nRun {BINARY_NAME} run --help for \
                     more information."
                ),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["run"]),
            (
                format!(
                    "a file to run is required\nRun {BINARY_NAME} run --help for more information."
                ),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["--version", "run", "a.js"]),
            ("--version takes no subcommand".to_owned(), EXIT_USAGE)
        );
    }

    /// An argument that isn't UTF-8 is refused wherever it comes, a script's
    /// after the file included, and the rest pass as they are.
    #[test]
    fn a_non_utf8_argument_is_refused() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let args = |tail: &[&[u8]]| {
            ["run", "a.js"]
                .iter()
                .map(OsString::from)
                .chain(tail.iter().map(|arg| OsStr::from_bytes(arg).to_owned()))
                .collect::<Vec<_>>()
        };
        for tail in [&[b"\xff".as_slice()][..], &[b"x", b"--", b"a\xffb"]] {
            let error = utf8_args(args(tail)).expect_err("refused");
            assert!(matches!(error, CliError::NotUtf8(_)), "{error}");
            assert_eq!(error.exit_code(), EXIT_USAGE);
        }
        // named up to its first `=`, escaped, so a value that follows isn't printed
        let error = utf8_args(args(&[b"--allow-env=TOKEN=hunter2\xff"])).expect_err("refused");
        assert_eq!(
            error.to_string(),
            "an argument is not valid UTF-8: --allow-env=…"
        );
        let error = utf8_args(args(&[b"\x1b[2J\xff"])).expect_err("refused");
        assert_eq!(
            error.to_string(),
            "an argument is not valid UTF-8: \\u{1b}[2J\u{fffd}"
        );
        assert_eq!(
            utf8_args(args(&[b"", b"--", "\u{e9}".as_bytes()])).ok(),
            Some(
                ["run", "a.js", "", "--", "\u{e9}"]
                    .map(str::to_owned)
                    .to_vec()
            )
        );
    }
}
