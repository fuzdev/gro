//! gro's JS host: a minimal web-standards runtime on V8.
//!
//! [`run_main`] runs an ES module from disk to completion, and
//! [`run_generated`] one the embedder generated ([`GeneratedEntry`]): it
//! resolves the entry, evaluates it on a `deno_core` `JsRuntime` driven by a current-thread
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
//! forget it instead. A host module that buffers stdout or stderr must have
//! it written before the runtime is handed back and before `exit` calls the
//! embedder's step, since the embedder may end the process next; none does
//! today: the console and the stdout and stderr host modules write inline,
//! unbuffered, through one path (`host/stdio.rs`). A program
//! granted the exit host module can end the run from inside a call, and
//! since this crate doesn't end its embedder's
//! process for a program, it does so by the step the embedder gave
//! ([`RuntimeOptions::exit_step`], [`Exit`]). It ends the process itself in two
//! places, neither a program's choice: eval's out-of-memory backstop exits
//! 1 from V8's out-of-memory callback (`eval/limits.rs`), and a confined
//! engine whose random source fails aborts (`engine.rs`).
//!
//! A run that grants a program also changes the process at its first
//! launch: it installs a handler for `SIGINT`, `SIGTERM`, and `SIGHUP`
//! (`gro_sys::guard_children_on_signal`), process-wide, in place of any the
//! embedder set (one the process ignores stays ignored), which kills the
//! run's child groups, passes a `SIGTERM` on to its other children, and
//! then ends the process by the signal's default action. The process
//! itself is the embedder's to make ready before such a run starts, as
//! [`RuntimeOptions::spawn_env`] says.
//!
//! - [`resolver`]: the strict ESM resolver, a pure module (relative and
//!   absolute specifiers with mandatory extensions, and packages in
//!   `node_modules` through their `"exports"`, each module kept inside its
//!   project's or package's root; `.mjs`, `.ts`, and `.mts` load, and `.js`
//!   under `"type": "module"`; a specifier of Gro's package is a builtin or
//!   Gro's embedded library, never a package)
//! - [`file_id`]: a file's identity, which the resolver takes as it finds a
//!   file, and the loader and the fs host module hold a file they open to
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
//! - [`confine`]: the boxes a process is held in, eval's and the run's, and
//!   the type state ([`confine::Confinement`]) the engine takes, confined in
//!   one or named unconfined
//! - [`snapshot`]: the startup snapshot, built in a process of its own and
//!   embedded by the binary that starts from it
//! - [`embed`]: Gro's own library as the binary carries it, the modules of
//!   `src/lib` that `src/embed.json` lists, stripped of their types at build
//!   time and stored as one blob of offsets and text, which the embedder
//!   hands a run ([`RuntimeOptions::embed`]) and the loader serves as they
//!   are, their origin Gro's, which holds no grant
//! - [`phases`]: a timestamp at each phase boundary of a run, for the
//!   measurement harness, recorded only under the `phases` feature
//! - [`prices`]: switches that each turn off one of eval's defaults or a step
//!   of a command's box, for the measurement harness to price them, read only
//!   under the `prices` feature
//! - with the `testing` feature, which only tests enable (`gro_cli`'s, from
//!   its dev-dependencies): a launch's spawn step (`spawn_held`, `Spawned`,
//!   `Unspawned`, `Guard`) and an observer of its steps
//!   (`observe_launch_steps`, `LaunchStep`), so a test outside the crate
//!   launches in a program's box and signals a launch's threads between two
//!   of its steps
//! - [`eval`]: `fgro eval`'s protocol: a module evaluated to JSON data, with
//!   an optional driver module, the clock and randomness denied and the heap
//!   and time limited, written by a strict host serializer into one
//!   envelope that reports success or the kind of failure
//! - [`verb`]: the verb's pre-pass, read before any JS runs: the grants
//!   table the binary carries, `src/builtin_grants.json`, and a command
//!   line's closure, the builtins it runs and the union of their grants
//!
//! `deno_core`'s types stay out of this crate's public API, and the crate
//! reads no environment variable (but the `phases` feature's file,
//! [`phases::PHASES_FILE_ENV`], and the `prices` feature's switches,
//! [`prices::PRICES_OFF_ENV`]): what a runtime decides per run, such as the
//! console's color and streams ([`ConsoleOptions`]), comes in through
//! [`RuntimeOptions`] from the embedder. Its dependencies do read some:
//! `deno_core`'s diagnostics (`DENO_STARTUP_PHASES` and `DENO_LOG_LAZY_LOAD`,
//! printed to stderr) and its import graph file (`DENO_SNAPSHOT_IMPORT_GRAPH`,
//! which it appends every module's imports to), and ICU's locale. So an
//! embedder that wants none read clears its environment before it starts a
//! thread, as `fgro eval` does and `fgro run` doesn't; a command of Gro's
//! removes those three.
//!
//! Without a [`RuntimeOptions::snapshot`], the runtime starts from source:
//! `deno_web`'s JS is read from its crate's path on the build machine, so such
//! a binary fails to start off its build tree. Started from a snapshot, it
//! reads nothing from the build tree. Either way, the steps that must happen
//! in each run, never in a snapshot (the time origin, the console's
//! [`ConsoleOptions`], removing the internals from the global object), run
//! once the runtime has started.
//!
//! Linux-only, as are `gro_cli`, `gro_sys`, and `gro_bench`: the box is
//! Landlock and seccomp, the host's filesystem calls are `openat2`'s, and the
//! V8 archive the build links is Linux's. Built for another OS the crate
//! fails to compile.

#[cfg(not(target_os = "linux"))]
compile_error!(
    "gro_runtime is Linux-only: its box, its filesystem calls, and its V8 build are Linux's"
);

mod bootstrap;
pub mod confine;
pub mod embed;
pub mod engine;
pub mod eval;
pub mod file_id;
mod free_global;
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
pub mod verb;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::file_id::FileId;
use crate::resolver::Capability;
use crate::snapshot::StartupSnapshot;

pub use crate::free_global::FreeGlobal;
pub use crate::host::exit::{Exit, ExitStep};
pub use crate::host::spawn::RunPins;
#[cfg(feature = "testing")]
pub use crate::host::spawn::launch::{
    LaunchStep, Spawned, Unspawned, observe_launch_steps, spawn_held,
};
pub use crate::host::spawn::pins::find_on_path;
#[cfg(feature = "testing")]
pub use crate::host::spawn::watch::Guard;
pub use crate::loader::refused::{
    FsNeed, MAX_LISTED_MESSAGE, MAX_LISTED_REFUSALS, NameKind, NeededGrant, Refusal, Refusals,
    RefusedBy,
};
pub use crate::report::Report;
pub use crate::run::{
    GENERATED_ENTRY_URL, GeneratedEntry, RunError, RunOutcome, StartError, run_generated, run_main,
};
pub use crate::runtime::SpentRuntime;

/// The stack to reserve for the thread [`run_main`] runs on: tsv's
/// `STACK_SIZE`, 32 MiB, twice what its parsers need in a debug build (see
/// [`run_main`]'s `# Stack`).
///
/// A reservation, not an allocation: pages commit as they're touched, so a
/// run pays only for the stack it uses.
pub const STACK_SIZE: usize = tsv_ts::STACK_SIZE;

/// The name of the binary the runtime is built into, `fgro` while it's
/// developed beside the Node `gro`.
///
/// It's the one source of the name every message prints, the runtime's and
/// its embedder's (`gro_cli`'s help, hints, version line, and
/// `runtime_version`), and the name tests put in what they expect, so
/// renaming the binary renames them all. A constant, not the name the process
/// was invoked as: what it prints is the same however the file is renamed,
/// linked, or spawned, and never bytes its caller chose.
pub const BINARY_NAME: &str = "fgro";

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
    /// How the run gets Gro's own library, the embedder's copy, which a
    /// specifier of Gro's package that isn't a builtin resolves to, never a
    /// package: called the first time one does, so a run that names none
    /// decodes nothing. Without one, such a specifier is refused. The
    /// runtime embeds no library of its own.
    pub embed: Option<embed::LoadEmbed>,
    /// The conditions the run's packages' `"exports"` and `"imports"`
    /// match: the resolver's, and `development` when the embedder adds it,
    /// as `fgro`'s verb does unless `NODE_ENV` is `production`. The default
    /// adds none.
    pub conditions: resolver::Conditions,
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
    /// The OS's temporary directory, which `fs.temp_dir` makes its
    /// directories under when the authority grants it: data, not
    /// authority. It's an absolute path, its real path as the embedder
    /// read it when the run started or, where it can't be resolved then,
    /// its path with `.` and `..` resolved by their text, and
    /// `fs.temp_dir` refuses a symlink on it, never following one: one
    /// made on the way since, by a run that writes there, would lead its
    /// directories anywhere. An embedder names it only for a run that
    /// grants it, so a run that doesn't learns nothing of where it is.
    pub temp_dir: Option<PathBuf>,
    /// What a write through the fs host module that the kernel refuses
    /// gains as its cause after the OS's, in a run the embedder holds in a
    /// box whose rules are narrower than its grants ([`DeniedWriteCause`]).
    pub denied_write_cause: Option<DeniedWriteCause>,
    /// The environment variables the spawn host module may hand a child, as
    /// the embedder read them once, before the run: data, not authority,
    /// which no member a script calls returns. A program's environment is
    /// made of them where its handle is made, by the rules of the run's
    /// `spawn` grant ([`SpawnGrant`]): the base set every child gets, and
    /// what a program's `pass_env` names and its grant forwards, every
    /// variable for a program granted the whole environment
    /// ([`PassEnvGrant::All`]) and in an unconfined run. So an embedder
    /// passes no more than those: none for a run that grants no program.
    ///
    /// An embedder that runs a program granting one makes its process
    /// ready first, as `fgro` does, since the runtime changes no
    /// process-wide state but the signal handler it installs at the first
    /// launch (the crate's doc): `SIGCHLD` at its default where it was
    /// inherited ignored (`gro_sys::child::default_child_signal`), or the
    /// kernel reaps a child before its pidfd can say how it ended; every
    /// descriptor it inherited above stderr close-on-exec
    /// (`gro_sys::cloexec_inherited_fds`), or a child is handed them; the
    /// soft limit on open descriptors raised (`gro_sys::raise_open_files_limit`),
    /// as Node raises it; and no other waiter on its children, and its main
    /// thread live, not blocking those three signals, and not the thread the
    /// run launches from, since the handler on any other thread hands a
    /// signal on to it and the main thread's handler can't wait for a launch
    /// of its own.
    pub spawn_env: EnvVars,
}

/// What a write the kernel refuses gains as its cause, in a run the
/// embedder holds in a box.
///
/// It's data: text the embedder made of its box's write rules, naming
/// where the box lets the run write, and those rules' paths. A refusal
/// gains it when what Landlock checks for a path the op writes, resolved as
/// `fs.real_path` resolves it within the run's roots, lies beneath none of
/// them: for an op on the entry itself (`remove`, both ends of a rename,
/// `symlink`, `mkdir`) the directory holding it, and for one through it
/// (`write`, a copy's destination) the entry, a final symlink followed, or
/// the directory that would hold it. A refusal of a path the box writes
/// gains nothing, since something else refused it (a file's own mode); nor
/// does a refusal the run's grants make, or one of `set_mode` or
/// `set_times`, which Landlock never gates. Two refusals the box made print
/// no cause: a write through a dangling symlink, resolved only to its own
/// parent, and a recursive `mkdir` or copy through a symlink on the way,
/// whose parent doesn't exist yet and so isn't followed.
///
/// Kind: a review signal; the box is the wall.
#[derive(Debug, Clone)]
pub struct DeniedWriteCause {
    /// The text the refusal's cause gains, after the OS's.
    pub cause: String,
    /// The absolute real paths the box's rules let the run write.
    pub writable: Vec<PathBuf>,
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
    /// installed or linked package, or of Gro's embedded library, is refused
    /// every builtin that needs a grant, and imports a grant-free one as the
    /// project does.
    Confined(Grants),
    /// Every grant at its widest, to every module of the run, a package's
    /// included, but Gro's embedded library, which holds none: each builtin
    /// module the runtime serves, whatever it needs, and each scoped grant
    /// whole (every environment variable). What the
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
    /// The stdout host module's.
    pub stdout: Option<StdoutGrant>,
    /// The stderr host module's.
    pub stderr: Option<StderrGrant>,
    /// The env host module's.
    pub env: Option<EnvGrant>,
    /// The fs host module's.
    pub fs: Option<FsGrant>,
    /// The spawn host module's.
    pub spawn: Option<SpawnGrant>,
}

impl Grants {
    /// Whether `capability` is granted.
    pub const fn holds(&self, capability: Capability) -> bool {
        match capability {
            Capability::Args => self.args.is_some(),
            Capability::Exit => self.exit.is_some(),
            Capability::Stdout => self.stdout.is_some(),
            Capability::Stderr => self.stderr.is_some(),
            Capability::Env => self.env.is_some(),
            Capability::Fs => self.fs.is_some(),
            Capability::Spawn => self.spawn.is_some(),
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

/// The stdout host module's grant: raw writes to the process's standard
/// output. It scopes nothing. It doesn't gate the console, which writes
/// to the same descriptor from every module of a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StdoutGrant {}

/// The stderr host module's grant: raw writes to the process's standard
/// error, as [`StdoutGrant`] is to standard output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StderrGrant {}

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
/// Each is an absolute path, as granted, with the directory it named as
/// the run started ([`GrantedDir`]): its real path then and the identity
/// of the directory there. The run opens each by that real path when it
/// first imports `fs`, never reading where the granted path leads again,
/// and a root whose directory has another identity then, one replaced
/// since, is held as why, as one gone or no longer a directory is, failing
/// every op through it. The identity is the device, the inode, and the
/// birth time where the filesystem records one: a directory removed and
/// another made at its path can take its inode number, and passes where
/// the filesystem records no birth time or stamps both within one tick of
/// its clock ([`FileId::born`]). A root's scope joins another's only when
/// its real path lies beneath the other's and the other's descriptor
/// holds its directory at that path (one that isn't open, which reaches
/// nothing, by the path alone), so the nesting a root joins by is the one
/// these real paths name, the start's. A root matches
/// a path by the path as granted as well as by its real path. Write
/// doesn't imply read: a directory only written is read only if a read
/// grant holds it too. An unconfined run reads and writes everything, one
/// root at `/`, whatever this holds, and makes temporary directories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FsGrant {
    /// The directories read, each by the path it was granted as.
    pub read: BTreeMap<PathBuf, GrantedDir>,
    /// The directories written, each by the path it was granted as.
    pub write: BTreeMap<PathBuf, GrantedDir>,
    /// Whether `fs.temp_dir` makes fresh directories under the temporary
    /// directory the embedder names ([`RuntimeOptions::temp_dir`]), each a
    /// root its maker reads and writes, made as the run goes, and nothing
    /// else of that directory.
    pub temp: bool,
}

/// The directory a path the fs grant holds named as the run started, as
/// the embedder found it: its real path, read first, then the identity of
/// the directory at that real path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedDir {
    /// Its absolute real path, with no symlink on it, `.`, or `..`.
    pub real: PathBuf,
    /// The identity of the directory at `real`, by a `stat` that followed
    /// symlinks.
    pub id: FileId,
}

/// The variables every child is given where the embedder read them.
///
/// They're in the order a child's environment sets them, `LC_ALL=C` set
/// between `TERM` and `NO_COLOR`. An embedder reads them for a run that
/// grants a program ([`RuntimeOptions::spawn_env`]), and a `pass_env`
/// naming one exactly asks for nothing a child doesn't get.
pub const CHILD_BASE_ENV: [&str; 6] = ["PATH", "HOME", "TMPDIR", "TERM", "NO_COLOR", "FORCE_COLOR"];

/// The spawn host module's grant: the programs its root handle may make
/// handles of, each by the name a program asks for, with what each may be
/// passed of the environment.
///
/// A bare name is found on the `PATH` the embedder read for children
/// ([`RuntimeOptions::spawn_env`]), and a name holding `/` beneath the
/// project's root, each pinned as the run starts, before any module
/// evaluates; a name granted that finds nothing then makes `find` return
/// `null`. A name outside the set is refused, as every handle narrowed from
/// the root refuses a name outside its own. An unconfined run's root makes
/// a handle of any name, found when it's asked for, and passes any
/// variable, whatever this holds.
///
/// Beside the programs, its exec roots: the directories `program_in` and
/// `find_in` find a program beneath, each by the path it was granted as,
/// with the directory it named as the run started, as the fs grant holds
/// its directories ([`GrantedDir`]). The run opens each by that real path
/// as it starts, beside the pins, and holds it to that identity, so a
/// directory replaced between is refused; a program is then found beneath
/// the exec root's own descriptor, never by a path the kernel resolves
/// from `/`, so an exec root stays its directory wherever it's moved,
/// though a script found beneath one launches by the path it was found
/// at. An exec root that is also a directory the fs grant writes
/// runs what the run writes there. An unconfined run's roots find a
/// program beneath any directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnGrant {
    /// The programs granted, by name.
    pub programs: BTreeMap<String, ProgramGrant>,
    /// The exec roots granted, each by the path it was granted as.
    pub exec_roots: BTreeMap<PathBuf, ExecRootGrant>,
}

/// An exec root the spawn grant holds: the directory it named as the run
/// started, and what a program found beneath it may be passed of the
/// environment beyond the base set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecRootGrant {
    /// The directory, by its real path and identity then.
    pub dir: GrantedDir,
    /// What each program found beneath it may be passed, as a flag's
    /// program may be ([`ProgramGrant::Flag`]); it may be set any value.
    pub forward: Vec<EnvPattern>,
}

/// What one granted program may be given of the environment beyond the
/// base set every child gets: a flag's grant, or the grants table's
/// policy, each its own shape, so neither can be read as the other.
///
/// A call asking for more than its grant is `not_granted`. One asking for
/// a narrower list is made as it asks, its child given the base set and
/// `LC_ALL=C` as every list's child is, under a whole-environment grant
/// too, so long as that passes no name the grant omits. One leaving out a
/// value the table sets is made too, and its child then gets the host's
/// own value of that variable where its `pass_env` passes it, the base
/// set's `FORCE_COLOR` among them: a call can't add a value, but it can
/// drop the table's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramGrant {
    /// A program `fgro run`'s flags grant: its `pass_env` may name what
    /// `--allow-run-env=` forwards, never `'all'`, and its `env` may set
    /// any value.
    Flag {
        /// The variables its `pass_env` may name: an exact name passes
        /// itself, and a `PREFIX_*` glob every name it covers, or a glob
        /// within it.
        forward: Vec<EnvPattern>,
    },
    /// A program the grants table grants, by the fields of Gro's
    /// `BuiltinProgramGrant`. A program both a flag and the table grant
    /// takes the table's policy, the flag's forwards joining a list, which
    /// the embedder joins into [`PassEnvGrant::Names`] as it makes the
    /// grant.
    Table {
        /// What its `pass_env` may pass.
        pass_env: PassEnvGrant,
        /// The only values its `env` may set, each to its value here: an
        /// `env` setting another variable, or one of these to another
        /// value, is refused.
        env: BTreeMap<String, String>,
    },
}

/// The host variables a program the grants table grants may be passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassEnvGrant {
    /// These names and globs: an exact name passes itself, and a
    /// `PREFIX_*` glob every name it covers, or a glob within it; `'all'`
    /// is refused.
    Names(Vec<EnvPattern>),
    /// The whole environment the embedder read for children, `'all'`,
    /// but the names `omit_env` lists: `'all'` with an `omit_env` that
    /// lists each of them, or a list, whose child gets the base set and
    /// `LC_ALL=C` as every list's does, that passes none of them, by the
    /// base set, `LC_ALL`, a name, or a glob.
    All {
        /// The exact names left out.
        omit_env: BTreeSet<String>,
    },
}

/// A variable, or the variables of a prefix, that a spawn grant forwards
/// to a program.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EnvPattern {
    /// One variable, by its exact name.
    Exact(String),
    /// Every variable whose name starts with the prefix, written
    /// `PREFIX_*`.
    Prefix(String),
}

impl EnvPattern {
    /// The pattern `text` writes: a name, or a prefix followed by its one
    /// `*`, last.
    ///
    /// # Errors
    ///
    /// Returns why `text` names no pattern: a fault of its name as a
    /// variable's ([`env_name_fault`], a `*` allowed), a `*` other than
    /// the last character, or a `*` with no prefix, which would name every
    /// variable without saying so.
    pub fn parse(text: &str) -> Result<Self, EnvPatternFault> {
        if let Some(fault) = env_name_fault(text, true) {
            return Err(EnvPatternFault::Name(fault));
        }
        match text.find('*') {
            None => Ok(Self::Exact(text.to_owned())),
            Some(star) if star + 1 != text.len() => Err(EnvPatternFault::GlobNotLast),
            Some(0) => Err(EnvPatternFault::GlobAlone),
            Some(star) => Ok(Self::Prefix(text[..star].to_owned())),
        }
    }

    /// Whether the pattern forwards the variable `name`.
    pub fn covers(&self, name: &str) -> bool {
        match self {
            Self::Exact(exact) => exact == name,
            Self::Prefix(prefix) => name.starts_with(prefix.as_str()),
        }
    }

    /// Whether every variable `other` forwards, this one does too.
    pub fn holds(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Exact(exact), Self::Exact(name)) => exact == name,
            (Self::Exact(_), Self::Prefix(_)) => false,
            (Self::Prefix(prefix), Self::Exact(name) | Self::Prefix(name)) => {
                name.starts_with(prefix.as_str())
            }
        }
    }
}

impl std::fmt::Display for EnvPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exact(name) => f.write_str(name),
            Self::Prefix(prefix) => write!(f, "{prefix}*"),
        }
    }
}

/// Why a string names no [`EnvPattern`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvPatternFault {
    /// It can't name a variable.
    Name(EnvNameFault),
    /// It holds a `*` before its last character: a glob ends in its one
    /// `*`.
    GlobNotLast,
    /// It's a `*` alone, with no prefix.
    GlobAlone,
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
/// `=`, and, unless `glob` allows it, a `*`. A Rust string is always
/// well-formed; the module checks a JS string's well-formedness after a NUL
/// byte and before a `=`.
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

    /// The value of the variable `name`, if it was read.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.iter()
            .find_map(|(read, value)| (read == name).then_some(value))
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
