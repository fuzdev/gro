//! The command line's shape: the argh types for `fgro`'s flags and
//! subcommands, the internal `make-snapshot` name argh never sees, and the
//! exit code of argh's early exits.

use std::path::PathBuf;

use argh::{EarlyExit, FromArgs};

use crate::EXIT_USAGE;

/// The internal subcommand that writes the snapshot, handled before argh so
/// it stays out of `--help`.
pub const MAKE_SNAPSHOT: &str = "make-snapshot";

/// gro on its own JS runtime (experimental)
#[derive(FromArgs)]
pub struct Args {
    /// print the version, the git commit it was built from, the V8 version,
    /// and whether a startup snapshot is embedded
    #[argh(switch)]
    pub version: bool,
    #[argh(subcommand)]
    pub command: Option<Command>,
}

#[derive(FromArgs)]
#[argh(subcommand)]
pub enum Command {
    Run(RunArgs),
    Eval(EvalArgs),
}

/// run an ES module (.js, .mjs, .ts, or .mts) from disk
#[derive(FromArgs)]
#[argh(subcommand, name = "run")]
pub struct RunArgs {
    /// the module to run: a path relative to the working directory, or absolute
    #[argh(positional)]
    pub file: PathBuf,
}

/// evaluate an ES module to JSON data, written as one envelope on stdout
#[derive(FromArgs)]
#[argh(subcommand, name = "eval")]
pub struct EvalArgs {
    /// the module to evaluate: a path relative to the working directory, or absolute
    #[argh(positional)]
    pub module: PathBuf,
    /// read a driver module from stdin (pass `-`), whose default export is
    /// called with the module's namespace and returns the value
    #[argh(option)]
    pub driver: Option<String>,
    /// the export to serialize, with no driver (default: default)
    #[argh(option)]
    pub export: Option<String>,
    /// turn V8's JIT on, and with it WebAssembly
    #[argh(switch)]
    pub jit: bool,
    /// stop the eval after this many milliseconds, at least 1 (default: 10000)
    #[argh(option)]
    pub timeout: Option<u64>,
    /// add the module graph to the envelope, as `graph`
    #[argh(switch)]
    pub graph: bool,
}

/// The `--driver` value that reads the driver from stdin, the only one so far.
pub const DRIVER_STDIN: &str = "-";

/// The exit code of argh's early exit, `--help` or a usage error, printed:
/// argh's `from_env` exits `1` on a usage error, so its result is mapped by
/// hand.
pub fn early_exit(EarlyExit { output, status }: EarlyExit) -> u8 {
    if status.is_ok() {
        println!("{}", output.trim_end());
        0
    } else {
        eprintln!("{}", output.trim_end());
        EXIT_USAGE
    }
}
