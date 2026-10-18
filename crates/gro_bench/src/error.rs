//! The harness's errors.

use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use thiserror::Error;

/// Why a run, or reading its input, failed. A failed run records nothing.
#[derive(Debug, Error)]
pub enum BenchError {
    /// The suite file couldn't be read.
    #[error("failed to read the suite {path}")]
    SuiteRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The suite file isn't a suite: not JSON, or not the suite's shape.
    #[error("failed to parse the suite {path}")]
    SuiteParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    /// The suite parsed but fails a check the types can't make, saying which.
    #[error("invalid suite: {0}")]
    SuiteInvalid(String),
    /// A runtime's program isn't an executable on `PATH` or at its path.
    #[error("runtime `{runtime}`: program `{program}` not found")]
    ProgramNotFound { runtime: String, program: String },
    /// A runtime's program is a script (it starts with `#!`), whose hash
    /// would name the wrapper, not the runtime.
    #[error(
        "runtime `{runtime}`: `{}` is a script (it starts with `#!`), so its hash wouldn't name the runtime; name the real executable",
        path.display()
    )]
    ProgramIsScript { runtime: String, path: PathBuf },
    /// A script, file argument, or corpora checkout the suite names doesn't
    /// exist.
    #[error("the suite names {}, which doesn't exist", path.display())]
    SuiteFileMissing {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// A file the run hashes or inspects couldn't be read.
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// A process couldn't be spawned.
    #[error("failed to run `{command}`")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    /// A process (a sample, a version probe, `git`) exited unsuccessfully.
    #[error("`{command}` failed ({status})")]
    CommandFailed { command: String, status: ExitStatus },
    /// A version probe printed no non-empty line to stdout.
    #[error("`{command}` printed no version")]
    NoVersion { command: String },
    /// The corpora checkout isn't at the tree the suite pins.
    #[error(
        "the corpora at {} are at tree {found}, but the suite pins {pinned}",
        path.display()
    )]
    CorporaTree {
        path: PathBuf,
        pinned: String,
        found: String,
    },
    /// The corpora checkout has local changes under `collections/`.
    #[error(
        "the corpora at {} have local changes under `collections/`, so the tree id doesn't name the bytes the workloads read",
        path.display()
    )]
    CorporaDirty { path: PathBuf },
    /// An artifact's hash, or the corpora's tree, changed during the run.
    #[error(
        "artifacts changed during the run, so its samples are discarded: {}",
        paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    ArtifactsChanged { paths: Vec<PathBuf> },
    /// The record couldn't be appended to the history.
    #[error("failed to append to the history {path}")]
    HistoryWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl BenchError {
    /// Whether the input is at fault (the suite, a program or file it names,
    /// a command that fails or prints no version, or a corpora checkout off its
    /// pin), so the caller must change something before re-running: unchanged,
    /// it fails the same way.
    pub const fn is_input_error(&self) -> bool {
        matches!(
            self,
            Self::SuiteRead { .. }
                | Self::SuiteParse { .. }
                | Self::SuiteInvalid(_)
                | Self::SuiteFileMissing { .. }
                | Self::ProgramNotFound { .. }
                | Self::ProgramIsScript { .. }
                | Self::CommandFailed { .. }
                | Self::NoVersion { .. }
                | Self::CorporaTree { .. }
                | Self::CorporaDirty { .. }
        )
    }
}
