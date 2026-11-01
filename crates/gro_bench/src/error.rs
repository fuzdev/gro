//! The harness's errors.

use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use thiserror::Error;

use crate::check::Difference;

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
    /// A workload names a collection the pinned corpora tree doesn't have.
    #[error(
        "the corpora tree {tree} (in {}) has no collection `{name}`",
        corpora.display()
    )]
    CollectionNotFound {
        corpora: PathBuf,
        tree: String,
        name: String,
    },
    /// A collection holds something a materialized copy doesn't support: a
    /// symlink, a submodule, or a path that isn't UTF-8 or isn't plain
    /// relative segments.
    #[error(
        "the collection `{name}` has {} with mode {mode}, but a materialized copy holds only regular and executable files at plain UTF-8 relative paths",
        path.display()
    )]
    CollectionEntry {
        name: String,
        path: PathBuf,
        mode: String,
    },
    /// A workload's entry script would replace a file of its collection.
    #[error(
        "the workload `{workload}` writes its entry {} at the root of the collection `{collection}`, which already has it",
        entry.display()
    )]
    EntryCollides {
        workload: String,
        collection: String,
        entry: PathBuf,
    },
    /// A link of a workload's project names a checkout that doesn't exist,
    /// isn't a directory, or can't be read, saying which.
    #[error(
        "the workload `{workload}` links `{package}` to {}, which {}",
        path.display(),
        link_problem(source)
    )]
    ProjectLinkMissing {
        workload: String,
        package: String,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// A link of a workload's project names a directory in no git checkout,
    /// whose code the record couldn't name.
    #[error(
        "the workload `{workload}` links `{package}` to {}, which is in no git checkout git can read, so the record can't name the code it holds",
        path.display()
    )]
    ProjectLinkUnversioned {
        workload: String,
        package: String,
        path: PathBuf,
    },
    /// A workload's project directory holds something its copy doesn't
    /// support, saying why.
    #[error("the workload `{workload}`'s project has {}: {reason}", path.display())]
    ProjectEntry {
        workload: String,
        path: PathBuf,
        reason: String,
    },
    /// A row's untimed correctness run failed: it exited unsuccessfully, or
    /// its stdout isn't what its runtime's `stdout` says, saying why.
    #[error("`{command}` failed its correctness run: {reason}")]
    CheckFailed { command: String, reason: String },
    /// Two runtimes of the suite's `parity` gave different values on a
    /// workload, naming where and what each holds there (as JSON, cut short):
    /// `difference.a` is `runtime`'s, `difference.b` is `against`'s.
    #[error(
        "on the workload `{workload}`, the runtime `{runtime}`'s value differs from `{against}`'s at {}: {} against {}",
        difference.at,
        difference.a,
        difference.b
    )]
    ParityMismatch {
        workload: String,
        runtime: String,
        against: String,
        difference: Box<Difference>,
    },
    /// A file or directory the run writes couldn't be written or removed.
    #[error("failed to write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// An artifact's hash, the corpora's tree, or the state of another
    /// checkout a workload reads (its commit, or its changes) moved during
    /// the run, or a materialized copy gained or lost a path.
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

/// What's wrong with a project link's target, by the error reading it.
fn link_problem(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "doesn't exist".to_owned(),
        io::ErrorKind::NotADirectory => "isn't a directory".to_owned(),
        _ => format!("can't be read ({error})"),
    }
}

impl BenchError {
    /// Whether the input is at fault (the suite, a program or file it names,
    /// a command that fails or prints no version, a corpora checkout off its
    /// pin, a collection the pinned tree lacks or a copy can't hold, an
    /// entry colliding with a collection's file, a project's missing or
    /// unversioned link or unsupported entry, a failed correctness run, or values that break the
    /// suite's parity), so the caller must change
    /// something before re-running: unchanged, it fails the same way.
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
                | Self::CollectionNotFound { .. }
                | Self::CollectionEntry { .. }
                | Self::EntryCollides { .. }
                | Self::ProjectLinkMissing { .. }
                | Self::ProjectLinkUnversioned { .. }
                | Self::ProjectEntry { .. }
                | Self::CheckFailed { .. }
                | Self::ParityMismatch { .. }
        )
    }
}
