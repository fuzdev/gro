//! The command line's shape: the argh types for `fgro`'s flags and
//! subcommands, `run`'s arguments, which argh routes but never parses, the
//! internal `make-snapshot` name argh never sees, and [`parse`], which reads
//! them once into an [`Invocation`].

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use argh::{CommandInfo, EarlyExit, FromArgs, SubCommand};
use gro_runtime::confine::Jit;
use gro_runtime::{EnvNameFault, env_name_fault};

use crate::error::CliError;
use crate::print::{echo_argument, escape_controls};

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

/// The option that grants the env host module the names it lists.
pub const ALLOW_ENV: &str = "--allow-env";

/// `--allow-env`'s spelling with its names, which follow it.
const ALLOW_ENV_NAMES: &str = "--allow-env=";

/// The option that runs unconfined, every grant at its widest.
pub const ALLOW_ALL: &str = "-A";

/// `fgro run --help`, laid out as argh lays out the other commands' help.
fn run_help() -> String {
    [
        &format!("Usage: fgro {RUN} [{ALLOW_ENV}=<names>] [{ALLOW_ALL}] [--] <file> [<args...>]"),
        "",
        RUN_DESCRIPTION,
        "",
        "Positional Arguments:",
        "  file              the module to run: a path relative to the working directory,",
        "                    or absolute",
        "  args              the script's arguments, every one after the file as written,",
        "                    flags and `--` included, read from `@fuzdev/gro/args.ts`",
        "",
        "Options:",
        "  --allow-env=<names>",
        "                    grant the project's modules `@fuzdev/gro/env.ts` with these",
        "                    environment variables, a comma-separated list of exact names;",
        "                    repeatable",
        "  -A                run unconfined: every grant at its widest, to every module,",
        "                    every environment variable among them",
        "  --help, help      display usage information",
    ]
    .join("\n")
}

/// The authority a run's flags give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunGrant {
    /// The project's modules granted `args` and `exit`, and `env` for the
    /// names `--allow-env=` listed, when it listed any.
    Named {
        /// The environment variables granted, each an exact name.
        env: BTreeSet<String>,
    },
    /// `-A`: unconfined, every grant at its widest, the one option that
    /// grants beyond what it names.
    All,
}

/// `fgro run`'s arguments, checked.
pub struct RunInvocation {
    /// The module to run: relative to the working directory, or absolute.
    pub file: PathBuf,
    /// The script's arguments, every one after the file, as written.
    pub args: Vec<String>,
    /// What the flags grant.
    pub grant: RunGrant,
}

impl RunInvocation {
    /// What `args` ask for. fgro's own options are read up to the first
    /// argument that doesn't start with `-`, or the one after a bare `--`:
    /// that is the file, and every later argument is the script's, as
    /// written. The options are the help trigger (`--help`, or `help` as
    /// argh spells it for every command), which asks for the help whatever
    /// follows it; `--allow-env=<names>`, repeatable, its names one set;
    /// and `-A`, which grants everything, so names given beside it are
    /// checked and add nothing. A file starting with `-` comes after `--`.
    ///
    /// # Errors
    ///
    /// Returns the usage error, checked in the order the arguments come: an
    /// option before the file that isn't one of these (a bare `--allow-env`
    /// among them, which grants no name), a name `--allow-env=` lists that
    /// no variable can have ([`check_env_flag_names`]), or no file.
    fn check(args: RunArgs) -> Result<Task, CliError> {
        let mut help = false;
        let mut all = false;
        let mut env = BTreeSet::new();
        let mut args = args.args.into_iter();
        let file = loop {
            match args.next() {
                Some(arg) if HELP_TRIGGERS.contains(&arg.as_str()) => help = true,
                Some(arg) if arg == "--" => break args.next(),
                Some(arg) if arg == ALLOW_ALL => all = true,
                Some(arg) if arg == ALLOW_ENV => return Err(CliError::AllowEnvBare),
                Some(arg) if arg.starts_with(ALLOW_ENV_NAMES) => {
                    env.extend(check_env_flag_names(&arg[ALLOW_ENV_NAMES.len()..])?);
                }
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
                grant: if all {
                    RunGrant::All
                } else {
                    RunGrant::Named { env }
                },
            })),
        }
    }
}

/// The names `--allow-env=` lists in `names`, split at each comma, each
/// held to the rule the env host module's `narrow` holds a name to: not
/// empty, no NUL byte, no `=`, and no `*`, every name being exact.
///
/// # Errors
///
/// Returns [`CliError::AllowEnvName`] for the first name that breaks it.
fn check_env_flag_names(names: &str) -> Result<Vec<String>, CliError> {
    names
        .split(',')
        .map(|name| {
            env_name_fault(name, false).map_or_else(
                || Ok(name.to_owned()),
                |fault| {
                    Err(CliError::AllowEnvName(match fault {
                        EnvNameFault::Glob => EnvFlagFault::Glob(name.to_owned()),
                        EnvNameFault::Empty => EnvFlagFault::Empty,
                        EnvNameFault::Nul => EnvFlagFault::Nul,
                        EnvNameFault::Equals => EnvFlagFault::Equals,
                    }))
                },
            )
        })
        .collect()
}

/// Why a name `--allow-env=` lists can't be granted. Only a glob's name is
/// kept, echoed with its control and bidirectional formatting characters
/// escaped: a name holding a `=` is never echoed, since what follows it can
/// be a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvFlagFault {
    /// An empty name: a list that's empty, or starts, ends, or doubles a
    /// comma.
    Empty,
    /// A name holding a NUL byte.
    Nul,
    /// A name holding a `=`.
    Equals,
    /// A name holding a `*`, which names no glob.
    Glob(String),
}

impl std::fmt::Display for EnvFlagFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str(
                "an --allow-env name is empty: --allow-env=A,B lists names separated by single \
                 commas",
            ),
            Self::Nul => f.write_str("an --allow-env name holds a NUL byte"),
            Self::Equals => {
                f.write_str("an --allow-env name holds a `=`, which no variable's name holds")
            }
            Self::Glob(name) => write!(
                f,
                "an --allow-env name is exact, never a glob: {}",
                escape_controls(name)
            ),
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
/// are mapped by hand, since its `from_env` exits `1` on a usage error. Of
/// argh's text only its help is printed, made of the commands' own names
/// and docs; a refusal is [`invalid_arguments`], made of the arguments.
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
        // argh's text for a refusal can carry any part of an argument, so
        // the error is made of the arguments themselves
        Err(EarlyExit {
            status: Err(()), ..
        }) => Err(invalid_arguments(args)),
    }
}

/// The most arguments [`invalid_arguments`] shows; past these it counts
/// the rest.
pub const MAX_SHOWN_ARGUMENTS: usize = 16;

/// The most characters [`invalid_arguments`] shows of one argument, as
/// [`echo_argument`] prints it; past these it cuts it with `…`.
pub const MAX_SHOWN_ARGUMENT: usize = 80;

/// The usage error for arguments argh refused, made of `args` alone, never
/// of argh's text, which can carry any part of an argument: the command
/// they were given to, `fgro eval` when the first argument that isn't an
/// option is `eval`, else `fgro`, and the first [`MAX_SHOWN_ARGUMENTS`]
/// arguments as given, each as [`echo_argument`] prints it (up to its
/// first `=`, its control and bidirectional formatting characters escaped),
/// cut past [`MAX_SHOWN_ARGUMENT`] characters, an empty one as `""`, then
/// how many more there were.
fn invalid_arguments(args: &[&str]) -> CliError {
    let command = if args.iter().find(|arg| !arg.starts_with('-')) == Some(&"eval") {
        "fgro eval"
    } else {
        "fgro"
    };
    let mut shown: Vec<String> = args
        .iter()
        .take(MAX_SHOWN_ARGUMENTS)
        .map(|arg| {
            if arg.is_empty() {
                return "\"\"".to_owned();
            }
            let echoed = echo_argument(arg);
            if echoed.chars().count() > MAX_SHOWN_ARGUMENT {
                let cut: String = echoed.chars().take(MAX_SHOWN_ARGUMENT).collect();
                format!("{cut}…")
            } else {
                echoed
            }
        })
        .collect();
    if args.len() > MAX_SHOWN_ARGUMENTS {
        shown.push(format!("… ({} more)", args.len() - MAX_SHOWN_ARGUMENTS));
    }
    CliError::InvalidArguments {
        command,
        arguments: shown.join(" "),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{EnvFlagFault, Invocation, RUN_DESCRIPTION, RunGrant, Task, parse, run_help};
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
        /// [`CliError::AllowEnvBare`].
        AllowEnvBare,
        /// [`CliError::AllowEnvName`].
        AllowEnvName(EnvFlagFault),
    }

    fn run_outcome(args: &[&str]) -> Run {
        match parse(args) {
            Ok(Invocation::Sized(Task::Run(run))) => {
                Run::Runs(run.file.to_str().unwrap().to_owned(), run.args)
            }
            Ok(Invocation::Sized(Task::Help(help))) if help == run_help() => Run::Help,
            Err(CliError::RunNoFile) => Run::NoFile,
            Err(CliError::RunOption(option)) => Run::Option(option),
            Err(CliError::AllowEnvBare) => Run::AllowEnvBare,
            Err(CliError::AllowEnvName(fault)) => Run::AllowEnvName(fault),
            _ => panic!("{args:?} is not one of run's outcomes"),
        }
    }

    /// What `fgro run` with `args` (after `run`) grants.
    fn grant(args: &[&str]) -> RunGrant {
        let args = [&["run"][..], args].concat();
        match parse(&args) {
            Ok(Invocation::Sized(Task::Run(run))) => run.grant,
            _ => panic!("{args:?} runs a file"),
        }
    }

    fn env(names: &[&str]) -> RunGrant {
        RunGrant::Named {
            env: names.iter().map(|&name| name.to_owned()).collect(),
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
        use Run::{AllowEnvBare, AllowEnvName, Help, NoFile};
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
            // the grant options before the file, the help trigger beside them
            (&["run", "-A", "a.js"], runs("a.js", &[])),
            (
                &["run", "--allow-env=A", "a.js", "-A"],
                runs("a.js", &["-A"]),
            ),
            (
                &["run", "--allow-env=A", "-A", "--", "-a.js", "--allow-env="],
                runs("-a.js", &["--allow-env="]),
            ),
            (&["run", "-A", "--help", "a.js"], Help),
            (&["run", "--allow-env=A", "help"], Help),
            // an option before the file, named as written
            (&["run", "--x", "a.js"], option("--x")),
            (&["run", "-AB", "a.js"], option("-AB")),
            (&["run", "-a", "a.js"], option("-a")),
            (&["run", "--allow-all", "a.js"], option("--allow-all")),
            (&["run", "--allow-envA", "a.js"], option("--allow-envA")),
            (&["run", "--allow-read=.", "a.js"], option("--allow-read=.")),
            (&["run", "-"], option("-")),
            (&["run", "--help", "--x"], option("--x")),
            (&["run", "--x", "--help"], option("--x")),
            // `--allow-env` with no names, and a name no variable can have
            (&["run", "--allow-env", "a.js"], AllowEnvBare),
            (&["run", "--allow-env", "A", "a.js"], AllowEnvBare),
            (&["run", "--help", "--allow-env"], AllowEnvBare),
            (&["run", "-A", "--allow-env", "a.js"], AllowEnvBare),
            (
                &["run", "--allow-env=", "a.js"],
                AllowEnvName(EnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-env=A,", "a.js"],
                AllowEnvName(EnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-env=,A", "a.js"],
                AllowEnvName(EnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-env=A,,B", "a.js"],
                AllowEnvName(EnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-env=A=B", "a.js"],
                AllowEnvName(EnvFlagFault::Equals),
            ),
            (
                &["run", "--allow-env==", "a.js"],
                AllowEnvName(EnvFlagFault::Equals),
            ),
            (
                &["run", "--allow-env=A\0", "a.js"],
                AllowEnvName(EnvFlagFault::Nul),
            ),
            (
                &["run", "--allow-env=A,B*", "a.js"],
                AllowEnvName(EnvFlagFault::Glob("B*".to_owned())),
            ),
            (
                &["run", "-A", "--allow-env=*", "a.js"],
                AllowEnvName(EnvFlagFault::Glob("*".to_owned())),
            ),
            // the first fault in the order the arguments come
            (
                &["run", "--allow-env=A=1", "--x", "a.js"],
                AllowEnvName(EnvFlagFault::Equals),
            ),
            (&["run", "--x", "--allow-env=", "a.js"], option("--x")),
            (&["run", "--allow-env=A"], NoFile),
        ];
        for (args, expected) in cases {
            assert_eq!(run_outcome(args), expected, "{args:?}");
        }
    }

    /// With neither grant option `run` grants the env host module no name;
    /// `--allow-env=`, repeatable, grants the names it lists, one set,
    /// as written, case and all; `-A`, wherever it comes before the file,
    /// grants everything, and the names beside it add nothing.
    #[test]
    fn run_s_grant_options_name_what_they_grant() {
        assert_eq!(grant(&["a.js"]), env(&[]));
        assert_eq!(grant(&["a.js", "-A", "--allow-env=A"]), env(&[]));
        assert_eq!(grant(&["--allow-env=A", "a.js"]), env(&["A"]));
        assert_eq!(
            grant(&[
                "--allow-env=b,A,a",
                "--allow-env=A",
                "--allow-env=Ä, x",
                "a.js"
            ]),
            RunGrant::Named {
                env: BTreeSet::from(["A", "a", "b", "Ä", " x"].map(ToOwned::to_owned)),
            }
        );
        for args in [
            &["-A", "a.js"][..],
            &["--allow-env=A", "-A", "a.js"],
            &["-A", "--allow-env=A", "a.js"],
            &["-A", "-A", "--", "a.js"],
        ] {
            assert_eq!(grant(args), RunGrant::All, "{args:?}");
        }
    }

    /// The usage errors of the grant options say what was wrong without
    /// echoing what follows a `=`, which can be a value, and point at the
    /// help; exit 2, as every usage error.
    #[test]
    fn the_grant_options_usage_errors_echo_no_value() {
        let message = |args: &[&str]| match parse(&[&["run"][..], args].concat()) {
            Err(error) => {
                assert_eq!(error.exit_code(), crate::EXIT_USAGE, "{args:?}");
                error.to_string()
            }
            Ok(_) => panic!("{args:?} is a usage error"),
        };
        let help = "\nRun fgro run --help for more information.";
        assert_eq!(
            message(&["--allow-env", "a.js"]),
            format!("--allow-env takes the names it grants, as --allow-env=A,B{help}")
        );
        assert_eq!(
            message(&["--allow-env=TOKEN=hunter2", "a.js"]),
            format!("an --allow-env name holds a `=`, which no variable's name holds{help}")
        );
        assert_eq!(
            message(&["--allow-env=A,", "a.js"]),
            format!(
                "an --allow-env name is empty: --allow-env=A,B lists names separated by single \
                 commas{help}"
            )
        );
        assert_eq!(
            message(&["--allow-env=AWS_*", "a.js"]),
            format!("an --allow-env name is exact, never a glob: AWS_*{help}")
        );
        assert_eq!(
            message(&["--allow-env=\u{1b}[2J\u{202e}*", "a.js"]),
            format!("an --allow-env name is exact, never a glob: \\u{{1b}}[2J\\u{{202e}}*{help}")
        );
        assert_eq!(
            message(&["--allow-env=A\0", "a.js"]),
            format!("an --allow-env name holds a NUL byte{help}")
        );
    }

    /// `fgro run --help`, written by hand, reads as argh writes the other
    /// commands' help, and `fgro --help` lists `run` first, as described
    /// there.
    #[test]
    fn run_s_help_reads_as_argh_s() {
        const EXPECTED: &str = "\
Usage: fgro run [--allow-env=<names>] [-A] [--] <file> [<args...>]

run an ES module (.js, .mjs, .ts, or .mts) from disk

Positional Arguments:
  file              the module to run: a path relative to the working directory,
                    or absolute
  args              the script's arguments, every one after the file as written,
                    flags and `--` included, read from `@fuzdev/gro/args.ts`

Options:
  --allow-env=<names>
                    grant the project's modules `@fuzdev/gro/env.ts` with these
                    environment variables, a comma-separated list of exact names;
                    repeatable
  -A                run unconfined: every grant at its widest, to every module,
                    every environment variable among them
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

    /// A usage error argh refused prints no part of an argument past its
    /// first `=` and no raw control or bidirectional formatting character,
    /// whatever the arguments: lists of every size up to thousands, of
    /// arguments empty to very long, of controls, line breaks, bidi marks,
    /// and non-ASCII, each value after a first `=` made of characters
    /// nothing else holds, so one shown anywhere is a value shown. Its one
    /// line break is gro's own, between its two lines, and its length is
    /// bounded.
    #[test]
    fn a_refused_argument_list_prints_no_value_and_no_raw_control() {
        use super::{MAX_SHOWN_ARGUMENT, MAX_SHOWN_ARGUMENTS, invalid_arguments};
        const PLAIN: &[char] = &[
            'a',
            'Z',
            '-',
            ' ',
            '\u{e9}',
            '\u{1f600}',
            '\u{1b}',
            '\n',
            '\r',
            '\t',
            '\0',
            '\u{7f}',
            '\u{9b}',
            '\u{202e}',
            '\u{2066}',
            '\u{200e}',
            '"',
            '\u{2026}',
            ':',
            '[',
        ];
        // what follows a first `=`: any of PLAIN, another `=`, and three
        // characters only a value holds
        const MARKS: &[char] = &['\u{5024}', '\u{79d8}', '\u{5bc6}'];
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |bound: usize| -> usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % u64::try_from(bound).unwrap()).unwrap()
        };
        let text = |len: usize, value: bool, next: &mut dyn FnMut(usize) -> usize| -> String {
            (0..len)
                .map(|_| {
                    if value && next(3) == 0 {
                        MARKS[next(MARKS.len())]
                    } else if value && next(8) == 0 {
                        '='
                    } else {
                        PLAIN[next(PLAIN.len())]
                    }
                })
                .collect()
        };
        for round in 0..300 {
            let count = [0, 1, 2, 5, 16, 17, 40, 3000][round % 8];
            let args: Vec<String> = (0..count)
                .map(|_| {
                    let len = [0, 1, 3, 40, 200, 5000][next(6)];
                    let head = text(len, false, &mut next);
                    if next(2) == 0 {
                        let value_len = [0, 1, 20, 3000][next(4)];
                        // at least one mark, so the value is never all plain
                        format!("{head}={}{}", MARKS[0], text(value_len, true, &mut next))
                    } else {
                        head
                    }
                })
                .collect();
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let message = invalid_arguments(&args).to_string();
            assert!(
                !message.contains(MARKS),
                "{round}: a value was printed: {message:?}"
            );
            assert_eq!(message.matches('\n').count(), 1, "{round}: {message:?}");
            assert!(
                !message
                    .chars()
                    .any(|c| c != '\n' && crate::print::is_escaped(c)),
                "{round}: a raw control: {message:?}"
            );
            let bound = MAX_SHOWN_ARGUMENTS * (MAX_SHOWN_ARGUMENT + 2) + 200;
            assert!(
                message.chars().count() <= bound,
                "{round}: {}",
                message.len()
            );
        }
    }

    /// argh's refusals print gro's own line naming the command, then the
    /// arguments as [`echo_argument`] prints them, and the help pointer:
    /// an argument crafted to look like argh's own words, to run into the
    /// next argument, to be part of another, or to forge a line changes
    /// nothing of that.
    #[test]
    fn a_refusal_by_argh_is_made_of_the_arguments() {
        let message = |args: &[&str]| match parse(args) {
            Err(error @ CliError::InvalidArguments { .. }) => error.to_string(),
            _ => panic!("{args:?} is refused by argh"),
        };
        assert_eq!(
            message(&["eval", "--export", ": T=", "a.js", "T=hunter2"]),
            "invalid arguments to fgro eval: eval --export : T=\u{2026} a.js T=\u{2026}\n\
             Run fgro eval --help for more information."
        );
        assert_eq!(
            message(&[
                "eval",
                "--export",
                "argument: a=b",
                "a.js",
                "a=b\nerror: fake second line"
            ]),
            "invalid arguments to fgro eval: eval --export argument: a=\u{2026} a.js a=\u{2026}\n\
             Run fgro eval --help for more information."
        );
        assert_eq!(
            message(&["eval", "--export", "=", "a.js", "TOKEN=hunter2"]),
            "invalid arguments to fgro eval: eval --export =\u{2026} a.js TOKEN=\u{2026}\n\
             Run fgro eval --help for more information."
        );
        assert_eq!(
            message(&["eval", "--export", "x=", "a.js", "x=\u{1b}[2Jsecret"]),
            "invalid arguments to fgro eval: eval --export x=\u{2026} a.js x=\u{2026}\n\
             Run fgro eval --help for more information."
        );
        assert_eq!(
            message(&["--unknown", "-\u{1b}[2J", ""]),
            "invalid arguments to fgro: --unknown -\\u{1b}[2J \"\"\n\
             Run fgro --help for more information."
        );
        let many: Vec<String> = (0..20).map(|i| format!("--x{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(
            message(&many).contains(" --x15 \u{2026} (4 more)\n"),
            "{}",
            message(&many)
        );
        let long = format!("--{}", "y".repeat(200));
        assert!(message(&[&long]).contains(&format!("--{}\u{2026}\n", "y".repeat(78))));
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
