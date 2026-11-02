//! What a bench prints: the run's summary, the gate table, and an A/B's moves.

use gro_bench::record::RunRecord;
use gro_bench::report::INSTRUCTIONS_FLAG;
use gro_bench::suite::{Gate, Suite};

use super::{BASE_SUFFIX, BaseRun, is_fgro};

/// Prints an A/B's moves: each fgro row's median instructions, cycles, and
/// wall time against its base copy's, an instruction move past
/// [`INSTRUCTIONS_FLAG`] (the perf rule's 2%) flagged.
pub(super) fn print_base(record: &RunRecord, base: &BaseRun) {
    println!(
        "\n[xtask] A/B: this checkout ({}{}) against --base {} ({}), same session, interleaved",
        record.git.sha.get(..12).unwrap_or(&record.git.sha),
        if record.git.dirty { "-dirty" } else { "" },
        base.rev,
        base.sha,
    );
    println!(
        "[xtask] toolchains: this checkout {}, the base {}",
        base.head_rustc, base.rustc
    );
    println!(
        "{:<10} {:<16} {:>14} {:>14} {:>9} {:>9} {:>9}",
        "runtime", "workload", "instructions", "base", "x instr", "x cycles", "x wall"
    );
    let mut flagged = 0;
    for row in &record.rows {
        let Some(base) = record.rows.iter().find(|base| {
            base.workload == row.workload && base.runtime == format!("{}{BASE_SUFFIX}", row.runtime)
        }) else {
            continue;
        };
        let median = |metric: Option<&gro_bench::record::Metric>| {
            metric.map_or(f64::NAN, |metric| metric.stats.median)
        };
        let instructions = median(row.instructions.as_ref());
        let base_instructions = median(base.instructions.as_ref());
        let ratio = instructions / base_instructions;
        let moved = (ratio - 1.0).abs() > INSTRUCTIONS_FLAG;
        if moved {
            flagged += 1;
        }
        println!(
            "{:<10} {:<16} {:>14.0} {:>14.0} {:>8.4}x{} {:>8.4}x {:>8.4}x",
            row.runtime,
            row.workload,
            instructions,
            base_instructions,
            ratio,
            if moved { "!" } else { " " },
            median(row.cycles.as_ref()) / median(base.cycles.as_ref()),
            row.wall_ns.stats.median / base.wall_ns.stats.median,
        );
    }
    println!(
        "(user-space medians, this checkout over the base; the noise row's instructions sat {} apart; `!` marks a move past {:.0}%, which needs a written reason)",
        record.noise.instructions_median_deviation.map_or_else(
            || "-".to_owned(),
            |deviation| format!("{:.3}%", deviation * 100.0)
        ),
        INSTRUCTIONS_FLAG * 100.0
    );
    if flagged > 0 {
        println!("[xtask] {flagged} row(s) moved past the perf rule");
    }
}

pub(super) fn print_summary(record: &RunRecord) {
    // the runtime column fits the longest name
    let width = record
        .runtimes
        .iter()
        .map(|runtime| runtime.name.len())
        .fold("runtime".len(), usize::max);
    println!(
        "\n{:<width$} {:<16} {:>12} {:>10} {:>7} {:>12} {:>9}",
        "runtime", "workload", "median ms", "mad ms", "cv", "min ms", "outliers"
    );
    for row in &record.rows {
        let stats = &row.wall_ns.stats;
        println!(
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
    if record.rows.iter().any(|row| row.instructions.is_some()) {
        println!(
            "\n{:<width$} {:<16} {:>14} {:>9} {:>14} {:>14} {:>14}",
            "runtime", "workload", "instructions", "cv", "min", "cycles", "kernel instr"
        );
        for row in &record.rows {
            let (Some(instructions), Some(cycles)) = (&row.instructions, &row.cycles) else {
                continue;
            };
            println!(
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
        println!(
            "(medians under perf stat, user space but the kernel's instructions; the wall times above include perf's startup)"
        );
    }
    if record.rows.iter().any(|row| !row.phases.is_empty()) {
        println!(
            "\nphase medians in ms (start: spawn to entry; reap: the last mark to the reaped exit)"
        );
        for row in record.rows.iter().filter(|row| !row.phases.is_empty()) {
            let phases: Vec<String> = row
                .phases
                .iter()
                .map(|phase| format!("{} {:.3}", phase.name, phase.ns.stats.median / 1e6))
                .collect();
            println!("{} {}: {}", row.runtime, row.workload, phases.join(", "));
        }
    }
    if record.rows.iter().any(|row| row.peak_rss_kib.is_some()) {
        let mb_of = |kib: f64| kib * 1024.0 / 1e6;
        println!(
            "\n{:<width$} {:<16} {:>10} {:>10}",
            "runtime", "workload", "peak MB", "max MB"
        );
        for row in &record.rows {
            let Some(peak) = &row.peak_rss_kib else {
                continue;
            };
            #[expect(
                clippy::cast_precision_loss,
                reason = "peaks in KiB are far below 2^52"
            )]
            let max = peak.max() as f64;
            println!(
                "{:<width$} {:<16} {:>9.1}{} {:>10.1}",
                row.runtime,
                row.workload,
                mb_of(peak.stats.median),
                if gro_bench::report::near_rss_floor(peak, record.rss_floor_kib) {
                    "*"
                } else {
                    " "
                },
                mb_of(max),
            );
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "peaks in KiB are far below 2^52"
        )]
        let floor = record.rss_floor_kib.map_or_else(
            || "-".to_owned(),
            |kib| format!("{:.1} MB", mb_of(kib as f64)),
        );
        println!(
            "(peak resident memory per process, median and max, as reaped; the floor every sample reads at least is {floor}, and `*` marks a row within {:.0}% of it, so not the runtime's own)",
            (gro_bench::report::RSS_FLOOR_MARGIN - 1.0) * 100.0
        );
    }
    // the version column fits the longest line, fgro's with its full commit
    let version_width = record
        .runtimes
        .iter()
        .map(|runtime| runtime.version.len())
        .fold("version".len(), usize::max);
    println!(
        "\n{:<width$} {:<version_width$} {:>10} {:>10}",
        "runtime", "version", "MB", "gzip MB"
    );
    for runtime in &record.runtimes {
        println!(
            "{:<width$} {:<version_width$} {:>10.1} {:>10.1}",
            runtime.name,
            runtime.version,
            mb(runtime.bytes),
            mb(runtime.gzip_bytes),
        );
    }
    if let Some(bytes) = record.snapshot_bytes {
        println!("fgro's startup snapshot: {:.1} MB", mb(bytes));
    }
    let noise = &record.noise;
    println!(
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

/// The incumbent a warm row gates against, Node at its best configuration.
const GATE_WARM: &str = "node_compile_cache";
/// The incumbent a cold row gates against.
const GATE_COLD: &str = "node";
/// Reported beside the gate.
const BESIDE: &str = "deno";

/// Prints the gate table, report only; nothing fails on it: the suite's own
/// `gate` when it has one ([`print_suite_gate`]), else each `fgro` row's
/// median ([`is_fgro`] runtimes only) against its gate row's, same-session:
/// warm rows gate against Node with its compile cache, cold rows against
/// plain Node, with Deno beside.
pub(super) fn print_gate(suite: &Suite, record: &RunRecord) {
    if let Some(gate) = &suite.gate {
        print_suite_gate(suite, gate, record);
        return;
    }
    let median = |runtime: &str, workload: &str| {
        record
            .rows
            .iter()
            .find(|row| row.runtime == runtime && row.workload == workload)
            .map(|row| row.wall_ns.stats.median / 1e6)
    };
    let ms =
        |value: Option<f64>| value.map_or_else(|| "-".to_owned(), |value| format!("{value:.3}"));
    let mut lines = Vec::new();
    for (workload, runtime) in suite.rows().filter(|(_, runtime)| is_fgro(runtime)) {
        let Some(fgro) = median(&runtime.name, &workload.name) else {
            continue;
        };
        let gate = if workload.cold { GATE_COLD } else { GATE_WARM };
        let ratio = |other: Option<f64>| {
            other.map_or_else(|| "-".to_owned(), |other| format!("{:.2}x", fgro / other))
        };
        lines.push(format!(
            "{:<10} {:<16} {:>10.3} {:<18} {:>10} {:>9} {:>10} {:>9}",
            runtime.name,
            workload.name,
            fgro,
            gate,
            ms(median(gate, &workload.name)),
            ratio(median(gate, &workload.name)),
            ms(median(BESIDE, &workload.name)),
            ratio(median(BESIDE, &workload.name)),
        ));
    }
    if lines.is_empty() {
        return;
    }
    println!(
        "\n{:<10} {:<16} {:>10} {:<18} {:>10} {:>9} {:>10} {:>9}",
        "runtime", "workload", "median ms", "gate", "gate ms", "x gate", "deno ms", "x deno"
    );
    for line in lines {
        println!("{line}");
    }
    println!(
        "(medians, same session; the noise row ({} {} against its copy) sat {:.2}% apart, and each row's mad above bounds its own spread)",
        record.noise.runtime,
        record.noise.workload,
        record.noise.median_deviation * 100.0
    );
}

/// The median of the row `runtime` × `workload`, in milliseconds.
fn median_ms(record: &RunRecord, runtime: &str, workload: &str) -> Option<f64> {
    record
        .rows
        .iter()
        .find(|row| row.runtime == runtime && row.workload == workload)
        .map(|row| row.wall_ns.stats.median / 1e6)
}

/// Prints a suite's own `gate`: each of its runtimes on every workload
/// against `gate.against`, marked `gate` on the workloads it names and
/// `report` on the rest, then the runtimes reported only. Report only.
fn print_suite_gate(suite: &Suite, gate: &Gate, record: &RunRecord) {
    let ms =
        |value: Option<f64>| value.map_or_else(|| "-".to_owned(), |value| format!("{value:.3}"));
    println!(
        "\n{:<10} {:<16} {:>10} {:<10} {:>10} {:>9} {:<6}",
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
            println!(
                "{:<10} {:<16} {:>10} {:<10} {:>10} {:>9} {:<6}",
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
        println!(
            "reported only, in the summary above: {}",
            reported.join(", ")
        );
    }
    println!(
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
