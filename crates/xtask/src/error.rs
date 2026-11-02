//! The xtask's errors, each printed once, by `main`, with the exit code it
//! maps to ([`XtaskError::exit_code`]).

use std::error::Error as _;
use std::io;
use std::process::ExitStatus;

use gro_bench::error::BenchError;

use crate::EXIT_INPUT;
use crate::v8_archive::ArchiveError;

/// Why a step failed.
#[derive(Debug, thiserror::Error)]
pub enum XtaskError {
    /// A command line the step refuses, such as flags that don't combine.
    #[error("bench: {0}")]
    BenchUsage(String),
    /// A bench run failed, by the harness's taxonomy.
    #[error("bench failed: {0}")]
    Bench(BenchError),
    /// The V8 archive isn't ready.
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    /// A program a step ran exited unsuccessfully; its own output says why.
    #[error("{display} failed ({status})")]
    StepFailed { display: String, status: ExitStatus },
    /// A program a step runs couldn't start.
    #[error("failed to run {display}: {source}")]
    Spawn {
        display: String,
        #[source]
        source: io::Error,
    },
    /// A build the bench needs before it runs failed, so the run can't go
    /// ahead until it's fixed.
    #[error(transparent)]
    BuildFailed(Box<Self>),
    /// Anything else the caller must change before re-running: unchanged, it
    /// fails the same way.
    #[error("{0}")]
    Input(String),
    /// Anything else, such as a failed read or write.
    #[error("{0}")]
    Failed(String),
}

impl XtaskError {
    /// The exit code the xtask returns: a failed step's own code (1 when a
    /// signal ended it), `2` ([`EXIT_INPUT`]) when the caller must change
    /// something first, else 1.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::BenchUsage(_) | Self::BuildFailed(_) | Self::Input(_) => EXIT_INPUT,
            Self::Bench(error) if error.is_input_error() => EXIT_INPUT,
            Self::StepFailed { status, .. } => status
                .code()
                .and_then(|code| u8::try_from(code).ok())
                .filter(|code| *code != 0)
                .unwrap_or(1),
            Self::Bench(_) | Self::Archive(_) | Self::Spawn { .. } | Self::Failed(_) => 1,
        }
    }

    /// A failure of a build the bench runs first: a compile or a step that
    /// exits unsuccessfully is the run's input to fix (exit `2`), while a
    /// failure to read, write, or spawn keeps its own code.
    pub fn in_bench_build(self) -> Self {
        match self {
            Self::StepFailed { .. } => Self::BuildFailed(Box::new(self)),
            other => other,
        }
    }

    /// Prints the error to stderr, each cause of a bench failure on a line
    /// of its own.
    pub fn print(&self) {
        eprintln!("[xtask] {self}");
        if let Self::Bench(error) = self {
            let mut source = error.source();
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
        }
    }
}

impl From<BenchError> for XtaskError {
    fn from(error: BenchError) -> Self {
        Self::Bench(error)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt as _;

    use super::*;

    #[test]
    fn steps_exit_with_their_code_and_bench_builds_as_input() {
        let failed = |raw| XtaskError::StepFailed {
            display: "cargo".to_owned(),
            status: ExitStatus::from_raw(raw),
        };
        // an exit code is the high byte of a raw wait status
        assert_eq!(failed(101 << 8).exit_code(), 101);
        // killed by a signal
        assert_eq!(failed(9).exit_code(), 1);
        assert_eq!(failed(101 << 8).in_bench_build().exit_code(), EXIT_INPUT);
        assert_eq!(
            failed(101 << 8).in_bench_build().to_string(),
            "cargo failed (exit status: 101)"
        );
        let io_failure = XtaskError::Failed("failed to write x".to_owned());
        assert_eq!(io_failure.in_bench_build().exit_code(), 1);
        let spawn = XtaskError::Spawn {
            display: "cargo metadata".to_owned(),
            source: io::Error::other("test"),
        };
        assert_eq!(spawn.in_bench_build().exit_code(), 1);
        assert_eq!(
            XtaskError::BenchUsage("x".to_owned()).to_string(),
            "bench: x"
        );
    }
}
