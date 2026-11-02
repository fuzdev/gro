//! `fgro`: gro on its own JS runtime, the binary's name while it's developed
//! beside the Node `gro`.
//!
//! `fgro run <file>` runs an ES module from disk (a path relative to the
//! working directory, or absolute) until its event loop drains, TypeScript
//! stripped of its types first. Its imports resolve strictly: files by their
//! full names, and packages in `node_modules` through their `"exports"`. It
//! takes only the entry file: there are no script arguments yet. Errors print
//! to stderr, prefixed `error:`; an uncaught error's report carries its stack
//! with source positions (`file:///path:line:column`).
//!
//! `fgro eval <module> [--driver -] [--export NAME] [--jit] [--timeout MS]
//! [--graph]` evaluates a module to JSON data and prints one envelope on stdout
//! (`gro_runtime::eval`): with no driver, the module's export (`default`
//! unless `--export` names another); with `--driver -`, a driver module read
//! from stdin, whose default export is called with the module's namespace.
//! It runs jitless unless `--jit`, with code generation from strings off,
//! the locale `en-US`, the clock and randomness denied, the heap capped, and
//! a timeout (`--timeout`, else `gro_runtime::eval::DEFAULT_TIMEOUT`), and
//! its console writes every level to stderr, uncolored. It clears its
//! environment but `TZ` before it starts a thread, so past exec no variable
//! configures it or a library it runs, then caps the process's memory
//! (`RLIMIT_DATA`, at `gro_runtime::eval::MEMORY_LIMIT`), the default
//! thread stack lowered first so an inherited stack limit doesn't size V8's
//! workers, and refuses a time zone other than UTC, which ICU reads from
//! `TZ`. Out of memory, the runtime writes a prepared envelope through its
//! own handle to stdout, duplicated here before the run. With `--graph`, the envelope also carries the module
//! graph the run loaded, as `graph` (`gro_runtime::eval::GraphDump`).
//! Once eval starts, the envelope is always written, `ok: false` with an
//! error kind on failure, unless the process dies first (a panic, or an
//! allocation past the memory cap that nothing reports: V8's on a thread of
//! its own, or Rust's); a usage error prints to stderr with no envelope.
//!
//! Every command but `eval` runs on one thread with [`gro_runtime::STACK_SIZE`] reserved,
//! not on the main thread, so a debug build stays inside the 16 MiB of stack
//! tsv's TypeScript strip is sized for (a release build needs 2 MiB), whatever
//! the platform's main thread has. A module nested or chained past tsv's
//! depth limits fails to load (exit `1`) with a message naming the limit,
//! rather than overflowing the stack.
//!
//! `eval` keeps its process confinable: the main thread parses the
//! arguments, closes every file descriptor above stderr the process
//! inherited (which the box, gating only new opens, wouldn't reach; failing
//! that, eval fails as `environment`), opens the phases file in a phases
//! build, clears the environment but `TZ` (failing that, as `environment`),
//! fixes the default thread stack and caps the process's memory (failing
//! that, as `environment`), and reads the driver from stdin, the working directory, and the
//! out-of-memory handle; a thread with [`gro_runtime::STACK_SIZE`] reserved resolves the
//! entry and computes the box (`gro_runtime::eval::prepass`) and is joined;
//! the main thread, then the process's only one, confines it
//! (`gro_runtime::confine`); and only then does a second thread with
//! [`gro_runtime::STACK_SIZE`] reserved start the engine and evaluate, so every thread
//! the engine and the runtime start comes after the box. The box is Linux's,
//! and needs Landlock ABI 6; elsewhere, and without it, eval refuses. A build
//! with no snapshot, whose runtime would read its sources from the build
//! tree, outside the box, refuses once the entry resolves, before the box,
//! so the same on every host.
//!
//! `fgro --version` prints one line,
//! `fgro <version> (commit <sha>[-dirty], V8 <version>, snapshot)`: the git
//! commit it was built from, full, with `-dirty` when the working tree
//! differed from it (`commit unknown` when no build embedded one, as a plain
//! `cargo build` doesn't), and whether a startup snapshot is embedded
//! (`no snapshot` otherwise), then `, phases` in a phases build and
//! `, prices` in a priced build.
//!
//! Exit codes, the same for `run` and `eval`, are `0` for success, `1` for a
//! run that fails, and `2` for a usage error, an entry that doesn't resolve
//! included; the full table is `src/docs/run.md`'s, and eval's kinds
//! `src/docs/eval.md`'s.
//!
//! Built with the `phases` feature, which release and snapshot builds never
//! enable, `fgro` takes a timestamp at each phase boundary of `run` and `eval`
//! and writes them at exit to the file `GRO_PHASES_FILE` names
//! (`gro_runtime::phases`), for `cargo xtask bench --phases`; eval opens the
//! file before its box goes up. Without the feature it reads no such
//! variable.
//!
//! Built with the `prices` feature, which release and snapshot builds never
//! enable, `fgro` reads `GRO_PRICES_OFF`, a comma-separated list of eval's
//! defaults to turn off, one at a time, for the bench's prices suite
//! (`gro_runtime::prices`); a name it doesn't know is a usage error. Without
//! the feature it reads no such variable.
//!
//! Built with the `snapshot` feature, `fgro` starts from a startup snapshot
//! embedded at compile time, and runs off its build tree. `cargo xtask build`
//! makes that build in two stages: a plain `fgro` writes the snapshot with
//! `fgro make-snapshot <dir>` (internal to the xtask, so not in `--help`),
//! then `fgro` is rebuilt with the feature, reading the directory from
//! `GRO_SNAPSHOT_DIR`; that build also embeds the commit, from `GRO_COMMIT`.
//! Without the feature, `fgro` evaluates the runtime's sources at startup,
//! reading the web extensions' JS from the build tree.

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
    let args = std::env::args_os()
        .skip(1)
        .map(OsString::into_string)
        .collect::<Result<Vec<String>, OsString>>()
        .map_err(CliError::NotUtf8)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    cli::parse(&args)
}

/// Runs `task`, returning the exit code and printing a failure to stderr.
fn run(task: &Task) -> u8 {
    let result = match task {
        Task::Version => stdout_line(version::version_line()).map_err(CliError::Stdout),
        Task::Help(text) => stdout_line(text.trim_end()).map_err(CliError::Stdout),
        Task::MakeSnapshot(dir) => make_snapshot(dir),
        Task::Run(file) => run::run_module(file),
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
        assert_eq!(code(&["run", "a.js", "b.js"]), EXIT_USAGE);
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
}
