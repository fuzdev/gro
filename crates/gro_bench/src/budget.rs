//! The suite's budgets: limits on a runtime's executable, gzipped, and
//! startup snapshot, checked against the sizes measured.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::BenchError;
use crate::record::RuntimeInfo;
use crate::run::display_command;
use crate::suite::{Budget, Suite};

/// Fails the run when a runtime's size crosses one of the suite's budgets
/// for it, naming the budget, the size, and the limit.
pub(crate) fn check_budgets(
    suite: &Suite,
    runtimes: &[RuntimeInfo],
    snapshot_bytes: Option<u64>,
) -> Result<(), BenchError> {
    for (name, budget) in &suite.budgets {
        let Some(runtime) = runtimes.iter().find(|runtime| &runtime.name == name) else {
            continue;
        };
        let sizes = Sizes {
            bytes: Some(runtime.bytes),
            gzip_bytes: Some(runtime.gzip_bytes),
            snapshot_bytes,
        };
        check_budget(name, budget, &sizes)?;
    }
    Ok(())
}

/// A runtime's sizes as measured, each in bytes, for [`check_budget`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sizes {
    /// The executable as installed.
    pub bytes: Option<u64>,
    /// The executable gzipped ([`gzip_size`]).
    pub gzip_bytes: Option<u64>,
    /// Its startup snapshot.
    pub snapshot_bytes: Option<u64>,
}

/// Fails when one of `sizes` crosses its limit in `budget`, the runtime
/// `runtime`'s, naming the budget, the size, and the limit.
///
/// # Errors
///
/// [`BenchError::BudgetExceeded`] for the first size over its limit, and
/// [`BenchError::SuiteInvalid`] for a limit with no size measured.
pub fn check_budget(runtime: &str, budget: &Budget, sizes: &Sizes) -> Result<(), BenchError> {
    let checks = [
        ("bytes", budget.bytes, sizes.bytes),
        ("gzip_bytes", budget.gzip_bytes, sizes.gzip_bytes),
        (
            "snapshot_bytes",
            budget.snapshot_bytes,
            sizes.snapshot_bytes,
        ),
    ];
    for (field, limit, current) in checks {
        let Some(limit) = limit else {
            continue;
        };
        let Some(current) = current else {
            return Err(BenchError::SuiteInvalid(format!(
                "the runtime `{runtime}` has a `{field}` budget, but no size was measured for it (a snapshot budget needs the snapshot named)"
            )));
        };
        if current > limit {
            return Err(BenchError::BudgetExceeded {
                runtime: runtime.to_owned(),
                budget: field,
                current,
                limit,
            });
        }
    }
    Ok(())
}

/// The size of `path` gzipped at gzip's default level, counted from `gzip`'s
/// output as it streams. `-n` leaves the file's name and time out of the
/// header, so the size depends on the bytes alone.
///
/// # Errors
///
/// Fails when `gzip` can't run or fails.
pub fn gzip_size(path: &Path) -> Result<u64, BenchError> {
    let command = display_command(
        Path::new("gzip"),
        &[OsStr::new("-c"), OsStr::new("-n"), path.as_os_str()],
    );
    let spawn_error = |source| BenchError::Spawn {
        command: command.clone(),
        source,
    };
    let mut child = Command::new("gzip")
        .args(["-c", "-n"])
        .arg(path)
        // gzip reads default options from it
        .env_remove("GZIP")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(spawn_error)?;
    let counted = child
        .stdout
        .take()
        .map_or(Ok(0), |mut stdout| io::copy(&mut stdout, &mut io::sink()));
    let status = child.wait().map_err(spawn_error)?;
    let size = counted.map_err(spawn_error)?;
    if !status.success() {
        return Err(BenchError::CommandFailed { command, status });
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn gzip_size_counts_the_compressed_bytes() {
        let path = std::env::temp_dir().join(format!("gro_bench_gzip_{}", std::process::id()));
        fs::write(&path, "a".repeat(100_000)).unwrap();
        let size = gzip_size(&path);
        fs::remove_file(&path).unwrap();
        let size = size.unwrap();
        // a run of one byte compresses to a few hundred bytes, header included
        assert!((20..1000).contains(&size), "{size}");
    }
}
