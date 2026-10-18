//! The run record: one JSON line per run, appended to the history.
//!
//! A line carries what a number needs to be read later: the suite and its
//! version, the gro checkout's commit, the config hash, the runtimes'
//! versions, binary hashes, and sizes, the hashes of the scripts and files the suite
//! names, the machine, the corpus pins, and every row's
//! samples beside their statistics. Raw samples stay in the line, so later
//! comparisons can recompute from them.
//!
//! The types are plain data, so the JSON schema can be derived from them when
//! the renderer needs it.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::error::BenchError;
use crate::stats::Stats;

/// The line shape's version, bumped when it changes incompatibly.
pub const RECORD_VERSION: u32 = 3;

/// One run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// [`RECORD_VERSION`] when written.
    pub record_version: u32,
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
    /// The workspace repos a suite reads, by name, at their commit SHAs.
    pub workspace: BTreeMap<String, String>,
    /// The runtimes, in suite order.
    pub runtimes: Vec<RuntimeInfo>,
    /// The blake3 hash of every workload script and file argument, by its
    /// path as the suite writes it (relative to the suite's directory).
    pub artifacts: BTreeMap<String, String>,
    /// Every row, workload by workload, each in runtime order.
    pub rows: Vec<RowRecord>,
    /// The A/A noise row.
    pub noise: NoiseRecord,
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
}

/// One metric's samples and their statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub stats: Stats,
    /// The samples in the order they were taken.
    pub samples: Vec<u64>,
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
}

impl RunRecord {
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

impl GitState {
    /// Reads the state of the checkout at `root`. The history file, when it's
    /// inside the checkout, doesn't count as a change: every run appends to
    /// it. A `history` that climbs out with `..` can't be named to git as an
    /// exclusion, so it gets none (callers pass a normalized path).
    ///
    /// # Errors
    ///
    /// Fails when `git` can't run or fails, as outside a checkout.
    pub fn read(root: &Path, history: &Path) -> Result<Self, BenchError> {
        let sha = git(root, &["rev-parse", "HEAD"])?.trim().to_owned();
        let mut status_args = vec!["status".to_owned(), "--porcelain".to_owned()];
        if let Some(relative) = history.strip_prefix(root).ok().filter(|relative| {
            relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        }) {
            status_args.push("--".to_owned());
            status_args.push(".".to_owned());
            status_args.push(format!(":(exclude){}", relative.display()));
        }
        let status_args: Vec<&str> = status_args.iter().map(String::as_str).collect();
        let dirty = !git(root, &status_args)?.trim().is_empty();
        Ok(Self { sha, dirty })
    }
}

/// The subtree of a corpora checkout that is the corpus, what its tree id
/// names.
const CORPORA_TREE: &str = "collections";

/// Reads the tree id of the corpora checkout at `path`, refusing local changes
/// under its `collections/`, which the tree id wouldn't name.
///
/// # Errors
///
/// Fails when `git` can't run or fails, as outside a checkout, or with
/// [`BenchError::CorporaDirty`].
pub fn read_corpora_tree(path: &Path) -> Result<String, BenchError> {
    let tree = git(path, &["rev-parse", &format!("HEAD:{CORPORA_TREE}")])?
        .trim()
        .to_owned();
    // untracked files count: a workload could import one
    let status = git(path, &["status", "--porcelain", "--", CORPORA_TREE])?;
    if !status.trim().is_empty() {
        return Err(BenchError::CorporaDirty {
            path: path.to_owned(),
        });
    }
    Ok(tree)
}

fn git(root: &Path, args: &[&str]) -> Result<String, BenchError> {
    let command = format!("git -C {} {}", root.display(), args.join(" "));
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|source| BenchError::Spawn {
            command: command.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(BenchError::CommandFailed {
            command,
            status: output.status,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
            workspace: BTreeMap::from([("fuz_util".to_owned(), "0123abcd".to_owned())]),
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
            }],
            noise: NoiseRecord {
                runtime: "node".to_owned(),
                workload: "empty".to_owned(),
                wall_ns: metric(vec![41_000_003, 40_100_007, 42_222_229]),
                median_deviation: 0.012_345_678_901_234_5,
            },
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
    fn line_shape() {
        let value: serde_json::Value = serde_json::from_str(&record().to_line()).unwrap();
        assert_eq!(value["machine_quiet"], "attested");
        assert_eq!(value["runtimes"][1]["v8"], serde_json::Value::Null);
        assert_eq!(value["rows"][0]["wall_ns"]["stats"]["n"], 5);
        assert_eq!(value["rows"][0]["wall_ns"]["samples"][4], 120_000_000);
        assert_eq!(value["workspace"]["fuz_util"], "0123abcd");
        assert_eq!(value["artifacts"]["shims/deno.js"], "01".repeat(32));
        assert_eq!(value["runtimes"][0]["gzip_bytes"], 43_625_117);
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

    /// A fresh checkout with one commit, at `<temp>/<name>/repo`.
    fn git_checkout(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
        let root = base.join("repo");
        fs::create_dir_all(&root).unwrap();
        for args in [
            &["init", "--quiet"][..],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "init",
            ],
        ] {
            git(&root, args).unwrap();
        }
        (base, root)
    }

    #[test]
    fn the_history_inside_the_checkout_is_not_a_change() {
        let (base, root) = git_checkout("gro_bench_git_inside");
        let history = root.join("sub/history.jsonl");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, "{}\n").unwrap();
        let clean = GitState::read(&root, &history).unwrap().dirty;
        // any other new file is a change
        fs::write(root.join("other.txt"), "x").unwrap();
        let dirty = GitState::read(&root, &history).unwrap().dirty;
        fs::remove_dir_all(&base).unwrap();
        assert!(!clean);
        assert!(dirty);
    }

    #[test]
    fn the_corpora_tree_is_read_and_local_changes_refused() {
        let (base, root) = git_checkout("gro_bench_git_corpora");
        fs::create_dir_all(root.join("collections/a")).unwrap();
        fs::write(root.join("collections/a/x.ts"), "export {};\n").unwrap();
        git(&root, &["add", "."]).unwrap();
        git(
            &root,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "add",
            ],
        )
        .unwrap();
        let tree = read_corpora_tree(&root).unwrap();
        let expected = git(&root, &["rev-parse", "HEAD:collections"]).unwrap();
        // a change outside `collections/` doesn't count
        fs::write(root.join("README.md"), "x").unwrap();
        let outside = read_corpora_tree(&root).unwrap();
        // an untracked file inside it does
        fs::write(root.join("collections/a/y.ts"), "").unwrap();
        let inside = read_corpora_tree(&root);
        fs::remove_dir_all(&base).unwrap();
        assert_eq!(tree, expected.trim());
        assert_eq!(tree.len(), 40);
        assert_eq!(outside, tree);
        assert!(matches!(inside, Err(BenchError::CorporaDirty { .. })));
    }

    #[test]
    fn a_history_outside_the_checkout_gets_no_exclusion() {
        let (base, root) = git_checkout("gro_bench_git_outside");
        // the shape `std::path::absolute` leaves for `--history ../x.jsonl`
        for history in [root.join("../x.jsonl"), base.join("x.jsonl")] {
            let state = GitState::read(&root, &history).unwrap();
            assert!(!state.dirty, "{}", history.display());
        }
        fs::write(root.join("other.txt"), "x").unwrap();
        let state = GitState::read(&root, &root.join("../x.jsonl")).unwrap();
        fs::remove_dir_all(&base).unwrap();
        assert!(state.dirty);
    }
}
