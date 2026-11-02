//! The run record: one JSON line per run, appended to the history.
//!
//! A line carries what a number needs to be read later: the suite and its
//! version, the gro checkout's commit, the config hash, the runtimes'
//! versions, binary hashes, and sizes, the hashes of the scripts and files the suite
//! names, the state of the other checkouts the workloads read, the machine,
//! the corpus pins, and every row's
//! samples beside their statistics. Raw samples stay in the line, so later
//! comparisons can recompute from them.
//!
//! A run measures in one [`Mode`]: wall time alone, wall time with fgro's
//! phase timings, or wall time with hardware counters (`perf stat`). The
//! mode is part of the line, since a phases build or a counted process times
//! more than the plain one.
//!
//! The types are plain data, so the JSON schema can be derived from them when
//! the renderer needs it.
//!
//! Line versions: 5 added `checkouts`; 6 added `mode`, `counters`, and each
//! row's (and the noise row's) `instructions`, `cycles`, and `phases`; 7
//! added each row's `peak_rss_kib` and `kernel_instructions`, the run's
//! `rss_floor_kib`, and `snapshot_bytes`. A line of an older version still reads as this shape,
//! as a wall run, with the fields it lacks empty.
//!
//! A row's peak memory is each process's `ru_maxrss` as its reaping returns
//! it (`wait4`), which counts from the memory the process ran in before it
//! executed its program: its spawner's (the harness's, or `perf`'s in a
//! counter run). So a sample reads at least that spawner's own peak, and the
//! run measures that floor too (`rss_floor_kib`): a row whose samples sit
//! well above it read the runtime's own peak; one at or near it doesn't.
//! Only a wall run records peaks: a counter run's spawner is `perf`, whose
//! own peak would mask a small runtime's, and a phases run's `fgro` is a
//! measurement build.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::BenchError;
use crate::stats::Stats;

/// The line shape's version, bumped on any change to it, additive ones
/// included, so a harness older than a line skips it rather than read it
/// without the fields it doesn't know.
pub const RECORD_VERSION: u32 = 7;

/// One run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// [`RECORD_VERSION`] when written.
    pub record_version: u32,
    /// What the run measured. Absent from a line before version 6, which
    /// measured wall time alone.
    #[serde(default)]
    pub mode: Mode,
    /// The counter tool a counter run read, absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counters: Option<CounterTool>,
    /// The suite that ran.
    pub suite: SuiteId,
    /// The suite's [`config_hash`](crate::suite::Suite::config_hash).
    pub config_hash: String,
    /// When the run started, in seconds since the Unix epoch.
    pub started_unix_s: u64,
    /// The gro checkout the harness ran from.
    pub git: GitState,
    /// The machine the run measured on.
    pub machine: Machine,
    /// Whether the machine was quieted for the run.
    pub machine_quiet: MachineQuiet,
    /// The tree id of the corpora's `collections/`, when the suite reads
    /// them: the suite's pin, checked before and after the run.
    pub corpora: Option<String>,
    /// The runtimes, in suite order.
    pub runtimes: Vec<RuntimeInfo>,
    /// The blake3 hash of every workload script and file argument, by its
    /// path as the suite writes it (relative to the suite's directory).
    pub artifacts: BTreeMap<String, String>,
    /// The git state of each checkout outside the suite's own that the
    /// workloads read, by the path as the suite writes it: a project link's
    /// target, and a script in another checkout (naming that checkout).
    /// Read before the correctness run and again after the samples, and the
    /// run discarded if one moved. Left out of the line when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub checkouts: BTreeMap<String, CheckoutState>,
    /// Every row, workload by workload, each in runtime order.
    pub rows: Vec<RowRecord>,
    /// The A/A noise row.
    pub noise: NoiseRecord,
    /// The peak, in KiB, that a sample's `peak_rss_kib` reads at least,
    /// whatever it runs: the largest of a trivial program's (`true`) run as
    /// the samples are, after them, in a wall run. Absent before version 7,
    /// in other modes, and where the harness reads no peak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rss_floor_kib: Option<u64>,
    /// The size in bytes of the startup snapshot the run's `fgro` embeds,
    /// when the caller named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_bytes: Option<u64>,
}

/// What a run measured, beside wall time.
///
/// The variants are declared in the order of their names, so the derived
/// order, which the report sorts its series by, is the names' order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Wall time and user-space hardware counters, each sample run under
    /// `perf stat` ([`RowRecord::instructions`], [`RowRecord::cycles`]); the
    /// wall times include perf's own startup.
    Counters,
    /// Wall time, and each sample's phase timings from the runtimes a
    /// phases build runs ([`RowRecord::phases`]); that build times a little
    /// more than the plain one.
    Phases,
    /// Wall time alone, each process run as is.
    #[default]
    Wall,
}

impl std::fmt::Display for Mode {
    /// The mode's name as a line writes it, from the serde attributes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| std::fmt::Error)?;
        f.write_str(value.as_str().ok_or(std::fmt::Error)?)
    }
}

/// The tool a counter run read its counts with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterTool {
    /// The first line its `--version` printed (`perf version 6.12.0`).
    pub version: String,
    /// The events counted, as passed to it.
    pub events: Vec<String>,
}

/// A suite's identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuiteId {
    pub name: String,
    pub version: u32,
}

/// The commit a checkout is at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitState {
    /// The `HEAD` commit SHA.
    pub sha: String,
    /// Whether the working tree differs from it, so `sha` alone doesn't
    /// name the code that ran.
    pub dirty: bool,
}

/// The state of a checkout a run reads outside the gro checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutState {
    /// The `HEAD` commit SHA.
    pub sha: String,
    /// Whether the working tree differs from it, untracked files included:
    /// exactly when there are changes to hash.
    pub dirty: bool,
    /// The blake3 hash of what makes it dirty, which names the code that ran
    /// where `sha` can't: `git diff HEAD` (binary, full index ids, and the
    /// output options a flag can pin set to git's default, so a user's diff
    /// settings don't change the bytes; attribute-driven filters and hunk
    /// headers, and `core.fileMode`, still apply), then each untracked
    /// file's path and contents. Ignored files
    /// (a checkout's `node_modules`, its build output) are in neither part,
    /// so what a workload reaches through them isn't named here. A
    /// submodule shows only as its commit and whether it's dirty, so edits
    /// inside an already-dirty submodule aren't seen. `None` exactly when
    /// it's clean.
    pub changes_blake3: Option<String>,
}

/// The machine a run measured on: what makes numbers comparable, with no
/// hostname and nothing that changes from run to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    /// The CPU model name, where the OS reports one.
    pub cpu_model: Option<String>,
    /// The OS family (`std::env::consts::OS`).
    pub os: String,
    /// The CPU architecture (`std::env::consts::ARCH`).
    pub arch: String,
}

/// Whether the machine was quieted for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineQuiet {
    /// Nobody checked.
    Unchecked,
    /// The operator quieted it by hand, following the machine protocol.
    Attested,
}

/// A runtime as it was measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub name: String,
    /// The first line its version arguments printed.
    pub version: String,
    /// Its V8 version, when the suite gives `v8_args` to probe it (fgro's
    /// version line names its V8).
    pub v8: Option<String>,
    /// The blake3 hash of its executable, which names a local build its
    /// version string can't.
    pub blake3: String,
    /// The executable's size in bytes, as installed, stripped or not: node's
    /// release binary carries debug symbols, which stripping drops, so compare
    /// like with like when gro's binary joins them.
    pub bytes: u64,
    /// The executable's size gzipped at gzip's default level, in bytes.
    pub gzip_bytes: u64,
}

/// One row's measurements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowRecord {
    pub runtime: String,
    pub workload: String,
    /// The untimed processes before its samples.
    pub warmup: u32,
    /// Wall time per process, in nanoseconds.
    pub wall_ns: Metric,
    /// User-space instructions retired per process, in a counter run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Metric>,
    /// User-space cycles per process, in a counter run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycles: Option<Metric>,
    /// Kernel-space instructions retired per process, in a counter run,
    /// from version 7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_instructions: Option<Metric>,
    /// Each phase's duration per process, in nanoseconds, in a phases run,
    /// in the order the phases ran: `start` (from the harness's spawn to the
    /// runtime's entry), each phase the runtime marked, then `reap` (from its
    /// last mark to its exit being reaped). They sum to about the wall time.
    /// Empty for a runtime that marks none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<PhaseMetric>,
    /// Peak resident set size per process, in KiB, its `ru_maxrss`, which
    /// reads at least the run's `rss_floor_kib`, in a wall run. Absent
    /// before version 7, in other modes, and where the harness reads no
    /// peak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_rss_kib: Option<Metric>,
}

/// One phase's durations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseMetric {
    /// The phase's name.
    pub name: String,
    /// Its duration per process, in nanoseconds.
    pub ns: Metric,
}

/// One metric's samples and their statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub stats: Stats,
    /// The samples in the order they were taken.
    pub samples: Vec<u64>,
}

impl Metric {
    /// The largest sample, outliers included: for a peak, the worst seen.
    pub fn max(&self) -> u64 {
        self.samples.iter().copied().max().unwrap_or(0)
    }
}

/// The A/A noise row: the run's first row measured a second time, as a row of
/// its own, interleaved with the rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoiseRecord {
    pub runtime: String,
    pub workload: String,
    /// The copy's wall times, beside the first row's own.
    pub wall_ns: Metric,
    /// How far the two medians sit apart, as a ratio deviation (0.01 is 1%):
    /// a difference between rows smaller than this is noise.
    pub median_deviation: f64,
    /// The copy's instruction counts, in a counter run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Metric>,
    /// How far the two instruction medians sit apart, as a ratio deviation,
    /// in a counter run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions_median_deviation: Option<f64>,
}

impl NoiseRecord {
    /// The wall medians' deviation, unless it is unusable: [`f64::MAX`],
    /// what [`ratio_deviation`](crate::stats::ratio_deviation) gives when
    /// one median is 0, which says nothing of the noise.
    pub fn wall_deviation(&self) -> Option<f64> {
        (self.median_deviation < f64::MAX).then_some(self.median_deviation)
    }
}

impl RunRecord {
    /// The row of `runtime` on `workload`, when the run has it.
    pub fn row(&self, runtime: &str, workload: &str) -> Option<&RowRecord> {
        self.rows
            .iter()
            .find(|row| row.runtime == runtime && row.workload == workload)
    }

    /// The record as one JSON line, newline included.
    ///
    /// # Panics
    ///
    /// Never: a record always serializes.
    pub fn to_line(&self) -> String {
        #[expect(
            clippy::expect_used,
            reason = "a record serializes infallibly: string-keyed maps, no custom impls"
        )]
        let mut line = serde_json::to_string(self).expect("a record serializes to JSON");
        line.push('\n');
        line
    }

    /// Appends the record to the history at `path`, creating the file if
    /// needed. A history whose last line is unterminated (an interrupted
    /// write, a hand edit) gets a newline first, so the record starts a line
    /// of its own.
    ///
    /// # Errors
    ///
    /// Fails when the file can't be opened, read, or written.
    pub fn append_to(&self, path: &Path) -> Result<(), BenchError> {
        let write = || {
            let mut file = OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(path)?;
            let mut text = String::new();
            if file.metadata()?.len() > 0 {
                let mut last = [0];
                file.seek(SeekFrom::End(-1))?;
                file.read_exact(&mut last)?;
                if last[0] != b'\n' {
                    text.push('\n');
                }
            }
            text.push_str(&self.to_line());
            // writes in append mode go to the end, wherever the read left off
            file.write_all(text.as_bytes())
        };
        write().map_err(|source| BenchError::HistoryWrite {
            path: path.to_owned(),
            source,
        })
    }
}

impl Machine {
    /// Reads the current machine's block.
    pub fn detect() -> Self {
        Self {
            cpu_model: cpu_model(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }
}

/// The CPU model from `/proc/cpuinfo`, on Linux on x86.
fn cpu_model() -> Option<String> {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
    cpuinfo.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "model name").then(|| value.trim().to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(samples: Vec<u64>) -> Metric {
        Metric {
            stats: Stats::from_samples(&samples).unwrap(),
            samples,
        }
    }

    fn record() -> RunRecord {
        RunRecord {
            record_version: RECORD_VERSION,
            mode: Mode::Counters,
            counters: Some(CounterTool {
                version: "perf version 6.12.0".to_owned(),
                events: vec!["instructions:u".to_owned(), "cycles:u".to_owned()],
            }),
            suite: SuiteId {
                name: "smoke".to_owned(),
                version: 1,
            },
            config_hash: "c0ffee".to_owned(),
            started_unix_s: 1_760_000_000,
            git: GitState {
                sha: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                dirty: true,
            },
            machine: Machine {
                cpu_model: Some("Example CPU @ 3.00GHz".to_owned()),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
            },
            machine_quiet: MachineQuiet::Attested,
            corpora: Some("4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_owned()),
            runtimes: vec![
                RuntimeInfo {
                    name: "node".to_owned(),
                    version: "v24.14.1".to_owned(),
                    v8: Some("13.6.233.17-node.44".to_owned()),
                    blake3: "ab".repeat(32),
                    bytes: 122_502_392,
                    gzip_bytes: 43_625_117,
                },
                RuntimeInfo {
                    name: "bun".to_owned(),
                    version: "1.4.3".to_owned(),
                    v8: None,
                    blake3: "cd".repeat(32),
                    bytes: 88_979_072,
                    gzip_bytes: 30_000_001,
                },
            ],
            artifacts: BTreeMap::from([
                ("empty.js".to_owned(), "ef".repeat(32)),
                ("shims/deno.js".to_owned(), "01".repeat(32)),
            ]),
            checkouts: BTreeMap::from([(
                "../zap".to_owned(),
                CheckoutState {
                    sha: "89abcdef0123456789abcdef0123456789abcdef".to_owned(),
                    dirty: true,
                    changes_blake3: Some("23".repeat(32)),
                },
            )]),
            rows: vec![RowRecord {
                runtime: "node".to_owned(),
                workload: "empty".to_owned(),
                warmup: 1,
                // includes values a lossy float parse would round wrong
                wall_ns: metric(vec![
                    41_234_567,
                    40_987_654,
                    43_000_001,
                    39_999_999,
                    120_000_000,
                ]),
                instructions: Some(metric(vec![9_000_001, 9_000_003, 9_000_002])),
                cycles: Some(metric(vec![7_000_001, 7_000_003, 7_000_002])),
                kernel_instructions: Some(metric(vec![3_000_001, 3_000_003])),
                phases: vec![PhaseMetric {
                    name: "start".to_owned(),
                    ns: metric(vec![300_001, 300_002, 300_003]),
                }],
                peak_rss_kib: Some(metric(vec![40_001, 40_003, 40_002])),
            }],
            noise: NoiseRecord {
                runtime: "node".to_owned(),
                workload: "empty".to_owned(),
                wall_ns: metric(vec![41_000_003, 40_100_007, 42_222_229]),
                median_deviation: 0.012_345_678_901_234_5,
                instructions: Some(metric(vec![9_000_002, 9_000_004])),
                instructions_median_deviation: Some(0.000_000_3),
            },
            rss_floor_kib: Some(9_001),
            snapshot_bytes: Some(7_000_001),
        }
    }

    #[test]
    fn round_trips_through_a_line() {
        let record = record();
        let line = record.to_line();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1, "one line per run");
        let parsed: RunRecord = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, record);
    }

    #[test]
    fn a_record_of_zero_samples_round_trips() {
        // a counter that read 0, against a copy that read 0 and one that didn't
        let mut record = record();
        record.rows[0].kernel_instructions = Some(metric(vec![0, 0, 0]));
        record.noise.instructions_median_deviation = Some(crate::stats::ratio_deviation(0.0, 0.0));
        record.noise.median_deviation = crate::stats::ratio_deviation(0.0, 5.0);
        let parsed: RunRecord = serde_json::from_str(&record.to_line()).unwrap();
        assert_eq!(parsed, record);
    }

    #[test]
    fn a_mode_displays_and_orders_by_its_name() {
        let modes = [Mode::Counters, Mode::Phases, Mode::Wall];
        let names = modes.map(|mode| mode.to_string());
        assert_eq!(names, ["counters", "phases", "wall"]);
        let mut sorted = [Mode::Wall, Mode::Counters, Mode::Phases];
        sorted.sort();
        assert_eq!(sorted, modes);
        for (mode, name) in modes.iter().zip(&names) {
            assert_eq!(serde_json::to_value(mode).unwrap(), name.as_str());
        }
    }

    #[test]
    fn line_shape() {
        let value: serde_json::Value = serde_json::from_str(&record().to_line()).unwrap();
        assert_eq!(value["machine_quiet"], "attested");
        assert_eq!(value["runtimes"][1]["v8"], serde_json::Value::Null);
        assert_eq!(value["rows"][0]["wall_ns"]["stats"]["n"], 5);
        assert_eq!(value["rows"][0]["wall_ns"]["samples"][4], 120_000_000);
        assert_eq!(value["artifacts"]["shims/deno.js"], "01".repeat(32));
        assert_eq!(value["runtimes"][0]["gzip_bytes"], 43_625_117);
    }

    /// The record's top-level keys, pinned with [`RECORD_VERSION`]: a change
    /// to this list is a change to the line shape, so it bumps the version in
    /// the same edit (the history is append-only, and older lines keep theirs).
    #[test]
    fn the_line_shape_is_pinned_with_its_version() {
        let value: serde_json::Value = serde_json::from_str(&record().to_line()).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            (RECORD_VERSION, keys),
            (
                7,
                vec![
                    "artifacts",
                    "checkouts",
                    "config_hash",
                    "corpora",
                    "counters",
                    "git",
                    "machine",
                    "machine_quiet",
                    "mode",
                    "noise",
                    "record_version",
                    "rows",
                    "rss_floor_kib",
                    "runtimes",
                    "snapshot_bytes",
                    "started_unix_s",
                    "suite",
                ]
            )
        );
        assert_eq!(value["record_version"], RECORD_VERSION);
    }

    #[test]
    fn a_wall_run_leaves_out_the_counter_and_phase_keys() {
        let mut record = record();
        record.mode = Mode::Wall;
        record.counters = None;
        for row in &mut record.rows {
            row.instructions = None;
            row.cycles = None;
            row.kernel_instructions = None;
            row.phases.clear();
        }
        record.noise.instructions = None;
        record.noise.instructions_median_deviation = None;
        let line = record.to_line();
        for key in ["counters", "instructions", "cycles", "phases"] {
            assert!(!line.contains(key), "{key}: {line}");
        }
        assert!(line.contains(r#""mode":"wall""#), "{line}");
        assert_eq!(serde_json::from_str::<RunRecord>(&line).unwrap(), record);
    }

    /// A line from before version 6 reads as a wall run with the new
    /// fields empty.
    #[test]
    fn an_older_line_reads_as_a_wall_run() {
        let mut value: serde_json::Value = serde_json::from_str(&record().to_line()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.insert("record_version".to_owned(), 5.into());
        object.remove("mode");
        object.remove("counters");
        let row = value["rows"][0].as_object_mut().unwrap();
        for key in ["instructions", "cycles", "phases"] {
            row.remove(key);
        }
        let parsed: RunRecord = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.record_version, 5);
        assert_eq!(parsed.mode, Mode::Wall);
        assert_eq!(parsed.counters, None);
        assert_eq!(parsed.rows[0].instructions, None);
        assert!(parsed.rows[0].phases.is_empty());
    }

    #[test]
    fn no_checkouts_leave_the_line_without_the_key() {
        let mut record = record();
        record.checkouts.clear();
        let line = record.to_line();
        assert!(!line.contains("checkouts"), "{line}");
        assert_eq!(serde_json::from_str::<RunRecord>(&line).unwrap(), record);
    }

    #[test]
    fn appends_after_an_unterminated_line() {
        let dir = std::env::temp_dir().join(format!(
            "gro_bench_history_unterminated_{}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        // an interrupted write left half a line
        fs::write(&path, "{\"record_version\":").unwrap();
        let record = record();
        record.append_to(&path).unwrap();
        record.append_to(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "{\"record_version\":");
        for line in &lines[1..] {
            assert_eq!(serde_json::from_str::<RunRecord>(line).unwrap(), record);
        }
    }

    #[test]
    fn appends_lines() {
        let dir = std::env::temp_dir().join(format!("gro_bench_history_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.jsonl");
        let record = record();
        record.append_to(&path).unwrap();
        record.append_to(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            assert_eq!(serde_json::from_str::<RunRecord>(line).unwrap(), record);
        }
    }
}
