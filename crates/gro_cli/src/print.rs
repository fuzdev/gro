//! Writes to stdout and stderr that never panic: std's `println!` and
//! `eprintln!` panic when their stream fails (a closed pipe, a full disk).

use std::fmt::Display;
use std::io::Write as _;

/// Writes `line` and a newline to stdout, flushed.
///
/// # Errors
///
/// Returns the write's failure, which the caller reports and exits by.
pub fn stdout_line(line: impl Display) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()
}

/// Writes `line` and a newline to stderr, ignoring a failure, which has
/// nowhere left to be reported.
pub fn stderr_line(line: impl Display) {
    // stderr is unbuffered, so there's nothing to flush
    drop(writeln!(std::io::stderr().lock(), "{line}"));
}
