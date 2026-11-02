//! `CliError`: every failure of `fgro`'s commands but eval's, which writes
//! its own as an envelope, with its exit code, printed in one place.

use std::ffi::OsString;
use std::path::PathBuf;

use gro_runtime::RunError;
use gro_runtime::engine::EngineError;
use gro_runtime::prices::PricesError;
use gro_runtime::snapshot::SnapshotError;
use thiserror::Error;

use crate::cli::MAKE_SNAPSHOT;
use crate::print::stderr_line;
use crate::{EXIT_FAILURE, EXIT_USAGE};

/// Why `fgro` failed, outside eval's envelope.
#[derive(Debug, Error)]
pub enum CliError {
    /// A priced build's switches name a default there is none of.
    #[error(transparent)]
    Prices(#[from] PricesError),
    /// An argument isn't UTF-8.
    #[error("an argument is not valid UTF-8: {}", .0.to_string_lossy())]
    NotUtf8(OsString),
    /// argh refused the arguments, its usage text as it wrote it.
    #[error("{}", .0.trim_end())]
    Parse(String),
    /// `--version` came with a subcommand.
    #[error("--version takes no subcommand")]
    VersionWithCommand,
    /// Neither `--version` nor a subcommand was given.
    #[error("a subcommand is required\nRun fgro --help for more information.")]
    NoCommand,
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
    #[error("{}: {source}", path.display())]
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
    /// `fgro run`'s module failed.
    #[error(transparent)]
    Run(#[from] RunError),
    /// Output couldn't be written to stdout (a closed pipe, a full disk).
    #[error("failed to write to stdout: {0}")]
    Stdout(#[source] std::io::Error),
}

impl CliError {
    /// The exit code: [`EXIT_USAGE`] when the caller must change the
    /// arguments, an entry that doesn't resolve included, else
    /// [`EXIT_FAILURE`].
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Prices(_)
            | Self::NotUtf8(_)
            | Self::Parse(_)
            | Self::VersionWithCommand
            | Self::NoCommand
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

    /// Whether the message prints as it is, with no `error:` label: argh's
    /// usage text and `make-snapshot`'s usage line, which carry their own.
    const fn is_unlabeled(&self) -> bool {
        matches!(
            self,
            Self::Parse(_) | Self::SnapshotUsage | Self::SnapshotFlag
        )
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
