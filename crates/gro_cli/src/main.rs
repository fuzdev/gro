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
//! The whole command runs on one thread with [`STACK_SIZE`] reserved, not on
//! the main thread, so how deeply nested a TypeScript module can be is
//! stated here rather than inherited from the platform (see the constant).
//!
//! `fgro --version` prints `fgro <version> (V8 <version>, snapshot)`, where
//! the last part says whether a startup snapshot is embedded (`no snapshot`
//! otherwise).
//!
//! Exit codes:
//!
//! - `0`: the program ran to completion, `--help`, or `--version`
//! - `1`: the program threw, rejected a promise no one handled, or failed to
//!   load a module (a TypeScript module the strip refuses included), or the
//!   runtime failed to start
//! - `2`: a usage error, so the command line must change: an unknown
//!   subcommand or flag, a missing argument (including no arguments at all),
//!   an argument that isn't UTF-8, or an entry file that doesn't resolve
//!   (missing, not a `.js`, `.mjs`, `.ts`, or `.mts` file, or a `.js` file
//!   not under `"type": "module"`)
//!
//! Built with the `snapshot` feature, `fgro` starts from a startup snapshot
//! embedded at compile time, and runs off its build tree. `cargo xtask build`
//! makes that build in two stages: a plain `fgro` writes the snapshot with
//! `fgro make-snapshot <dir>` (internal to the xtask, so not in `--help`),
//! then `fgro` is rebuilt with the feature, reading the directory from
//! `GRO_SNAPSHOT_DIR`. Without the feature, `fgro` evaluates the runtime's
//! sources at startup, reading the web extensions' JS from the build tree.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use argh::{EarlyExit, FromArgs};
use gro_runtime::snapshot::StartupSnapshot;
use gro_runtime::{RunError, RuntimeOptions};

/// The snapshot this binary was built with: `snapshot.bin` and `residual.rs`
/// from `fgro make-snapshot`, in the directory `GRO_SNAPSHOT_DIR` names.
#[cfg(feature = "snapshot")]
mod embedded {
    pub static SNAPSHOT: &[u8] = include_bytes!(concat!(env!("GRO_SNAPSHOT_DIR"), "/snapshot.bin"));
    include!(concat!(env!("GRO_SNAPSHOT_DIR"), "/residual.rs"));
}

#[cfg(feature = "snapshot")]
const STARTUP_SNAPSHOT: Option<StartupSnapshot> = Some(StartupSnapshot {
    bytes: embedded::SNAPSHOT,
    residual_lazy_js: embedded::RESIDUAL_LAZY_JS,
    residual_lazy_esm: embedded::RESIDUAL_LAZY_ESM,
});

#[cfg(not(feature = "snapshot"))]
const STARTUP_SNAPSHOT: Option<StartupSnapshot> = None;

/// The internal subcommand that writes the snapshot, handled before argh so
/// it stays out of `--help`.
const MAKE_SNAPSHOT: &str = "make-snapshot";

/// The caller must change the arguments before re-running.
const EXIT_USAGE: u8 = 2;

/// The program failed.
const EXIT_FAILURE: u8 = 1;

/// A panic, Rust's own exit code for one, restated because the panicking
/// thread is joined rather than unwinding out of `main`. Only a debug build
/// reaches it: the release profile aborts on a panic.
const EXIT_PANIC: u8 = 101;

/// The stack reserved for the thread `fgro` runs on, the same 32 MiB tsv's
/// CLI states, for the same reason.
///
/// The TypeScript strip is `tsv_ts`'s recursive-descent parser, so nesting
/// depth costs native stack, and an overflow aborts the process (`SIGABRT`)
/// rather than failing the load. The JS V8 runs is unaffected: V8 sets its
/// own checked limit below where the isolate starts on this thread, so deep
/// JS recursion, and a deeply nested `.js` module, stay a catchable
/// `RangeError` at the same depth as before. Inherited, the
/// ceiling would be the process `RLIMIT_STACK` on the main thread (commonly
/// 8 MiB, whatever the machine says) and 1 MiB on Windows.
///
/// In a release build the reservation reaches deeper than V8 itself
/// compiles, so a module nested past V8's depth fails with the same
/// `RangeError` the JS would; a debug build's far larger frames stop the
/// strip near V8's depth instead. The main thread's 8 MiB would stop at a
/// quarter of the reservation's depth. The deepest real code nests under a
/// hundred levels; the exposure is generated and minified code.
/// The reservation is address space, committed only as it's touched, so a run
/// pays only for the stack it uses.
// TODO: past that depth the strip still overflows, since `tsv_ts` has no
// depth guard of its own; one that returns a parse error closes it
const STACK_SIZE: usize = 32 * 1024 * 1024;

/// gro on its own JS runtime (experimental)
#[derive(FromArgs)]
struct Args {
    /// print the version, the V8 version, and whether a startup snapshot is
    /// embedded
    #[argh(switch)]
    version: bool,
    #[argh(subcommand)]
    command: Option<Command>,
}

/// The `--version` line: `fgro <version> (V8 <version>, snapshot)`, or
/// `no snapshot` for a build without one.
fn version_line() -> String {
    format!(
        "fgro {} (V8 {}, {})",
        env!("CARGO_PKG_VERSION"),
        gro_runtime::v8_version(),
        if STARTUP_SNAPSHOT.is_some() {
            "snapshot"
        } else {
            "no snapshot"
        }
    )
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Run(RunArgs),
}

/// run an ES module (.js, .mjs, .ts, or .mts) from disk
#[derive(FromArgs)]
#[argh(subcommand, name = "run")]
struct RunArgs {
    /// the module to run: a path relative to the working directory, or absolute
    #[argh(positional)]
    file: PathBuf,
}

fn main() -> ExitCode {
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
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    ExitCode::from(run_on_sized_stack(&args))
}

/// Runs [`run`] on a thread named `fgro` with [`STACK_SIZE`] reserved, so the
/// runtime, the module loads, and the strip all share the one stated stack.
///
/// A refused spawn runs on this thread instead, and says so, since a deep
/// module would then overflow at the inherited ceiling with nothing saying
/// why. That rescues only the commands that don't start V8 (`--version`,
/// `--help`, a usage error): V8 needs threads of its own, so a process that
/// can't start this one can't start the runtime either.
fn run_on_sized_stack(args: &[&str]) -> u8 {
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("fgro".to_owned())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || run(args));
        match spawned {
            // a panic has printed by the time it's joined
            Ok(handle) => handle.join().unwrap_or(EXIT_PANIC),
            Err(error) => {
                eprintln!(
                    "warning: could not start the fgro thread ({error}); running on the \
                     inherited stack"
                );
                run(args)
            }
        }
    })
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
    let options = RuntimeOptions {
        snapshot: STARTUP_SNAPSHOT,
        ..RuntimeOptions::default()
    };
    match Args::from_args(&["fgro"], args) {
        Ok(Args {
            version: true,
            command: None,
        }) => {
            println!("{}", version_line());
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
        }) => match gro_runtime::run_main(&options, &file) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("error: {error}");
                exit_code(&error)
            }
        },
        // argh's `from_env` exits `1` on a usage error, so map its result by hand
        Err(EarlyExit {
            output,
            status: Ok(()),
        }) => {
            println!("{}", output.trim_end());
            0
        }
        Err(EarlyExit {
            output,
            status: Err(()),
        }) => {
            eprintln!("{}", output.trim_end());
            EXIT_USAGE
        }
    }
}

/// Writes `snapshot.bin` and `residual.rs` into `dir`, creating it first so an
/// unwritable directory fails before the snapshot is built.
fn make_snapshot(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    // the xtask's stamp decides when to rerun
    gro_runtime::snapshot::make_snapshot()?.write_to(dir)?;
    Ok(())
}

const fn exit_code(error: &RunError) -> u8 {
    match error {
        RunError::Entry(error) if error.kind.is_specifier_error() => EXIT_USAGE,
        _ => EXIT_FAILURE,
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
    }
}
