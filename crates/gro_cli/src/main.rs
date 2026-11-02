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
mod eval;
mod harden;
mod run;
mod snapshot;
mod thread;
mod version;

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use argh::FromArgs as _;
use gro_runtime::phases;
use gro_runtime::prices;

use crate::cli::{Args, Command, MAKE_SNAPSHOT, RunArgs, early_exit};
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
    // a priced build's switches, refused before anything runs when one is
    // unknown; nothing without the `prices` feature
    if let Err(message) = prices::start() {
        eprintln!("error: {message}");
        return EXIT_USAGE;
    }
    let args: Result<Vec<String>, OsString> = std::env::args_os()
        .skip(1)
        .map(OsString::into_string)
        .collect();
    let args = match args {
        Ok(args) => args,
        Err(arg) => {
            eprintln!(
                "error: an argument is not valid UTF-8: {}",
                arg.to_string_lossy()
            );
            return EXIT_USAGE;
        }
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // eval's process must keep one thread until it is confined, so it runs
    // here and starts its own
    if runs_on_main_thread(&args) {
        return run(&args);
    }
    run_on_sized_stack(&args)
}

/// Whether `args` parse as `fgro eval`, which confines its process and so
/// runs on the main thread: decided from what argh parses, not the first
/// argument, so every spelling it accepts (`fgro -- eval`) runs there.
fn runs_on_main_thread(args: &[&str]) -> bool {
    args.first() != Some(&MAKE_SNAPSHOT)
        && matches!(
            Args::from_args(&["fgro"], args),
            Ok(Args {
                version: false,
                command: Some(Command::Eval(_)),
            })
        )
}

/// Runs `fgro` with `args` (the program name excluded), returning the exit code.
fn run(args: &[&str]) -> u8 {
    if args.first() == Some(&MAKE_SNAPSHOT) {
        // no flags, so `--help` can't become a directory's name
        let [_, dir] = args else {
            eprintln!("usage: fgro {MAKE_SNAPSHOT} <dir>");
            return EXIT_USAGE;
        };
        if dir.starts_with('-') {
            eprintln!("usage: fgro {MAKE_SNAPSHOT} <dir> (it takes no flags)");
            return EXIT_USAGE;
        }
        return make_snapshot(Path::new(dir)).map_or_else(
            |error| {
                eprintln!("error: {error}");
                EXIT_FAILURE
            },
            |()| 0,
        );
    }
    match Args::from_args(&["fgro"], args) {
        Ok(Args {
            version: true,
            command: None,
        }) => {
            println!("{}", version::version_line());
            0
        }
        Ok(Args {
            version: true,
            command: Some(_),
        }) => {
            eprintln!("error: --version takes no subcommand");
            EXIT_USAGE
        }
        Ok(Args {
            version: false,
            command: None,
        }) => {
            eprintln!("error: a subcommand is required\nRun fgro --help for more information.");
            EXIT_USAGE
        }
        Ok(Args {
            version: false,
            command: Some(Command::Run(RunArgs { file })),
        }) => run::run_module(&file),
        Ok(Args {
            version: false,
            command: Some(Command::Eval(args)),
        }) => eval::eval(args),
        Err(early) => early_exit(early),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_exits_0() {
        assert_eq!(run(&["--help"]), 0);
        assert_eq!(run(&["--version"]), 0);
        assert_eq!(run(&["run", "--help"]), 0);
        assert_eq!(run(&["eval", "--help"]), 0);
    }

    /// Every spelling argh parses as `eval` runs on the main thread, `--`
    /// before the subcommand included, and nothing else does.
    #[test]
    fn eval_alone_runs_on_the_main_thread() {
        for args in [
            &["eval", "a.js"][..],
            &["--", "eval", "a.js"],
            &["eval", "--jit", "a.js"],
        ] {
            assert!(runs_on_main_thread(args), "{args:?}");
        }
        for args in [
            &["run", "a.js"][..],
            &["--version"],
            &["eval"],
            &["--version", "eval", "a.js"],
            &["make-snapshot", "eval"],
            &[],
        ] {
            assert!(!runs_on_main_thread(args), "{args:?}");
        }
    }

    #[test]
    fn usage_error_exits_2() {
        assert_eq!(run(&[]), EXIT_USAGE);
        assert_eq!(run(&["--unknown"]), EXIT_USAGE);
        assert_eq!(run(&["positional"]), EXIT_USAGE);
        assert_eq!(run(&["run"]), EXIT_USAGE);
        assert_eq!(run(&["run", "a.js", "b.js"]), EXIT_USAGE);
        assert_eq!(run(&["--version", "run", "a.js"]), EXIT_USAGE);
        assert_eq!(run(&["make-snapshot"]), EXIT_USAGE);
        assert_eq!(run(&["make-snapshot", "a", "b"]), EXIT_USAGE);
        assert_eq!(run(&["make-snapshot", "--help"]), EXIT_USAGE);
        assert_eq!(run(&["make-snapshot", "-x"]), EXIT_USAGE);
        assert_eq!(run(&["eval"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "b.js"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "--driver", "d.js"]), EXIT_USAGE);
        assert_eq!(
            run(&["eval", "a.js", "--driver", "-", "--export", "x"]),
            EXIT_USAGE
        );
        assert_eq!(run(&["eval", "a.js", "--timeout", "0"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "--timeout", "-1"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "--timeout", "1s"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "--unknown"]), EXIT_USAGE);
    }
}
