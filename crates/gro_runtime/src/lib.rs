//! gro's JS host: a minimal web-standards runtime on V8.
//!
//! [`run_main`] runs an ES module from disk to completion: it resolves the
//! entry, evaluates it on a `deno_core` `JsRuntime` driven by a current-thread
//! tokio runtime, and drains the event loop (timers, pending promises). An
//! uncaught error or an unhandled promise rejection ends the run with a
//! [`RunError::Uncaught`] carrying the report, stack and source positions
//! included. A run in which one of gro's rules refused an `import()`, or a
//! host function refused a call for a grant its handle lacks, has failed too,
//! whether or not the program caught the refusal: its [`RunOutcome`] lists
//! them ([`Refusals`]).
//!
//! A run that ends on its own hands its runtime back in the outcome
//! ([`SpentRuntime`]), with the exit code its program set, if it set one:
//! dropped, the runtime is torn down, and a process about to exit can
//! forget it instead. A program granted the exit host module can end the
//! run from inside a call, and since this crate doesn't end its embedder's
//! process for a program, it does so by the step the embedder gave
//! ([`RuntimeOptions::exit_step`], [`Exit`]). It ends the process itself in two
//! places, neither a program's choice: eval's out-of-memory backstop exits
//! 1 from V8's out-of-memory callback (`eval/limits.rs`), and a confined
//! engine whose random source fails aborts (`engine.rs`).
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
//!   modules, the files consulted, and what failed; and lists the
//!   `import()`s gro's rules refused
//! - `bootstrap`: the extension that installs the pure web globals from
//!   `deno_web` (console, URL, encoding, timers, streams, `structuredClone`,
//!   `AbortController`, `performance`, and the rest), serves the builtins
//!   (the host modules the runtime implements, each a synthetic module over
//!   a script made from its row, which no snapshot holds, run when a run
//!   first imports it), and removes `Deno` and `__bootstrap` from the
//!   global object
//! - `host`: the native half of the builtins, their members, each declared
//!   through the one choke point that checks the run's [`Grants`] first, or,
//!   for a grant-free builtin's, that the runtime holds an [`Authority`],
//!   and the one host op a row's script asks for its namespace by
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
mod host;
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

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::resolver::{Capability, FileId};
use crate::snapshot::StartupSnapshot;

pub use crate::host::exit::{Exit, ExitStep};
pub use crate::loader::refused::{
    MAX_LISTED_MESSAGE, MAX_LISTED_REFUSALS, NeededGrant, Refusal, Refusals, RefusedBy,
};
pub use crate::run::{RunError, RunOutcome, StartError, run_main};
pub use crate::runtime::SpentRuntime;

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
    /// The authority the runtime's code holds: confined, granting nothing,
    /// by default.
    pub authority: Authority,
    /// The snapshot to start from, made by [`snapshot::make_snapshot`] from
    /// this same build. Without one, the runtime evaluates its extensions'
    /// sources at startup.
    pub snapshot: Option<StartupSnapshot>,
    /// Where the console writes and whether it colors.
    pub console: ConsoleOptions,
    /// The script's arguments, every one after the entry on the command
    /// line, as written: data, not authority. A script reads them as the
    /// args builtin's `args`, which needs the `args` grant.
    pub args: Vec<String>,
    /// The runtime's name and version, as the embedder names itself, like
    /// `fgro/0.1.0`: data a script reads as `runtime_version`, from the
    /// builtin of that name, which needs no grant.
    pub runtime_version: String,
    /// The environment variables the env host module reads, as the
    /// embedder read them once, before the run: data, not authority. A
    /// script reads only the names its handle holds, the root handle's
    /// being the names the `env` grant gives ([`EnvGrant`]), or every name
    /// in an unconfined run, so an embedder passes no more than those.
    pub env: EnvVars,
    /// The step that ends the process for a program's `exit`, which a run
    /// that grants `exit` must be given: one that grants it with none
    /// fails before it starts ([`StartError::NoExitStep`]).
    ///
    /// It's called inside `exit`, with the program's frames on the
    /// stack, so it ends the process and never returns (`host/exit.rs` has
    /// why stopping V8 isn't enough): an embedder that must outlive its
    /// program doesn't grant `exit`.
    pub exit_step: Option<ExitStep>,
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

/// The authority a run's code holds: which builtin modules its modules may
/// import, and so which native members can run, and what each grant
/// reaches.
///
/// Grants are per run: they go into the loader and the runtime's `OpState`
/// when it starts, never into the snapshot or the extension list. Eval's
/// runtime is started with none at all, not with an empty set: its start
/// takes no authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authority {
    /// The grants given, to the project's own modules only: a module of an
    /// installed or linked package is refused every builtin that needs a
    /// grant, and imports a grant-free one as the project does.
    Confined(Grants),
    /// Every grant at its widest, to every module of the run, a package's
    /// included: each builtin module the runtime serves, whatever it needs,
    /// and each scoped grant whole (every environment variable). What the
    /// resolver and the loader refuse by rule apart from a grant stays
    /// refused: a host module not built yet, a reserved specifier, a
    /// package's import of the project, an `import()` in code compiled
    /// from a string. It adds no global.
    Unconfined,
}

impl Default for Authority {
    /// Confined, with nothing granted.
    fn default() -> Self {
        Self::Confined(Grants::default())
    }
}

/// The grant set: what the runtime's code may reach beyond pure computation,
/// each host module's grant `None` when it isn't granted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    /// The args host module's.
    pub args: Option<ArgsGrant>,
    /// The exit host module's.
    pub exit: Option<ExitGrant>,
    /// The env host module's.
    pub env: Option<EnvGrant>,
    /// The fs host module's.
    pub fs: Option<FsGrant>,
}

impl Grants {
    /// Whether `capability` is granted.
    pub const fn holds(&self, capability: Capability) -> bool {
        match capability {
            Capability::Args => self.args.is_some(),
            Capability::Exit => self.exit.is_some(),
            Capability::Env => self.env.is_some(),
            Capability::Fs => self.fs.is_some(),
        }
    }
}

/// The args host module's grant. What it reads, the script's arguments, is
/// data, so it scopes nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArgsGrant {}

/// The exit host module's grant: how the run ends, and the code it ends
/// with. It scopes nothing; the step that ends the process is the
/// embedder's, given beside it ([`RuntimeOptions::exit_step`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExitGrant {}

/// The env host module's grant: the environment variables its root handle
/// may read.
///
/// Each is an exact name. A name granted that the environment lacks reads
/// as unset; one outside the set is refused, as every handle narrowed from
/// the root refuses a name outside its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvGrant {
    /// The names granted.
    pub names: BTreeSet<String>,
}

/// The fs host module's grant: the directories it reads and writes beside
/// the project's root, which it always reads and never writes.
///
/// Each is an absolute path, as granted, with the identity of the
/// directory it named when the embedder checked it, by a `stat` that
/// followed symlinks. The run opens each when it first imports `fs`, and a
/// root whose directory has another identity then, one replaced since, is
/// held as why, as one gone or no longer a directory is, failing every op
/// through it. The identity is the device, the inode, and the birth time
/// where the filesystem records one: a directory removed and another made
/// at its path can take its inode number, and passes where the filesystem
/// records no birth time or stamps both within one tick of its clock
/// ([`FileId::born`]). A root matches a path by the path as
/// granted as well as by its real path. Write doesn't imply read: a
/// directory only written is read only if a read grant holds it too. An
/// unconfined run reads and writes everything, one root at `/`, whatever
/// this holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FsGrant {
    /// The directories read, each with its identity when it was checked.
    pub read: BTreeMap<PathBuf, FileId>,
    /// The directories written, each with its identity when it was checked.
    pub write: BTreeMap<PathBuf, FileId>,
}

/// Why a string can't name an environment variable, as the env host
/// module refuses it.
///
/// It's the rule a name a program asks for is held to, and, with a glob
/// refused, the names `narrow` takes and the names an embedder grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvNameFault {
    /// The name is empty.
    Empty,
    /// The name holds a NUL byte.
    Nul,
    /// The name holds a `=`, which ends a variable's name.
    Equals,
    /// The name holds a `*`, which a reader could take for a glob, where
    /// every name is exact.
    Glob,
}

/// Why `name` can't name an environment variable, if it can't.
///
/// Checked in the order the env host module checks: empty, a NUL byte, a
/// `=`, and, unless `glob` allows it, a `*`. A Rust string is well-formed,
/// which a JS string the module checks first must be too.
pub fn env_name_fault(name: &str, glob: bool) -> Option<EnvNameFault> {
    if name.is_empty() {
        Some(EnvNameFault::Empty)
    } else if name.contains('\0') {
        Some(EnvNameFault::Nul)
    } else if name.contains('=') {
        Some(EnvNameFault::Equals)
    } else if !glob && name.contains('*') {
        Some(EnvNameFault::Glob)
    } else {
        None
    }
}

/// Environment variables, as an embedder read them: each name once, the
/// first value read for it kept, in the order read.
///
/// A value can be a secret, so its `Debug` names the variables and never
/// prints a value.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EnvVars(Vec<(String, String)>);

impl EnvVars {
    /// Each variable, `(name, value)`, in the order read.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

impl FromIterator<(String, String)> for EnvVars {
    /// The variables `vars` gives, a name given again ignored.
    fn from_iter<I: IntoIterator<Item = (String, String)>>(vars: I) -> Self {
        let mut seen = BTreeSet::new();
        Self(
            vars.into_iter()
                .filter(|(name, _)| seen.insert(name.clone()))
                .collect(),
        )
    }
}

impl std::fmt::Debug for EnvVars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set()
            .entries(self.0.iter().map(|(name, _)| name))
            .finish()
    }
}
