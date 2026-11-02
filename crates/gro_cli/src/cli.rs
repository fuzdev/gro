//! The command line's shape: the argh types for `fgro`'s flags and
//! subcommands, the internal `make-snapshot` name argh never sees, and
//! [`parse`], which reads them once into an [`Invocation`].

use std::path::PathBuf;

use argh::{EarlyExit, FromArgs};

use crate::error::CliError;

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

/// What the arguments ask for, parsed once.
pub enum Invocation {
    /// `fgro eval`, which confines its process and so runs on the main
    /// thread.
    Eval(EvalArgs),
    /// Every other command, run on a thread with a stated stack.
    Sized(Task),
}

/// A command that runs on a thread with a stated stack.
pub enum Task {
    /// `--version`.
    Version,
    /// `--help`, of `fgro` or a subcommand: argh's text, for stdout.
    Help(String),
    /// `make-snapshot <dir>`.
    MakeSnapshot(PathBuf),
    /// `run <file>`.
    Run(PathBuf),
}

/// Parses `args` (the program name excluded): `make-snapshot` by hand, so
/// it stays out of `--help`, and everything else by argh, whose early exits
/// are mapped by hand, since its `from_env` exits `1` on a usage error.
/// `eval` is decided from what argh parses, not the first argument, so every
/// spelling it accepts (`fgro -- eval`) runs on the main thread.
///
/// # Errors
///
/// Returns the usage error the arguments make.
pub fn parse(args: &[&str]) -> Result<Invocation, CliError> {
    if args.first() == Some(&MAKE_SNAPSHOT) {
        // no flags, so `--help` can't become a directory's name
        let [_, dir] = args else {
            return Err(CliError::SnapshotUsage);
        };
        if dir.starts_with('-') {
            return Err(CliError::SnapshotFlag);
        }
        return Ok(Invocation::Sized(Task::MakeSnapshot(PathBuf::from(dir))));
    }
    match Args::from_args(&["fgro"], args) {
        Ok(Args {
            version: true,
            command: None,
        }) => Ok(Invocation::Sized(Task::Version)),
        Ok(Args {
            version: true,
            command: Some(_),
        }) => Err(CliError::VersionWithCommand),
        Ok(Args {
            version: false,
            command: None,
        }) => Err(CliError::NoCommand),
        Ok(Args {
            version: false,
            command: Some(Command::Run(RunArgs { file })),
        }) => Ok(Invocation::Sized(Task::Run(file))),
        Ok(Args {
            version: false,
            command: Some(Command::Eval(args)),
        }) => Ok(Invocation::Eval(args)),
        Err(EarlyExit {
            output,
            status: Ok(()),
        }) => Ok(Invocation::Sized(Task::Help(output))),
        Err(EarlyExit {
            output,
            status: Err(()),
        }) => Err(CliError::Parse(output)),
    }
}
