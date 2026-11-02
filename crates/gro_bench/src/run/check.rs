//! The correctness run: each row once, untimed, with its stdout checked and
//! the parity runtimes' values compared.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::Path;
use std::process::Stdio;

use super::command::RowCommand;
use crate::check::{first_difference, stdout_value, truncated};
use crate::error::BenchError;
use crate::suite::Suite;
use crate::watchdog::Watchdog;

/// Runs each row once, untimed, and checks it: it exits successfully, its
/// stdout is what its runtime's `stdout` says, and the values of the suite's
/// `parity` runtimes are equal on each workload.
pub(super) fn check_rows(
    suite: &Suite,
    rows: &[RowCommand],
    dir: &Path,
    watchdog: &Watchdog,
) -> Result<(), BenchError> {
    // each checked row's value, by workload and then runtime
    let mut values: BTreeMap<&str, BTreeMap<&str, serde_json::Value>> = BTreeMap::new();
    for row in rows {
        if let Some(value) = check_row(row, dir, watchdog)? {
            values
                .entry(row.workload.as_str())
                .or_default()
                .insert(row.runtime.as_str(), value);
        }
    }
    let Some((first, others)) = suite.parity.split_first() else {
        return Ok(());
    };
    for workload in &suite.workloads {
        let values = &values[workload.name.as_str()];
        // validation gives every parity runtime a `stdout`, so each has a value
        let expected = &values[first.as_str()];
        for other in others {
            if let Some(difference) = first_difference(&values[other.as_str()], expected) {
                return Err(BenchError::ParityMismatch {
                    workload: workload.name.clone(),
                    runtime: other.clone(),
                    against: first.clone(),
                    difference: Box::new(difference),
                });
            }
        }
    }
    Ok(())
}

/// Runs one process of `row`, untimed, capturing its output: it must exit
/// successfully, and its stdout must be what `row.stdout` says, whose value
/// it returns. Past `watchdog`'s deadline it's killed, and fails the run.
fn check_row(
    row: &RowCommand,
    dir: &Path,
    watchdog: &Watchdog,
) -> Result<Option<serde_json::Value>, BenchError> {
    let spawn_error = |source| BenchError::Spawn {
        command: row.display(),
        source,
    };
    let mut command = row.command(dir)?;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // its own group, so a kill reaches whatever holds its pipes
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(spawn_error)?;
    let watch = watchdog.watch_group(child.id());
    // stdout whole only when it's checked: a large output kept unchecked
    // would raise the harness's peak, which every later sample reads at least
    let stdout_kept = if row.stdout.is_some() {
        usize::MAX
    } else {
        UNCHECKED_KEPT
    };
    let (stdout, stderr) = std::thread::scope(|scope| {
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout =
            scope.spawn(move || stdout.map_or(Ok(Vec::new()), |out| read_kept(out, stdout_kept)));
        let stderr = stderr.map_or(Ok(Vec::new()), |err| read_kept(err, UNCHECKED_KEPT));
        let stdout = stdout
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        (stdout, stderr)
    });
    let status = child.wait().map_err(spawn_error)?;
    if watch.disarm() {
        return Err(BenchError::TimedOut {
            command: row.display(),
            timeout_ms: watchdog.timeout().as_millis(),
        });
    }
    let (stdout, stderr) = (stdout.map_err(spawn_error)?, stderr.map_err(spawn_error)?);
    let failed = |reason: String| BenchError::CheckFailed {
        command: row.display(),
        reason,
    };
    if !status.success() {
        return Err(failed(format!(
            "it exited unsuccessfully ({status}); stdout: {}; stderr: {}",
            truncated(String::from_utf8_lossy(&stdout).trim()),
            truncated(String::from_utf8_lossy(&stderr).trim()),
        )));
    }
    row.stdout
        .map(|check| stdout_value(check, &stdout).map_err(failed))
        .transpose()
}

/// The bytes of an output the correctness run keeps when it checks nothing
/// in it, enough for its message: 64 KiB.
const UNCHECKED_KEPT: usize = 64 << 10;

/// Reads `reader` to its end, keeping its first `kept` bytes.
fn read_kept(mut reader: impl Read, kept: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut reader)
        .take(u64::try_from(kept).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)?;
    io::copy(&mut reader, &mut io::sink())?;
    Ok(bytes)
}
