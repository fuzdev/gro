//! `CliError`: every failure of `fgro`'s commands but eval's, which writes
//! its own as an envelope, with its exit code, printed in one place.

use std::ffi::OsString;
use std::path::PathBuf;

use gro_runtime::RunError;
use gro_runtime::engine::EngineError;
use gro_runtime::prices::PricesError;
use gro_runtime::snapshot::SnapshotError;
use thiserror::Error;

use crate::cli::{DirFlag, DirFlagFault, EnvFlagFault, MAKE_SNAPSHOT};
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
        /// The command, `fgro` or `fgro eval`.
        command: &'static str,
        /// The arguments, as printed.
        arguments: String,
    },
    /// `--version` came with a subcommand.
    #[error("--version takes no subcommand")]
    VersionWithCommand,
    /// Neither `--version` nor a subcommand was given.
    #[error("a subcommand is required\nRun fgro --help for more information.")]
    NoCommand,
    /// `run` with an option before the file that it doesn't take, named up
    /// to its first `=`, escaped ([`echo_argument`]).
    #[error(
        "unknown option \"{}\" before the file: the script's arguments go after it\n\
         Run fgro run --help for more information.",
        echo_argument(.0)
    )]
    RunOption(String),
    /// `run` without a file.
    #[error("a file to run is required\nRun fgro run --help for more information.")]
    RunNoFile,
    /// `run --allow-env` with no names, which grants none.
    #[error(
        "--allow-env takes the names it grants, as --allow-env=A,B\n\
         Run fgro run --help for more information."
    )]
    AllowEnvBare,
    /// `run --allow-env=` listing a name no variable can have.
    #[error("{0}\nRun fgro run --help for more information.")]
    AllowEnvName(EnvFlagFault),
    /// `run --allow-read` or `--allow-write` with no directories, which
    /// grants none.
    #[error(
        "{0} takes the directories it grants, as {0}=a,/b\n\
         Run fgro run --help for more information."
    )]
    AllowDirsBare(DirFlag),
    /// `run --allow-read=` or `--allow-write=` listing a path that names
    /// nothing.
    #[error(
        "{}\nRun fgro run --help for more information.",
        match .1 {
            DirFlagFault::Empty => format!(
                "a {0} directory is empty: {0}=a,/b lists directories separated by single \
                 commas",
                .0
            ),
            DirFlagFault::Nul => format!("a {} directory holds a NUL byte", .0),
        }
    )]
    AllowDir(DirFlag, DirFlagFault),
    /// A directory `--allow-read=` or `--allow-write=` lists isn't one as
    /// the run starts: missing, something else, or unreadable. Its path is
    /// named as resolved against the working directory, escaped.
    #[error(
        "a {flag} directory {}: {}{}\nRun fgro run --help for more information.",
        match fault {
            DirFault::Missing => "doesn't exist",
            DirFault::NotADirectory => "isn't a directory",
            DirFault::Unreadable(_) => "can't be looked at",
        },
        escape_controls(&path.to_string_lossy()),
        match fault {
            DirFault::Unreadable(error) => format!(" ({error})"),
            DirFault::Missing | DirFault::NotADirectory => String::new(),
        }
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
    /// `make-snapshot` without exactly one argument.
    #[error("usage: fgro {} <dir>", MAKE_SNAPSHOT)]
    SnapshotUsage,
    /// `make-snapshot` with a flag, so `--help` can't become a directory's
    /// name.
    #[error("usage: fgro {} <dir> (it takes no flags)", MAKE_SNAPSHOT)]
    SnapshotFlag,
    /// The snapshot's directory couldn't be created.
    #[error("failed to create the snapshot directory {}: {source}", path.display())]
    SnapshotDir {
        /// The directory.
        path: PathBuf,
        /// The failure creating it.
        source: std::io::Error,
    },
    /// The snapshot couldn't be made or written.
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    /// The engine couldn't be set up.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// The working directory couldn't be read.
    #[error("failed to read the working directory: {0}")]
    Cwd(#[source] std::io::Error),
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
            | Self::AllowDir(..)
            | Self::NoDir { .. }
            | Self::TimeoutZero
            | Self::DriverWithExport
            | Self::DriverPath(_)
            | Self::SnapshotUsage
            | Self::SnapshotFlag => EXIT_USAGE,
            Self::Run(RunError::Entry(error)) if error.kind.is_refusal() => EXIT_USAGE,
            Self::SnapshotDir { .. }
            | Self::Snapshot(_)
            | Self::Engine(_)
            | Self::Cwd(_)
            | Self::Run(_)
            | Self::Stdout(_) => EXIT_FAILURE,
        }
    }

    /// Whether the message prints as it is, with no `error:` label:
    /// `make-snapshot`'s usage line, which carries its own.
    const fn is_unlabeled(&self) -> bool {
        matches!(self, Self::SnapshotUsage | Self::SnapshotFlag)
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
