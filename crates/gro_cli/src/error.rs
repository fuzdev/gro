//! `CliError`: every failure of `fgro`'s commands but eval's, which writes
//! its own as an envelope, with its exit code, printed in one place.

use std::ffi::OsString;
use std::path::PathBuf;

use gro_runtime::embed::EmbedError;
use gro_runtime::engine::EngineError;
use gro_runtime::prices::PricesError;
use gro_runtime::snapshot::SnapshotError;
use gro_runtime::{BINARY_NAME, RunError};
use thiserror::Error;

use crate::cli::{DirFlag, DirFlagFault, EnvFlagFault, RunEnvFlagFault, RunFlagFault};
use crate::print::{echo_argument, escape_controls, stderr_line};
use crate::{EXIT_FAILURE, EXIT_USAGE};

/// Why `fgro` failed, outside eval's envelope.
#[derive(Debug, Error)]
pub enum CliError {
    /// A priced build's switches name a default there is none of.
    #[error(transparent)]
    Prices(#[from] PricesError),
    /// An argument isn't UTF-8, named up to its first `=`, escaped
    /// ([`echo_argument`]), its invalid sequences as U+FFFD.
    #[error("an argument is not valid UTF-8: {}", echo_argument(&.0.to_string_lossy()))]
    NotUtf8(OsString),
    /// argh refused the arguments: the command they were given to, and the
    /// arguments as given, each as [`echo_argument`] prints it, bounded in
    /// number and length (`cli::invalid_arguments`). argh's own text isn't
    /// printed: it can carry any part of an argument.
    #[error(
        "invalid arguments to {command}: {arguments}\nRun {command} --help for more information."
    )]
    InvalidArguments {
        /// The command, the binary's name, followed by ` eval` for eval's.
        command: String,
        /// The arguments, as printed.
        arguments: String,
    },
    /// `--version` came with a subcommand.
    #[error("--version takes no subcommand")]
    VersionWithCommand,
    /// Neither `--version` nor a subcommand was given.
    #[error(
        "a subcommand is required\nRun {bin} --help for more information.",
        bin = BINARY_NAME
    )]
    NoCommand,
    /// `run` with an option before the file that it doesn't take, named up
    /// to its first `=`, escaped ([`echo_argument`]).
    #[error(
        "unknown option \"{}\" before the file: the script's arguments go after it\n\
         Run {bin} run --help for more information.",
        echo_argument(.0),
        bin = BINARY_NAME
    )]
    RunOption(String),
    /// `run` without a file.
    #[error(
        "a file to run is required\nRun {bin} run --help for more information.",
        bin = BINARY_NAME
    )]
    RunNoFile,
    /// `run --allow-env` with no names, which grants none.
    #[error(
        "--allow-env takes the names it grants, as --allow-env=A,B\n\
         Run {bin} run --help for more information.",
        bin = BINARY_NAME
    )]
    AllowEnvBare,
    /// `run --allow-env=` listing a name no variable can have.
    #[error("{0}\nRun {bin} run --help for more information.", bin = BINARY_NAME)]
    AllowEnvName(EnvFlagFault),
    /// `run --allow-read`, `--allow-write`, or `--allow-run-in` with no
    /// directories, which grants none.
    #[error(
        "{0} takes the directories it grants, as {0}=a,/b\n\
         Run {bin} run --help for more information.",
        bin = BINARY_NAME
    )]
    AllowDirsBare(DirFlag),
    /// `run --allow-temp=` with a value, which it takes none of; the value
    /// isn't echoed.
    #[error(
        "--allow-temp takes no value: it grants fresh directories under the OS's temporary \
         directory, as --allow-temp\n\
         Run {bin} run --help for more information.",
        bin = BINARY_NAME
    )]
    AllowTempValue,
    /// `run --allow-run` with no names, which grants none.
    #[error(
        "--allow-run takes the programs it grants, as --allow-run=git,cargo\n\
         Run {bin} run --help for more information.",
        bin = BINARY_NAME
    )]
    AllowRunBare,
    /// `run --allow-run=` listing a name it can't grant.
    #[error("{0}\nRun {bin} run --help for more information.", bin = BINARY_NAME)]
    AllowRunName(RunFlagFault),
    /// `run --allow-run-env` with no names, which passes none.
    #[error(
        "--allow-run-env takes the variables it lets programs be passed, as \
         --allow-run-env=A,B_*\n\
         Run {bin} run --help for more information.",
        bin = BINARY_NAME
    )]
    AllowRunEnvBare,
    /// `run --allow-run-env=` listing a name that names no variable or
    /// glob.
    #[error("{0}\nRun {bin} run --help for more information.", bin = BINARY_NAME)]
    AllowRunEnvName(RunEnvFlagFault),
    /// `run --allow-run-env=` without `--allow-run=` or `--allow-run-in=`,
    /// which grant no program to pass a variable to.
    #[error(
        "--allow-run-env lets the programs --allow-run and --allow-run-in grant be passed \
         variables, and neither grants one\n\
         Run {bin} run --help for more information.",
        bin = BINARY_NAME
    )]
    AllowRunEnvAlone,
    /// `run --allow-read=`, `--allow-write=`, or `--allow-run-in=` listing
    /// a path that names nothing.
    #[error(
        "{}\nRun {bin} run --help for more information.",
        match .1 {
            DirFlagFault::Empty => format!(
                "a {0} directory is empty: {0}=a,/b lists directories separated by single \
                 commas",
                .0
            ),
            DirFlagFault::Nul => format!("a {} directory holds a NUL byte", .0),
        },
        bin = BINARY_NAME
    )]
    AllowDir(DirFlag, DirFlagFault),
    /// A directory `--allow-read=`, `--allow-write=`, or `--allow-run-in=`
    /// lists isn't one as the run starts: missing, something else, or
    /// unreadable. Its path is named as resolved against the working
    /// directory, escaped.
    #[error(
        "a {flag} directory {}: {}{}\nRun {bin} run --help for more information.",
        match fault {
            DirFault::Missing => "doesn't exist",
            DirFault::NotADirectory => "isn't a directory",
            DirFault::Unreadable(_) => "can't be looked at",
        },
        escape_controls(&path.to_string_lossy()),
        match fault {
            DirFault::Unreadable(error) => format!(" ({error})"),
            DirFault::Missing | DirFault::NotADirectory => String::new(),
        },
        bin = BINARY_NAME
    )]
    NoDir {
        /// The flag that listed it.
        flag: DirFlag,
        /// Its absolute path.
        path: PathBuf,
        /// What's wrong with it.
        fault: DirFault,
    },
    /// `eval --timeout 0`.
    #[error("--timeout takes a number of milliseconds of at least 1")]
    TimeoutZero,
    /// `eval --driver - --export`.
    #[error("--export reads an export with no driver; a driver is handed the whole namespace")]
    DriverWithExport,
    /// `eval --driver` with a path rather than `-`.
    #[error(
        "--driver takes `-`, to read the driver from stdin, not {0:?}: a driver from a file \
         isn't supported yet"
    )]
    DriverPath(String),
    /// An internal subcommand (`make-snapshot`, `make-embed`) without the
    /// arguments it takes: its name and their usage.
    #[error("usage: {} {} {}", BINARY_NAME, .0, .1)]
    InternalUsage(&'static str, &'static str),
    /// An internal subcommand with a flag, so `--help` can't become a
    /// directory's name: its name and its arguments' usage.
    #[error("usage: {} {} {} (it takes no flags)", BINARY_NAME, .0, .1)]
    InternalFlag(&'static str, &'static str),
    /// The directory an internal subcommand writes into couldn't be
    /// created.
    #[error("failed to create the directory {}: {source}", path.display())]
    OutputDir {
        /// The directory.
        path: PathBuf,
        /// The failure creating it.
        source: std::io::Error,
    },
    /// The snapshot couldn't be made or written.
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// The embed couldn't be made, or a blob didn't decode.
    #[error(transparent)]
    Embed(#[from] EmbedError),
    /// The embed's file couldn't be written.
    #[error("failed to write {}: {source}", path.display())]
    EmbedWrite {
        /// The file.
        path: PathBuf,
        /// The failure writing it.
        source: std::io::Error,
    },
    /// The binary's own embed differs from the one `make-embed` made, so it
    /// was built from other sources than the directory holds now.
    #[error(
        "this binary's embed differs from the one the sources make now, first at {path:?}: \
         it was built from other sources"
    )]
    EmbedStale {
        /// The first module's path, relative to `src/lib`, that differs, or
        /// that one holds and the other doesn't, else the first path left
        /// out of one and not the other, else `../package.json` when the
        /// peers differ.
        path: String,
    },
    /// The engine couldn't be set up.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The working directory couldn't be read.
    #[error("failed to read the working directory: {0}")]
    Cwd(#[source] std::io::Error),
    /// A run that grants programs couldn't make ready to start them: the
    /// inherited descriptors marked close-on-exec, `SIGCHLD` at its default.
    /// The soft limit on open descriptors is raised best-effort, never
    /// failing the run.
    #[error("failed to make ready to start programs: {0}")]
    SpawnReady(#[source] std::io::Error),
    /// `fgro run`'s module failed, printed as [`run_failure`] prints it.
    #[error("{}", run_failure(.0))]
    Run(#[from] RunError),
    /// Output couldn't be written to stdout (a closed pipe, a full disk).
    #[error("failed to write to stdout: {0}")]
    Stdout(#[source] std::io::Error),
}

/// `error` as `fgro` prints it: a report, which can hold any text the
/// program chose, each of its lines escaped whole, its line breaks included
/// ([`escape_controls`]), so its only breaks are the report's own; an entry
/// that didn't resolve escaped as a directory a flag names is; and a
/// runtime that didn't start, before any of the program ran, as it is.
fn run_failure(error: &RunError) -> String {
    match error {
        RunError::Uncaught(report) => report
            .lines()
            .iter()
            .map(|line| escape_controls(line))
            .collect::<Vec<_>>()
            .join("\n"),
        RunError::Entry(error) => escape_controls(&error.to_string()).into_owned(),
        RunError::Start(_) => error.to_string(),
    }
}

/// Why a directory a flag lists can't be granted, found by a `stat` as the
/// run starts.
#[derive(Debug)]
pub enum DirFault {
    /// Nothing is at its path.
    Missing,
    /// Something other than a directory is at its path.
    NotADirectory,
    /// Its path couldn't be looked at, as the OS said.
    Unreadable(std::io::Error),
}

impl CliError {
    /// The exit code: [`EXIT_USAGE`] when the caller must change the
    /// arguments, an entry the resolver refuses included
    /// (`ResolveErrorKind::is_refusal`), else [`EXIT_FAILURE`], an entry
    /// whose resolution failed reading the tree (an I/O error, a symlink
    /// loop) among them.
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Prices(_)
            | Self::NotUtf8(_)
            | Self::InvalidArguments { .. }
            | Self::VersionWithCommand
            | Self::NoCommand
            | Self::RunOption(_)
            | Self::RunNoFile
            | Self::AllowEnvBare
            | Self::AllowEnvName(_)
            | Self::AllowDirsBare(_)
            | Self::AllowTempValue
            | Self::AllowDir(..)
            | Self::AllowRunBare
            | Self::AllowRunName(_)
            | Self::AllowRunEnvBare
            | Self::AllowRunEnvName(_)
            | Self::AllowRunEnvAlone
            | Self::NoDir { .. }
            | Self::TimeoutZero
            | Self::DriverWithExport
            | Self::DriverPath(_)
            | Self::InternalUsage(..)
            | Self::InternalFlag(..) => EXIT_USAGE,
            Self::Run(RunError::Entry(error)) if error.kind.is_refusal() => EXIT_USAGE,
            Self::OutputDir { .. }
            | Self::Snapshot(_)
            | Self::Embed(_)
            | Self::EmbedWrite { .. }
            | Self::EmbedStale { .. }
            | Self::Engine(_)
            | Self::Cwd(_)
            | Self::SpawnReady(_)
            | Self::Run(_)
            | Self::Stdout(_) => EXIT_FAILURE,
        }
    }

    /// Whether the message prints as it is, with no `error:` label: an
    /// internal subcommand's usage line, which carries its own.
    const fn is_unlabeled(&self) -> bool {
        matches!(self, Self::InternalUsage(..) | Self::InternalFlag(..))
    }

    /// Prints the error to stderr, labeled `error:` unless
    /// [`Self::is_unlabeled`], returning its exit code.
    pub fn report(&self) -> u8 {
        if self.is_unlabeled() {
            stderr_line(self);
        } else {
            stderr_line(format_args!("error: {self}"));
        }
        self.exit_code()
    }
}
