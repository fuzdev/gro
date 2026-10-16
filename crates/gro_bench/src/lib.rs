//! gro's measurement harness: runs workloads under JS runtimes, one fresh
//! process per sample, and records the results.
//!
//! It is independent of the runtime crates, so it measures other runtimes
//! (Node, Deno, Bun) as readily as gro's own.
//!
//! - [`suite`]: the declarative input, runtimes × workloads, read from JSON
//! - [`run`]: the process runner, which measures a suite's rows interleaved,
//!   with an A/A noise row, and refuses a run whose artifacts changed
//! - [`schedule`]: the interleaved order of a run's samples
//! - [`stats`]: MAD-cleaned statistics
//! - [`record`]: the run record, one JSON line per run in an append-only
//!   history
//!
//! `cargo xtask bench <suite.json>` runs a suite and appends its record.

pub mod error;
pub mod record;
pub mod run;
pub mod schedule;
pub mod stats;
pub mod suite;
