//! gro's measurement harness: runs workloads under JS runtimes, one fresh
//! process per sample, and records the results.
//!
//! It is independent of the runtime crates, so it measures other runtimes
//! (Node, Deno, Bun) as readily as gro's own.
//!
//! - [`suite`]: the declarative input, runtimes × workloads, read from JSON
//! - [`collection`]: a fresh copy of one collection of the pinned corpora,
//!   for workloads that run inside the collection's own root
//! - [`project`]: a fresh copy of a committed project with a generated
//!   `node_modules`, for workloads that run inside a project
//! - [`counters`]: counter mode, each sample under `perf stat`
//! - [`phases`]: phases mode, each sample's phase timings from a phases
//!   build of fgro
//! - [`prices`]: a prices suite's price table, what each of the default
//!   runtime's defaults costs
//! - [`check`]: the untimed correctness run's checks of a row's stdout
//! - [`run`]: the process runner, which measures a suite's rows interleaved,
//!   with an A/A noise row, and refuses a run whose artifacts changed
//! - [`budget`]: the suite's size budgets, checked against the sizes measured
//! - [`schedule`]: the interleaved order of a run's samples
//! - [`watchdog`]: the deadline on each process a run spawns
//! - [`lock`]: the exclusive lock a run holds on its work directory
//! - [`stats`]: MAD-cleaned statistics
//! - [`git`]: the git reads behind the record: each checkout's state
//! - [`record`]: the run record, one JSON line per run in an append-only
//!   history
//! - [`summary`]: what a run prints after its samples: the summary, the
//!   gate table, and an A/B's table
//! - [`report`]: a reading of the history, each row's latest record against
//!   its previous and its last attested one
//!
//! `cargo xtask bench <suite.json>` runs a suite and appends its record;
//! `cargo xtask bench report` reads the history.

pub mod budget;
pub mod check;
pub mod collection;
pub mod counters;
pub mod error;
pub mod git;
pub mod lock;
pub mod phases;
pub mod prices;
pub mod project;
pub mod record;
pub mod report;
pub mod run;
pub mod schedule;
pub mod stats;
pub mod suite;
pub mod summary;
pub mod watchdog;
