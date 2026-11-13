//! The command line's shape: the argh types for `fgro`'s flags and
//! subcommands, `run`'s arguments, which argh routes but never parses, the
//! internal `make-snapshot` name argh never sees, and [`parse`], which reads
//! them once into an [`Invocation`].

use std::path::PathBuf;
use std::time::Duration;

use argh::{CommandInfo, EarlyExit, FromArgs, SubCommand};
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

/// `fgro run`'s arguments, every one after `run` as written, which argh
/// routes here without parsing them: an argument starting with `-` that
/// follows the file is the script's, where argh would read it as an option of
/// fgro's. [`RunInvocation::check`] parses them.
pub struct RunArgs {
    args: Vec<String>,
}

impl FromArgs for RunArgs {
    /// Takes `args` as they are, argh's help trigger included when `help` or
    /// `--help` came before `run` (`fgro help run`): argh puts `help` first.
    fn from_args(_command_name: &[&str], args: &[&str]) -> Result<Self, EarlyExit> {
        Ok(Self {
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
        })
    }
}

impl SubCommand for RunArgs {
    const COMMAND: &'static CommandInfo = &CommandInfo {
        name: RUN,
        short: &'\0',
        description: RUN_DESCRIPTION,
    };
}

/// `run`'s name.
const RUN: &str = "run";

/// `run`'s description, in `fgro --help`'s list of commands and at the top of
/// its own help.
const RUN_DESCRIPTION: &str = "run an ES module (.js, .mjs, .ts, or .mts) from disk";

/// The arguments before the file that ask for `run`'s help: argh's help
/// triggers, the same for every command.
const HELP_TRIGGERS: [&str; 2] = ["--help", "help"];

/// `fgro run --help`, laid out as argh lays out the other commands' help.
fn run_help() -> String {
    [
        &format!("Usage: fgro {RUN} [--] <file> [<args...>]"),
        "",
        RUN_DESCRIPTION,
        "",
        "Positional Arguments:",
        "  file              the module to run: a path relative to the working directory,",
        "                    or absolute",
        "  args              the script's arguments, every one after the file as written,",
        "                    flags and `--` included (the script can't read them yet)",
        "",
        "Options:",
        "  --help, help      display usage information",
    ]
    .join("\n")
}

/// `fgro run`'s arguments, checked.
pub struct RunInvocation {
    /// The module to run: relative to the working directory, or absolute.
    pub file: PathBuf,
    /// The script's arguments, every one after the file, as written.
    pub args: Vec<String>,
}

impl RunInvocation {
    /// What `args` ask for. fgro's own options are read up to the first
    /// argument that doesn't start with `-`, or the one after a bare `--`:
    /// that is the file, and every later argument is the script's, as
    /// written. The only option is the help trigger (`--help`, or `help` as
    /// argh spells it for every command), which asks for the help whatever
    /// follows it. A file starting with `-` comes after `--`.
    ///
    /// # Errors
    ///
    /// Returns the usage error: an option before the file other than the
    /// help trigger, or no file, checked in the order the arguments come.
    fn check(args: RunArgs) -> Result<Task, CliError> {
        let mut help = false;
        let mut args = args.args.into_iter();
        let file = loop {
            match args.next() {
                Some(arg) if HELP_TRIGGERS.contains(&arg.as_str()) => help = true,
                Some(arg) if arg == "--" => break args.next(),
                Some(arg) if arg.starts_with('-') => return Err(CliError::RunOption(arg)),
                file => break file,
            }
        };
        match (help, file) {
            (true, _) => Ok(Task::Help(run_help())),
            (false, None) => Err(CliError::RunNoFile),
            (false, Some(file)) => Ok(Task::Run(Self {
                file: PathBuf::from(file),
                args: args.collect(),
            })),
        }
    }
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
    /// `run <file> [<args...>]`.
    Run(RunInvocation),
}

/// Parses `args` (the program name excluded): `make-snapshot` by hand, so
/// it stays out of `--help`, and everything else by argh, whose early exits
/// are mapped by hand, since its `from_env` exits `1` on a usage error.
/// argh routes `run`, and [`RunInvocation::check`] parses its arguments.
/// `eval` is decided from what argh parses, not the first argument, so every
/// spelling it accepts (`fgro -- eval`) runs on the main thread.
///
/// # Errors
///
/// Returns the usage error the arguments make, run's and eval's checked
/// arguments included ([`RunInvocation::check`], [`EvalInvocation::check`]).
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
            command: Some(Command::Run(args)),
        }) => RunInvocation::check(args).map(Invocation::Sized),
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
    use super::{Invocation, RUN_DESCRIPTION, Task, parse, run_help};
    use crate::error::CliError;

    /// What `parse` makes of `fgro run`'s arguments, in a form to compare.
    #[derive(Debug, PartialEq)]
    enum Run {
        /// The file, then the script's arguments.
        Runs(String, Vec<String>),
        /// `run`'s help.
        Help,
        /// [`CliError::RunNoFile`].
        NoFile,
        /// [`CliError::RunOption`], naming the option.
        Option(String),
    }

    fn run_outcome(args: &[&str]) -> Run {
        match parse(args) {
            Ok(Invocation::Sized(Task::Run(run))) => {
                Run::Runs(run.file.to_str().unwrap().to_owned(), run.args)
            }
            Ok(Invocation::Sized(Task::Help(help))) if help == run_help() => Run::Help,
            Err(CliError::RunNoFile) => Run::NoFile,
            Err(CliError::RunOption(option)) => Run::Option(option),
            _ => panic!("{args:?} is not one of run's outcomes"),
        }
    }

    fn runs(file: &str, args: &[&str]) -> Run {
        Run::Runs(
            file.to_owned(),
            args.iter().map(|&arg| arg.to_owned()).collect(),
        )
    }

    fn option(option: &str) -> Run {
        Run::Option(option.to_owned())
    }

    /// `run` reads fgro's options up to the file, the first argument not
    /// starting with `-` or the one after `--`, and passes every later
    /// argument to the script as written.
    #[test]
    fn run_parses_its_options_up_to_the_file() {
        use Run::{Help, NoFile};
        let cases: Vec<(&[&str], Run)> = vec![
            // the file alone, and the file with the script's arguments
            (&["run", "a.js"], runs("a.js", &[])),
            (&["run", "a.js", "b.js", "c"], runs("a.js", &["b.js", "c"])),
            // arguments that look like fgro's options are the script's
            (
                &[
                    "run",
                    "a.js",
                    "--help",
                    "--x",
                    "-A",
                    "--allow-env=A",
                    "help",
                ],
                runs("a.js", &["--help", "--x", "-A", "--allow-env=A", "help"]),
            ),
            // `--` before the file ends fgro's options, after it is the script's
            (&["run", "--", "-a.js"], runs("-a.js", &[])),
            (&["run", "--", "--"], runs("--", &[])),
            (&["run", "--", "help"], runs("help", &[])),
            (
                &["run", "--", "a.js", "--", "x"],
                runs("a.js", &["--", "x"]),
            ),
            (&["run", "a.js", "--"], runs("a.js", &["--"])),
            // an empty string is an argument like any other
            (&["run", "a.js", "", "x", ""], runs("a.js", &["", "x", ""])),
            (&["run", ""], runs("", &[])),
            // every spelling argh routes to `run`
            (&["--", "run", "a.js", "--x"], runs("a.js", &["--x"])),
            // the help trigger before the file, whatever follows it
            (&["run", "--help"], Help),
            (&["run", "help"], Help),
            (&["run", "--help", "a.js", "--x"], Help),
            (&["run", "--help", "--", "a.js"], Help),
            (&["run", "--help", "--"], Help),
            (&["help", "run"], Help),
            (&["--help", "run", "a.js"], Help),
            // no file
            (&["run"], NoFile),
            (&["run", "--"], NoFile),
            // an option before the file, named as written
            (&["run", "--x", "a.js"], option("--x")),
            (&["run", "-A", "a.js"], option("-A")),
            (&["run", "--allow-env=A", "a.js"], option("--allow-env=A")),
            (&["run", "-"], option("-")),
            (&["run", "--help", "--x"], option("--x")),
            (&["run", "--x", "--help"], option("--x")),
        ];
        for (args, expected) in cases {
            assert_eq!(run_outcome(args), expected, "{args:?}");
        }
    }

    /// `fgro run --help`, written by hand, reads as argh writes the other
    /// commands' help, and `fgro --help` lists `run` first, as described
    /// there.
    #[test]
    fn run_s_help_reads_as_argh_s() {
        const EXPECTED: &str = "\
Usage: fgro run [--] <file> [<args...>]

run an ES module (.js, .mjs, .ts, or .mts) from disk

Positional Arguments:
  file              the module to run: a path relative to the working directory,
                    or absolute
  args              the script's arguments, every one after the file as written,
                    flags and `--` included (the script can't read them yet)

Options:
  --help, help      display usage information";
        assert_eq!(run_help(), EXPECTED);
        let Ok(Invocation::Sized(Task::Help(eval))) = parse(&["eval", "--help"]) else {
            panic!("eval --help prints its help");
        };
        // argh's own lines for the help trigger and a positional's wrapping
        assert_eq!(eval.trim_end().lines().last(), run_help().lines().last());
        assert!(
            eval.contains("\nPositional Arguments:\n  module            the module to evaluate")
        );
        let Ok(Invocation::Sized(Task::Help(top))) = parse(&["--help"]) else {
            panic!("--help prints the help");
        };
        let run = top.find(&format!("\n  run               {RUN_DESCRIPTION}\n"));
        let eval = top.find("\n  eval              evaluate an ES module");
        assert!(run.is_some() && run < eval, "{top}");
    }

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
