//! `fgro`: gro on its own JS runtime, the binary's name while it's developed
//! beside the Node `gro`.
//!
//! `fgro run <file>` runs an ES module from disk (a path relative to the
//! working directory, or absolute) until its event loop drains. Errors print to
//! stderr, prefixed `error:`; an uncaught error's report carries its stack with
//! source positions (`file:///path:line:column`).
//!
//! `fgro --version` prints `fgro <version> (V8 <version>, snapshot)`, where
//! the last part says whether a startup snapshot is embedded (`no snapshot`
//! otherwise).
//!
//! Exit codes:
//!
//! - `0`: the program ran to completion, `--help`, or `--version`
//! - `1`: the program threw, rejected a promise no one handled, or failed to
//!   load a module, or the runtime failed to start
//! - `2`: a usage error, so the command line must change: an unknown
//!   subcommand or flag, a missing argument (including no arguments at all),
//!   an argument that isn't UTF-8, or an entry file that doesn't resolve
//!   (missing, or not a `.js` or `.mjs` file)
//!
//! Built with the `snapshot` feature, `fgro` starts from a startup snapshot
//! embedded at compile time, and runs off its build tree. `cargo xtask build`
//! makes that build in two stages: a plain `fgro` writes the snapshot with
//! `fgro make-snapshot <dir>` (internal to the xtask, so not in `--help`),
//! then `fgro` is rebuilt with the feature, reading the directory from
//! `GRO_SNAPSHOT_DIR`. Without the feature, `fgro` evaluates the runtime's
//! sources at startup, reading the web extensions' JS from the build tree.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use argh::{EarlyExit, FromArgs};
use gro_runtime::snapshot::{ResidualSource, StartupSnapshot};
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

/// run an ES module (.js or .mjs) from disk
#[derive(FromArgs)]
#[argh(subcommand, name = "run")]
struct RunArgs {
    /// the module to run, relative to the working directory
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
    ExitCode::from(run(&args))
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
    let in_dir = |path: &Path| {
        let path = path.to_path_buf();
        move |error: io::Error| io::Error::new(error.kind(), format!("{}: {error}", path.display()))
    };
    std::fs::create_dir_all(dir).map_err(in_dir(dir))?;
    let snapshot = gro_runtime::snapshot::make_snapshot()?;
    let residual_rs = format!(
        "// generated by `fgro {MAKE_SNAPSHOT}`: the lazily loaded extension sources the\n\
         // snapshot doesn't carry, sorted by specifier and embedded from the build machine\n\
         pub static RESIDUAL_LAZY_JS: &[(&str, &str)] = &[\n{}];\n\
         pub static RESIDUAL_LAZY_ESM: &[(&str, &str)] = &[\n{}];\n",
        residual_entries(&snapshot.residual_lazy_js)?,
        residual_entries(&snapshot.residual_lazy_esm)?,
    );
    // always written: snapshots differ from run to run, and the xtask's stamp decides when to rerun
    for (name, content) in [
        ("snapshot.bin", &snapshot.bytes[..]),
        ("residual.rs", residual_rs.as_bytes()),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, content).map_err(in_dir(&path))?;
    }
    Ok(())
}

/// The table's entries as Rust source, one `(specifier, include_str!(path))`
/// a line.
fn residual_entries(sources: &[ResidualSource]) -> io::Result<String> {
    let mut entries = String::new();
    for source in sources {
        let path = source.path.to_str().ok_or_else(|| {
            io::Error::other(format!("{} is not a UTF-8 path", source.path.display()))
        })?;
        // `{:?}` quotes and escapes both as Rust string literals
        let _ = writeln!(
            entries,
            "    ({:?}, include_str!({path:?})),",
            source.specifier
        );
    }
    Ok(entries)
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
