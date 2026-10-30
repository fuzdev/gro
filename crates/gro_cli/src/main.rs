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
//! the main thread, so a debug build stays inside the 16 MiB of stack tsv's
//! TypeScript strip is sized for (a release build needs 2 MiB), whatever the
//! platform's main thread has. A module nested or chained past tsv's depth
//! limits fails to load (exit `1`) with a message naming the limit, rather
//! than overflowing the stack.
//!
//! `fgro --version` prints one line,
//! `fgro <version> (commit <sha>[-dirty], V8 <version>, snapshot)`: the git
//! commit it was built from, full, with `-dirty` when the working tree
//! differed from it (`commit unknown` when no build embedded one, as a plain
//! `cargo build` doesn't), and whether a startup snapshot is embedded
//! (`no snapshot` otherwise).
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
//! `GRO_SNAPSHOT_DIR`; that build also embeds the commit, from `GRO_COMMIT`.
//! Without the feature, `fgro` evaluates the runtime's sources at startup,
//! reading the web extensions' JS from the build tree.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use argh::{EarlyExit, FromArgs};
use gro_runtime::snapshot::StartupSnapshot;
use gro_runtime::{RunError, RuntimeOptions, STACK_SIZE};

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

/// gro on its own JS runtime (experimental)
#[derive(FromArgs)]
struct Args {
    /// print the version, the git commit it was built from, the V8 version,
    /// and whether a startup snapshot is embedded
    #[argh(switch)]
    version: bool,
    #[argh(subcommand)]
    command: Option<Command>,
}

/// The commit this binary was built from, read from `GRO_COMMIT` at compile
/// time, never at runtime: a full SHA, with `-dirty` appended when the working
/// tree differed from it. `cargo xtask build` sets it for its stage-two build,
/// and only that build's `snapshot` feature reads it, so a plain `cargo build`
/// has none whatever the shell sets; unset or empty, there is none either.
/// Cargo records the variable as an input, so a changed value rebuilds the
/// crate.
const COMMIT: Option<&str> = if cfg!(feature = "snapshot") {
    commit(option_env!("GRO_COMMIT"))
} else {
    None
};

/// What `--version` prints for a build without [`COMMIT`].
const COMMIT_UNKNOWN: &str = "unknown";

/// The suffix [`COMMIT`] carries when the working tree differed from the commit.
const DIRTY_SUFFIX: &str = "-dirty";

/// Validates a `GRO_COMMIT` value: `None` when it's unset or empty, else a
/// full SHA (40 or 64 lowercase hex digits), optionally followed by
/// [`DIRTY_SUFFIX`]. A malformed value panics, which, evaluated for
/// [`COMMIT`], fails the build rather than printing it.
const fn commit(value: Option<&'static str>) -> Option<&'static str> {
    let Some(value) = value else {
        return None;
    };
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let suffix = DIRTY_SUFFIX.as_bytes();
    let mut sha_len = bytes.len();
    if sha_len > suffix.len() {
        let start = sha_len - suffix.len();
        let mut i = 0;
        while i < suffix.len() && bytes[start + i] == suffix[i] {
            i += 1;
        }
        if i == suffix.len() {
            sha_len = start;
        }
    }
    assert!(
        sha_len == 40 || sha_len == 64,
        "GRO_COMMIT must be a full commit SHA, optionally followed by `-dirty`"
    );
    let mut i = 0;
    while i < sha_len {
        assert!(
            matches!(bytes[i], b'0'..=b'9' | b'a'..=b'f'),
            "GRO_COMMIT's SHA must be lowercase hex"
        );
        i += 1;
    }
    Some(value)
}

/// The `--version` line:
/// `fgro <version> (commit <sha>[-dirty], V8 <version>, snapshot)`, with
/// `commit unknown` for a build without [`COMMIT`] and `no snapshot` for a
/// build without one.
fn version_line() -> String {
    format!(
        "fgro {} (commit {}, V8 {}, {})",
        env!("CARGO_PKG_VERSION"),
        COMMIT.unwrap_or(COMMIT_UNKNOWN),
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
/// runtime, the module loads, and the strip all share the one stated stack:
/// tsv's, so a debug build keeps inside its 16 MiB contract (a release build
/// needs 2 MiB), and the main thread's platform stack (commonly 8 MiB on
/// Linux, 1 MiB on Windows) never decides whether a module within tsv's depth
/// limits strips.
///
/// Not `tsv_ts::run_with_stack`: that names its thread `tsv`, and resumes a
/// panic rather than mapping it to [`EXIT_PANIC`], and drops its work when the
/// spawn is refused.
///
/// A refused spawn runs on this thread instead, and says so, since a module
/// could then overflow the inherited stack with nothing saying why. That
/// rescues only the commands that don't start V8 (`--version`, `--help`, a
/// usage error): V8 needs threads of its own, so a process that can't start
/// this one can't start the runtime either.
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
    fn commit_accepts_a_full_sha_and_the_dirty_suffix() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(commit(None), None);
        assert_eq!(commit(Some("")), None);
        assert_eq!(commit(Some(sha)), Some(sha));
        let dirty = "0123456789abcdef0123456789abcdef01234567-dirty";
        assert_eq!(commit(Some(dirty)), Some(dirty));
        let sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(commit(Some(sha256)), Some(sha256));
    }

    #[test]
    fn commit_refuses_anything_else() {
        for value in [
            "unknown",
            "0123456",
            "0123456789abcdef0123456789abcdef01234567-",
            "0123456789abcdef0123456789abcdef01234567-clean",
            "0123456789ABCDEF0123456789ABCDEF01234567",
            "0123456789abcdef0123456789abcdef0123456g",
            "-dirty",
        ] {
            let result = std::panic::catch_unwind(|| commit(Some(value)));
            assert!(result.is_err(), "{value}");
        }
    }

    /// Only a `snapshot` build reads `GRO_COMMIT`, so the plain build
    /// `cargo test` runs has no commit, whatever the shell sets.
    #[test]
    fn version_line_names_the_commit() {
        let line = version_line();
        let expected = option_env!("GRO_COMMIT")
            .filter(|value| cfg!(feature = "snapshot") && !value.is_empty())
            .unwrap_or(COMMIT_UNKNOWN);
        assert!(
            line.starts_with(&format!(
                "fgro {} (commit {expected}, V8 ",
                env!("CARGO_PKG_VERSION")
            )),
            "{line}"
        );
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
