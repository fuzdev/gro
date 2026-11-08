//! gro's JS host: a minimal web-standards runtime on V8.
//!
//! [`run_main`] runs an ES module from disk to completion: it resolves the
//! entry, evaluates it on a `deno_core` `JsRuntime` driven by a current-thread
//! tokio runtime, and drains the event loop (timers, pending promises). An
//! uncaught error or an unhandled promise rejection ends the run with a
//! [`RunError::Uncaught`] carrying the report, stack and source positions
//! included.
//!
//! - [`resolver`]: the strict ESM resolver, a pure module (relative and
//!   absolute specifiers with mandatory extensions, and packages in
//!   `node_modules` through their `"exports"`, each module kept inside its
//!   project's or package's root; `.mjs`, `.ts`, and `.mts` load, and `.js`
//!   under `"type": "module"`)
//! - `loader`: the `deno_core` module loader, an adapter over the resolver
//!   that strips TypeScript's types in process with `tsv_ts`, and keeps the
//!   run's module graph: its roots and each module's root and file, and,
//!   recording in full, each load's format and hash, the imports between
//!   modules, the files consulted, and what failed
//! - `bootstrap`: the extension that installs the pure web globals from
//!   `deno_web` (console, URL, encoding, timers, streams, `structuredClone`,
//!   `AbortController`, `performance`, and the rest) and removes `Deno` and
//!   `__bootstrap` from the global object
//! - `stack`: an error's stack, with V8's detailed capture for uncaught
//!   exceptions off, and its `.stack` text formatted as `deno_core` formats
//!   it by a cheaper callback, with no source map applied (the vendored
//!   `deno_core` is patched to apply none anywhere)
//! - [`engine`]: V8's process-wide setup ([`engine::EngineOptions`]), applied
//!   once per process by [`engine::init_engine`] before the first runtime,
//!   whose token ([`engine::Engine`]) every runtime start takes, apart from
//!   each runtime's [`RuntimeOptions`]
//! - [`confine`]: the box a run's process is held in, and the type state
//!   ([`confine::Confinement`]) the engine takes, confined or named
//!   unconfined
//! - [`snapshot`]: the startup snapshot, built in a process of its own and
//!   embedded by the binary that starts from it
//! - [`phases`]: a timestamp at each phase boundary of a run, for the
//!   measurement harness, recorded only under the `phases` feature
//! - [`prices`]: switches that each turn off one of eval's defaults, for the
//!   measurement harness to price them, read only under the `prices` feature
//! - [`eval`]: `fgro eval`'s protocol: a module evaluated to JSON data, with
//!   an optional driver module, the clock and randomness denied and the heap
//!   and time limited, written by a strict host serializer into one
//!   envelope that reports success or the kind of failure
//!
//! `deno_core`'s types stay out of this crate's public API, and the crate
//! reads no environment variable (but the `phases` feature's file,
//! [`phases::PHASES_FILE_ENV`], and the `prices` feature's switches,
//! [`prices::PRICES_OFF_ENV`]): what a runtime decides per run, such as the
//! console's color and streams ([`ConsoleOptions`]), comes in through
//! [`RuntimeOptions`] from the embedder. Its dependencies do read some
//! (`deno_core`'s diagnostics and import graph file, ICU's locale), so an
//! embedder that wants none read clears its environment before it starts a
//! thread, as `fgro eval` does.
//!
//! Without a [`RuntimeOptions::snapshot`], the runtime starts from source:
//! `deno_web`'s JS is read from its crate's path on the build machine, so such
//! a binary fails to start off its build tree. Started from a snapshot, it
//! reads nothing from the build tree. Either way, the steps that must happen
//! in each run, never in a snapshot (the time origin, the console's
//! [`ConsoleOptions`], removing the internals from the global object), run
//! once the runtime has started.

mod bootstrap;
pub mod confine;
pub mod engine;
pub mod eval;
mod loader;
pub mod phases;
pub mod prices;
#[cfg(test)]
mod process_tests;
mod report;
pub mod resolver;
mod run;
mod runtime;
pub mod snapshot;
mod stack;
#[cfg(test)]
mod test_support;

use crate::snapshot::StartupSnapshot;

pub use crate::run::{RunError, StartError, run_main};

/// The stack to reserve for the thread [`run_main`] runs on: tsv's
/// `STACK_SIZE`, 32 MiB, twice what its parsers need in a debug build (see
/// [`run_main`]'s `# Stack`).
///
/// A reservation, not an allocation: pages commit as they're touched, so a
/// run pays only for the stack it uses.
pub const STACK_SIZE: usize = tsv_ts::STACK_SIZE;

/// The version string of the V8 the runtime embeds, as V8 reports it.
pub fn v8_version() -> &'static str {
    deno_core::v8::V8::get_version()
}

/// How a runtime is constructed.
#[derive(Debug, Clone, Default)]
pub struct RuntimeOptions {
    /// The authority the runtime's code holds. Empty by default, the only
    /// value today.
    pub grants: Grants,
    /// The snapshot to start from, made by [`snapshot::make_snapshot`] from
    /// this same build. Without one, the runtime evaluates its extensions'
    /// sources at startup.
    pub snapshot: Option<StartupSnapshot>,
    /// Where the console writes and whether it colors.
    pub console: ConsoleOptions,
}

/// The console's output: which stream each level writes to, and whether each
/// stream is colored.
///
/// Decided per runtime, read when it starts, never carried in a snapshot. The
/// default writes by level, uncolored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsoleOptions {
    /// Which stream each level writes to.
    pub routing: ConsoleRouting,
    /// Whether output to stdout is colored.
    pub color_stdout: bool,
    /// Whether output to stderr is colored.
    pub color_stderr: bool,
}

/// Which stream each console level writes to. Output is colored by the
/// stream it goes to, whatever its level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConsoleRouting {
    /// `log`, `info`, and `debug` to stdout; `warn`, `error`, and `trace` to
    /// stderr, as `fgro run` writes.
    #[default]
    ByLevel,
    /// Every level to stderr, leaving stdout to the host.
    Stderr,
}

/// The grant set: what the runtime's code may reach beyond pure computation.
/// Nothing can be granted yet, so it has no fields.
#[derive(Debug, Clone, Default)]
pub struct Grants {}
