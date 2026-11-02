//! A reading of the history: for each suite, each row's latest record
//! against the record before it and the last attested one.
//!
//! Records compare only within a suite's name and version and within one
//! [`Mode`], since a phases build or a counted process times more than the
//! plain one. For each such series, the report takes its latest record and
//! sets it against the one before it (`previous`) and the last attested one
//! before that (`attested`, the quiet machine's, so never the previous
//! itself), row by row:
//!
//! - wall medians, a move flagged `!` when it's past the noise floor: the
//!   larger of the two records' A/A noise rows ([`NoiseRecord`](crate::record::NoiseRecord)),
//!   or, when larger, the two rows' MADs added over the smaller median, the
//!   bound a prices table calls noise ([`prices`](crate::prices)), since one
//!   A/A draw can land near 0;
//! - instruction medians when both records counted them, a move flagged `!`
//!   past [`INSTRUCTIONS_FLAG`];
//! - the latest record's phase medians when it has them, each beside the
//!   previous record's;
//! - peak memory medians and maxima when the latest record has them, each
//!   median beside the previous record's, flagged past [`RSS_FLAG`], a row
//!   whose peak sits within
//!   [`RSS_FLOOR_MARGIN`] of the run's floor marked as not the runtime's.
//!
//! A move is measured the same either way, as a [`ratio_deviation`]: a 2%
//! rise and a 2% drop are flagged alike.
//!
//! A line it can't read (not JSON, a version newer than this harness, a shape
//! that doesn't parse) is named and skipped, never fatal; a line of an older
//! version reads as a wall run with what it lacks left empty.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::record::{MachineQuiet, Metric, Mode, RECORD_VERSION, RowRecord, RunRecord};
use crate::stats::ratio_deviation;

/// The instruction move a report flags, as a ratio: the perf rule's 2%.
pub const INSTRUCTIONS_FLAG: f64 = 0.02;

/// The peak memory move a report flags, as a ratio: 5%.
pub const RSS_FLAG: f64 = 0.05;

/// How far above the run's peak memory floor a row's peak must sit.
///
/// As a ratio of `rss_floor_kib`, for the row's smallest peak to read as the
/// runtime's own: within it, the sample may be reading the harness's peak,
/// which every spawned process starts from.
pub const RSS_FLOOR_MARGIN: f64 = 1.25;

/// Whether `peak`'s smallest sample sits within [`RSS_FLOOR_MARGIN`] of
/// `floor`, so it may be the spawner's peak rather than the runtime's.
#[expect(
    clippy::cast_precision_loss,
    reason = "peaks in KiB are far below 2^52"
)]
pub fn near_rss_floor(peak: &Metric, floor: Option<u64>) -> bool {
    let least = peak.samples.iter().copied().min().unwrap_or(0);
    floor.is_some_and(|floor| least as f64 <= floor as f64 * RSS_FLOOR_MARGIN)
}

/// Kibibytes in megabytes (10^6), for printing.
pub(crate) fn kib_to_mb(kib: f64) -> f64 {
    kib * 1024.0 / 1e6
}

/// A peak memory row's two cells in MB, as the summary and the report
/// print them: the median, `*` after it when the row sits near the `floor`
/// ([`near_rss_floor`]), and the largest peak.
#[expect(
    clippy::cast_precision_loss,
    reason = "peaks in KiB are far below 2^52"
)]
pub(crate) fn memory_cells(peak: &Metric, floor: Option<u64>) -> (String, String) {
    (
        format!(
            "{:>9.1}{}",
            kib_to_mb(peak.stats.median),
            if near_rss_floor(peak, floor) {
                "*"
            } else {
                " "
            }
        ),
        format!("{:>10.1}", kib_to_mb(peak.max() as f64)),
    )
}

/// Renders the report of the history `text`, one record per line.
pub fn report(text: &str) -> String {
    let mut out = String::new();
    let mut series: BTreeMap<(String, u32, Mode), Vec<RunRecord>> = BTreeMap::new();
    let mut skipped = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        match read_line(line) {
            Ok(record) => {
                let key = (record.suite.name.clone(), record.suite.version, record.mode);
                series.entry(key).or_default().push(record);
            }
            Err(why) => skipped.push(format!("line {number}: {why}")),
        }
    }
    if series.is_empty() {
        out.push_str("no records to report\n");
    }
    for ((name, version, mode), records) in &series {
        write_series(&mut out, name, *version, *mode, records);
    }
    if !skipped.is_empty() {
        out.push_str("\nskipped, unreadable here:\n");
        for line in skipped {
            let _ = writeln!(out, "  {line}");
        }
    }
    out
}

/// One line as a record, or why it can't be read.
fn read_line(line: &str) -> Result<RunRecord, String> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|error| format!("not JSON ({error})"))?;
    let version = value
        .get("record_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "no record_version".to_owned())?;
    if version > u64::from(RECORD_VERSION) {
        return Err(format!(
            "record version {version}, newer than this harness reads ({RECORD_VERSION})"
        ));
    }
    serde_json::from_value(value)
        .map_err(|error| format!("record version {version} doesn't read as a record ({error})"))
}

/// Writes one series: its latest record against the previous and the last
/// attested one before it.
fn write_series(out: &mut String, name: &str, version: u32, mode: Mode, records: &[RunRecord]) {
    let Some((latest, earlier)) = records.split_last() else {
        return;
    };
    let (previous, older) = match earlier.split_last() {
        Some((previous, older)) => (Some(previous), older),
        None => (None, earlier),
    };
    // before the previous, so the column never repeats it
    let attested = older
        .iter()
        .rev()
        .find(|record| record.machine_quiet == MachineQuiet::Attested);
    let _ = writeln!(
        out,
        "\n{name} v{version}, {mode}: {} record{}",
        records.len(),
        if records.len() == 1 { "" } else { "s" }
    );
    let _ = writeln!(out, "  latest:   {}", describe(latest));
    let previous_line = previous.map_or_else(|| "none".to_owned(), describe);
    let _ = writeln!(out, "  previous: {previous_line}");
    let attested_line = attested.map_or_else(|| "none".to_owned(), describe);
    let _ = writeln!(out, "  attested: {attested_line}");
    for (label, other) in [("previous", previous), ("attested", attested)] {
        if let Some(other) = other
            && other.config_hash != latest.config_hash
        {
            let _ = writeln!(
                out,
                "  (the {label} record ran another config of the suite, so its rows may differ)"
            );
        }
        if let Some(other) = other
            && other.machine != latest.machine
        {
            let _ = writeln!(
                out,
                "  (the {label} record ran on another machine: {})",
                other.machine.cpu_model.as_deref().unwrap_or("unknown CPU")
            );
        }
    }
    let width = latest
        .rows
        .iter()
        .map(|row| row.runtime.len())
        .fold("runtime".len(), usize::max);
    let _ = writeln!(
        out,
        "  {:<width$} {:<16} {:>10} {:>10} {:>9} {:>10} {:>9}",
        "runtime", "workload", "latest ms", "prev ms", "x prev", "attest ms", "x attest"
    );
    for row in &latest.rows {
        let cell = |other: Option<&RunRecord>| compare_wall(latest, row, other);
        let (previous_ms, previous_move) = cell(previous);
        let (attested_ms, attested_move) = cell(attested);
        let _ = writeln!(
            out,
            "  {:<width$} {:<16} {:>10.3} {:>10} {:>9} {:>10} {:>9}",
            row.runtime,
            row.workload,
            row.wall_ns.stats.median / 1e6,
            previous_ms,
            previous_move,
            attested_ms,
            attested_move,
        );
    }
    let _ = writeln!(
        out,
        "  (wall medians; `!` marks a move past the noise floor, the larger noise row of the two records or the two rows' MADs added)"
    );
    if latest.rows.iter().any(|row| row.instructions.is_some()) {
        write_instructions(out, latest, previous, width);
    }
    if latest.rows.iter().any(|row| !row.phases.is_empty()) {
        write_phases(out, latest, previous);
    }
    if latest.rows.iter().any(|row| row.peak_rss_kib.is_some()) {
        write_memory(out, latest, previous, width);
    }
}

/// The peak memory table: each row's median and largest peak in MB, the
/// median against the previous record's, a row near the floor marked `*`.
#[expect(
    clippy::cast_precision_loss,
    reason = "peaks in KiB are far below 2^52"
)]
fn write_memory(out: &mut String, latest: &RunRecord, previous: Option<&RunRecord>, width: usize) {
    let mb = kib_to_mb;
    let _ = writeln!(
        out,
        "  {:<width$} {:<16} {:>10} {:>10} {:>10} {:>9}",
        "runtime", "workload", "peak MB", "max MB", "prev MB", "x prev"
    );
    for row in &latest.rows {
        let Some(peak) = &row.peak_rss_kib else {
            continue;
        };
        let other = previous
            .and_then(|previous| find_row(previous, row))
            .and_then(|other| other.peak_rss_kib.as_ref())
            .map(|other| other.stats.median);
        let (median, max) = memory_cells(peak, latest.rss_floor_kib);
        let _ = writeln!(
            out,
            "  {:<width$} {:<16} {median} {max} {:>10} {:>9}",
            row.runtime,
            row.workload,
            other.map_or_else(|| "-".to_owned(), |other| format!("{:.1}", mb(other))),
            other.map_or_else(
                || "-".to_owned(),
                |other| format_move(peak.stats.median, other, RSS_FLAG)
            ),
        );
    }
    let _ = writeln!(
        out,
        "  (peak resident memory per process, median and max; `!` marks a median move past 5%; `*` marks a row within {:.0}% of the floor, {}, which every sample reads at least, so not the runtime's own)",
        (RSS_FLOOR_MARGIN - 1.0) * 100.0,
        latest.rss_floor_kib.map_or_else(
            || "unmeasured".to_owned(),
            |floor| format!("{:.1} MB", mb(floor as f64))
        ),
    );
}

/// A record's one-line description: when, which commit, whether quiet, its
/// noise.
fn describe(record: &RunRecord) -> String {
    format!(
        "{} {}{} {}, noise {}",
        format_date(record.started_unix_s),
        record.git.sha.get(..8).unwrap_or(&record.git.sha),
        if record.git.dirty { "-dirty" } else { "" },
        match record.machine_quiet {
            MachineQuiet::Attested => "attested",
            MachineQuiet::Unchecked => "unchecked",
        },
        record.noise.wall_deviation().map_or_else(
            || "-".to_owned(),
            |deviation| format!("{:.2}%", deviation * 100.0)
        )
    )
}

/// The row's median in `other` and the latest's move against it, flagged
/// past the noise floor ([`wall_floor`]): `-` when `other` is absent or
/// lacks the row.
fn compare_wall(
    latest: &RunRecord,
    row: &RowRecord,
    other: Option<&RunRecord>,
) -> (String, String) {
    let Some((other, other_row)) = other.and_then(|other| Some((other, find_row(other, row)?)))
    else {
        return ("-".to_owned(), "-".to_owned());
    };
    let floor = wall_floor(latest, row, other, other_row);
    (
        format!("{:.3}", other_row.wall_ns.stats.median / 1e6),
        format_move(
            row.wall_ns.stats.median,
            other_row.wall_ns.stats.median,
            floor,
        ),
    )
}

/// The ratio deviation a wall move must pass to be flagged: the larger of
/// the two records' A/A noise rows, each where usable
/// ([`NoiseRecord::wall_deviation`](crate::record::NoiseRecord::wall_deviation)),
/// or of the two rows' MADs added over the smaller median, so a row whose
/// own spread is wider than one A/A draw isn't flagged for it.
fn wall_floor(
    latest: &RunRecord,
    row: &RowRecord,
    other: &RunRecord,
    other_row: &RowRecord,
) -> f64 {
    let (a, b) = (&row.wall_ns.stats, &other_row.wall_ns.stats);
    let least = a.median.min(b.median);
    let spread = if least > 0.0 {
        (a.mad + b.mad) / least
    } else {
        0.0
    };
    [latest.noise.wall_deviation(), other.noise.wall_deviation()]
        .into_iter()
        .flatten()
        .fold(spread, f64::max)
}

/// The row of `record` with `row`'s runtime and workload.
fn find_row<'a>(record: &'a RunRecord, row: &RowRecord) -> Option<&'a RowRecord> {
    record.row(&row.runtime, &row.workload)
}

/// `latest` over `other` as a ratio, `!` appended when it moved by more than
/// `flag` either way, as a [`ratio_deviation`].
fn format_move(latest: f64, other: f64, flag: f64) -> String {
    let ratio = latest / other;
    let moved = ratio_deviation(latest, other) > flag;
    format!("{ratio:.3}x{}", if moved { "!" } else { " " })
}

/// The instruction table: each counted row's median against the previous
/// record's, flagged past [`INSTRUCTIONS_FLAG`], with cycles beside.
fn write_instructions(
    out: &mut String,
    latest: &RunRecord,
    previous: Option<&RunRecord>,
    width: usize,
) {
    let _ = writeln!(
        out,
        "  {:<width$} {:<16} {:>14} {:>14} {:>9} {:>14}",
        "runtime", "workload", "instructions", "prev", "x prev", "cycles"
    );
    let median = |metric: Option<&Metric>| metric.map(|metric| metric.stats.median);
    for row in &latest.rows {
        let Some(instructions) = median(row.instructions.as_ref()) else {
            continue;
        };
        let other = previous
            .and_then(|previous| find_row(previous, row))
            .and_then(|other| median(other.instructions.as_ref()));
        let _ = writeln!(
            out,
            "  {:<width$} {:<16} {:>14.0} {:>14} {:>9} {:>14}",
            row.runtime,
            row.workload,
            instructions,
            other.map_or_else(|| "-".to_owned(), |other| format!("{other:.0}")),
            other.map_or_else(
                || "-".to_owned(),
                |other| format_move(instructions, other, INSTRUCTIONS_FLAG)
            ),
            median(row.cycles.as_ref())
                .map_or_else(|| "-".to_owned(), |cycles| format!("{cycles:.0}")),
        );
    }
    let _ = writeln!(
        out,
        "  (user-space medians; `!` marks an instruction move past {:.0}%; the noise row's instructions sat {} apart)",
        INSTRUCTIONS_FLAG * 100.0,
        latest.noise.instructions_median_deviation.map_or_else(
            || "-".to_owned(),
            |deviation| format!("{:.3}%", deviation * 100.0)
        )
    );
}

/// Each row's phase medians, in milliseconds, with the previous record's
/// beside each in parentheses where it has the phase.
fn write_phases(out: &mut String, latest: &RunRecord, previous: Option<&RunRecord>) {
    let _ = writeln!(
        out,
        "  phase medians in ms (the previous record's in parentheses):"
    );
    for row in latest.rows.iter().filter(|row| !row.phases.is_empty()) {
        let other = previous.and_then(|previous| find_row(previous, row));
        let phases: Vec<String> = row
            .phases
            .iter()
            .map(|phase| {
                let before = other
                    .and_then(|other| other.phases.iter().find(|p| p.name == phase.name))
                    .map(|p| format!(" ({:.3})", p.ns.stats.median / 1e6))
                    .unwrap_or_default();
                format!("{} {:.3}{before}", phase.name, phase.ns.stats.median / 1e6)
            })
            .collect();
        let _ = writeln!(
            out,
            "  {} {}: {}",
            row.runtime,
            row.workload,
            phases.join(", ")
        );
    }
}

/// `unix_s` as a UTC date and time, `2026-11-01 12:00`.
fn format_date(unix_s: u64) -> String {
    let days = unix_s / 86_400;
    let seconds = unix_s % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`, for days on or after the epoch).
const fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{GitState, Machine, NoiseRecord, SuiteId};
    use crate::stats::Stats;

    fn metric(samples: Vec<u64>) -> Metric {
        Metric {
            stats: Stats::from_samples(&samples).unwrap(),
            samples,
        }
    }

    fn record(sha: &str, quiet: MachineQuiet, wall: u64, instructions: Option<u64>) -> RunRecord {
        let row = RowRecord {
            runtime: "fgro".to_owned(),
            workload: "empty".to_owned(),
            warmup: 0,
            wall_ns: metric(vec![wall, wall]),
            instructions: instructions.map(|count| metric(vec![count, count])),
            cycles: instructions.map(|count| metric(vec![count * 2, count * 2])),
            kernel_instructions: instructions.map(|count| metric(vec![count * 3, count * 3])),
            phases: Vec::new(),
            // a peak a tenth of the wall time's nanoseconds, in KiB
            peak_rss_kib: Some(metric(vec![wall / 100, wall / 100 + 1000])),
        };
        RunRecord {
            record_version: RECORD_VERSION,
            mode: if instructions.is_some() {
                Mode::Counters
            } else {
                Mode::Wall
            },
            counters: None,
            suite: SuiteId {
                name: "baseline".to_owned(),
                version: 5,
            },
            config_hash: "c".to_owned(),
            started_unix_s: 1_761_955_200,
            git: GitState {
                sha: sha.repeat(40),
                dirty: false,
            },
            machine: Machine {
                cpu_model: None,
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            },
            machine_quiet: quiet,
            corpora: None,
            runtimes: Vec::new(),
            artifacts: BTreeMap::new(),
            checkouts: BTreeMap::new(),
            noise: NoiseRecord {
                runtime: row.runtime.clone(),
                workload: row.workload.clone(),
                wall_ns: row.wall_ns.clone(),
                median_deviation: 0.01,
                instructions: None,
                instructions_median_deviation: None,
            },
            rows: vec![row],
            rss_floor_kib: Some(10_000),
            snapshot_bytes: None,
        }
    }

    #[test]
    fn compares_the_latest_against_the_previous_and_the_last_attested() {
        let lines = [
            record("a", MachineQuiet::Attested, 10_000_000, None),
            record("b", MachineQuiet::Unchecked, 10_050_000, None),
            record("c", MachineQuiet::Unchecked, 12_000_000, None),
        ]
        .map(|record| record.to_line())
        .concat();
        let report = report(&lines);
        assert!(report.contains("baseline v5, wall: 3 records"), "{report}");
        assert!(
            report.contains("latest:   2025-11-01 00:00 cccccccc unchecked"),
            "{report}"
        );
        assert!(
            report.contains("attested: 2025-11-01 00:00 aaaaaaaa attested"),
            "{report}"
        );
        // 12 against 10.05 is past the 1% floor, flagged; so is 12 against 10
        assert!(report.contains("1.194x!"), "{report}");
        assert!(report.contains("1.200x!"), "{report}");
        // the peaks, none near the floor
        assert!(report.contains("peak MB"), "{report}");
        assert!(report.contains("123.4 "), "{report}");
        // a peak at the floor is marked
        let mut near = record("d", MachineQuiet::Unchecked, 1_000_000, None);
        if let Some(peak) = &mut near.rows[0].peak_rss_kib {
            *peak = metric(vec![10_500, 30_000]);
        }
        let near = super::report(&near.to_line());
        assert!(near.contains("20.7*"), "{near}");
        // an attested previous isn't the attested column too
        let lines = [
            record("a", MachineQuiet::Attested, 10_000_000, None),
            record("b", MachineQuiet::Attested, 10_000_000, None),
        ]
        .map(|record| record.to_line())
        .concat();
        let report = super::report(&lines);
        assert!(
            report.contains("previous: 2025-11-01 00:00 aaaaaaaa"),
            "{report}"
        );
        assert!(report.contains("attested: none"), "{report}");
    }

    #[test]
    fn flags_an_instruction_move_past_two_percent() {
        let lines = [
            record("a", MachineQuiet::Unchecked, 10_000_000, Some(1_000_000)),
            record("b", MachineQuiet::Unchecked, 10_000_000, Some(1_010_000)),
        ]
        .map(|record| record.to_line())
        .concat();
        let report = report(&lines);
        assert!(report.contains("baseline v5, counters"), "{report}");
        assert!(report.contains("1.010x "), "{report}");
        let lines = [
            record("a", MachineQuiet::Unchecked, 10_000_000, Some(1_000_000)),
            record("b", MachineQuiet::Unchecked, 10_000_000, Some(1_030_000)),
        ]
        .map(|record| record.to_line())
        .concat();
        assert!(super::report(&lines).contains("1.030x!"));
    }

    #[test]
    fn a_rise_and_a_drop_of_one_size_are_flagged_alike() {
        // within 2% either way
        assert_eq!(format_move(1.0, 1.0198, INSTRUCTIONS_FLAG), "0.981x ");
        assert_eq!(format_move(1.0198, 1.0, INSTRUCTIONS_FLAG), "1.020x ");
        // past it either way: the drop's `|ratio - 1|` is under 2% (1.9992%)
        assert_eq!(format_move(1.0, 1.0204, INSTRUCTIONS_FLAG), "0.980x!");
        assert_eq!(format_move(1.0204, 1.0, INSTRUCTIONS_FLAG), "1.020x!");
    }

    #[test]
    fn a_wall_move_within_the_rows_mads_is_not_flagged() {
        let mut previous = record("a", MachineQuiet::Unchecked, 10_000_000, None);
        let mut latest = record("b", MachineQuiet::Unchecked, 10_300_000, None);
        // the A/A draws landed close, but each row spreads 2% sample to sample
        for (record, wall) in [(&mut previous, 10_000_000), (&mut latest, 10_300_000)] {
            record.noise.median_deviation = 0.001;
            record.rows[0].wall_ns = metric(vec![wall - 200_000, wall, wall + 200_000]);
        }
        let lines = [&previous, &latest].map(RunRecord::to_line).concat();
        let report = report(&lines);
        assert!(report.contains("1.030x "), "{report}");
        // a move past the MADs is flagged
        latest.rows[0].wall_ns = metric(vec![10_400_000, 10_600_000, 10_800_000]);
        let lines = [&previous, &latest].map(RunRecord::to_line).concat();
        assert!(super::report(&lines).contains("1.060x!"));
    }

    /// An A/A row with a zero median, whose deviation is `f64::MAX`, is no
    /// noise row: it neither disables flagging nor prints as a percent.
    #[test]
    fn an_unusable_noise_row_is_left_out() {
        let mut previous = record("a", MachineQuiet::Unchecked, 10_000_000, None);
        let mut latest = record("b", MachineQuiet::Unchecked, 10_600_000, None);
        for record in [&mut previous, &mut latest] {
            record.noise.median_deviation = f64::MAX;
        }
        let lines = [&previous, &latest].map(RunRecord::to_line).concat();
        let report = report(&lines);
        assert!(report.contains("1.060x!"), "{report}");
        assert!(
            describe(&latest).ends_with(", noise -"),
            "{}",
            describe(&latest)
        );
        latest.noise.median_deviation = 0.001;
        assert!(
            describe(&latest).ends_with(", noise 0.10%"),
            "{}",
            describe(&latest)
        );
    }

    #[test]
    fn modes_compare_apart_and_unreadable_lines_are_named() {
        let mut lines = [
            record("a", MachineQuiet::Unchecked, 10_000_000, None),
            record("b", MachineQuiet::Unchecked, 10_000_000, Some(1)),
        ]
        .map(|record| record.to_line())
        .concat();
        lines.push_str("not json\n");
        lines.push_str("{\"record_version\": 99}\n");
        lines.push_str("{\"record_version\": 2, \"suite\": 1}\n");
        let report = report(&lines);
        assert!(report.contains("baseline v5, wall: 1 record\n"), "{report}");
        assert!(
            report.contains("baseline v5, counters: 1 record\n"),
            "{report}"
        );
        assert!(report.contains("line 3: not JSON"), "{report}");
        assert!(
            report.contains("line 4: record version 99, newer"),
            "{report}"
        );
        assert!(
            report.contains("line 5: record version 2 doesn't read as a record"),
            "{report}"
        );
    }

    #[test]
    fn the_history_reads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("history.jsonl");
        let report = report(&std::fs::read_to_string(path).unwrap());
        assert!(!report.contains("skipped"), "{report}");
    }

    #[test]
    fn dates_are_utc() {
        assert_eq!(format_date(0), "1970-01-01 00:00");
        assert_eq!(format_date(951_782_400), "2000-02-29 00:00");
        assert_eq!(format_date(1_761_955_200 + 3_660), "2025-11-01 01:01");
    }
}
