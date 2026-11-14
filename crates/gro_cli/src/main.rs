//! `fgro`: gro on its own JS runtime, the binary's name while it's developed
//! beside the Node `gro`.
//!
//! - `fgro run <file> [<args...>]` runs an ES module (JS or TypeScript) from
//!   disk until its event loop drains, every argument after the file the
//!   script's (`cli::RunInvocation`); the reference is `src/docs/run.md`
//! - `fgro eval <module> [--driver -] [--export NAME] [--jit] [--timeout MS]
//!   [--graph]` evaluates a module to JSON data in a kernel box and prints
//!   one envelope on stdout; the reference is `src/docs/eval.md`
//! - `fgro --version` prints one line naming the commit, the V8 version, and
//!   whether a snapshot is embedded (`version`)
//!
//! Every command but `eval` runs on one thread with [`gro_runtime::STACK_SIZE`]
//! reserved, not on the main thread, so a debug build stays inside the stack
//! tsv's TypeScript strip is sized for, whatever the platform's main thread
//! has. `eval` keeps its process at one thread until it is confined; the
//! order of its steps is `eval::confine_eval`'s.
//!
//! Exit codes, the same for `run` and `eval`: `0` success, `1` a run that
//! fails, `2` a usage error, an entry that doesn't resolve included.
//!
//! Features (the binary that ships never enables `phases` or `prices`):
//!
//! - `snapshot` embeds a startup snapshot read from `GRO_SNAPSHOT_DIR` and
//!   the commit from `GRO_COMMIT`; `cargo xtask build` makes it in two
//!   stages, the first running the internal `fgro make-snapshot <dir>`.
//!   Without it, `fgro` evaluates the runtime's sources at startup, reading
//!   the web extensions' JS from the build tree
//! - `phases` writes a timestamp at each phase boundary to the file
//!   `GRO_PHASES_FILE` names (`gro_runtime::phases`), for
//!   `cargo xtask bench --phases`
//! - `prices` reads `GRO_PRICES_OFF`, eval's defaults to turn off
//!   (`gro_runtime::prices`), for the bench's prices suite
//!
//! Without its feature, no such variable is read.

mod cli;
mod color;
mod error;
mod eval;
mod harden;
mod print;
mod run;
mod snapshot;
mod thread;
mod version;

use std::ffi::OsString;
use std::process::ExitCode;

use gro_runtime::phases;
use gro_runtime::prices;

use crate::cli::{Invocation, Task};
use crate::error::CliError;
use crate::print::stdout_line;
use crate::snapshot::make_snapshot;
use crate::thread::run_on_sized_stack;

/// The caller must change the arguments before re-running.
const EXIT_USAGE: u8 = 2;

/// The program failed.
const EXIT_FAILURE: u8 = 1;

/// A panic, Rust's own exit code for one, restated because the panicking
/// thread is joined rather than unwinding out of `main`. Only a debug build
/// reaches it: the release profile aborts on a panic.
const EXIT_PANIC: u8 = 101;

fn main() -> ExitCode {
    phases::start();
    let code = main_code();
    phases::finish();
    ExitCode::from(code)
}

/// [`main`]'s work, returning the exit code.
fn main_code() -> u8 {
    match invocation() {
        // eval's process must keep one thread until it is confined, so it
        // runs here and starts its own
        Ok(Invocation::Eval(args)) => eval::eval(args),
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
        // it prints its own failure, before its warnings
        Task::Run(invocation) => return run::run_module(invocation),
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
            Err(error) => error.report(),
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
        for args in [
            &["eval", "a.js"][..],
            &["--", "eval", "a.js"],
            &["eval", "--jit", "a.js"],
        ] {
            assert!(on_main(args), "{args:?}");
        }
        for args in [
            &["run", "a.js"][..],
            &["--version"],
            &["eval"],
            &["eval", "--help"],
            &["--version", "eval", "a.js"],
            &["make-snapshot", "eval"],
            &[],
        ] {
            assert!(!on_main(args), "{args:?}");
        }
    }

    #[test]
    fn usage_error_exits_2() {
        assert_eq!(code(&[]), EXIT_USAGE);
        assert_eq!(code(&["--unknown"]), EXIT_USAGE);
        assert_eq!(code(&["positional"]), EXIT_USAGE);
        assert_eq!(code(&["run"]), EXIT_USAGE);
        assert_eq!(code(&["run", "--"]), EXIT_USAGE);
        assert_eq!(code(&["run", "--x", "a.js"]), EXIT_USAGE);
        assert_eq!(code(&["run", "-", "a.js"]), EXIT_USAGE);
        assert_eq!(code(&["--version", "run", "a.js"]), EXIT_USAGE);
        assert_eq!(code(&["make-snapshot"]), EXIT_USAGE);
        assert_eq!(code(&["make-snapshot", "a", "b"]), EXIT_USAGE);
        assert_eq!(code(&["make-snapshot", "--help"]), EXIT_USAGE);
        assert_eq!(code(&["make-snapshot", "-x"]), EXIT_USAGE);
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
            ("usage: fgro make-snapshot <dir>".to_owned(), EXIT_USAGE)
        );
        assert_eq!(
            message(&["make-snapshot", "-x"]),
            (
                "usage: fgro make-snapshot <dir> (it takes no flags)".to_owned(),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["run", "--allow-env=A", "a.js"]),
            (
                "unknown option \"--allow-env=A\" before the file: the script's arguments go \
                 after it\nRun fgro run --help for more information."
                    .to_owned(),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["run"]),
            (
                "a file to run is required\nRun fgro run --help for more information.".to_owned(),
                EXIT_USAGE
            )
        );
        assert_eq!(
            message(&["--version", "run", "a.js"]),
            ("--version takes no subcommand".to_owned(), EXIT_USAGE)
        );
        assert_eq!(
            message(&[]),
            (
                "a subcommand is required\nRun fgro --help for more information.".to_owned(),
                EXIT_USAGE
            )
        );
    }

    /// An argument that isn't UTF-8 is refused wherever it comes, a script's
    /// after the file included, and the rest pass as they are.
    #[cfg(unix)]
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
