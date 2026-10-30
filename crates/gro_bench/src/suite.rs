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
//! A row's command is the runtime's `program`, its `args`, its `cold` args
//! when the workload is cold, then the workload's `script`. Files (a
//! `{"file": …}` argument and every script) are relative to the suite file's
//! directory, which is also each command's working directory, and are passed
//! to the runtime as absolute paths. A `program` containing a `/` is a file
//! too; any other is looked up on `PATH`.
//!
//! A `program` names the real executable, not a script wrapping it (a file
//! starting with `#!`), which the run refuses: the hash would name the wrapper,
//! not the runtime. A multi-call binary that dispatches on its name (a version
//! manager's shim) still works, since the run passes the `program` as written
//! as its `argv[0]`.
//!
//! Every command runs with a cleared environment holding only `PATH` and
//! `HOME`, plus what the suite sets for the runtime (its `env`, and its
//! `cold` env on a cold row), so nothing in the caller's environment
//! (`NODE_OPTIONS`, `DENO_DIR`, cargo's `LD_LIBRARY_PATH`) changes what a run
//! measures.
//!
//! A workload can name a `collection` of the pinned corpora: the run then
//! writes a fresh copy of that collection from the pinned tree, puts the
//! workload's script at the copy's root under its own file name, and runs it
//! from there ([`collection`](crate::collection)), so the script's relative
//! imports are relative to the copy (`./src/lib/array.ts`) and stay inside
//! the collection's own root. The command still runs from the suite file's
//! directory.
//!
//! Everything runtime-specific (a flag, a variable, a shim file) sits in the
//! runtime's entry and counts against that runtime; workload scripts are web
//! standards only, run unmodified by every runtime.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
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
    /// The runtimes the workloads run under. The run's A/A noise row copies
    /// its first row: the first workload under the first runtime.
    pub runtimes: Vec<Runtime>,
    /// The workloads, each run by every runtime.
    pub workloads: Vec<Workload>,
    /// The corpora checkout the workloads read, pinned: the run refuses a
    /// checkout at another tree or with local changes, so a re-pin is a suite
    /// change (and a version bump), never a silent one.
    #[serde(default)]
    pub corpora: Option<CorporaPin>,
}

/// A pinned corpora checkout, a clone of `fuzdev/corpora`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorporaPin {
    /// The checkout, relative to the suite file's directory. A workload with
    /// a `collection` runs inside a copy written from this checkout's objects;
    /// one without must import from this same checkout, since its imports
    /// hard-code their own relative path to it: the pin checks this path, the
    /// scripts read theirs.
    pub path: PathBuf,
    /// The tree id of its `collections/` at `HEAD`
    /// (`git rev-parse HEAD:collections`), which names the bytes the
    /// workloads read, and the tree a workload's collection is copied from.
    pub tree: String,
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
    /// Variables set for every process of the runtime, beside `PATH` and
    /// `HOME`, such as a switch keeping background work out of the samples.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// How the runtime runs a cold workload. Required when it runs one, so
    /// each runtime states how it goes cold.
    #[serde(default)]
    pub cold: Option<Cold>,
}

/// What a cold row adds to the runtime's command.
///
/// These are the switches that turn off the runtime's on-disk caches of
/// transpile output and compiled code, so a cold load neither reads nor
/// writes them. Empty for a runtime with no such cache by default, whose cold
/// and warm rows run the same command.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cold {
    /// Arguments after the runtime's `args`, before the script.
    #[serde(default)]
    pub args: Vec<Arg>,
    /// Variables beside the runtime's `env`, overriding it where both set one.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
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
    /// Whether its rows run cold, with each runtime's [`Cold`] switches;
    /// otherwise a row runs with the runtime's on-disk caches as they are by
    /// default, warmed by the warmup processes.
    #[serde(default)]
    pub cold: bool,
    /// The workload's weight in the suite's objective, a weighted geomean of
    /// per-workload scores, relative to the other workloads' weights. Part of
    /// the suite's version, like the workload set.
    #[serde(default = "default_weight")]
    pub weight: u32,
    /// A collection of the suite's pinned corpora (a directory of its
    /// `collections/`) the workload runs inside: the run copies it from the
    /// pinned tree and runs the script from the copy's root, so the script's
    /// relative imports are written relative to that root. Absent runs the
    /// script where it is, and serializes as no key, so the config hash of a
    /// suite that names none doesn't depend on the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
}

const fn default_weight() -> u32 {
    1
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

    /// The rows a run measures, workload by workload, each in runtime order:
    /// every workload paired with every runtime.
    pub fn rows(&self) -> impl Iterator<Item = (&Workload, &Runtime)> {
        self.workloads
            .iter()
            .flat_map(|workload| self.runtimes.iter().map(move |runtime| (workload, runtime)))
    }

    /// Checks what the types can't: something to run, an even sample count,
    /// unique names, a `cold` entry on every runtime when a workload is cold,
    /// and workload collections that name a corpora pin's directory, each with
    /// one script per entry file name.
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
        unique("workload", self.workloads.iter().map(|w| w.name.as_str()))?;
        if let Some(workload) = self.workloads.iter().find(|w| w.cold)
            && let Some(runtime) = self.runtimes.iter().find(|r| r.cold.is_none())
        {
            return Err(BenchError::SuiteInvalid(format!(
                "the workload `{}` is cold, but the runtime `{}` has no `cold` entry saying how it goes cold (`{{}}` when it has no on-disk cache)",
                workload.name, runtime.name
            )));
        }
        self.validate_collections()
    }

    /// Checks each workload's `collection`: it needs the suite's `corpora`, it
    /// names a directory of `collections/` (one plain path segment), and the
    /// scripts sharing a collection's copy have distinct file names, since
    /// each sits at the copy's root under its own.
    fn validate_collections(&self) -> Result<(), BenchError> {
        let mut entries: BTreeMap<(&str, &OsStr), &Path> = BTreeMap::new();
        for workload in &self.workloads {
            let Some(collection) = &workload.collection else {
                continue;
            };
            if self.corpora.is_none() {
                return Err(BenchError::SuiteInvalid(format!(
                    "the workload `{}` names the collection `{collection}`, but the suite pins no `corpora`",
                    workload.name
                )));
            }
            if !is_plain_segment(collection) {
                return Err(BenchError::SuiteInvalid(format!(
                    "the workload `{}` names the collection `{collection}`, which isn't a single directory name",
                    workload.name
                )));
            }
            let Some(entry) = workload.script.file_name() else {
                return Err(BenchError::SuiteInvalid(format!(
                    "the workload `{}` has the script `{}`, which names no file",
                    workload.name,
                    workload.script.display()
                )));
            };
            let script = entries
                .entry((collection, entry))
                .or_insert(&workload.script);
            if *script != workload.script {
                return Err(BenchError::SuiteInvalid(format!(
                    "the scripts `{}` and `{}` would both be the entry `{}` of the collection `{collection}`",
                    script.display(),
                    workload.script.display(),
                    entry.to_string_lossy()
                )));
            }
        }
        Ok(())
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
            reason = "a suite serializes infallibly: its maps have string keys, and paths parsed from JSON are UTF-8"
        )]
        let json = serde_json::to_vec(self).expect("a suite serializes to JSON");
        blake3::hash(&json).to_hex().to_string()
    }
}

/// Whether `name` is one plain path segment: not empty, `.`, or `..`, and
/// with no separator.
fn is_plain_segment(name: &str) -> bool {
    !matches!(name, "" | "." | "..") && !name.contains(['/', '\\', '\0'])
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
        assert!(runtime.env.is_empty());
        assert_eq!(runtime.cold, None);
        assert_eq!(suite.workloads[0].warmup, 0);
        assert!(!suite.workloads[0].cold);
        assert_eq!(suite.workloads[0].weight, 1);
        assert_eq!(suite.corpora, None);
    }

    #[test]
    fn a_cold_workload_needs_every_runtime_to_say_how_it_goes_cold() {
        let suite = |node_cold: &str| {
            format!(
                r#"{{
                    "name": "s", "version": 1, "samples": 2,
                    "runtimes": [
                        {{"name": "deno", "program": "deno", "env": {{"A": "1"}}, "cold": {{"args": ["--no-code-cache"], "env": {{"B": "0"}}}}}},
                        {{"name": "node", "program": "node"{node_cold}}}
                    ],
                    "workloads": [{{"name": "w", "script": "w.js", "cold": true, "weight": 3}}],
                    "corpora": {{"path": "../corpora", "tree": "abc"}}
                }}"#
            )
        };
        assert!(matches!(
            parse(&suite("")),
            Err(BenchError::SuiteInvalid(message)) if message.contains("`node`")
        ));
        let parsed = parse(&suite(r#", "cold": {}"#)).unwrap();
        assert_eq!(parsed.runtimes[1].cold, Some(Cold::default()));
        let deno = &parsed.runtimes[0];
        assert_eq!(deno.env, BTreeMap::from([("A".to_owned(), "1".to_owned())]));
        let cold = deno.cold.as_ref().unwrap();
        assert_eq!(cold.args, [Arg::Text("--no-code-cache".to_owned())]);
        assert_eq!(cold.env, BTreeMap::from([("B".to_owned(), "0".to_owned())]));
        assert_eq!(parsed.workloads[0].weight, 3);
        assert_eq!(
            parsed.corpora,
            Some(CorporaPin {
                path: PathBuf::from("../corpora"),
                tree: "abc".to_owned()
            })
        );
    }

    #[test]
    fn rows_pair_each_workload_with_every_runtime() {
        let suite = parse(
            r#"{"name":"s","version":1,"samples":2,
                "runtimes":[{"name":"a","program":"a"},{"name":"b","program":"b"}],
                "workloads":[{"name":"x","script":"x.js"},{"name":"y","script":"y.js"}]}"#,
        )
        .unwrap();
        let rows: Vec<(&str, &str)> = suite
            .rows()
            .map(|(workload, runtime)| (workload.name.as_str(), runtime.name.as_str()))
            .collect();
        assert_eq!(rows, [("x", "a"), ("x", "b"), ("y", "a"), ("y", "b")]);
        // the per-runtime list of workloads is gone, so naming one is an unknown field
        assert!(matches!(
            parse(
                r#"{"name":"s","version":1,"samples":2,
                    "runtimes":[{"name":"a","program":"a","workloads":["x"]}],
                    "workloads":[{"name":"x","script":"x.js"}]}"#
            ),
            Err(BenchError::SuiteParse { .. })
        ));
    }

    #[test]
    fn a_workload_can_run_inside_a_corpora_collection() {
        let suite = |corpora: bool, workloads: &str| {
            let corpora = if corpora {
                r#", "corpora": {"path": "../corpora", "tree": "abc"}"#
            } else {
                ""
            };
            format!(
                r#"{{"name": "s", "version": 1, "samples": 2,
                    "runtimes": [{{"name": "r", "program": "r"}}],
                    "workloads": [{workloads}]{corpora}}}"#
            )
        };
        let parsed = parse(&suite(
            true,
            r#"{"name": "w", "script": "dir/w.js", "collection": "fuz_util"},
               {"name": "v", "script": "dir/w.js", "collection": "fuz_util"},
               {"name": "x", "script": "x.js", "collection": "fuz_util"},
               {"name": "y", "script": "x.js"}"#,
        ))
        .unwrap();
        assert_eq!(parsed.workloads[0].collection.as_deref(), Some("fuz_util"));
        assert_eq!(parsed.workloads[3].collection, None);
        // a collection needs the corpora it's copied from
        assert!(matches!(
            parse(&suite(false, r#"{"name": "w", "script": "w.js", "collection": "c"}"#)),
            Err(BenchError::SuiteInvalid(message)) if message.contains("pins no `corpora`")
        ));
        // one plain directory name
        for bad in ["", ".", "..", "a/b", "/a", "a/", r"a\\b"] {
            assert!(
                matches!(
                    parse(&suite(true, &format!(r#"{{"name": "w", "script": "w.js", "collection": "{bad}"}}"#))),
                    Err(BenchError::SuiteInvalid(ref message)) if message.contains("single directory name")
                ),
                "{bad}"
            );
        }
        // two scripts can't be one entry of a collection
        assert!(matches!(
            parse(&suite(
                true,
                r#"{"name": "w", "script": "a/w.js", "collection": "c"},
                   {"name": "v", "script": "b/w.js", "collection": "c"}"#
            )),
            Err(BenchError::SuiteInvalid(message)) if message.contains("entry `w.js`")
        ));
        // but they can in different collections
        assert!(
            parse(&suite(
                true,
                r#"{"name": "w", "script": "a/w.js", "collection": "c"},
                   {"name": "v", "script": "b/w.js", "collection": "d"}"#
            ))
            .is_ok()
        );
        // a script naming no file
        assert!(matches!(
            parse(&suite(true, r#"{"name": "w", "script": "..", "collection": "c"}"#)),
            Err(BenchError::SuiteInvalid(message)) if message.contains("names no file")
        ));
    }

    #[test]
    fn an_absent_collection_leaves_the_config_hash_alone() {
        let json = |collection: &str| {
            format!(
                r#"{{"name":"s","version":1,"samples":4,"runtimes":[{{"name":"r","program":"r"}}],
                    "workloads":[{{"name":"w","script":"w.js"{collection}}}],
                    "corpora":{{"path":"c","tree":"t"}}}}"#
            )
        };
        let absent = parse(&json("")).unwrap();
        let value = serde_json::to_value(&absent).unwrap();
        // absent serializes as no key, so the hash of a suite without collections is unchanged
        assert!(value["workloads"][0].get("collection").is_none(), "{value}");
        let present = parse(&json(r#","collection":"c""#)).unwrap();
        assert_ne!(absent.config_hash(), present.config_hash());
        let other = parse(&json(r#","collection":"d""#)).unwrap();
        assert_ne!(present.config_hash(), other.config_hash());
    }

    #[test]
    fn the_baseline_suite_is_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("suites/baseline");
        let suite = Suite::load(&dir.join("suite.json")).unwrap();
        for workload in &suite.workloads {
            assert!(dir.join(&workload.script).is_file(), "{workload:?}");
        }
        assert!(suite.workloads.iter().any(|w| w.cold));
        let corpora = suite.corpora.as_ref().unwrap();
        // the module-graph rows run inside fuz_util's root, importing only below it
        let graph: Vec<&Workload> = suite
            .workloads
            .iter()
            .filter(|w| w.collection.is_some())
            .collect();
        assert_eq!(graph.len(), 2);
        for workload in graph {
            let collection = workload.collection.as_deref().unwrap();
            assert_eq!(collection, "fuz_util");
            let script = fs::read_to_string(dir.join(&workload.script)).unwrap();
            let imports: Vec<&str> = script
                .lines()
                .filter_map(|line| line.strip_prefix("import '")?.strip_suffix("';"))
                .collect();
            assert!(!imports.is_empty());
            // in the sibling checkout, when there is one: each import is a file of the collection
            let checkout = dir.join(&corpora.path).join("collections").join(collection);
            for import in imports {
                let path = Path::new(import);
                assert!(import.starts_with("./src/lib/"), "{import}");
                assert!(
                    path.components()
                        .skip(1)
                        .all(|part| matches!(part, std::path::Component::Normal(_))),
                    "{import}"
                );
                assert!(
                    !checkout.is_dir() || checkout.join(path).is_file(),
                    "{import}"
                );
            }
        }
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
