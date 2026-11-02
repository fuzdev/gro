//! The command line's shape: the argh types for `fgro`'s flags and
//! subcommands, the internal `make-snapshot` name argh never sees, and
//! [`parse`], which reads them once into an [`Invocation`].

use std::path::PathBuf;
use std::time::Duration;

use argh::{EarlyExit, FromArgs};
use gro_runtime::confine::Jit;

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

/// `fgro eval`'s arguments, checked.
pub struct EvalInvocation {
    /// The module to evaluate: relative to the working directory, or
    /// absolute.
    pub module: PathBuf,
    /// Where the value comes from.
    pub value: EvalValue,
    /// Whether V8's JIT is on.
    pub jit: Jit,
    /// How long the eval may run, at least a millisecond.
    pub timeout: Duration,
    /// Whether the envelope carries the module graph.
    pub graph: bool,
}

/// Where an eval's value comes from.
pub enum EvalValue {
    /// A driver read from stdin (`--driver -`), handed the namespace.
    StdinDriver,
    /// The export named, `default` when unset.
    Export(Option<String>),
}

impl EvalInvocation {
    /// The checked form of `args`.
    ///
    /// # Errors
    ///
    /// Returns the usage error: a `--timeout` of 0, `--export` with a
    /// driver, or a `--driver` other than [`DRIVER_STDIN`], checked in that
    /// order.
    fn check(args: EvalArgs) -> Result<Self, CliError> {
        let EvalArgs {
            module,
            driver,
            export,
            jit,
            timeout,
            graph,
        } = args;
        let timeout = match timeout {
            None => gro_runtime::eval::DEFAULT_TIMEOUT,
            Some(0) => return Err(CliError::TimeoutZero),
            Some(ms) => Duration::from_millis(ms),
        };
        let value = match (driver, export) {
            (None, export) => EvalValue::Export(export),
            (Some(driver), None) if driver == DRIVER_STDIN => EvalValue::StdinDriver,
            (Some(driver), Some(_)) if driver == DRIVER_STDIN => {
                return Err(CliError::DriverWithExport);
            }
            (Some(path), _) => return Err(CliError::DriverPath(path)),
        };
        Ok(Self {
            module,
            value,
            jit: if jit { Jit::On } else { Jit::Off },
            timeout,
            graph,
        })
    }
}

/// What the arguments ask for, parsed once.
pub enum Invocation {
    /// `fgro eval`, which confines its process and so runs on the main
    /// thread.
    Eval(EvalInvocation),
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
/// Returns the usage error the arguments make, eval's checked flags
/// included ([`EvalInvocation::check`]).
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
        }) => EvalInvocation::check(args).map(Invocation::Eval),
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

#[cfg(test)]
mod tests {
    use super::{Invocation, Task, parse};

    /// `--timeout`'s help names the default eval takes.
    #[test]
    fn the_timeout_s_help_names_the_default() {
        let Ok(Invocation::Sized(Task::Help(help))) = parse(&["eval", "--help"]) else {
            panic!("eval --help prints its help");
        };
        let default = format!(
            "(default: {})",
            gro_runtime::eval::DEFAULT_TIMEOUT.as_millis()
        );
        assert!(help.contains(&default), "{help}");
    }
}
