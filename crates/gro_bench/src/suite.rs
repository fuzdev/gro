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
//! A workload can name a `collection` of the corpora checkout the suite pins,
//! and the suite pins each collection its workloads name by that
//! collection's own tree id, so a corpora commit that leaves those
//! collections alone doesn't move the pin. The run writes a fresh copy of the
//! collection from its pinned tree, puts the
//! workload's script at the copy's root under its own file name, and runs it
//! from there ([`collection`](crate::collection)), so the script's relative
//! imports are relative to the copy (`./src/lib/array.ts`) and stay inside
//! the collection's own root. The command still runs from the suite file's
//! directory.
//!
//! A workload can instead name a `project`, a committed directory holding its
//! script and pinned by the hash of its files: the run writes a fresh copy
//! of it, refused when it hashes otherwise, with a generated `node_modules`
//! of stub packages and links to sibling checkouts
//! ([`project`](crate::project)), and runs the script from the copy.
//!
//! A runtime can read a file on stdin (`stdin`, opened fresh for each
//! process), and can say what its stdout holds (`stdout`): before any timed
//! sample, each row runs once, untimed, and must exit successfully with
//! stdout of that shape, or the run fails. A suite's `parity` names runtimes
//! whose stdout values must be equal on every workload, and its `gate` names
//! the rows judged against an incumbent; every other row is reported only.
//!
//! Every process of a runtime a run spawns has a deadline, the suite's
//! `timeout_ms` or two minutes: one still running past it is killed, and the
//! run fails.
//!
//! Everything runtime-specific (a flag, a variable, a shim file) sits in the
//! runtime's entry and counts against that runtime; workload scripts are web
//! standards only, run unmodified by every runtime.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::num::{NonZeroU32, NonZeroU64};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::BenchError;

/// A suite file's contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    /// The suite's name, recorded with its version.
    pub name: String,
    /// Bumped whenever the workload set or a project's files change, since a
    /// score compares only within one version.
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
    /// The corpora checkout the workloads' collections are copied from, each
    /// collection pinned: the run refuses a collection at another tree or
    /// with local changes, so a re-pin is a suite change, never a silent
    /// one.
    #[serde(default)]
    pub corpora: Option<CorporaPin>,
    /// Runtimes whose stdout values ([`Stdout`]) must be equal on every
    /// workload, checked on the untimed run before any sample: the run fails
    /// naming the workload and where the values first differ. Absent or
    /// empty serializes as no key, like every field added to the suite since
    /// the first record, so an older suite's config hash doesn't move.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parity: Vec<String>,
    /// The rows judged against an incumbent in the report. Without one, the
    /// report gates each `fgro` row against Node, where the suite has it;
    /// with one, only the rows it names, and every other row is reported
    /// only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<Gate>,
    /// Size limits, by runtime name, that fail a run crossing one before
    /// any sample ([`Budget`]). Left out of the config hash, since a limit
    /// changes nothing a run measures.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub budgets: BTreeMap<String, Budget>,
    /// Makes the suite a prices suite: every other runtime is a variant of
    /// the one it names, the default, with one of its defaults turned off,
    /// and the run prints what each costs ([`Prices`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prices: Option<Prices>,
    /// How long any one process of the run may take, in milliseconds,
    /// before it's killed and the run fails: every sample, warmup,
    /// correctness run, and probe. Absent, [`DEFAULT_TIMEOUT_MS`]. Left out
    /// of the config hash, since a deadline changes nothing a run measures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<NonZeroU64>,
}

/// The deadline on each process when a suite gives no `timeout_ms`: two
/// minutes, far past any startup a suite times, so only a hung process
/// reaches it.
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// A runtime's size limits, each in bytes; a run whose runtime crosses one
/// fails before its samples, naming the budget, the size, and the limit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// The executable as installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// The executable gzipped at gzip's default level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gzip_bytes: Option<u64>,
    /// The startup snapshot the run was given
    /// ([`RunContext::snapshot`](crate::run::RunContext::snapshot)), which a
    /// run without one fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_bytes: Option<u64>,
}

/// A prices suite's default: the runtime every other one is priced against.
///
/// Each other runtime is the default with one of its defaults
/// turned off (by a variable or a flag in its entry), named for that
/// default, and its price on a workload is the default's median less its
/// own: what keeping the default costs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prices {
    /// The runtime with every default on.
    pub default: String,
}

/// Which rows a suite judges, and against which runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gate {
    /// The incumbent each gated row is judged against, on the same workload.
    pub against: String,
    /// The runtimes judged.
    pub runtimes: Vec<String>,
    /// The workloads they're judged on; their other workloads are reported
    /// only.
    pub workloads: Vec<String>,
}

// TODO: a check of a failing envelope (exit 1, `ok: false` of a named kind),
// for the memory suite's large-`AggregateError` workload, once eval's
// uncaught-error report stops copying each member's message

/// What a runtime's stdout holds, checked on each row's untimed run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stdout {
    /// One `fgro eval` envelope with `ok: true`; its value is the envelope's
    /// `value`.
    Envelope,
}

/// A corpora checkout, a clone of `fuzdev/corpora`, and the collections of it
/// the workloads read, each pinned,
/// `{"path": …, "collections": {"fuz_util": "<tree id>"}}`.
///
/// Only what a workload reads is pinned: a corpora commit that adds, renames,
/// or edits other collections leaves the suite valid. A workload's
/// `collection`, a copy written from the pinned tree, is the supported way to
/// read the corpora; a script that imports the corpora checkout by path reads
/// it unpinned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorporaPin {
    /// The checkout, relative to the suite file's directory.
    pub path: PathBuf,
    /// Each collection a workload names, by its directory name under
    /// `collections/`, with the tree id of that directory at `HEAD`
    /// (`git rev-parse HEAD:collections/<name>`): the bytes its workloads
    /// read, and the tree its copy is written from.
    pub collections: BTreeMap<String, String>,
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
    /// A file every process of the runtime reads on stdin, hashed as an
    /// artifact and opened fresh for each process before its timed span.
    /// Absent, stdin is the null device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<FileArg>,
    /// What the runtime's stdout holds, checked once per row before the
    /// samples. Absent, only the exit status is checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<Stdout>,
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
    /// The workload's weight relative to the other workloads', reserved for
    /// a suite objective nothing computes yet (a weighted geomean of
    /// per-workload scores); for now it only feeds the config hash. Part of
    /// the suite's version, like the workload set.
    #[serde(default = "default_weight")]
    pub weight: u32,
    /// A collection of the suite's pinned corpora (a directory of its
    /// `collections/`) the workload runs inside: the run copies it from its
    /// pin in `corpora.collections` and runs the script from the copy's
    /// root, so the script's relative imports are written relative to that
    /// root. Absent runs the script where it is, and serializes as no key, so
    /// the config hash of a suite that names none doesn't depend on the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// A committed project the workload runs inside: the run copies it fresh,
    /// writes its `node_modules`, and runs the script from the copy. The
    /// script is a file inside the project's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Project>,
}

/// A workload's project, `{"dir": …, "stub_packages": N, "links": {…}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// The committed directory, relative to the suite file's directory,
    /// holding the project's own files and no `node_modules`.
    pub dir: PathBuf,
    /// The pin of the directory's files, their
    /// [`project_blake3`](crate::project::project_blake3): the run refuses a
    /// copy that hashes otherwise, so the config hash covers the files the
    /// workload reads. A change to them bumps the suite's `version`, which a
    /// test holds against the history: no record of the suite's current
    /// version may name other files.
    pub blake3: String,
    /// How many stub packages the copy's `node_modules` holds: directories
    /// each holding only a `package.json` with a name, `"type": "module"`,
    /// and `"exports"`, so a runtime that walks `node_modules` has the
    /// breadth of a real project's to walk.
    #[serde(default)]
    pub stub_packages: u32,
    /// Packages linked into the copy's `node_modules`, by package name (`a`
    /// or `@scope/a`), each a symlink to a checkout given relative to the
    /// suite file's directory. A missing checkout fails the run.
    #[serde(default)]
    pub links: BTreeMap<String, PathBuf>,
    /// Files the run generates into the copy, by their path in it (plain
    /// segments, outside `node_modules`, and none the committed directory
    /// has), so a large input needn't be committed. Absent serializes as no
    /// key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub generate: BTreeMap<PathBuf, Generate>,
}

/// A file a project's copy gets generated, the same bytes on every run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Generate {
    /// An ES module whose default export is an array of small records, as
    /// JSON text, of at most `bytes` bytes and within a record's length of
    /// it ([`write_data_module`](crate::project::write_data_module)).
    DataModule {
        /// The module's size.
        bytes: u64,
    },
}

const fn default_weight() -> u32 {
    1
}

impl Suite {
    /// The deadline on each process of the run: `timeout_ms`, or
    /// [`DEFAULT_TIMEOUT_MS`].
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.map_or(DEFAULT_TIMEOUT_MS, NonZeroU64::get))
    }

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
    /// workload projects that hold their scripts, workload collections that
    /// the corpora pin names, each with one script per entry file name, a
    /// pin for no collection no workload names, and `parity` and `gate`
    /// naming what the suite has.
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
        self.validate_projects()?;
        self.validate_collections()?;
        self.validate_checks()?;
        self.validate_budgets_and_prices()
    }

    /// Checks that `budgets` and `prices` name the suite's runtimes, and that
    /// a prices suite has a variant to price.
    fn validate_budgets_and_prices(&self) -> Result<(), BenchError> {
        let has = |name: &str| self.runtimes.iter().any(|r| r.name == name);
        if let Some(name) = self.budgets.keys().find(|name| !has(name)) {
            return Err(BenchError::SuiteInvalid(format!(
                "`budgets` names the runtime `{name}`, which the suite doesn't have"
            )));
        }
        if let Some(prices) = &self.prices {
            if !has(&prices.default) {
                return Err(BenchError::SuiteInvalid(format!(
                    "`prices` names the runtime `{}`, which the suite doesn't have",
                    prices.default
                )));
            }
            if self.runtimes.len() < 2 {
                return Err(BenchError::SuiteInvalid(
                    "`prices` prices variants of its default, so the suite needs another runtime"
                        .to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Each project's pin by its directory as the suite writes it, which
    /// the run's record carries: one entry for workloads sharing a
    /// directory, whose pins validation holds equal.
    pub fn project_pins(&self) -> BTreeMap<String, String> {
        self.workloads
            .iter()
            .filter_map(|workload| workload.project.as_ref())
            .map(|project| {
                (
                    project.dir.to_string_lossy().into_owned(),
                    project.blake3.clone(),
                )
            })
            .collect()
    }

    /// Checks each workload's `project`: no `collection` beside it, a name
    /// that's one plain path segment (it names the copy), a script inside
    /// its directory by plain segments, links naming packages, and a pin
    /// that's a blake3 hash in lowercase hex, the same for every workload
    /// sharing the directory.
    fn validate_projects(&self) -> Result<(), BenchError> {
        let mut pins: BTreeMap<&Path, &str> = BTreeMap::new();
        for workload in &self.workloads {
            let Some(project) = &workload.project else {
                continue;
            };
            let invalid = |problem: String| {
                Err(BenchError::SuiteInvalid(format!(
                    "the workload `{}` {problem}",
                    workload.name
                )))
            };
            if workload.collection.is_some() {
                return invalid("names both a collection and a project".to_owned());
            }
            if !is_plain_segment(&workload.name) {
                return invalid(
                    "has a project, so its name must be a single directory name".to_owned(),
                );
            }
            let inside = workload
                .script
                .strip_prefix(&project.dir)
                .ok()
                .filter(|rest| is_plain_relative(rest));
            if inside.is_none() {
                return invalid(format!(
                    "has the script `{}`, which isn't a file inside its project `{}`",
                    workload.script.display(),
                    project.dir.display()
                ));
            }
            let is_hex = project.blake3.len() == 64
                && project
                    .blake3
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
            if !is_hex {
                return invalid(format!(
                    "pins its project by `{}`, which isn't a blake3 hash in lowercase hex",
                    project.blake3
                ));
            }
            if *pins
                .entry(project.dir.as_path())
                .or_insert(project.blake3.as_str())
                != project.blake3
            {
                return invalid(format!(
                    "pins its project `{}` otherwise than another workload does",
                    project.dir.display()
                ));
            }
            for name in project.links.keys() {
                if !is_package_name(name) {
                    return invalid(format!(
                        "links `{name}`, which isn't a package name (`a` or `@scope/a`)"
                    ));
                }
                if crate::project::is_stub_name(name, project.stub_packages) {
                    return invalid(format!(
                        "links `{name}`, which is one of the {} stub packages its `node_modules` gets",
                        project.stub_packages
                    ));
                }
            }
            for (path, generate) in &project.generate {
                let in_node_modules = path
                    .components()
                    .next()
                    .is_some_and(|first| first.as_os_str() == "node_modules");
                if !is_plain_relative(path) || in_node_modules {
                    return invalid(format!(
                        "generates `{}`, which isn't a path inside its project outside `node_modules`",
                        path.display()
                    ));
                }
                let Generate::DataModule { bytes } = generate;
                if *bytes < crate::project::DATA_MODULE_MIN_BYTES {
                    return invalid(format!(
                        "generates `{}` of {bytes} bytes, below a data module's least, {}",
                        path.display(),
                        crate::project::DATA_MODULE_MIN_BYTES
                    ));
                }
            }
        }
        Ok(())
    }

    /// Checks the names `parity` and `gate` use: known runtimes and
    /// workloads, each once, parity's runtimes each with a `stdout`, and a
    /// gate's incumbent not among the runtimes it judges.
    fn validate_checks(&self) -> Result<(), BenchError> {
        let runtime = |name: &str| self.runtimes.iter().find(|r| r.name == name);
        let unknown = |kind: &str, name: &str, field: &str| {
            Err(BenchError::SuiteInvalid(format!(
                "`{field}` names the {kind} `{name}`, which the suite doesn't have"
            )))
        };
        if !self.parity.is_empty() {
            if self.parity.len() < 2 {
                return Err(BenchError::SuiteInvalid(
                    "`parity` compares runtimes, so it names at least two".to_owned(),
                ));
            }
            unique("parity runtime", self.parity.iter().map(String::as_str))?;
            for name in &self.parity {
                match runtime(name) {
                    None => return unknown("runtime", name, "parity"),
                    Some(found) if found.stdout.is_none() => {
                        return Err(BenchError::SuiteInvalid(format!(
                            "`parity` compares the runtime `{name}`, which has no `stdout` saying what its value is"
                        )));
                    }
                    Some(_) => {}
                }
            }
        }
        if let Some(gate) = &self.gate {
            for name in std::iter::once(&gate.against).chain(&gate.runtimes) {
                if runtime(name).is_none() {
                    return unknown("runtime", name, "gate");
                }
            }
            for name in &gate.workloads {
                if !self.workloads.iter().any(|w| &w.name == name) {
                    return unknown("workload", name, "gate");
                }
            }
            unique("gate runtime", gate.runtimes.iter().map(String::as_str))?;
            unique("gate workload", gate.workloads.iter().map(String::as_str))?;
            if gate.runtimes.is_empty() || gate.workloads.is_empty() {
                return Err(BenchError::SuiteInvalid(
                    "`gate` names at least one runtime and one workload".to_owned(),
                ));
            }
            if gate.runtimes.contains(&gate.against) {
                return Err(BenchError::SuiteInvalid(format!(
                    "`gate` judges the runtime `{}` against itself",
                    gate.against
                )));
            }
        }
        Ok(())
    }

    /// Checks each workload's `collection`: it needs the suite's `corpora`
    /// and a pin there, it names a directory of `collections/` (one plain
    /// path segment), and the scripts sharing a collection's copy have
    /// distinct file names, since each sits at the copy's root under its own.
    /// Then checks the pin names only collections a workload names.
    fn validate_collections(&self) -> Result<(), BenchError> {
        let mut entries: BTreeMap<(&str, &OsStr), &Path> = BTreeMap::new();
        for workload in &self.workloads {
            let Some(collection) = &workload.collection else {
                continue;
            };
            let Some(corpora) = &self.corpora else {
                return Err(BenchError::SuiteInvalid(format!(
                    "the workload `{}` names the collection `{collection}`, but the suite pins no `corpora`",
                    workload.name
                )));
            };
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
            if !corpora.collections.contains_key(collection) {
                return Err(BenchError::SuiteInvalid(format!(
                    "the workload `{}` names the collection `{collection}`, which `corpora.collections` doesn't pin",
                    workload.name
                )));
            }
        }
        if let Some(corpora) = &self.corpora {
            if corpora.collections.is_empty() {
                return Err(BenchError::SuiteInvalid(
                    "`corpora` pins no collection, so no workload reads it".to_owned(),
                ));
            }
            let named = |name: &str| {
                self.workloads
                    .iter()
                    .any(|w| w.collection.as_deref() == Some(name))
            };
            if let Some(name) = corpora.collections.keys().find(|name| !named(name)) {
                return Err(BenchError::SuiteInvalid(format!(
                    "`corpora.collections` pins `{name}`, which no workload names"
                )));
            }
        }
        Ok(())
    }

    /// The blake3 hash of the suite as parsed (defaults filled, formatting
    /// ignored), its `budgets` and `timeout_ms` left out: two runs with the
    /// same hash ran the same rows the same way.
    ///
    /// # Panics
    ///
    /// Never: a suite always serializes.
    pub fn config_hash(&self) -> String {
        // a size limit or a deadline changes nothing a run measures
        let measured = Self {
            budgets: BTreeMap::new(),
            timeout_ms: None,
            ..self.clone()
        };
        #[expect(
            clippy::expect_used,
            reason = "a suite serializes infallibly: its maps have string keys, and paths parsed from JSON are UTF-8"
        )]
        let json = serde_json::to_vec(&measured).expect("a suite serializes to JSON");
        blake3::hash(&json).to_hex().to_string()
    }
}

/// Whether `name` is one plain path segment: not empty, `.`, or `..`, and
/// with no separator.
fn is_plain_segment(name: &str) -> bool {
    !matches!(name, "" | "." | "..") && !name.contains(['/', '\\', '\0'])
}

/// Whether `path` is one or more plain segments, with no root, `.`, or `..`.
pub(crate) fn is_plain_relative(path: &Path) -> bool {
    path.components().next().is_some()
        && path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
}

/// Whether `name` is a package name as a `node_modules` path spells it: one
/// plain segment, or `@scope/` then one, never starting with `.`.
pub(crate) fn is_package_name(name: &str) -> bool {
    let segment =
        |part: &str| is_plain_segment(part) && !part.starts_with('.') && !part.starts_with('@');
    name.strip_prefix('@').map_or_else(
        || segment(name),
        |scoped| {
            scoped
                .split_once('/')
                .is_some_and(|(scope, rest)| segment(scope) && segment(rest))
        },
    )
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
                    "workloads": [{{"name": "w", "script": "w.js", "cold": true, "weight": 3, "collection": "c"}}],
                    "corpora": {{"path": "../corpora", "collections": {{"c": "abc"}}}}
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
                collections: BTreeMap::from([("c".to_owned(), "abc".to_owned())])
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
        // `pins` is the `corpora.collections` object's body, none without one
        let suite = |pins: Option<&str>, workloads: &str| {
            let corpora = pins.map_or(String::new(), |pins| {
                format!(r#", "corpora": {{"path": "../corpora", "collections": {{{pins}}}}}"#)
            });
            format!(
                r#"{{"name": "s", "version": 1, "samples": 2,
                    "runtimes": [{{"name": "r", "program": "r"}}],
                    "workloads": [{workloads}]{corpora}}}"#
            )
        };
        let parsed = parse(&suite(
            Some(r#""fuz_util": "abc""#),
            r#"{"name": "w", "script": "dir/w.js", "collection": "fuz_util"},
               {"name": "v", "script": "dir/w.js", "collection": "fuz_util"},
               {"name": "x", "script": "x.js", "collection": "fuz_util"},
               {"name": "y", "script": "x.js"}"#,
        ))
        .unwrap();
        assert_eq!(parsed.workloads[0].collection.as_deref(), Some("fuz_util"));
        assert_eq!(parsed.workloads[3].collection, None);
        assert_eq!(
            parsed.corpora.unwrap().collections,
            BTreeMap::from([("fuz_util".to_owned(), "abc".to_owned())])
        );
        // a collection needs the corpora it's copied from
        assert!(matches!(
            parse(&suite(None, r#"{"name": "w", "script": "w.js", "collection": "c"}"#)),
            Err(BenchError::SuiteInvalid(message)) if message.contains("pins no `corpora`")
        ));
        // one plain directory name
        for bad in ["", ".", "..", "a/b", "/a", "a/", r"a\\b"] {
            assert!(
                matches!(
                    parse(&suite(Some(r#""c": "a""#), &format!(r#"{{"name": "w", "script": "w.js", "collection": "{bad}"}}"#))),
                    Err(BenchError::SuiteInvalid(ref message)) if message.contains("single directory name")
                ),
                "{bad}"
            );
        }
        // two scripts can't be one entry of a collection
        assert!(matches!(
            parse(&suite(
                Some(r#""c": "a""#),
                r#"{"name": "w", "script": "a/w.js", "collection": "c"},
                   {"name": "v", "script": "b/w.js", "collection": "c"}"#
            )),
            Err(BenchError::SuiteInvalid(message)) if message.contains("entry `w.js`")
        ));
        // but they can in different collections
        assert!(
            parse(&suite(
                Some(r#""c": "a", "d": "b""#),
                r#"{"name": "w", "script": "a/w.js", "collection": "c"},
                   {"name": "v", "script": "b/w.js", "collection": "d"}"#
            ))
            .is_ok()
        );
        // a script naming no file
        assert!(matches!(
            parse(&suite(Some(r#""c": "a""#), r#"{"name": "w", "script": "..", "collection": "c"}"#)),
            Err(BenchError::SuiteInvalid(message)) if message.contains("names no file")
        ));
    }

    #[test]
    fn the_corpora_pin_names_exactly_the_collections_read() {
        let suite = |pins: &str, collections: &[&str]| {
            let workloads: Vec<String> = collections
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    format!(r#"{{"name": "w{i}", "script": "w{i}.js", "collection": "{c}"}}"#)
                })
                .chain(std::iter::once(
                    r#"{"name": "plain", "script": "p.js"}"#.to_owned(),
                ))
                .collect();
            format!(
                r#"{{"name": "s", "version": 1, "samples": 2,
                    "runtimes": [{{"name": "r", "program": "r"}}],
                    "workloads": [{}],
                    "corpora": {{"path": "../corpora", "collections": {{{pins}}}}}}}"#,
                workloads.join(", ")
            )
        };
        assert!(parse(&suite(r#""a": "1", "b": "2""#, &["a", "b", "a"])).is_ok());
        // a workload naming a collection the pin doesn't
        assert!(matches!(
            parse(&suite(r#""a": "1""#, &["a", "b"])),
            Err(BenchError::SuiteInvalid(message))
                if message.contains("`w1` names the collection `b`, which `corpora.collections` doesn't pin")
        ));
        // a pin for a collection no workload names
        assert!(matches!(
            parse(&suite(r#""a": "1", "z": "2""#, &["a"])),
            Err(BenchError::SuiteInvalid(message))
                if message.contains("pins `z`, which no workload names")
        ));
        // a pin of nothing
        assert!(matches!(
            parse(&suite("", &[])),
            Err(BenchError::SuiteInvalid(message)) if message.contains("pins no collection")
        ));
        // the whole-tree pin is gone, so naming it is an unknown field
        assert!(matches!(
            parse(&suite(r#""a": "1"}, "tree": {"x": "y""#, &["a"])),
            Err(BenchError::SuiteParse { .. })
        ));
    }

    #[test]
    fn an_absent_collection_leaves_the_config_hash_alone() {
        let json = |collection: &str, corpora: &str| {
            format!(
                r#"{{"name":"s","version":1,"samples":4,"runtimes":[{{"name":"r","program":"r"}}],
                    "workloads":[{{"name":"w","script":"w.js"{collection}}}]{corpora}}}"#
            )
        };
        let absent = parse(&json("", "")).unwrap();
        let value = serde_json::to_value(&absent).unwrap();
        // absent serializes as no key, so the hash of a suite without collections is unchanged
        assert!(value["workloads"][0].get("collection").is_none(), "{value}");
        let present = parse(&json(
            r#","collection":"c""#,
            r#","corpora":{"path":"p","collections":{"c":"t"}}"#,
        ))
        .unwrap();
        assert_ne!(absent.config_hash(), present.config_hash());
        let other = parse(&json(
            r#","collection":"d""#,
            r#","corpora":{"path":"p","collections":{"d":"t"}}"#,
        ))
        .unwrap();
        assert_ne!(present.config_hash(), other.config_hash());
        // a re-pin moves the hash
        let repinned = parse(&json(
            r#","collection":"c""#,
            r#","corpora":{"path":"p","collections":{"c":"u"}}"#,
        ))
        .unwrap();
        assert_ne!(present.config_hash(), repinned.config_hash());
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

    /// The baseline suite's config hash is the one its records carry: the
    /// fields added since serialize as no key when absent.
    #[test]
    fn absent_checks_leave_the_config_hash_alone() {
        let json = r#"{"name":"s","version":1,"samples":4,
            "runtimes":[{"name":"r","program":"r"}],"workloads":[{"name":"w","script":"w.js"}]}"#;
        let suite = parse(json).unwrap();
        let value = serde_json::to_value(&suite).unwrap();
        for key in ["parity", "gate"] {
            assert!(value.get(key).is_none(), "{value}");
        }
        for key in ["stdin", "stdout"] {
            assert!(value["runtimes"][0].get(key).is_none(), "{value}");
        }
        assert!(value["workloads"][0].get("project").is_none(), "{value}");
    }

    #[test]
    fn parity_and_gate_name_what_the_suite_has() {
        let suite = |extra: &str| {
            format!(
                r#"{{"name": "s", "version": 1, "samples": 2,
                    "runtimes": [
                        {{"name": "fgro", "program": "f", "stdin": {{"file": "d.js"}}, "stdout": "envelope"}},
                        {{"name": "deno", "program": "d", "stdout": "envelope"}},
                        {{"name": "box", "program": "f"}}
                    ],
                    "workloads": [{{"name": "w", "script": "w.js"}}, {{"name": "v", "script": "v.js"}}]{extra}}}"#
            )
        };
        let parsed = parse(&suite(
            r#", "parity": ["fgro", "deno"], "gate": {"against": "deno", "runtimes": ["fgro"], "workloads": ["w"]}"#,
        ))
        .unwrap();
        assert_eq!(parsed.runtimes[0].stdout, Some(Stdout::Envelope));
        assert_eq!(
            parsed.runtimes[0].stdin,
            Some(FileArg {
                file: PathBuf::from("d.js")
            })
        );
        assert_eq!(parsed.parity, ["fgro", "deno"]);
        assert_eq!(parsed.gate.as_ref().unwrap().against, "deno");
        for (extra, says) in [
            (r#", "parity": ["fgro"]"#, "at least two"),
            (r#", "parity": ["fgro", "nope"]"#, "`nope`"),
            (r#", "parity": ["fgro", "box"]"#, "no `stdout`"),
            (r#", "parity": ["fgro", "fgro"]"#, "used twice"),
            (
                r#", "gate": {"against": "nope", "runtimes": ["fgro"], "workloads": ["w"]}"#,
                "`nope`",
            ),
            (
                r#", "gate": {"against": "deno", "runtimes": ["fgro"], "workloads": ["x"]}"#,
                "`x`",
            ),
            (
                r#", "gate": {"against": "deno", "runtimes": ["deno"], "workloads": ["w"]}"#,
                "against itself",
            ),
            (
                r#", "gate": {"against": "deno", "runtimes": [], "workloads": ["w"]}"#,
                "at least one",
            ),
        ] {
            assert!(
                matches!(parse(&suite(extra)), Err(BenchError::SuiteInvalid(ref message)) if message.contains(says)),
                "{extra}"
            );
        }
        assert!(matches!(
            parse(&suite(r#", "runtimes": []"#)),
            Err(BenchError::SuiteParse { .. })
        ));
    }

    /// A pin in the shape a project's `blake3` takes, for suites whose
    /// directories aren't hashed.
    const PIN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_project_holds_its_script_and_names_its_links() {
        // every project here pins its files by `PIN`
        let suite = |workload: &str| {
            let workload =
                workload.replace(r#""dir": "#, &format!(r#""blake3": "{PIN}", "dir": "#));
            format!(
                r#"{{"name": "s", "version": 1, "samples": 2,
                    "runtimes": [{{"name": "r", "program": "r"}}],
                    "workloads": [{workload}]}}"#
            )
        };
        let parsed = parse(&suite(
            r#"{"name": "w", "script": "p/src/zap.ts", "project": {"dir": "p", "stub_packages": 3, "links": {"@a/b": "../b", "c": "../c"}}}"#,
        ))
        .unwrap();
        let project = parsed.workloads[0].project.as_ref().unwrap();
        assert_eq!(project.stub_packages, 3);
        assert_eq!(project.links.len(), 2);
        for (workload, says) in [
            (
                r#"{"name": "w", "script": "q/zap.ts", "project": {"dir": "p"}}"#,
                "inside its project",
            ),
            (
                r#"{"name": "w", "script": "p", "project": {"dir": "p"}}"#,
                "inside its project",
            ),
            (
                r#"{"name": "w", "script": "p/../q.ts", "project": {"dir": "p"}}"#,
                "inside its project",
            ),
            (
                r#"{"name": "a/b", "script": "p/z.ts", "project": {"dir": "p"}}"#,
                "single directory name",
            ),
            (
                r#"{"name": "w", "script": "p/z.ts", "project": {"dir": "p", "links": {"@a": "x"}}}"#,
                "package name",
            ),
            (
                r#"{"name": "w", "script": "p/z.ts", "project": {"dir": "p", "links": {".bin": "x"}}}"#,
                "package name",
            ),
            (
                r#"{"name": "w", "script": "p/z.ts", "project": {"dir": "p", "links": {"a/b": "x"}}}"#,
                "package name",
            ),
            (
                r#"{"name": "w", "script": "p/z.ts", "collection": "c", "project": {"dir": "p"}}"#,
                "both",
            ),
            (
                r#"{"name": "w", "script": "p/z.ts", "project": {"dir": "p", "stub_packages": 3, "links": {"stub_0002": "x"}}}"#,
                "stub packages",
            ),
        ] {
            assert!(
                matches!(parse(&suite(workload)), Err(BenchError::SuiteInvalid(ref message)) if message.contains(says)),
                "{workload}: {:?}",
                parse(&suite(workload))
            );
        }
    }

    #[test]
    fn a_project_pins_its_files_once() {
        let suite = |pins: [&str; 2], dirs: [&str; 2]| {
            parse(&format!(
                r#"{{"name": "s", "version": 1, "samples": 2,
                    "runtimes": [{{"name": "r", "program": "r"}}],
                    "workloads": [
                        {{"name": "w", "script": "{}/w.js", "project": {{"dir": "{}", "blake3": "{}"}}}},
                        {{"name": "v", "script": "{}/v.js", "project": {{"dir": "{}", "blake3": "{}"}}}}
                    ]}}"#,
                dirs[0], dirs[0], pins[0], dirs[1], dirs[1], pins[1]
            ))
        };
        let other = PIN.replace('0', "f");
        let parsed = suite([PIN, PIN], ["p", "p"]).unwrap();
        assert_eq!(
            parsed.project_pins(),
            BTreeMap::from([("p".to_owned(), PIN.to_owned())])
        );
        let parsed = suite([PIN, &other], ["p", "q"]).unwrap();
        assert_eq!(parsed.project_pins().len(), 2);
        // two pins of one directory
        assert!(matches!(
            suite([PIN, &other], ["p", "p"]),
            Err(BenchError::SuiteInvalid(message)) if message.contains("otherwise than another workload")
        ));
        // not a blake3 hash in lowercase hex
        for bad in [
            &PIN[1..],
            &PIN.to_uppercase(),
            &format!("{PIN}0"),
            &PIN.replace('a', "g"),
            "",
        ] {
            assert!(
                matches!(
                    suite([bad, bad], ["p", "p"]),
                    Err(BenchError::SuiteInvalid(ref message)) if message.contains("lowercase hex")
                ),
                "{bad}"
            );
        }
        // the pin is part of the config
        assert_ne!(
            suite([PIN, PIN], ["p", "p"]).unwrap().config_hash(),
            suite([&other, &other], ["p", "p"]).unwrap().config_hash()
        );
        // and required
        assert!(matches!(
            parse(
                r#"{"name": "s", "version": 1, "samples": 2, "runtimes": [{"name": "r", "program": "r"}],
                    "workloads": [{"name": "w", "script": "p/w.js", "project": {"dir": "p"}}]}"#
            ),
            Err(BenchError::SuiteParse { .. })
        ));
    }

    /// Each committed suite, by its directory, from `suites/*/suite.json`.
    fn committed_suites() -> Vec<(String, PathBuf, Suite)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("suites");
        let mut suites: Vec<(String, PathBuf, Suite)> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|dir| dir.join("suite.json").is_file())
            .map(|dir| {
                let suite = Suite::load(&dir.join("suite.json")).unwrap();
                let name = dir.file_name().unwrap().to_string_lossy().into_owned();
                (name, dir, suite)
            })
            .collect();
        suites.sort_by(|a, b| a.0.cmp(&b.0));
        suites
    }

    /// Each project of each committed suite whose files don't hash to its
    /// pin, saying what to do.
    fn unpinned_projects(suites: &[(String, PathBuf, Suite)]) -> Vec<String> {
        let mut problems = Vec::new();
        for (name, dir, suite) in suites {
            for workload in &suite.workloads {
                let Some(project) = &workload.project else {
                    continue;
                };
                let found = crate::project::project_blake3(&workload.name, &dir.join(&project.dir))
                    .unwrap();
                if found != project.blake3 {
                    problems.push(format!(
                        "the suite `{name}` (v{}) pins its project `{}` by {}, but its files hash to {found}: a change to them bumps the suite's `version` and re-pins the project's `blake3`, or, if the change is a file git doesn't track, remove it",
                        suite.version,
                        project.dir.display(),
                        project.blake3
                    ));
                }
            }
        }
        problems
    }

    /// Each record in `history` of a suite's current version that names
    /// other project files than the suite pins: those files changed with the
    /// version kept, so records of one version measured different
    /// workloads. A record from before projects were recorded names none,
    /// and isn't checked.
    fn records_of_other_files(suites: &[(String, PathBuf, Suite)], history: &str) -> Vec<String> {
        let mut problems = Vec::new();
        for (index, line) in history.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record = crate::report::read_line(line)
                .unwrap_or_else(|error| panic!("the history's line {}: {error}", index + 1));
            if record.projects.is_empty() {
                continue;
            }
            for (_, _, suite) in suites {
                if record.suite.name == suite.name
                    && record.suite.version == suite.version
                    && record.projects != suite.project_pins()
                {
                    problems.push(format!(
                        "the history's line {} is a record of the suite `{}` v{} that read the project files {:?}, but the suite now pins {:?}: they changed with the version kept, so bump the suite's `version`",
                        index + 1,
                        suite.name,
                        suite.version,
                        record.projects,
                        suite.project_pins()
                    ));
                }
            }
        }
        problems
    }

    /// The guard on the committed suites: each project's files hash to the
    /// pin, and no record of a suite's current version read other files, so
    /// a change to a project's files moves its suite's `version`.
    #[test]
    fn the_committed_projects_are_pinned_within_their_version() {
        let suites = committed_suites();
        assert!(
            suites
                .iter()
                .any(|(_, _, suite)| !suite.project_pins().is_empty())
        );
        let unpinned = unpinned_projects(&suites);
        assert!(unpinned.is_empty(), "{}", unpinned.join("\n"));
        let history =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("history.jsonl"))
                .unwrap();
        let moved = records_of_other_files(&suites, &history);
        assert!(moved.is_empty(), "{}", moved.join("\n"));
    }

    #[test]
    fn a_record_of_the_version_with_other_files_fails_the_guard() {
        let suite = parse(&format!(
            r#"{{"name": "s", "version": 2, "samples": 2, "runtimes": [{{"name": "r", "program": "r"}}],
                "workloads": [{{"name": "w", "script": "p/w.js", "project": {{"dir": "p", "blake3": "{PIN}"}}}}]}}"#
        ))
        .unwrap();
        let suites = [("s".to_owned(), PathBuf::new(), suite)];
        let line = |version: u32, pins: &[(&str, &str)]| {
            let mut record = crate::test_support::run_record(vec![crate::test_support::row(
                "r",
                "w",
                crate::test_support::metric(vec![1, 2]),
            )]);
            record.suite = crate::record::SuiteId {
                name: "s".to_owned(),
                version,
            };
            record.projects = pins
                .iter()
                .map(|(dir, pin)| ((*dir).to_owned(), (*pin).to_owned()))
                .collect();
            record.to_line()
        };
        let other = PIN.replace('0', "f");
        // the pinned files, an older version's other files, and a record naming none
        let fine = [
            line(2, &[("p", PIN)]),
            line(1, &[("p", &other)]),
            line(2, &[]),
        ]
        .concat();
        assert!(records_of_other_files(&suites, &fine).is_empty());
        // the files changed under version 2, or another project was read
        for pins in [&[("p", other.as_str())][..], &[("p", PIN), ("q", PIN)][..]] {
            let moved = records_of_other_files(&suites, &[fine.clone(), line(2, pins)].concat());
            assert_eq!(moved.len(), 1, "{moved:?}");
            assert!(
                moved[0].starts_with("the history's line 4 is a record of the suite `s` v2"),
                "{}",
                moved[0]
            );
        }
    }

    #[test]
    fn the_eval_suite_is_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("suites/eval");
        let suite = Suite::load(&dir.join("suite.json")).unwrap();
        // `fgro` alone, with no incumbent: the perf rule's A/B against a base judges it
        let runtimes: Vec<&str> = suite.runtimes.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(runtimes, ["fgro"]);
        assert_eq!(suite.runtimes[0].stdout, Some(Stdout::Envelope));
        assert!(suite.gate.is_none() && suite.parity.is_empty());
        let project = suite.workloads[0].project.as_ref().unwrap();
        // the fixture's files are committed, its node_modules generated
        assert!(dir.join(&suite.workloads[0].script).is_file());
        assert!(dir.join(&project.dir).join("package.json").is_file());
        assert!(!dir.join(&project.dir).join("node_modules").exists());
    }

    #[test]
    fn the_prices_suite_is_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("suites/prices");
        let suite = Suite::load(&dir.join("suite.json")).unwrap();
        let prices = suite.prices.as_ref().unwrap();
        let default = suite
            .runtimes
            .iter()
            .find(|runtime| runtime.name == prices.default)
            .unwrap();
        assert!(!default.env.contains_key("GRO_PRICES_OFF"));
        // each variant turns off something of its own, by a switch or a flag
        let mut seen = HashSet::new();
        for runtime in &suite.runtimes {
            let switch = format!("{:?} {:?}", runtime.env.get("GRO_PRICES_OFF"), runtime.args);
            assert!(seen.insert(switch), "{}", runtime.name);
            assert_eq!(runtime.program, default.program, "{}", runtime.name);
            assert_eq!(runtime.stdin, default.stdin, "{}", runtime.name);
        }
        // every variant's value must be the default's
        assert_eq!(suite.parity.len(), suite.runtimes.len());
        assert!(suite.budgets.is_empty());
    }

    #[test]
    fn the_memory_suite_is_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("suites/memory");
        let suite = Suite::load(&dir.join("suite.json")).unwrap();
        let project = suite.workloads[0].project.as_ref().unwrap();
        let Some(Generate::DataModule { bytes }) = project.generate.get(Path::new("data.js"))
        else {
            panic!("{project:?}");
        };
        // under the loader's cap on a module's source, 16 MiB, so it loads
        assert!(*bytes < 16 << 20, "{bytes}");
        assert!(!dir.join(&project.dir).join("data.js").exists());
    }

    #[test]
    fn the_baseline_suite_budgets_fgro() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("suites/baseline");
        let suite = Suite::load(&dir.join("suite.json")).unwrap();
        let budget = &suite.budgets["fgro"];
        assert!(budget.bytes.is_some() && budget.gzip_bytes.is_some());
        assert!(budget.snapshot_bytes.is_some());
    }

    #[test]
    fn budgets_prices_and_generated_files_name_what_they_may() {
        let runtime = r#"{"name": "r", "program": "r"}"#;
        let workload = r#"{"name": "w", "script": "p/w.js", "project": {"dir": "p", "blake3": "PIN", "generate": GEN}}"#;
        let suite = |extra: &str, generate: &str| {
            parse(&format!(
                r#"{{"name": "s", "version": 1, "samples": 2, "runtimes": [{runtime}],
                    "workloads": [{}]{extra}}}"#,
                workload.replace("GEN", generate).replace("PIN", PIN)
            ))
        };
        let ok = r#"{"gen/d.js": {"data_module": {"bytes": 1000}}}"#;
        assert!(suite("", ok).is_ok());
        for (extra, generate) in [
            (r#", "budgets": {"x": {"bytes": 1}}"#, ok),
            (r#", "prices": {"default": "x"}"#, ok),
            // a prices suite needs a variant
            (r#", "prices": {"default": "r"}"#, ok),
            (
                "",
                r#"{"node_modules/d.js": {"data_module": {"bytes": 1000}}}"#,
            ),
            ("", r#"{"../d.js": {"data_module": {"bytes": 1000}}}"#),
            ("", r#"{"d.js": {"data_module": {"bytes": 10}}}"#),
        ] {
            assert!(
                matches!(suite(extra, generate), Err(BenchError::SuiteInvalid(_))),
                "{extra} {generate}"
            );
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
        // a deadline measures nothing
        let mut d = a.clone();
        d.timeout_ms = NonZeroU64::new(5);
        assert_eq!(a.config_hash(), d.config_hash());
        assert_eq!(d.timeout(), Duration::from_millis(5));
        assert_eq!(a.timeout(), Duration::from_millis(DEFAULT_TIMEOUT_MS));
    }
}
