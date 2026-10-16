//! A suite: the declarative input to a run, read from a JSON file.
//!
//! A suite lists runtimes and workloads, and a run measures every pair of the
//! two, a *row*. Adding a workload or a runtime shim is a change to a suite
//! file and the scripts it names, never to the runner.
//!
//! ```json
//! {
//!   "name": "smoke",
//!   "version": 1,
//!   "samples": 4,
//!   "runtimes": [
//!     {"name": "node", "program": "node", "v8_args": ["-p", "process.versions.v8"]},
//!     {"name": "deno", "program": "deno", "args": ["run", {"file": "shims/deno.js"}]}
//!   ],
//!   "workloads": [{"name": "empty", "script": "empty.js", "warmup": 1}]
//! }
//! ```
//!
//! A row's command is the runtime's `program`, its `args`, then the workload's
//! `script`. Files (a `{"file": …}` argument and every script) are relative to
//! the suite file's directory, which is also each command's working
//! directory, and are passed to the runtime as absolute paths. A `program`
//! containing a `/` is a file too; any other is looked up on `PATH`.
//!
//! A `program` names the real executable, not a script wrapping it (a file
//! starting with `#!`), which the run refuses: the hash would name the wrapper,
//! not the runtime. A multi-call binary that dispatches on its name (a version
//! manager's shim) still works, since the run passes the `program` as written
//! as its `argv[0]`.
//!
//! Every command runs with a cleared environment holding only `PATH` and
//! `HOME`, so nothing in the caller's environment (`NODE_OPTIONS`,
//! `DENO_DIR`, cargo's `LD_LIBRARY_PATH`) changes what a run measures.

use std::collections::HashSet;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::BenchError;

/// A suite file's contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    /// The suite's name, recorded with its version.
    pub name: String,
    /// Bumped whenever the workload set changes, since a score compares only
    /// within one version.
    pub version: u32,
    /// The timed samples per row (and for the noise row's copy), each a
    /// fresh process. Even, so the schedule's round pairs balance a linear
    /// drift across rows: exactly for a row's mean, approximately for its
    /// median.
    pub samples: NonZeroU32,
    /// The runtimes every workload runs under.
    pub runtimes: Vec<Runtime>,
    /// The workloads every runtime runs.
    pub workloads: Vec<Workload>,
}

/// A JS runtime and how to invoke it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runtime {
    /// The name rows and the record use, unique in the suite.
    pub name: String,
    /// The executable: a path relative to the suite (when it contains a `/`)
    /// or a name looked up on `PATH`.
    pub program: String,
    /// Arguments before the workload's script, such as a subcommand or a
    /// shim.
    #[serde(default)]
    pub args: Vec<Arg>,
    /// Arguments that print the runtime's version as the first line of
    /// stdout.
    #[serde(default = "default_version_args")]
    pub version_args: Vec<String>,
    /// Arguments that print the runtime's V8 version, for runtimes on V8.
    #[serde(default)]
    pub v8_args: Option<Vec<String>>,
}

fn default_version_args() -> Vec<String> {
    vec!["--version".to_owned()]
}

/// One command-line argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    /// Passed as written.
    Text(String),
    /// A file the run hashes as an artifact, passed as an absolute path.
    File(FileArg),
}

/// A file argument, `{"file": "path"}` in a suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileArg {
    /// The path, relative to the suite file's directory.
    pub file: PathBuf,
}

/// A workload: a script every runtime runs unmodified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workload {
    /// The name rows and the record use, unique in the suite.
    pub name: String,
    /// The script, relative to the suite file's directory.
    pub script: PathBuf,
    /// Untimed processes per row before its timed samples, interleaved like
    /// them. They warm what outlives a process (the page cache, a runtime's
    /// on-disk caches); warming inside a process is the workload's own
    /// business.
    #[serde(default)]
    pub warmup: u32,
}

impl Suite {
    /// Reads and validates the suite file at `path`.
    ///
    /// # Errors
    ///
    /// Fails when the file can't be read, isn't a suite, or fails
    /// [`validate`](Self::validate).
    pub fn load(path: &Path) -> Result<Self, BenchError> {
        let text = fs::read_to_string(path).map_err(|source| BenchError::SuiteRead {
            path: path.to_owned(),
            source,
        })?;
        let suite: Self = serde_json::from_str(&text).map_err(|source| BenchError::SuiteParse {
            path: path.to_owned(),
            source,
        })?;
        suite.validate()?;
        Ok(suite)
    }

    /// Checks what the types can't: something to run, an even sample count,
    /// and unique names.
    ///
    /// # Errors
    ///
    /// Fails with [`BenchError::SuiteInvalid`] saying what's wrong.
    pub fn validate(&self) -> Result<(), BenchError> {
        if self.runtimes.is_empty() || self.workloads.is_empty() {
            return Err(BenchError::SuiteInvalid(
                "a suite needs at least one runtime and one workload".to_owned(),
            ));
        }
        if self.samples.get() % 2 == 1 {
            return Err(BenchError::SuiteInvalid(format!(
                "`samples` is {}, but must be even: the schedule balances drift across rows only over pairs of rounds",
                self.samples
            )));
        }
        unique("runtime", self.runtimes.iter().map(|r| r.name.as_str()))?;
        unique("workload", self.workloads.iter().map(|w| w.name.as_str()))
    }

    /// The blake3 hash of the suite as parsed (defaults filled, formatting
    /// ignored): two runs with the same hash ran the same rows the same way.
    ///
    /// # Panics
    ///
    /// Never: a suite always serializes.
    pub fn config_hash(&self) -> String {
        #[expect(
            clippy::expect_used,
            reason = "a suite serializes infallibly: no maps, and paths parsed from JSON are UTF-8"
        )]
        let json = serde_json::to_vec(self).expect("a suite serializes to JSON");
        blake3::hash(&json).to_hex().to_string()
    }
}

fn unique<'a>(kind: &str, names: impl Iterator<Item = &'a str>) -> Result<(), BenchError> {
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(BenchError::SuiteInvalid(format!(
                "the {kind} name `{name}` is used twice"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Suite, BenchError> {
        let suite: Suite = serde_json::from_str(json).map_err(|source| BenchError::SuiteParse {
            path: PathBuf::from("test.json"),
            source,
        })?;
        suite.validate()?;
        Ok(suite)
    }

    #[test]
    fn parses_with_defaults() {
        let suite = parse(
            r#"{
                "name": "s", "version": 2, "samples": 6,
                "runtimes": [{"name": "deno", "program": "deno", "args": ["run", {"file": "shim.js"}]}],
                "workloads": [{"name": "w", "script": "w.js"}]
            }"#,
        )
        .unwrap();
        let runtime = &suite.runtimes[0];
        assert_eq!(
            runtime.args,
            [
                Arg::Text("run".to_owned()),
                Arg::File(FileArg {
                    file: PathBuf::from("shim.js")
                }),
            ]
        );
        assert_eq!(runtime.version_args, ["--version"]);
        assert_eq!(runtime.v8_args, None);
        assert_eq!(suite.workloads[0].warmup, 0);
    }

    #[test]
    fn rejects_bad_suites() {
        let runtime = r#"{"name": "r", "program": "r"}"#;
        let workload = r#"{"name": "w", "script": "w.js"}"#;
        let suite = |samples: &str, runtimes: &str, workloads: &str| {
            format!(
                r#"{{"name": "s", "version": 1, "samples": {samples}, "runtimes": [{runtimes}], "workloads": [{workloads}]}}"#
            )
        };
        assert!(parse(&suite("2", runtime, workload)).is_ok());
        // zero samples
        assert!(matches!(
            parse(&suite("0", runtime, workload)),
            Err(BenchError::SuiteParse { .. })
        ));
        // an odd count, which the schedule can't balance
        assert!(matches!(
            parse(&suite("3", runtime, workload)),
            Err(BenchError::SuiteInvalid(message)) if message.contains("even")
        ));
        // nothing to run
        assert!(matches!(
            parse(&suite("2", "", workload)),
            Err(BenchError::SuiteInvalid(_))
        ));
        // a duplicate name
        assert!(matches!(
            parse(&suite("2", &format!("{runtime}, {runtime}"), workload)),
            Err(BenchError::SuiteInvalid(_))
        ));
        // an unknown field
        assert!(matches!(
            parse(&suite(
                "2",
                r#"{"name": "r", "program": "r", "flags": []}"#,
                workload
            )),
            Err(BenchError::SuiteParse { .. })
        ));
    }

    #[test]
    fn config_hash_ignores_formatting_but_not_content() {
        let a = parse(r#"{"name":"s","version":1,"samples":4,"runtimes":[{"name":"r","program":"r"}],"workloads":[{"name":"w","script":"w.js"}]}"#).unwrap();
        let b = parse(
            r#"{
                "workloads": [{"script": "w.js", "name": "w"}],
                "runtimes": [{"name": "r", "program": "r", "version_args": ["--version"]}],
                "name": "s", "version": 1, "samples": 4
            }"#,
        )
        .unwrap();
        assert_eq!(a.config_hash(), b.config_hash());
        let mut c = a.clone();
        c.samples = NonZeroU32::new(6).unwrap();
        assert_ne!(a.config_hash(), c.config_hash());
    }
}
