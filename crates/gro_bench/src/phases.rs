//! Phases mode: each timed sample of a runtime built with fgro's `phases`
//! feature writes a timestamp per phase boundary, which the run turns into
//! per-phase durations ([`crate::record::PhaseMetric`]).
//!
//! The run names the file through [`PHASES_FILE_ENV`], set only on the
//! processes of the runtimes it names, and reads it after each sample. The
//! file is text, one record per line, fields separated by a space: the
//! header [`PHASES_HEADER`]; `main_unix_ns`, the runtime's wall clock
//! (`CLOCK_REALTIME`) at its entry, in nanoseconds since the Unix epoch; then
//! each phase's name and the nanoseconds from the entry to its end, by the
//! monotonic clock, in the order they ran. The writer is `gro_runtime`'s
//! `phases` module; the variable and header here are the same contract.
//!
//! A sample's durations are `start`, from the harness's spawn to the
//! runtime's entry (both wall clocks, so a clock step mid-sample would skew
//! it, and a negative span fails the run); each phase, from the end of the
//! one before (or the entry) to its own; and `reap`, from the last phase's
//! end to the harness reaping the exit. Together they span the sample, so
//! they sum to about its wall time. Process start is priced this way rather
//! than left out because the time from exec to `main` (the dynamic loader,
//! the binary's relocations, the Rust runtime) is part of what a startup
//! row measures.

use crate::error::BenchError;

/// The variable naming the file a phases build writes its marks to.
pub const PHASES_FILE_ENV: &str = "GRO_PHASES_FILE";

/// The file's first line, its format and version.
pub const PHASES_HEADER: &str = "gro_phases 1";

/// The duration before the runtime's first phase.
pub const START: &str = "start";

/// The duration after the runtime's last phase.
pub const REAP: &str = "reap";

/// The wall clock read on each side of one sample, in nanoseconds since the
/// Unix epoch.
#[derive(Debug, Clone, Copy)]
pub struct SampleSpan {
    pub spawn_unix_ns: u128,
    pub reap_unix_ns: u128,
}

/// The durations of one sample's phases, by name in order, from the file's
/// `text` and the harness's `span`.
///
/// # Errors
///
/// Fails with [`BenchError::Phases`] when the text isn't the format, names
/// no phase, or gives a span that runs backward.
pub fn durations(text: &str, span: SampleSpan) -> Result<Vec<(String, u64)>, BenchError> {
    let invalid = |why: String| BenchError::Phases(why);
    let mut lines = text.lines();
    if lines.next() != Some(PHASES_HEADER) {
        return Err(invalid(format!(
            "the phases file doesn't start with `{PHASES_HEADER}`"
        )));
    }
    let field = |line: Option<&str>| -> Result<(String, u128), BenchError> {
        let line = line.ok_or_else(|| invalid("the phases file ends early".to_owned()))?;
        let (name, value) = line
            .split_once(' ')
            .ok_or_else(|| invalid(format!("the phases file has the line `{line}`")))?;
        let value = value
            .parse()
            .map_err(|_| invalid(format!("the phases file has the line `{line}`")))?;
        Ok((name.to_owned(), value))
    };
    let (anchor, main_unix_ns) = field(lines.next())?;
    if anchor != "main_unix_ns" {
        return Err(invalid(format!(
            "the phases file's second line is `{anchor}`, not `main_unix_ns`"
        )));
    }
    let to_u64 = |ns: u128| u64::try_from(ns).unwrap_or(u64::MAX);
    let backward = |what: &str| {
        invalid(format!(
            "the {what} span runs backward, so a clock stepped during the sample; re-run"
        ))
    };
    let start = main_unix_ns
        .checked_sub(span.spawn_unix_ns)
        .ok_or_else(|| backward(START))?;
    let mut durations = vec![(START.to_owned(), to_u64(start))];
    let mut last = 0;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, at) = field(Some(line))?;
        let duration = at.checked_sub(last).ok_or_else(|| backward(&name))?;
        durations.push((name, to_u64(duration)));
        last = at;
    }
    if durations.len() == 1 {
        return Err(invalid("the phases file names no phase".to_owned()));
    }
    let reap = span
        .reap_unix_ns
        .checked_sub(main_unix_ns + last)
        .ok_or_else(|| backward(REAP))?;
    durations.push((REAP.to_owned(), to_u64(reap)));
    Ok(durations)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPAN: SampleSpan = SampleSpan {
        spawn_unix_ns: 1_000,
        reap_unix_ns: 10_000,
    };

    #[test]
    fn spans_the_sample_from_spawn_to_reap() {
        let text = "gro_phases 1\nmain_unix_ns 1500\nengine 1000\nruntime 3000\nexit 8000\n";
        let durations = durations(text, SPAN).unwrap();
        assert_eq!(
            durations,
            [
                ("start".to_owned(), 500),
                ("engine".to_owned(), 1000),
                ("runtime".to_owned(), 2000),
                ("exit".to_owned(), 5000),
                ("reap".to_owned(), 500),
            ]
        );
        let total: u64 = durations.iter().map(|(_, ns)| ns).sum();
        assert_eq!(u128::from(total), SPAN.reap_unix_ns - SPAN.spawn_unix_ns);
    }

    #[test]
    fn refuses_a_bad_file_or_a_backward_span() {
        for (text, says) in [
            ("", "doesn't start"),
            ("gro_phases 2\nmain_unix_ns 1500\n", "doesn't start"),
            ("gro_phases 1\n", "ends early"),
            ("gro_phases 1\nmain 1500\nexit 1\n", "not `main_unix_ns`"),
            ("gro_phases 1\nmain_unix_ns 1500\n", "names no phase"),
            ("gro_phases 1\nmain_unix_ns 1500\nexit x\n", "the line"),
            ("gro_phases 1\nmain_unix_ns 500\nexit 1\n", "start span"),
            ("gro_phases 1\nmain_unix_ns 1500\na 9\nb 3\n", "b span"),
            ("gro_phases 1\nmain_unix_ns 1500\nexit 9000\n", "reap span"),
        ] {
            let error = durations(text, SPAN).unwrap_err().to_string();
            assert!(error.contains(says), "{text:?}: {error}");
        }
    }
}
