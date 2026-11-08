//! Base records the crate's tests build on, each test overriding the fields
//! it exercises with struct-update syntax.

use std::collections::BTreeMap;

use crate::record::{
    GitState, Machine, MachineQuiet, Metric, Mode, NoiseRecord, RECORD_VERSION, RowRecord,
    RunRecord, SuiteId,
};
use crate::stats::Stats;

/// The metric of `samples`, which must not be empty.
pub fn metric(samples: Vec<u64>) -> Metric {
    Metric {
        stats: Stats::from_samples(&samples).unwrap(),
        samples,
    }
}

/// A row of `runtime` on `workload` timed at `wall_ns`, with no warmup and
/// nothing else measured.
pub fn row(runtime: &str, workload: &str, wall_ns: Metric) -> RowRecord {
    RowRecord {
        runtime: runtime.to_owned(),
        workload: workload.to_owned(),
        warmup: 0,
        wall_ns,
        instructions: None,
        cycles: None,
        kernel_instructions: None,
        phases: Vec::new(),
        peak_rss_kib: None,
    }
}

/// A wall run of `baseline` v1 holding `rows`, its A/A row a copy of the
/// first row's wall time at no deviation, every optional field empty.
///
/// # Panics
///
/// Panics when `rows` is empty.
pub fn run_record(rows: Vec<RowRecord>) -> RunRecord {
    let first = &rows[0];
    RunRecord {
        record_version: RECORD_VERSION,
        mode: Mode::Wall,
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
        collections: BTreeMap::new(),
        runtimes: Vec::new(),
        artifacts: BTreeMap::new(),
        checkouts: BTreeMap::new(),
        noise: NoiseRecord {
            runtime: first.runtime.clone(),
            workload: first.workload.clone(),
            wall_ns: first.wall_ns.clone(),
            median_deviation: 0.0,
            instructions: None,
            instructions_median_deviation: None,
        },
        rows,
        rss_floor_kib: None,
        snapshot_bytes: None,
    }
}
