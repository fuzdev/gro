//! What a run prints after its samples: the summary of every row, the gate
//! table, and an A/B's moves, each rendered as text for the caller to print.

use std::fmt::Write as _;

use crate::record::{Metric, RunRecord};
use crate::report::{INSTRUCTIONS_FLAG, RSS_FLOOR_MARGIN, kib_to_mb, memory_cells};
use crate::stats::ratio_deviation;
use crate::suite::{Gate, Runtime, Suite};

/// The incumbent a warm row gates against, Node at its best configuration.
pub const GATE_WARM: &str = "node_compile_cache";
/// The incumbent a cold row gates against.
pub const GATE_COLD: &str = "node";
/// Reported beside the gate.
pub const BESIDE: &str = "deno";

/// An A/B's table, and how many of its rows it flagged.
///
/// Each row's median instructions, cycles, and wall time against its base
/// copy's (the runtime named with `base_suffix`), an instruction move past
/// [`INSTRUCTIONS_FLAG`] (the perf rule's 2%) either way, as a
/// [`ratio_deviation`], flagged.
pub fn ab_table(record: &RunRecord, base_suffix: &str) -> (String, usize) {
    let mut out = String::new();
    let width = runtime_width(record);
    let _ = writeln!(
        out,
        "{:<width$} {:<16} {:>14} {:>14} {:>9} {:>9} {:>9}",
        "runtime", "workload", "instructions", "base", "x instr", "x cycles", "x wall"
    );
    let mut flagged = 0;
    for row in &record.rows {
        let Some(base) = record.row(&format!("{}{base_suffix}", row.runtime), &row.workload) else {
            continue;
        };
        let median = |metric: Option<&Metric>| metric.map(|metric| metric.stats.median);
        let both = |head: Option<&Metric>, base: Option<&Metric>| median(head).zip(median(base));
        let instructions = both(row.instructions.as_ref(), base.instructions.as_ref());
        // either way alike, as the report measures a move
        let moved = instructions
            .is_some_and(|(head, base)| ratio_deviation(head, base) > INSTRUCTIONS_FLAG);
        if moved {
            flagged += 1;
        }
        // `-` for a count a row lacks, never `NaN`
        let count =
            |value: Option<f64>| value.map_or_else(|| "-".to_owned(), |v| format!("{v:.0}"));
        let ratio = |pair: Option<(f64, f64)>| {
            pair.map_or_else(
                || "-".to_owned(),
                |(head, base)| format!("{:.4}x", head / base),
            )
        };
        let _ = writeln!(
            out,
            "{:<width$} {:<16} {:>14} {:>14} {:>9}{} {:>9} {:>8.4}x",
            row.runtime,
            row.workload,
            count(instructions.map(|(head, _)| head)),
            count(median(base.instructions.as_ref())),
            ratio(instructions),
            if moved { "!" } else { " " },
            ratio(both(row.cycles.as_ref(), base.cycles.as_ref())),
            row.wall_ns.stats.median / base.wall_ns.stats.median,
        );
    }
    let _ = writeln!(
        out,
        "(user-space medians, this checkout over the base; the noise row's instructions sat {} apart; `!` marks a move past {:.0}%, which needs a written reason)",
        record.noise.instructions_median_deviation.map_or_else(
            || "-".to_owned(),
            |deviation| format!("{:.3}%", deviation * 100.0)
        ),
        INSTRUCTIONS_FLAG * 100.0
    );
    (out, flagged)
}

/// The run's summary: each row's wall statistics, then its counts, phase
/// medians, and peak memory when the run has them, the runtimes' versions
/// and sizes, and the noise row.
pub fn summary(record: &RunRecord) -> String {
    let mut out = String::new();
    write_wall(&mut out, record);
    write_counts(&mut out, record);
    write_phases(&mut out, record);
    write_memory(&mut out, record);
    write_sizes(&mut out, record);
    write_noise(&mut out, record);
    out
}

/// Writes each row's wall-time statistics.
fn write_wall(out: &mut String, record: &RunRecord) {
    let width = runtime_width(record);
    let _ = writeln!(
        out,
        "\n{:<width$} {:<16} {:>12} {:>10} {:>7} {:>12} {:>9}",
        "runtime", "workload", "median ms", "mad ms", "cv", "min ms", "outliers"
    );
    for row in &record.rows {
        let stats = &row.wall_ns.stats;
        let _ = writeln!(
            out,
            "{:<width$} {:<16} {:>12.3} {:>10.3} {:>6.1}% {:>12.3} {:>6}/{}",
            row.runtime,
            row.workload,
            stats.median / 1e6,
            stats.mad / 1e6,
            stats.cv * 100.0,
            stats.min / 1e6,
            stats.outliers,
            stats.n,
        );
    }
}

/// Writes each row's counter medians, when the run counted.
fn write_counts(out: &mut String, record: &RunRecord) {
    if !record.rows.iter().any(|row| row.instructions.is_some()) {
        return;
    }
    let width = runtime_width(record);
    let _ = writeln!(
        out,
        "\n{:<width$} {:<16} {:>14} {:>9} {:>14} {:>14} {:>14}",
        "runtime", "workload", "instructions", "cv", "min", "cycles", "kernel instr"
    );
    for row in &record.rows {
        let (Some(instructions), Some(cycles)) = (&row.instructions, &row.cycles) else {
            continue;
        };
        let _ = writeln!(
            out,
            "{:<width$} {:<16} {:>14.0} {:>8.3}% {:>14.0} {:>14.0} {:>14}",
            row.runtime,
            row.workload,
            instructions.stats.median,
            instructions.stats.cv * 100.0,
            instructions.stats.min,
            cycles.stats.median,
            row.kernel_instructions
                .as_ref()
                .map_or_else(|| "-".to_owned(), |k| format!("{:.0}", k.stats.median)),
        );
    }
    let _ = writeln!(
        out,
        "(medians under perf stat, user space but the kernel's instructions; the wall times above include perf's startup)"
    );
}

/// Writes each row's phase medians, when the run timed phases.
fn write_phases(out: &mut String, record: &RunRecord) {
    if !record.rows.iter().any(|row| !row.phases.is_empty()) {
        return;
    }
    let _ = writeln!(
        out,
        "\nphase medians in ms (start: spawn to entry; reap: the last mark to the reaped exit)"
    );
    for row in record.rows.iter().filter(|row| !row.phases.is_empty()) {
        let phases: Vec<String> = row
            .phases
            .iter()
            .map(|phase| format!("{} {:.3}", phase.name, phase.ns.stats.median / 1e6))
            .collect();
        let _ = writeln!(
            out,
            "{} {}: {}",
            row.runtime,
            row.workload,
            phases.join(", ")
        );
    }
}

/// Writes each row's peak memory, when the run read it.
fn write_memory(out: &mut String, record: &RunRecord) {
    if !record.rows.iter().any(|row| row.peak_rss_kib.is_some()) {
        return;
    }
    let width = runtime_width(record);
    let _ = writeln!(
        out,
        "\n{:<width$} {:<16} {:>10} {:>10}",
        "runtime", "workload", "peak MB", "max MB"
    );
    for row in &record.rows {
        let Some(peak) = &row.peak_rss_kib else {
            continue;
        };
        let (median, max) = memory_cells(peak, record.rss_floor_kib);
        let _ = writeln!(
            out,
            "{:<width$} {:<16} {median} {max}",
            row.runtime, row.workload,
        );
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "peaks in KiB are far below 2^52"
    )]
    let floor = record.rss_floor_kib.map_or_else(
        || "-".to_owned(),
        |kib| format!("{:.1} MB", kib_to_mb(kib as f64)),
    );
    let _ = writeln!(
        out,
        "(peak resident memory per process, median and max, as reaped; the floor every sample reads at least is {floor}, and `*` marks a row within {:.0}% of it, so not the runtime's own)",
        (RSS_FLOOR_MARGIN - 1.0) * 100.0
    );
}

/// Writes each runtime's version and sizes, and the snapshot's.
fn write_sizes(out: &mut String, record: &RunRecord) {
    let width = runtime_width(record);
    // the version column fits the longest line, fgro's with its full commit
    let version_width = record
        .runtimes
        .iter()
        .map(|runtime| runtime.version.len())
        .fold("version".len(), usize::max);
    let _ = writeln!(
        out,
        "\n{:<width$} {:<version_width$} {:>10} {:>10}",
        "runtime", "version", "MB", "gzip MB"
    );
    for runtime in &record.runtimes {
        let _ = writeln!(
            out,
            "{:<width$} {:<version_width$} {:>10.1} {:>10.1}",
            runtime.name,
            runtime.version,
            mb(runtime.bytes),
            mb(runtime.gzip_bytes),
        );
    }
    if let Some(bytes) = record.snapshot_bytes {
        let _ = writeln!(out, "fgro's startup snapshot: {:.1} MB", mb(bytes));
    }
}

/// Writes how far the A/A noise row's medians sit apart.
fn write_noise(out: &mut String, record: &RunRecord) {
    let noise = &record.noise;
    let _ = writeln!(
        out,
        "\nnoise ({} {} against its copy): the medians sit {:.2}% apart{}",
        noise.runtime,
        noise.workload,
        noise.median_deviation * 100.0,
        noise
            .instructions_median_deviation
            .map_or_else(String::new, |deviation| format!(
                ", the instruction medians {:.3}%",
                deviation * 100.0
            )),
    );
}

/// The gate table, report only: nothing fails on it.
///
/// It is the suite's own `gate` when it has one, else each row of a runtime
/// `is_subject` picks (`fgro`'s), its median against its gate row's,
/// same-session: warm rows gate against Node with its compile cache
/// ([`GATE_WARM`]), cold rows against plain Node ([`GATE_COLD`]), with Deno
/// beside ([`BESIDE`]). Empty when no row is gated.
pub fn gate(suite: &Suite, record: &RunRecord, is_subject: impl Fn(&Runtime) -> bool) -> String {
    if let Some(gate) = &suite.gate {
        return suite_gate(suite, gate, record);
    }
    let mut out = String::new();
    let width = runtime_width(record);
    let median = |runtime: &str, workload: &str| median_ms(record, runtime, workload);
    let ms =
        |value: Option<f64>| value.map_or_else(|| "-".to_owned(), |value| format!("{value:.3}"));
    let mut lines = Vec::new();
    for (workload, runtime) in suite.rows().filter(|(_, runtime)| is_subject(runtime)) {
        let Some(subject) = median(&runtime.name, &workload.name) else {
            continue;
        };
        let gate = if workload.cold { GATE_COLD } else { GATE_WARM };
        let ratio = |other: Option<f64>| {
            other.map_or_else(
                || "-".to_owned(),
                |other| format!("{:.2}x", subject / other),
            )
        };
        lines.push(format!(
            "{:<width$} {:<16} {:>10.3} {:<18} {:>10} {:>9} {:>10} {:>9}",
            runtime.name,
            workload.name,
            subject,
            gate,
            ms(median(gate, &workload.name)),
            ratio(median(gate, &workload.name)),
            ms(median(BESIDE, &workload.name)),
            ratio(median(BESIDE, &workload.name)),
        ));
    }
    if lines.is_empty() {
        return out;
    }
    let _ = writeln!(
        out,
        "\n{:<width$} {:<16} {:>10} {:<18} {:>10} {:>9} {:>10} {:>9}",
        "runtime", "workload", "median ms", "gate", "gate ms", "x gate", "deno ms", "x deno"
    );
    for line in lines {
        let _ = writeln!(out, "{line}");
    }
    write_same_session(&mut out, record);
    out
}

/// The runtime column's width, which fits the longest runtime name.
fn runtime_width(record: &RunRecord) -> usize {
    record
        .runtimes
        .iter()
        .map(|runtime| runtime.name.len())
        .fold("runtime".len(), usize::max)
}

/// The median of the row `runtime` × `workload`, in milliseconds.
fn median_ms(record: &RunRecord, runtime: &str, workload: &str) -> Option<f64> {
    record
        .row(runtime, workload)
        .map(|row| row.wall_ns.stats.median / 1e6)
}

/// A suite's own `gate`: each of its runtimes on every workload against
/// `gate.against`, marked `gate` on the workloads it names and `report` on
/// the rest, then the runtimes reported only. Report only.
fn suite_gate(suite: &Suite, gate: &Gate, record: &RunRecord) -> String {
    let mut out = String::new();
    let width = runtime_width(record);
    let ms =
        |value: Option<f64>| value.map_or_else(|| "-".to_owned(), |value| format!("{value:.3}"));
    let _ = writeln!(
        out,
        "\n{:<width$} {:<16} {:>10} {:<10} {:>10} {:>9} {:<6}",
        "runtime", "workload", "median ms", "against", "its ms", "x it", "row"
    );
    for runtime in &gate.runtimes {
        for workload in &suite.workloads {
            let median = median_ms(record, runtime, &workload.name);
            let against = median_ms(record, &gate.against, &workload.name);
            let ratio = median
                .zip(against)
                .map_or_else(|| "-".to_owned(), |(a, b)| format!("{:.2}x", a / b));
            let role = if gate.workloads.contains(&workload.name) {
                "gate"
            } else {
                "report"
            };
            let _ = writeln!(
                out,
                "{:<width$} {:<16} {:>10} {:<10} {:>10} {:>9} {:<6}",
                runtime,
                workload.name,
                ms(median),
                gate.against,
                ms(against),
                ratio,
                role,
            );
        }
    }
    let reported: Vec<&str> = suite
        .runtimes
        .iter()
        .map(|runtime| runtime.name.as_str())
        .filter(|name| *name != gate.against && !gate.runtimes.iter().any(|gated| gated == name))
        .collect();
    if !reported.is_empty() {
        let _ = writeln!(
            out,
            "reported only, in the summary above: {}",
            reported.join(", ")
        );
    }
    write_same_session(&mut out, record);
    out
}

/// The gate tables' footer: their medians are one session's, beside the
/// noise row.
fn write_same_session(out: &mut String, record: &RunRecord) {
    let _ = writeln!(
        out,
        "(medians, same session; the noise row ({} {} against its copy) sat {:.2}% apart, and each row's mad above bounds its own spread)",
        record.noise.runtime,
        record.noise.workload,
        record.noise.median_deviation * 100.0
    );
}

/// Bytes in megabytes (10^6), for printing.
#[expect(
    clippy::cast_precision_loss,
    reason = "a binary's size is far below 2^52 bytes"
)]
fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1e6
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::record::{
        GitState, Machine, MachineQuiet, Mode, NoiseRecord, RECORD_VERSION, RowRecord, RuntimeInfo,
        SuiteId,
    };
    use crate::stats::Stats;

    fn metric(samples: Vec<u64>) -> Metric {
        Metric {
            stats: Stats::from_samples(&samples).unwrap(),
            samples,
        }
    }

    /// A row of `runtime` on `workload` at `ms` milliseconds, counted at
    /// `instructions` when given, its peak `kib`.
    fn row(runtime: &str, workload: &str, ms: u64, instructions: Option<u64>) -> RowRecord {
        let ns = ms * 1_000_000;
        RowRecord {
            runtime: runtime.to_owned(),
            workload: workload.to_owned(),
            warmup: 1,
            wall_ns: metric(vec![ns - 1000, ns, ns + 1000, ns]),
            instructions: instructions.map(|count| metric(vec![count, count + 10])),
            cycles: instructions.map(|count| metric(vec![count * 2, count * 2])),
            kernel_instructions: None,
            phases: Vec::new(),
            peak_rss_kib: instructions
                .is_none()
                .then(|| metric(vec![ms * 1000, ms * 1000 + 500])),
        }
    }

    fn runtime(name: &str) -> RuntimeInfo {
        RuntimeInfo {
            name: name.to_owned(),
            version: format!("{name} 1.0"),
            v8: None,
            blake3: "ab".repeat(32),
            bytes: 100_000_000,
            gzip_bytes: 40_000_000,
        }
    }

    fn record(rows: Vec<RowRecord>, mode: Mode) -> RunRecord {
        let mut names: Vec<&str> = Vec::new();
        for row in &rows {
            if !names.contains(&row.runtime.as_str()) {
                names.push(&row.runtime);
            }
        }
        RunRecord {
            record_version: RECORD_VERSION,
            mode,
            counters: None,
            suite: SuiteId {
                name: "baseline".to_owned(),
                version: 1,
            },
            config_hash: String::new(),
            started_unix_s: 0,
            git: GitState {
                sha: "0".repeat(40),
                dirty: false,
            },
            machine: Machine {
                cpu_model: None,
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            },
            machine_quiet: MachineQuiet::Unchecked,
            corpora: None,
            runtimes: names.iter().map(|name| runtime(name)).collect(),
            artifacts: BTreeMap::new(),
            checkouts: BTreeMap::new(),
            noise: NoiseRecord {
                runtime: rows[0].runtime.clone(),
                workload: rows[0].workload.clone(),
                wall_ns: rows[0].wall_ns.clone(),
                median_deviation: 0.0125,
                instructions: None,
                instructions_median_deviation: rows[0].instructions.as_ref().map(|_| 0.0004),
            },
            rows,
            rss_floor_kib: (mode == Mode::Wall).then_some(9_000),
            snapshot_bytes: Some(7_000_000),
        }
    }

    fn suite(gate: &str) -> Suite {
        serde_json::from_str(&format!(
            r#"{{"name": "baseline", "version": 1, "samples": 4,
                "runtimes": [
                    {{"name": "fgro", "program": "fgro"}},
                    {{"name": "node_compile_cache", "program": "node"}},
                    {{"name": "node", "program": "node"}},
                    {{"name": "deno", "program": "deno"}}
                ],
                "workloads": [
                    {{"name": "warm", "script": "w.js"}},
                    {{"name": "cold", "script": "c.js", "cold": true}}
                ]{gate}}}"#
        ))
        .unwrap()
    }

    fn wall_record() -> RunRecord {
        let mut rows = Vec::new();
        for (workload, base) in [("warm", 10), ("cold", 20)] {
            for (runtime, ms) in [
                ("fgro", 0),
                ("node_compile_cache", 15),
                ("node", 30),
                ("deno", 7),
            ] {
                rows.push(row(runtime, workload, base + ms, None));
            }
        }
        record(rows, Mode::Wall)
    }

    fn counted_record() -> RunRecord {
        record(
            vec![
                row("fgro", "warm", 10, Some(1_000_000)),
                row("fgro_base", "warm", 10, Some(1_030_000)),
                row("fgro", "cold", 20, Some(2_000_000)),
                row("fgro_base", "cold", 20, Some(2_010_000)),
            ],
            Mode::Counters,
        )
    }

    const SUITE_GATE: &str =
        r#", "gate": {"runtimes": ["fgro"], "against": "deno", "workloads": ["warm"]}"#;

    #[test]
    fn the_summary_is_pinned() {
        assert_eq!(summary(&wall_record()), SUMMARY);
        assert_eq!(summary(&counted_record()), COUNTED_SUMMARY);
    }

    #[test]
    fn the_gate_tables_are_pinned() {
        let fgro = |runtime: &Runtime| runtime.name == "fgro";
        assert_eq!(gate(&suite(""), &wall_record(), fgro), GATE);
        assert_eq!(
            gate(&suite(SUITE_GATE), &wall_record(), fgro),
            SUITE_GATE_TABLE
        );
        // no row gated, no table
        assert_eq!(gate(&suite(""), &wall_record(), |_| false), "");
    }

    #[test]
    fn the_ab_table_is_pinned_and_counts_its_flags() {
        assert_eq!(ab_table(&counted_record(), "_base"), (AB.to_owned(), 1));
    }

    /// A row lacking counts prints `-` where they'd go, never `NaN`, and
    /// isn't flagged.
    #[test]
    fn an_ab_row_lacking_counts_prints_a_dash() {
        let mut record = counted_record();
        record.rows[2].instructions = None;
        record.rows[2].cycles = None;
        let (table, flagged) = ab_table(&record, "_base");
        assert_eq!(flagged, 1);
        assert!(!table.contains("NaN"), "{table}");
        let cold = table
            .lines()
            .find(|line| line.starts_with("fgro      cold"))
            .unwrap();
        assert_eq!(
            cold,
            "fgro      cold                          -        2010005         -          -   1.0000x"
        );
    }

    const SUMMARY: &str = r"
runtime            workload            median ms     mad ms      cv       min ms  outliers
fgro               warm                   10.000      0.001    0.0%        9.999      0/4
node_compile_cache warm                   25.000      0.001    0.0%       24.999      0/4
node               warm                   40.000      0.001    0.0%       39.999      0/4
deno               warm                   17.000      0.001    0.0%       16.999      0/4
fgro               cold                   20.000      0.001    0.0%       19.999      0/4
node_compile_cache cold                   35.000      0.001    0.0%       34.999      0/4
node               cold                   50.000      0.001    0.0%       49.999      0/4
deno               cold                   27.000      0.001    0.0%       26.999      0/4

runtime            workload            peak MB     max MB
fgro               warm                  10.5*       10.8
node_compile_cache warm                  25.9        26.1
node               warm                  41.2        41.5
deno               warm                  17.7        17.9
fgro               cold                  20.7        21.0
node_compile_cache cold                  36.1        36.4
node               cold                  51.5        51.7
deno               cold                  27.9        28.2
(peak resident memory per process, median and max, as reaped; the floor every sample reads at least is 9.2 MB, and `*` marks a row within 25% of it, so not the runtime's own)

runtime            version                        MB    gzip MB
fgro               fgro 1.0                    100.0       40.0
node_compile_cache node_compile_cache 1.0      100.0       40.0
node               node 1.0                    100.0       40.0
deno               deno 1.0                    100.0       40.0
fgro's startup snapshot: 7.0 MB

noise (fgro warm against its copy): the medians sit 1.25% apart
";

    const COUNTED_SUMMARY: &str = r"
runtime   workload            median ms     mad ms      cv       min ms  outliers
fgro      warm                   10.000      0.001    0.0%        9.999      0/4
fgro_base warm                   10.000      0.001    0.0%        9.999      0/4
fgro      cold                   20.000      0.001    0.0%       19.999      0/4
fgro_base cold                   20.000      0.001    0.0%       19.999      0/4

runtime   workload           instructions        cv            min         cycles   kernel instr
fgro      warm                    1000005    0.001%        1000000        2000000              -
fgro_base warm                    1030005    0.001%        1030000        2060000              -
fgro      cold                    2000005    0.000%        2000000        4000000              -
fgro_base cold                    2010005    0.000%        2010000        4020000              -
(medians under perf stat, user space but the kernel's instructions; the wall times above include perf's startup)

runtime   version               MB    gzip MB
fgro      fgro 1.0           100.0       40.0
fgro_base fgro_base 1.0      100.0       40.0
fgro's startup snapshot: 7.0 MB

noise (fgro warm against its copy): the medians sit 1.25% apart, the instruction medians 0.040%
";

    const GATE: &str = r"
runtime            workload          median ms gate                  gate ms    x gate    deno ms    x deno
fgro               warm                 10.000 node_compile_cache     25.000     0.40x     17.000     0.59x
fgro               cold                 20.000 node                   50.000     0.40x     27.000     0.74x
(medians, same session; the noise row (fgro warm against its copy) sat 1.25% apart, and each row's mad above bounds its own spread)
";

    const SUITE_GATE_TABLE: &str = r"
runtime            workload          median ms against        its ms      x it row   
fgro               warm                 10.000 deno           17.000     0.59x gate  
fgro               cold                 20.000 deno           27.000     0.74x report
reported only, in the summary above: node_compile_cache, node
(medians, same session; the noise row (fgro warm against its copy) sat 1.25% apart, and each row's mad above bounds its own spread)
";

    const AB: &str = r"runtime   workload           instructions           base   x instr  x cycles    x wall
fgro      warm                    1000005        1030005   0.9709x!   0.9709x   1.0000x
fgro      cold                    2000005        2010005   0.9950x    0.9950x   1.0000x
(user-space medians, this checkout over the base; the noise row's instructions sat 0.040% apart; `!` marks a move past 2%, which needs a written reason)
";
}
