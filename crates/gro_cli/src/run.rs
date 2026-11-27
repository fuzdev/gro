//! `fgro run` and a command of gro's on the binary: starts the engine and
//! runs one module, `fgro run`'s file or the entry the verb's pre-pass
//! generated, until its event loop drains or the program exits, printing
//! its failure, then each `import()` and host function's call gro's rules
//! refused, which fail the run too, and leaves the runtime as it is, since
//! the process exits next. [`exit_code`] is the one place the run's exit
//! code is decided, however the run ended.
//!
//! **`fgro run`** ([`run_file`]) resolves its entry on the main thread
//! before any thread starts, then runs it in the run's kernel box as a
//! command of Gro's does (`start`), its programs each in a box of its own;
//! under `-A`, or where the box can't go up, it runs in no box
//! ([`run_unboxed`]).
//!
//! **A command of gro's** ([`verb`]) is planned on the main thread before
//! any thread starts (`verb::prepass`), from the command line, the grants
//! table, the project at the working directory, and `GRO_FALLBACK`,
//! `NODE_ENV`, and `HOME`, read here once: it runs on the binary from its
//! generated entry in the run's kernel box, which its process enters on
//! the main thread before the run's thread starts (`start`), so the
//! project's code, the tools it grants, and an exploit of the engine are
//! held to the box's walls; or it execs the Node Gro the project installed
//! (`verb::fallback`); or it's refused before anything runs.
//!
//! **What a run is handed.** `gro_runtime` reads no environment variable of
//! its own but its features' (its module doc), so what a run takes from the
//! process `fgro` reads once, as the run starts, and hands in through
//! [`RuntimeOptions`] ([`runtime_options`]):
//!
//! - color, decided from the environment ([`color::console_options`]), as
//!   `RuntimeOptions::console`;
//! - the working directory, read here and passed beside the entry (a
//!   verb's, the project's root its pre-pass read), and `fgro`'s own name
//!   and version, as `RuntimeOptions::runtime_version`;
//! - the variables `--allow-env=` grants (every variable under `-A`), read
//!   through the [`Environment`] seam ([`env_vars`]), as
//!   `RuntimeOptions::env`;
//! - for a run that grants a program, the variables a child may be given
//!   ([`spawn_env`]: the base set and what `--allow-run-env=` names, with a
//!   verb's run what its table programs' lists name, the whole environment
//!   listed only when one of those is a glob, everything under `-A` and in
//!   a verb's run that grants a program the whole environment), as
//!   `RuntimeOptions::spawn_env`, which no member returns;
//! - for a verb's run, the `development` condition unless `NODE_ENV` is
//!   `production`, as `RuntimeOptions::conditions`;
//! - the directories `--allow-read=`, `--allow-write=`, and `--allow-run-in=`
//!   grant, each resolved against the working directory and checked to be a
//!   directory ([`granted_dirs`]), as the `fs` grant's paths and the `spawn`
//!   grant's exec roots, each with its real path then and the identity of
//!   the directory there;
//! - for a run that grants `fs.temp_dir`, the OS's temporary directory, read
//!   from `TMPDIR`, `TMP`, or `TEMP` as Node's `os.tmpdir()` reads it
//!   ([`temp_dir`]), by its real path then (where it can't be resolved then,
//!   its path with `.` and `..` resolved by their text), as
//!   `RuntimeOptions::temp_dir`.
//!
//! The host asks the OS for the process's id, `pid`, when a run first
//! imports the module. A run in no box leaves the process's environment
//! as it was, so the vendored `deno_core` still reads its own diagnostic
//! variables (`gro_runtime`'s module doc names them); `fgro eval` clears
//! it, and a run in the run's box removes those and points the temporary
//! and `XDG_*` data, cache, and state directories into the run's private
//! directory, where it has one, before it reads any of the above
//! (`start`).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use gro_runtime::confine::Confinement;
use gro_runtime::confine::plan::{DirFlag, GrantedFlagDir, RunWriteDir};
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::file_id::FileId;
use gro_runtime::phases::{self, Phase};
use gro_runtime::resolver::{
    BUILTINS, Capability, Conditions, DiskTree, EntryBound, GRO_PACKAGE, InstalledPackage,
    ProjectPackage, ResolveErrorKind, SPLIT_MODULES, find_builtin, normalize,
    read_installed_package, read_project_package,
};
use gro_runtime::verb::table::BuiltinGrantsTable;
use gro_runtime::{
    ArgsGrant, Authority, BINARY_NAME, CHILD_BASE_ENV, EnvGrant, EnvPattern, EnvVars,
    ExecRootGrant, Exit, ExitGrant, FileEntry, FreeGlobal, FsGrant, FsNeed, GrantedDir, Grants,
    NameKind, NeededGrant, PassEnvGrant, ProgramGrant, Refusal, Refusals, RefusedBy, RunError,
    RunOutcome, RuntimeOptions, SpawnGrant, StderrGrant, StdoutGrant, env_name_fault,
    resolve_file_entry,
};

use crate::cli::{
    ALLOW_ENV, ALLOW_READ, ALLOW_RUN, ALLOW_RUN_ENV, ALLOW_RUN_IN, ALLOW_TEMP, ALLOW_WRITE,
    FlagDirs, ListFlag, RunGrant, RunInvocation,
};
use crate::color;
use crate::embed::load_embed;
use crate::error::{CliError, DirFault, HomeRootBy};
use crate::print::{escape_controls, shell_quote, stderr_line};
use crate::snapshot::STARTUP_SNAPSHOT;
use crate::start::{BoxStartError, RunStart, StartEntry, listed, run_boxed};
use crate::thread::{Joined, on_sized_thread};
use crate::verb::fallback::{FallbackPolicy, GRO_FALLBACK, exec_node_gro};
use crate::verb::prepass::{ProjectReads, VerbInputs, VerbPlan, plan};
use crate::version::RUNTIME_VERSION;
use crate::version::VERSION;
use crate::{EXIT_FAILURE, EXIT_PANIC};

/// How a run ended, as its exit code is decided from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// The program ran to completion, having set this code with
    /// `set_exit_code`, or none.
    Completed(Option<u8>),
    /// The program called `exit`, with this code: the one it gave, else
    /// the one it had set, else 0.
    Exited(u8),
    /// The run failed, with this code of `fgro`'s
    /// ([`CliError::exit_code`]): an uncaught error, an unhandled
    /// rejection, a top-level `await` nothing settled, a module that
    /// didn't load, an entry that didn't resolve.
    Failed(u8),
}

/// What a run is handed beside its entry: `fgro run`'s flags, or a verb's
/// plan.
pub struct RunSpec<'a> {
    /// What the run grants by the flags' shape.
    grant: &'a RunGrant,
    /// The directories it grants, as listed.
    dirs: &'a FlagDirs,
    /// The table's policy of each program it grants by one; any other
    /// program it grants is a flag's.
    programs: &'a BTreeMap<String, ProgramGrant>,
    /// The script's arguments, or the command line without its grant flags.
    args: &'a [String],
    /// The conditions its packages match.
    conditions: Conditions,
    /// Whether it's a verb's run: it grants the env and spawn host modules
    /// whatever names they hold, none included, since its entry imports
    /// both, and a refusal of its flags names no `fgro run` help.
    verb: bool,
}

/// What `run`, a run started in the box or `fgro run`'s in none, hands the
/// run: a command of Gro's when it starts from the entry one generates
/// (`RunSpec::verb`); `fgro run`'s no table program, the resolver's
/// conditions alone, and the env and spawn host modules only when a flag
/// names something for them.
pub fn start_spec(run: &RunStart) -> RunSpec<'_> {
    RunSpec {
        grant: &run.grant,
        dirs: &run.dirs,
        programs: &run.programs,
        args: &run.cli_args,
        conditions: Conditions {
            development: run.development,
        },
        verb: matches!(run.entry, StartEntry::Generated { .. }),
    }
}

/// Runs the command of gro's `cli_args` names, the args after the
/// binary's own path: plans it on the calling thread, the main one, before
/// any thread starts ([`plan`]), then runs it in the run's box
/// ([`run_boxed`]), or execs the Node Gro the project installed
/// ([`exec_node_gro`]), or prints why it can't run, returning the exit
/// code.
pub fn verb(cli_args: &[String]) -> u8 {
    let environment = ProcessEnvironment;
    // read here, as a run reads it, and taken as the project's root
    let root = match std::env::current_dir() {
        Ok(root) => root,
        Err(error) => return CliError::Cwd(error).report(),
    };
    let table = match BuiltinGrantsTable::carried() {
        Ok(table) => table,
        Err(error) => return CliError::Table(error).report(),
    };
    let (fallback, node_env, home) = (
        environment.var(GRO_FALLBACK),
        environment.var(NODE_ENV),
        environment.var("HOME"),
    );
    let inputs = VerbInputs {
        cli_args,
        fallback: fallback.as_deref(),
        node_env: node_env.as_deref(),
        home: home.as_deref(),
        binary_version: VERSION,
        root: &root,
    };
    match plan(&inputs, &table, &DiskProject { root: &root }) {
        Ok(VerbPlan::Run(run)) => run_boxed(RunStart::from(*run), cli_args),
        Ok(VerbPlan::Fallback(fallback)) => exec_node_gro(
            &fallback,
            cli_args,
            environment.var("PATH").as_deref(),
            node_env.as_deref(),
        )
        .report(),
        Err(error) => CliError::Verb(error).report(),
    }
}

/// The variable that turns off the `development` condition when it's
/// `production`, for a verb's run and the Node Gro alike.
pub const NODE_ENV: &str = "NODE_ENV";

/// The project at `root`, read from the disk as resolution reads it.
struct DiskProject<'a> {
    /// The project's root, the working directory.
    root: &'a Path,
}

impl ProjectReads for DiskProject<'_> {
    fn package(&self) -> Result<Option<ProjectPackage>, ResolveErrorKind> {
        read_project_package(&DiskTree, self.root)
    }

    fn installed_gro(&self) -> Result<Option<InstalledPackage>, ResolveErrorKind> {
        read_installed_package(&DiskTree, self.root, GRO_PACKAGE)
    }
}

/// Runs `fgro run`'s module, `run.file` (a path relative to the working
/// directory, or absolute), on the main thread, before any thread starts:
/// its entry resolved first, with the project root its walk finds; then
/// under `-A` in no box, unconfined; else `GRO_FALLBACK`'s policy read,
/// refused when it names none, a root that is or holds the home directory
/// refused ([`home_root`]), and the run started in the run's box
/// ([`run_boxed`]), which runs in no box where it can't go up, after a note
/// saying so, unless `GRO_FALLBACK=command` refuses that. Each prints its
/// failure if it failed, then the `import()`s and host functions' calls
/// gro's rules refused, and returns the exit code ([`exit_code`]). A
/// program that calls `exit` never gets here: [`exit_now`] ends the process
/// from inside the call.
pub fn run_file(run: RunInvocation) -> u8 {
    let environment = ProcessEnvironment;
    // read here, not in the runtime, which takes it as an input
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => return CliError::Cwd(error).report(),
    };
    let entry = match resolve_file_entry(&run.file, &cwd) {
        Ok(entry) => entry,
        Err(error) => return CliError::Run(RunError::Entry(error)).report(),
    };
    let RunInvocation {
        args, grant, dirs, ..
    } = run;
    let (may_start, run_writes) = match &grant {
        RunGrant::Named { run, .. } => (
            run.clone(),
            dirs.run_write
                .keys()
                .map(|name| (name.clone(), name.clone()))
                .collect(),
        ),
        RunGrant::All => (BTreeSet::new(), BTreeMap::new()),
    };
    let mut start = RunStart {
        cli_args: args,
        grant_flags: Vec::new(),
        root: entry.root().to_owned(),
        cwd,
        grant,
        flag_dirs: dirs.clone(),
        dirs,
        writes_root: false,
        tools: BTreeMap::new(),
        declared: BTreeSet::new(),
        programs: BTreeMap::new(),
        run_writes,
        may_start,
        development: false,
        policy: FallbackPolicy::All,
        gro: None,
        entry: StartEntry::File(Box::new(entry.clone())),
    };
    // unconfined, in no box, as the escape hatch is
    if matches!(start.grant, RunGrant::All) {
        return run_unboxed(&start, entry);
    }
    start.policy = match FallbackPolicy::read(environment.var(GRO_FALLBACK).as_deref()) {
        Ok(policy) => policy,
        Err(error) => return CliError::Verb(error.into()).report(),
    };
    if let Some(error) = home_root(&entry, environment.var("HOME").as_deref()) {
        return error.report();
    }
    run_boxed(start, &[])
}

/// The refusal of `entry`'s project root when it is or holds `home`, the
/// home directory as `HOME` names it, read by its real path where it has
/// one: a root a project's `package.json` or a `node_modules` at or above
/// the entry found, which the run reads whole, so a stray
/// `~/package.json` would hand every script beneath it the whole home
/// directory. An entry with no project's `package.json` at or above it,
/// whose root is its own directory, is never refused: the realm and every
/// program the run starts read that only as a flag naming it grants, while
/// the box's rule on it, the run's own for its loader, reads it whole.
///
/// Kind: a native check before anything runs, of what the project's tree
/// makes the root (adversary: a stray or planted `package.json` above the
/// entry; asset: the home directory), against the realm's `fs.project`
/// and the box's rule on the root alike. Unsetting `HOME` defeats it, the
/// user's choice.
fn home_root(entry: &FileEntry, home: Option<&OsStr>) -> Option<CliError> {
    let home = PathBuf::from(home?);
    if !home.is_absolute() {
        return None;
    }
    let home = std::fs::canonicalize(&home).unwrap_or_else(|_| normalize(&home));
    let root = entry.root();
    if !home.starts_with(root) {
        return None;
    }
    let found = match entry.bound() {
        EntryBound::Found => HomeRootBy::PackageJson(root.join("package.json")),
        EntryBound::InNodeModules => HomeRootBy::NodeModules(root.join("node_modules")),
        EntryBound::Unbounded { .. } => return None,
    };
    Some(CliError::RootHoldsHome {
        root: root.to_owned(),
        holds: home != root,
        found,
    })
}

/// Runs `run`, `fgro run`'s, in no kernel box, its `entry` resolved: under
/// `-A`, unconfined, as the escape hatch is; or where the box can't go up,
/// with only the runtime's native checks holding its grants, its programs
/// in no box of their own. On a thread with [`STACK_SIZE`] reserved, the
/// engine named unconfined, returning the exit code.
///
/// [`STACK_SIZE`]: gro_runtime::STACK_SIZE
pub fn run_unboxed(run: &RunStart, entry: FileEntry) -> u8 {
    let ending = Ending {
        node_hint: None,
        entry_dir: entry_dir_flag(&entry, &run.cwd),
        walls: None,
    };
    let ran = on_sized_thread(BINARY_NAME.to_owned(), move || {
        let spec = start_spec(run);
        finish(start_unboxed(&spec, &run.cwd, entry), &ending)
    });
    match ran {
        Joined::Returned(code) => code,
        Joined::Unstarted(error) => CliError::from(BoxStartError::Thread(error)).report(),
        Joined::Panicked => EXIT_PANIC,
    }
}

/// `entry`'s project root where it's the entry's own directory, with no
/// project's `package.json` at or above it, which the run reads and writes
/// only as a flag naming it grants, with the flag's value naming it: `.`
/// where that's the working directory, `cwd`, else the root's real path.
/// None for any other entry.
pub fn entry_dir_flag(entry: &FileEntry, cwd: &Path) -> Option<EntryDir> {
    entry.is_unbounded().then(|| {
        let root = entry.root().to_string_lossy().into_owned();
        let flag = if entry.root() == cwd {
            ".".to_owned()
        } else {
            root.clone()
        };
        EntryDir { root, flag }
    })
}

/// An entry's own directory, its project's root where no project's
/// `package.json` is at or above it ([`entry_dir_flag`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryDir {
    /// Its real path.
    pub root: String,
    /// What a flag names it by: `.` from it, else its real path.
    pub flag: String,
}

/// What a run's ending prints beside its failure and refusals
/// ([`finish`]).
#[derive(Debug, Default)]
pub struct Ending {
    /// The hint when the run failed after the resolver refused a Node
    /// built-in in it.
    pub node_hint: Option<String>,
    /// The entry's own directory, where the project's root is that, with
    /// no project's `package.json` at or above it ([`entry_dir_flag`]),
    /// which a refusal there hints.
    pub entry_dir: Option<EntryDir>,
    /// The note when the run failed after a program it started ended other
    /// than by exiting 0.
    pub walls: Option<String>,
}

/// How a run that `started` ended, printed: its failure, then the refusals
/// with their hints ([`report_refused`], naming `ending.entry_dir` for a
/// refusal in the entry's own directory), then the hint for the global its
/// uncaught error says was free ([`free_global_hint`]), then
/// `ending.node_hint` when the run failed after the resolver refused a Node
/// built-in in it, then `ending.walls` when the run failed after a program
/// it started ended other than by exiting 0, returning the exit code
/// ([`exit_code`]).
///
/// Kind of the hint and the note: review signals the run's code controls,
/// which can set the hint's flag by catching a Node built-in's import and then
/// fail, steering the reader to the Node Gro, which confines nothing, and
/// the note's by starting a program that fails; it could print the same
/// lines itself.
pub fn finish(started: Result<RunOutcome, CliError>, ending: &Ending) -> u8 {
    let RunOutcome {
        result,
        refused,
        exit_code: set,
        runtime,
        node_builtin_refused,
        child_failed,
        free_global,
    } = match started {
        Ok(outcome) => outcome,
        Err(error) => return error.report(),
    };
    // the process exits once this returns, on every path, so the runtime
    // is never torn down
    if let Some(runtime) = runtime {
        runtime.forget();
    }
    let ended = match result {
        Ok(()) => Ended::Completed(set),
        Err(error) => Ended::Failed(CliError::from(error).report()),
    };
    report_refused(&refused, ending.entry_dir.as_ref());
    if let Some(global) = free_global {
        stderr_line(free_global_hint(global));
    }
    let code = exit_code(ended, !refused.is_empty());
    if code != 0
        && node_builtin_refused
        && let Some(hint) = &ending.node_hint
    {
        stderr_line(hint);
    }
    if code != 0
        && child_failed
        && let Some(walls) = &ending.walls
    {
        stderr_line(walls);
    }
    code
}

/// The step a program's `exit` ends the process by, called inside its host
/// op: prints what gro's rules refused so far, as a run that ends on its
/// own does, then ends the process with [`exit_code`]'s code,
/// by the step every command exits by.
fn exit_now(exit: &Exit) -> ! {
    // the hint naming an entry's own directory needs the embedder's root,
    // which the step isn't handed, so the fixed one stands in
    report_refused(&exit.refused, None);
    crate::end(exit_code(
        Ended::Exited(exit.code),
        !exit.refused.is_empty(),
    ))
}

/// The exit code of a run that `ended` so, and in which gro's rules
/// `refused` an `import()` or a host function's call, or didn't: the one place
/// the rules meet.
///
/// - A program's code passes as it is, 0 to 255: the one it gave `exit`, or
///   the one it set with `set_exit_code` when it then ran to completion or
///   called `exit` with none, else 0.
/// - A run that failed exits with `fgro`'s code for the failure, whatever
///   the program set.
/// - A refusal fails the run, whether or not the program caught it and
///   however the run ended, `exit(0)` included: it turns 0 into
///   [`EXIT_FAILURE`], and any other code stands.
const fn exit_code(ended: Ended, refused: bool) -> u8 {
    let code = match ended {
        Ended::Completed(None) => 0,
        Ended::Completed(Some(code)) | Ended::Exited(code) | Ended::Failed(code) => code,
    };
    if code == 0 && refused {
        EXIT_FAILURE
    } else {
        code
    }
}

/// Starts the engine, named unconfined, and runs `fgro run`'s `entry`, in
/// `cwd`, as `spec` says, in no box, returning how the run ended.
///
/// # Errors
///
/// Returns why a directory a flag grants ([`granted_dirs`]), making ready
/// to start programs, or the engine failed, before anything ran.
fn start_unboxed(spec: &RunSpec<'_>, cwd: &Path, entry: FileEntry) -> Result<RunOutcome, CliError> {
    let granted = granted_dirs(spec.dirs, cwd, DirCheck::Noted)?;
    // a directory `-A` names grants nothing more, so there's nothing to note
    if !matches!(spec.grant, RunGrant::All) {
        for note in granted.notes {
            stderr_line(note);
        }
    }
    if grants_programs(spec.grant, spec.dirs) {
        ready_to_spawn(gro_sys::raise_open_files_limit).map_err(CliError::SpawnReady)?;
    }
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default())?;
    phases::mark(Phase::Engine);
    let options = runtime_options(spec, &ProcessEnvironment, granted.dirs, cwd);
    Ok(gro_runtime::run_file_unboxed(engine, &options, entry, cwd))
}

/// Whether the run may start a program: `-A`, a program `--allow-run=`
/// names in `grant`, or an exec root `--allow-run-in=` lists in `dirs`.
pub fn grants_programs(grant: &RunGrant, dirs: &FlagDirs) -> bool {
    match grant {
        RunGrant::All => true,
        RunGrant::Named { run, .. } => !run.is_empty() || !dirs.run_in.is_empty(),
    }
}

/// Makes the process ready to start programs, as a run that grants one
/// does before anything runs: every descriptor it inherited above stderr
/// marked close-on-exec, so a child is handed only stdin, stdout, and
/// stderr, and `SIGCHLD` at its default where it was inherited ignored, so
/// the kernel leaves each child for its pidfd to reap; then the soft limit
/// on open descriptors raised to the hard one by `raise`, as Node raises
/// it, so a run holding many children, each with its pipes, isn't refused
/// at a low soft limit. The raise is best-effort, as Node's is: one the
/// kernel refuses leaves the limit as it was and the run going.
///
/// # Errors
///
/// Returns the error of the step that failed, but the raise's.
fn ready_to_spawn(raise: fn() -> std::io::Result<u64>) -> std::io::Result<()> {
    gro_sys::cloexec_inherited_fds()?;
    gro_sys::child::default_child_signal()?;
    // a refused raise leaves the soft limit, which the run starts with as
    // a run that grants no program does
    let _ = raise();
    Ok(())
}

/// How [`granted_dirs`] holds a directory a flag lists that leads through
/// a symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirCheck<'a> {
    /// A command of Gro's, of the project at this root, a real path:
    /// refused when it leads out of the project, or into an entry the box
    /// leaves out that its text doesn't name.
    Project(&'a Path),
    /// `fgro run`'s: noted, the path being the person's, and a directory
    /// that isn't one names `fgro run`'s help.
    Noted,
}

/// The directories the flags grant: the fs grant's, read and written, and
/// the spawn grant's exec roots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantedDirs {
    /// The fs grant of the directories `--allow-read=` and
    /// `--allow-write=` list.
    pub fs: FsGrant,
    /// The exec roots `--allow-run-in=` lists, each by the path it was
    /// granted as.
    pub exec_roots: BTreeMap<PathBuf, GrantedDir>,
}

/// The directories a run's flags list, resolved once ([`granted_dirs`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Granted {
    /// Their grants.
    pub dirs: GrantedDirs,
    /// The notes for the directories whose real path isn't their text.
    pub notes: Vec<String>,
    /// Each directory with its flag, as listed, and its real path, the
    /// one the grants hold.
    pub resolved: Vec<GrantedFlagDir>,
    /// Each directory `--allow-run-write=` lists, with the program it's
    /// listed for, as written, and its real path: in no grant of the
    /// realm's, a program's alone.
    pub run_writes: Vec<RunWriteDir>,
}

/// The grants of the directories `dirs` lists, each resolved against `cwd`
/// when it's relative and held to be a directory now: its real path read,
/// then the identity of the directory there, by one `stat`, which the
/// grant keeps ([`GrantedDir`]), the read ones first, then the written
/// ones, then the exec roots, then those `--allow-run-write=` lists. The
/// runtime opens each fs root by that real path when the run first
/// imports `fs`, and each exec root as the run starts, and holds it to
/// that identity, so a directory moved or replaced between, like one gone
/// by then, fails every op through it, and the fs roots nest as these
/// real paths do. With them, the notes ([`grant_note`]) for the
/// directories whose real path isn't their text, and each directory's
/// real path beside its flag, for the run's box. The directories
/// `--allow-run-write=` lists are in no grant of the run's: they widen
/// only the box, for the program each names.
///
/// For a command of Gro's (`DirCheck::Project`, with the project's root),
/// a directory typed inside the project, relative or absolute, by its text
/// or through a link to the project, whose real path leads out of it
/// through a symlink is refused ([`leaves_project`]): a committed link like
/// `src -> ~` would turn a flag typed as documented into a grant on
/// whatever it leads to. So is one whose real path lies in a top-level
/// dot-entry or `node_modules` its text doesn't name
/// ([`into_unnamed_entry`]): a committed `src/fixtures -> ../.git` would
/// turn one into a grant on `.git`, which the box otherwise leaves out.
/// `fgro run` (`DirCheck::Noted`) notes both instead, the path being the
/// person's.
///
/// # Errors
///
/// Returns [`CliError::NoDir`] for the first that's missing, isn't a
/// directory, or can't be looked at, naming `fgro run`'s help when its
/// flags listed them; and, for a command of Gro's,
/// [`CliError::LeavesProject`] for the first that leads out of the project
/// through a symlink, and [`CliError::LinksIntoEntry`] for the first that
/// leads through one into an entry it doesn't name.
pub fn granted_dirs(dirs: &FlagDirs, cwd: &Path, check: DirCheck<'_>) -> Result<Granted, CliError> {
    let mut notes = Vec::new();
    let root = match check {
        DirCheck::Project(root) => Some(root),
        DirCheck::Noted => None,
    };
    let run_help = root.is_none();
    let mut check = |flag: ListFlag, program: Option<&str>, dir: &String| {
        let path = cwd.join(dir);
        let found = std::fs::canonicalize(&path)
            .and_then(|real| std::fs::metadata(&real).map(|metadata| (real, metadata)));
        let fault = match found {
            Ok((real, metadata)) if metadata.is_dir() => {
                if let Some(root) = root
                    && leaves_project(&path, &real, root)
                {
                    return Err(CliError::LeavesProject {
                        flag,
                        program: program.map(str::to_owned),
                        dir: dir.clone(),
                        real,
                    });
                }
                if let Some(root) = root
                    && let Some(entry) = into_unnamed_entry(&path, &real, root)
                {
                    return Err(CliError::LinksIntoEntry {
                        flag,
                        program: program.map(str::to_owned),
                        dir: dir.clone(),
                        entry,
                    });
                }
                let id = FileId::of(&metadata);
                notes.extend(grant_note(flag, program, dir, &path, &real));
                return Ok((path, GrantedDir { real, id }));
            }
            Ok(_) => DirFault::NotADirectory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => DirFault::Missing,
            Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {
                DirFault::NotADirectory
            }
            Err(error) => DirFault::Unreadable(error),
        };
        Err(CliError::NoDir {
            flag,
            path,
            fault,
            run_help,
        })
    };
    let mut resolved = Vec::new();
    let mut granted = |flag: DirFlag, list: &BTreeSet<String>| {
        list.iter()
            .map(|dir| {
                let (path, granted) = check(flag.into(), None, dir)?;
                resolved.push(GrantedFlagDir {
                    flag,
                    dir: dir.clone(),
                    real: granted.real.clone(),
                });
                Ok((path, granted))
            })
            .collect::<Result<BTreeMap<_, _>, CliError>>()
    };
    let fs = FsGrant {
        read: granted(DirFlag::Read, &dirs.read)?,
        write: granted(DirFlag::Write, &dirs.write)?,
        temp: false,
    };
    let exec_roots = granted(DirFlag::RunIn, &dirs.run_in)?;
    let mut run_writes = Vec::new();
    for (program, listed) in &dirs.run_write {
        for dir in listed {
            let (_, granted) = check(ListFlag::RunWrite, Some(program), dir)?;
            run_writes.push(RunWriteDir {
                named: program.clone(),
                dir: dir.clone(),
                real: granted.real,
            });
        }
    }
    Ok(Granted {
        dirs: GrantedDirs { fs, exec_roots },
        notes,
        resolved,
        run_writes,
    })
}

/// Whether `path`, a directory typed for a grant flag resolved against
/// the project's `root` (a real path), lies inside the project, by its
/// text with its `.` and `..` resolved as written, or through a symlink to
/// the project or a directory in it on the way (`$PWD/src` where the
/// working directory's path holds a symlink), while `real`, its real path,
/// lies outside it: it leads out of the project through a symlink. The
/// ancestors are resolved only for a path whose real path is outside, so
/// a path inside costs nothing more.
///
/// Kind: a native check of what the trusted command line means, before any
/// project code runs (adversary: the project's tree, a committed symlink
/// like `src -> ~`; asset: whatever the link leads to). Defeated by the
/// user typing the outside path, which is their choice, and by a process
/// outside the run swapping a path between the check and the box, outside
/// the threat model.
fn leaves_project(path: &Path, real: &Path, root: &Path) -> bool {
    !real.starts_with(root)
        && normalize(path).ancestors().any(|ancestor| {
            ancestor.starts_with(root)
                || std::fs::canonicalize(ancestor).is_ok_and(|real| real.starts_with(root))
        })
}

/// The path beneath the project's `root` (a real path) that `real`, the
/// real path of `path`, a directory typed for a grant flag, lies at, when
/// its first component is a dot-entry or `node_modules` (the entries the
/// box's per-entry rules leave out) and the text `path` names, beneath
/// the root, doesn't start with that same entry: it leads there through a
/// symlink, like `src/fixtures -> ../.git`. The text beneath the root is
/// the path with its `.` and `..` resolved as written, or, reached through
/// a symlink to the root itself (`$PWD` holding one), what follows it;
/// a path typed outside the project names no entry. The text is read only
/// for a real path in such an entry, so any other costs nothing more.
///
/// Kind: as [`leaves_project`]'s (asset: `.git`, whose hooks the next
/// unconfined git command runs, and `node_modules`); defeated by naming
/// the entry, the command line's own choice.
fn into_unnamed_entry(path: &Path, real: &Path, root: &Path) -> Option<PathBuf> {
    let beneath = real.strip_prefix(root).ok()?;
    let entry = beneath.components().next()?.as_os_str();
    if !(entry.as_encoded_bytes().starts_with(b".") || entry == "node_modules") {
        return None;
    }
    let text = normalize(path);
    let typed = text
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            text.ancestors()
                .find(|ancestor| std::fs::canonicalize(ancestor).is_ok_and(|real| real == root))
                .and_then(|alias| text.strip_prefix(alias).ok().map(Path::to_path_buf))
        });
    let named = typed
        .as_deref()
        .and_then(|typed| typed.components().next())
        .is_some_and(|first| first.as_os_str() == entry);
    (!named).then(|| beneath.to_path_buf())
}

/// The note, if one is due, that `flag` listing `dir`, the text typed,
/// which is `path` resolved against the working directory, grants `real`,
/// its real path as the run starts: `note: --allow-read=<dir> grants
/// <real>`, or for `--allow-run-write=` with its `program`,
/// `note: --allow-run-write=<program>:<dir> grants <real>`, each escaped
/// ([`escape_controls`]), when `real` isn't `path`
/// with its `.` and `..` resolved by their text, so the path passes through
/// a symlink. A refusal names a path the program chose, and one copied into
/// a flag can lead through a symlink to another directory than it names.
///
/// Every symlink is noted, one in a directory above the working directory
/// too, as a symlinked home is: which symlinks the person made is past
/// knowing, and the note is fixed by typing the real path, or a path
/// relative to the working directory, which is real already.
fn grant_note(
    flag: ListFlag,
    program: Option<&str>,
    dir: &str,
    path: &Path,
    real: &Path,
) -> Option<String> {
    (normalize(path) != real).then(|| {
        let program = program.map_or_else(String::new, |program| {
            format!("{}:", escape_controls(program))
        });
        format!(
            "note: {flag}={program}{} grants {}",
            escape_controls(dir),
            escape_controls(&real.to_string_lossy())
        )
    })
}

/// What follows a run's refusals on stderr, however many and however the
/// run ended: a refusal fails the run, and a program can't catch its way
/// out of one.
const REFUSALS_FAIL: &str = "error: a refusal fails the run, whether or not the program caught it";

/// Prints to stderr each refusal `refused` lists, one line each
/// ([`refusal_line`]), then one line counting the refusals it doesn't
/// list, then, when there were any, [`REFUSALS_FAIL`]; then the hints
/// saying what grants the refused calls' needs ([`grant_hints`]), naming
/// `entry_dir` for one in the entry's own directory, if any does, which the
/// refusal that is the run's uncaught error, not listed, has too.
fn report_refused(refused: &Refusals, entry_dir: Option<&EntryDir>) {
    for refusal in &refused.listed {
        stderr_line(refusal_line(refusal));
    }
    match refused.unlisted {
        0 => {}
        1 => stderr_line("error: 1 more refusal, not listed"),
        more => stderr_line(format_args!("error: {more} more refusals, not listed")),
    }
    if !refused.is_empty() {
        stderr_line(REFUSALS_FAIL);
    }
    for hint in grant_hints(refused, entry_dir) {
        stderr_line(hint);
    }
}

/// A listed refusal's line, escaped ([`escape_controls`]): `error:
/// import() refused: <message>`, `error: env refused: <message>`, `error:
/// spawn refused: <message>`, or `error: fs refused <need>: <message>`,
/// the need in its hint's words.
/// `src/docs/run.md` §Errors and exit codes states the rules.
fn refusal_line(refusal: &Refusal) -> String {
    let message = escape_controls(&refusal.message);
    match refusal.by {
        RefusedBy::Import => format!("error: import() refused: {message}"),
        RefusedBy::Env => format!("error: env refused: {message}"),
        RefusedBy::Spawn => format!("error: spawn refused: {message}"),
        RefusedBy::Fs(need) => {
            let need = match need {
                Some(FsNeed::Read | FsNeed::EntryDirRead) => "a read",
                Some(FsNeed::LinkTarget) => "a symlink leaving its root",
                Some(FsNeed::Write | FsNeed::EntryDirWrite) => "a write",
                Some(FsNeed::OpenDir) => "what the run grants",
                Some(FsNeed::Temp) => "a temporary directory",
                None => "what no grant allows",
            };
            format!("error: fs refused {need}: {message}")
        }
    }
}

/// The hint lines saying what grants the refused calls' needs
/// (`refused.needed`), made of that data, never of a listed message: for
/// each builtin module the project imported that the run's grants lack the
/// row of, the flag that grants it ([`module_hint`]), naming no name; one
/// for `--allow-env=` naming each variable that's a [`hint_name`], one for
/// `--allow-run=` naming each program that's a [`hint_program`], and one
/// for `--allow-run-env=` naming each variable that's a [`hint_name`], a
/// glob never, each saying `some of what` when it can't show every name
/// of its kind; then each fs line once, then the exec root's, fixed text
/// naming no path but the project's root where it's the entry's own
/// directory, with no project's `package.json` at or above it, which the
/// embedder resolved, never a path the program chose: `entry_dir`, with the
/// flag's value naming it ([`entry_dir_flag`]), each quoted for a shell, or
/// none where the run can't name it or a flag can't list it as one
/// directory (it holds a `,`), which the fixed read and write lines stand
/// in for.
/// `src/docs/run.md` §Errors and exit codes states the rules.
fn grant_hints(refused: &Refusals, entry_dir: Option<&EntryDir>) -> Vec<String> {
    let mut names: Vec<&str> = Vec::new();
    let mut programs: Vec<&str> = Vec::new();
    let mut forwarded: Vec<&str> = Vec::new();
    let more = |kind: NameKind| refused.more_names.contains(&kind);
    let (mut some_env, mut some_run, mut some_run_env) = (
        more(NameKind::Env),
        more(NameKind::Run),
        more(NameKind::RunEnv),
    );
    let (mut read, mut link, mut write, mut open_dir, mut temp, mut run_in) =
        (false, false, false, false, false, false);
    let (mut entry_read, mut entry_write) = (false, false);
    let mut hints: Vec<String> = refused
        .needed
        .iter()
        .filter_map(|needed| match needed {
            NeededGrant::Module(capability) => module_hint(*capability),
            _ => None,
        })
        .collect();
    for needed in &refused.needed {
        match needed {
            NeededGrant::Module(_) => {}
            NeededGrant::Env(name) if hint_name(name) => names.push(name),
            NeededGrant::Env(_) => some_env = true,
            NeededGrant::Run(name) if hint_program(name) => programs.push(name),
            NeededGrant::Run(_) => some_run = true,
            NeededGrant::RunEnv(name) if hint_name(name) => forwarded.push(name),
            NeededGrant::RunEnv(_) => some_run_env = true,
            NeededGrant::Fs(FsNeed::Read) => read = true,
            NeededGrant::Fs(FsNeed::LinkTarget) => link = true,
            NeededGrant::Fs(FsNeed::Write) => write = true,
            NeededGrant::Fs(FsNeed::OpenDir) => open_dir = true,
            NeededGrant::Fs(FsNeed::Temp) => temp = true,
            NeededGrant::Fs(FsNeed::EntryDirRead) => entry_read = true,
            NeededGrant::Fs(FsNeed::EntryDirWrite) => entry_write = true,
            NeededGrant::RunIn => run_in = true,
        }
    }
    for (flag, listed, some, refused_by) in [
        (ALLOW_ENV, names, some_env, "env"),
        (ALLOW_RUN, programs, some_run, "spawn"),
        (ALLOW_RUN_ENV, forwarded, some_run_env, "spawn"),
    ] {
        if !listed.is_empty() {
            let what = if some { "some of what" } else { "what" };
            hints.push(format!(
                "hint: add {flag}={} to grant {what} {refused_by} refused",
                listed.join(",")
            ));
        }
    }
    // the entry's own directory by its flag, where the run names it and
    // the flag can list it as one directory
    let named = entry_dir.filter(|dir| !dir.flag.contains(','));
    if named.is_none() {
        read |= entry_read;
        write |= entry_write;
    }
    let named_read = named
        .filter(|_| entry_read)
        .map(|dir| entry_dir_hint(false, dir));
    let named_write = named
        .filter(|_| entry_write)
        .map(|dir| entry_dir_hint(true, dir));
    hints.extend(named_read);
    if read {
        hints.push(format!(
            "hint: add {ALLOW_READ} naming the directory to grant what fs refused"
        ));
    }
    if link {
        hints.push(
            "hint: fs refused a symlink leaving its root \
             (an absolute target, or a .. above the root, even one leading back in): \
             make the link relative within its root, \
             or open its target's directory by its own path with fs.open_dir"
                .to_owned(),
        );
    }
    hints.extend(named_write);
    if write {
        hints.push(format!(
            "hint: add {ALLOW_WRITE} naming the directory to grant what fs refused, \
             then open it with fs.open_dir and write through that handle"
        ));
    }
    if open_dir {
        hints.push(
            "hint: the run grants what fs refused: \
             open the directory with fs.open_dir and use that handle"
                .to_owned(),
        );
    }
    if temp {
        hints.push(format!("hint: add {ALLOW_TEMP} to grant what fs refused"));
    }
    if run_in {
        hints.push(format!(
            "hint: add {ALLOW_RUN_IN} naming the directory to grant what spawn refused"
        ));
    }
    hints
}

/// The hint for a refusal in the project's root where it's the entry's own
/// directory, `dir`, with no project's `package.json` at or above it (a
/// type marker's isn't one): the write flag where `write`, else the read
/// flag, naming it as [`entry_dir_flag`] does, quoted for a shell
/// ([`shell_quote`]), and its path escaped. A write's says what the flag
/// leaves out in the run's kernel box: the per-entry set, which a new
/// top-level entry isn't in.
fn entry_dir_hint(write: bool, dir: &EntryDir) -> String {
    let (flag, verb, rest) = if write {
        (
            ALLOW_WRITE,
            "writes",
            "; in the run's kernel box a new entry at its top level stays read-only, so write in \
             a directory already there",
        )
    } else {
        (ALLOW_READ, "reads", "")
    };
    format!(
        "hint: {} is in no project (no package.json at or above the entry holds more than \
         \"type\" or \"sideEffects\"), so the run {verb} it only as a flag grants: add {} to \
         grant what fs refused{rest}",
        escape_controls(&dir.root),
        shell_quote(&format!("{flag}={}", dir.flag))
    )
}

/// The hint for `global`, which the run's uncaught error says was free:
/// what of Gro's stands for it, from the builtin table, so it follows the
/// table as host modules land. `src/docs/run.md` §Errors and exit codes
/// states the set.
fn free_global_hint(global: FreeGlobal) -> String {
    let what = match global {
        FreeGlobal::Process => {
            let built: Vec<String> = SPLIT_MODULES
                .iter()
                .filter(|split| split.stem == "@fuzdev/gro/process")
                .flat_map(|split| split.modules.iter())
                .filter(|module| find_builtin(module).is_some())
                .map(|module| format!("'{module}'"))
                .collect();
            format!(
                "Gro's process modules are imports, and {BINARY_NAME} has {}",
                listed(&built)
            )
        }
        FreeGlobal::Deno | FreeGlobal::Bun => "a run reaches the system through Gro's host \
             modules, each an import, like '@fuzdev/gro/fs.ts', '@fuzdev/gro/spawn.ts', and \
             '@fuzdev/gro/env.ts'"
            .to_owned(),
        FreeGlobal::Require => "it loads ES modules only, so use import or import()".to_owned(),
        FreeGlobal::Dirname => "use import.meta.dirname".to_owned(),
        FreeGlobal::Filename => "use import.meta.filename".to_owned(),
        FreeGlobal::Global => "use globalThis".to_owned(),
        FreeGlobal::Buffer => {
            "use Uint8Array, with TextEncoder and TextDecoder for text".to_owned()
        }
        FreeGlobal::Fetch => {
            let net = "@fuzdev/gro/net.ts";
            if find_builtin(net).is_some() {
                format!("the network is an import, net.fetch from '{net}'")
            } else {
                format!(
                    "the network is an import, net.fetch from '{net}', which isn't on \
                     {BINARY_NAME} yet"
                )
            }
        }
        FreeGlobal::Crypto => {
            return format!(
                "hint: {BINARY_NAME} has no global 'crypto' yet: Web Crypto isn't built on it"
            );
        }
    };
    format!(
        "hint: {BINARY_NAME} has no global '{}': {what}",
        global.name()
    )
}

/// The hint naming the flag that grants a builtin module needing
/// `capability`, which the project imported and the run's grants lack:
/// `--allow-env` for `env`, `--allow-run` or `--allow-run-in` for `spawn`,
/// naming no name, since the import says none; none for the rows `fgro
/// run` grants the project whatever its flags.
fn module_hint(capability: Capability) -> Option<String> {
    let flags = match capability {
        Capability::Env => format!("{ALLOW_ENV} naming each variable the run reads"),
        Capability::Spawn => format!(
            "{ALLOW_RUN} naming each program the run starts, \
             or {ALLOW_RUN_IN} naming a directory holding them,"
        ),
        Capability::Args
        | Capability::Exit
        | Capability::Stdout
        | Capability::Stderr
        | Capability::Fs => return None,
    };
    let specifier = BUILTINS
        .iter()
        .find(|builtin| builtin.grant == Some(capability))?
        .specifier;
    Some(format!(
        "hint: add {flags} to grant the import of '{specifier}'"
    ))
}

/// Whether the hint names `name`: ASCII letters, digits, and `_` only
/// (`src/docs/run.md` §Errors and exit codes has why).
fn hint_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether the hint names the program `name`: a plain name, of ASCII
/// letters, digits, `.`, `_`, `+`, and `-`, starting with a letter or a
/// digit, so a shell passes it as printed, it's never taken for a flag,
/// and it's never `.` or `..`; a name holding `/` is a path, which the
/// hint leaves to its reader.
fn hint_program(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

/// Where `fgro run` reads the environment variables it hands the runtime:
/// the process's environment ([`ProcessEnvironment`]), or a test's.
pub trait Environment {
    /// The value of the variable `name`, if it's set.
    fn var(&self, name: &str) -> Option<OsString>;
    /// Every variable, in the order the environment holds them.
    fn vars(&self) -> Vec<(OsString, OsString)>;
}

/// The process's own environment.
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn vars(&self) -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }
}

/// The options a run as `spec` says starts with: the authority it grants
/// ([`authority`]), `fs` among them, and the environment variables that
/// authority reaches, read from `environment` now, once ([`env_vars`]),
/// and those a child may be passed ([`spawn_env`]); the OS's temporary
/// directory when the authority makes temporary directories, read from
/// `environment` too, resolved against `cwd` ([`temp_dir`]), by its real
/// path now ([`real_temp_dir`]); the exit step [`exit_now`]; its
/// arguments and conditions; and the runtime's name and version.
pub fn runtime_options(
    spec: &RunSpec<'_>,
    environment: &impl Environment,
    dirs: GrantedDirs,
    cwd: &Path,
) -> RuntimeOptions {
    let temp = matches!(
        spec.grant,
        RunGrant::All | RunGrant::Named { temp: true, .. }
    );
    let env = env_vars(spec.grant, environment);
    let spawn_env = match spec.grant {
        // the same read, every variable
        RunGrant::All => env.clone(),
        RunGrant::Named { run, run_env, .. } => {
            let granted = !run.is_empty() || !dirs.exec_roots.is_empty();
            let mut forwards = run_env.clone();
            let mut whole = false;
            for program in spec.programs.values() {
                match program {
                    ProgramGrant::Table {
                        pass_env: PassEnvGrant::Names(names),
                        ..
                    } => forwards.extend(names.iter().cloned()),
                    ProgramGrant::Table {
                        pass_env: PassEnvGrant::All { .. },
                        ..
                    } => whole = true,
                    ProgramGrant::Flag { .. } => {}
                }
            }
            spawn_env(granted, &forwards, whole, environment)
        }
    };
    RuntimeOptions {
        authority: authority(spec, dirs),
        env,
        spawn_env,
        temp_dir: temp.then(|| real_temp_dir(&cwd.join(temp_dir(environment).0))),
        denied_write_cause: None,
        exit_step: Some(exit_now),
        conditions: spec.conditions,
        snapshot: STARTUP_SNAPSHOT,
        embed: Some(load_embed),
        console: color::console_options(),
        args: spec.args.to_vec(),
        runtime_version: RUNTIME_VERSION.to_owned(),
    }
}

/// The authority `spec`'s grant gives: unconfined for `-A`, which reads
/// and writes every directory, `fs`'s directories adding nothing; else the
/// project's own modules granted the args host module, the exit host
/// module, the stdout and stderr host modules, the fs host module, reading
/// the project's tree and the directories `dirs` grants as it grants them,
/// and, when the grant names any variable, or `spec.verb`, the env
/// host module with those names, and, when it names any program or exec
/// root, or `spec.verb`, the spawn host module with those programs,
/// each by the table's policy where `spec.programs` holds one, else
/// forwarded what `--allow-run-env=` lists, and exec roots, each forwarded
/// what `--allow-run-env=` lists.
pub fn authority(spec: &RunSpec<'_>, dirs: GrantedDirs) -> Authority {
    let GrantedDirs { fs, exec_roots } = dirs;
    match spec.grant {
        RunGrant::All => Authority::Unconfined,
        RunGrant::Named {
            env,
            temp,
            run,
            run_env,
        } => Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
            exit: Some(ExitGrant {}),
            stdout: Some(StdoutGrant {}),
            stderr: Some(StderrGrant {}),
            env: (spec.verb || !env.is_empty()).then(|| EnvGrant { names: env.clone() }),
            fs: Some(FsGrant { temp: *temp, ..fs }),
            spawn: (spec.verb || !run.is_empty() || !exec_roots.is_empty()).then(|| SpawnGrant {
                programs: run
                    .iter()
                    .map(|name| {
                        let program = spec.programs.get(name).cloned().unwrap_or_else(|| {
                            ProgramGrant::Flag {
                                forward: run_env.iter().cloned().collect(),
                            }
                        });
                        (name.clone(), program)
                    })
                    .collect(),
                exec_roots: exec_roots
                    .into_iter()
                    .map(|(path, dir)| {
                        let forward = run_env.iter().cloned().collect();
                        (path, ExecRootGrant { dir, forward })
                    })
                    .collect(),
            }),
        }),
    }
}

/// The environment variables a confined run's children may be passed,
/// read from `environment`: none when the run grants no program and no
/// exec root, as `granted` says; every variable whose name the env host
/// module could ask for when a program it grants may be passed the whole
/// environment, as `whole` says, read as `-A` reads them; else the base
/// set every child is given ([`CHILD_BASE_ENV`]), then each exact name
/// `run_env` lists, each asked for by name, and, only when it lists a
/// glob, every variable a glob covers, from the whole environment's
/// listing, each whose name the env host module could ask for. A value
/// that isn't UTF-8 is decoded with U+FFFD in place of each invalid
/// sequence, as Node reads one.
fn spawn_env(
    granted: bool,
    run_env: &BTreeSet<EnvPattern>,
    whole: bool,
    environment: &impl Environment,
) -> EnvVars {
    if !granted {
        return EnvVars::default();
    }
    if whole {
        return env_vars(&RunGrant::All, environment);
    }
    let exact = run_env.iter().filter_map(|pattern| match pattern {
        EnvPattern::Exact(name) => Some(name.as_str()),
        EnvPattern::Prefix(_) => None,
    });
    let mut read: Vec<(String, OsString)> = CHILD_BASE_ENV
        .into_iter()
        .chain(exact)
        .filter_map(|name| Some((name.to_owned(), environment.var(name)?)))
        .collect();
    if run_env
        .iter()
        .any(|pattern| matches!(pattern, EnvPattern::Prefix(_)))
    {
        read.extend(environment.vars().into_iter().filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            let globbed = run_env
                .iter()
                .any(|pattern| matches!(pattern, EnvPattern::Prefix(_)) && pattern.covers(&name));
            (globbed && env_name_fault(&name, true).is_none()).then_some((name, value))
        }));
    }
    read.into_iter()
        .map(|(name, value)| (name, value.to_string_lossy().into_owned()))
        .collect()
}

/// The OS's temporary directory, as Node's `os.tmpdir()` names it, read
/// from `environment`: the first of `TMPDIR`, `TMP`, and `TEMP` that's set
/// and not empty, else `/tmp`, a trailing `/` dropped but from `/` itself;
/// and the variable that named it, none for the default. A value that isn't
/// UTF-8 is decoded with U+FFFD in place of each invalid sequence, as Node
/// reads one.
pub fn temp_dir(environment: &impl Environment) -> (PathBuf, Option<&'static str>) {
    let set = ["TMPDIR", "TMP", "TEMP"].into_iter().find_map(|name| {
        let value = environment.var(name)?.to_string_lossy().into_owned();
        (!value.is_empty()).then_some((value, name))
    });
    let (mut dir, variable) = set.map_or_else(
        || ("/tmp".to_owned(), None),
        |(dir, name)| (dir, Some(name)),
    );
    if dir.len() > 1 && dir.ends_with('/') {
        dir.pop();
    }
    (PathBuf::from(dir), variable)
}

/// `dir`, the OS's temporary directory, an absolute path, by its real path
/// as the run starts, so `fs.temp_dir`, which refuses a symlink on it,
/// makes its directories where it led then and never where a symlink the
/// run made since leads. With nothing there now, it's `dir` with its `.`
/// and `..` resolved by their text ([`normalize`]): `fs.temp_dir` fails
/// on it until the run makes it, by no symlink, and always when a symlink
/// is on it.
pub fn real_temp_dir(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| normalize(dir))
}

/// The environment variables `grant` reaches, read from `environment`:
/// each name `--allow-env=` granted that's set, asked for by name and no
/// other read, or, under `-A`, every variable whose name the env host
/// module could ask for (UTF-8, as a JS string is, and neither empty nor
/// holding a `=`). A value that isn't UTF-8 is decoded with U+FFFD in place
/// of each invalid sequence, as Node reads one.
fn env_vars(grant: &RunGrant, environment: &impl Environment) -> EnvVars {
    let read = |(name, value): (String, OsString)| (name, value.to_string_lossy().into_owned());
    match grant {
        RunGrant::Named { env, .. } => env
            .iter()
            .filter_map(|name| Some((name.clone(), environment.var(name)?)))
            .map(read)
            .collect(),
        RunGrant::All => environment
            .vars()
            .into_iter()
            .filter_map(|(name, value)| {
                let name = name.into_string().ok()?;
                env_name_fault(&name, true)
                    .is_none()
                    .then_some((name, value))
            })
            .map(read)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    use std::path::{Path, PathBuf};

    use super::{
        Conditions, EXIT_FAILURE, Ended, Environment, FsGrant, GrantedDir, GrantedDirs,
        ProgramGrant, RUNTIME_VERSION, RunSpec, exit_code, exit_now, real_temp_dir,
        runtime_options,
    };
    use crate::cli::{Invocation, RunInvocation, parse};

    /// No program granted by the table's policy, as `fgro run`'s flags grant
    /// none.
    static NO_TABLE_PROGRAMS: BTreeMap<String, ProgramGrant> = BTreeMap::new();

    /// What `fgro run`'s flags, `run`, hand the run, as [`super::start_spec`]
    /// hands a file's run: no table program, the resolver's conditions
    /// alone, and the env and spawn host modules only when a flag names
    /// something for them.
    fn spec(run: &RunInvocation) -> RunSpec<'_> {
        RunSpec {
            grant: &run.grant,
            dirs: &run.dirs,
            programs: &NO_TABLE_PROGRAMS,
            args: &run.args,
            conditions: Conditions::default(),
            verb: false,
        }
    }

    /// An environment of a test's own, never the process's, which records
    /// what was read of it.
    #[derive(Default)]
    struct TestEnvironment {
        /// Its variables, in order.
        vars: Vec<(OsString, OsString)>,
        /// Each name asked for by [`Environment::var`].
        asked: RefCell<Vec<String>>,
        /// Whether [`Environment::vars`] was called.
        listed: Cell<bool>,
    }

    impl TestEnvironment {
        fn of(vars: &[(&[u8], &[u8])]) -> Self {
            Self {
                vars: vars
                    .iter()
                    .map(|(name, value)| {
                        (
                            OsString::from_vec(name.to_vec()),
                            OsString::from_vec(value.to_vec()),
                        )
                    })
                    .collect(),
                ..Self::default()
            }
        }
    }

    impl Environment for TestEnvironment {
        fn var(&self, name: &str) -> Option<OsString> {
            self.asked.borrow_mut().push(name.to_owned());
            self.vars
                .iter()
                .find(|(var, _)| var.as_encoded_bytes() == name.as_bytes())
                .map(|(_, value)| value.clone())
        }

        fn vars(&self) -> Vec<(OsString, OsString)> {
            self.listed.set(true);
            self.vars.clone()
        }
    }

    /// The run `args` (after `run`) ask for.
    fn invocation(args: &[&str]) -> RunInvocation {
        let args = [&["run"][..], args].concat();
        let Ok(Invocation::Run(run)) = parse(&args) else {
            panic!("{args:?} runs a file");
        };
        run
    }

    /// `--allow-temp` grants `fs.temp_dir` and names the OS's temporary
    /// directory as Node's `os.tmpdir()` does: the first of `TMPDIR`, `TMP`,
    /// and `TEMP` set and not empty, else `/tmp`, a trailing `/` dropped but
    /// from `/`, a relative one resolved against the working directory; `-A`
    /// names it too; a run granting neither names none, and reads no
    /// variable for it.
    #[test]
    fn the_temp_flag_names_the_temporary_directory() {
        let named = |args: &[&str], vars: &[(&[u8], &[u8])]| {
            let environment = TestEnvironment::of(vars);
            let options = runtime_options(
                &spec(&invocation(args)),
                &environment,
                GrantedDirs::default(),
                Path::new("/w"),
            );
            (options, environment.asked.take())
        };
        for (vars, expected) in [
            (&[][..], "/tmp"),
            (&[(&b"TMPDIR"[..], &b"/t/"[..])], "/t"),
            (&[(b"TMPDIR", b""), (b"TMP", b"/x"), (b"TEMP", b"/y")], "/x"),
            (&[(b"TEMP", b"rel/")], "/w/rel"),
            (&[(b"TMPDIR", b"/")], "/"),
        ] {
            let (options, _) = named(&["--allow-temp", "a.js"], vars);
            assert_eq!(options.temp_dir, Some(PathBuf::from(expected)), "{vars:?}");
            let gro_runtime::Authority::Confined(grants) = options.authority else {
                panic!("a run without -A is confined");
            };
            assert!(grants.fs.is_some_and(|fs| fs.temp));
            let (options, _) = named(&["-A", "a.js"], vars);
            assert_eq!(options.temp_dir, Some(PathBuf::from(expected)), "{vars:?}");
        }
        let (options, asked) = named(&["a.js"], &[(b"TMPDIR", b"/t")]);
        assert_eq!(options.temp_dir, None);
        assert_eq!(asked, Vec::<String>::new());
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert!(grants.fs.is_some_and(|fs| !fs.temp));
    }

    /// The temporary directory is named by its real path as the run
    /// starts, a symlink on it resolved, so `fs.temp_dir` refuses one made
    /// since; a path with nothing there passes with its `.` and `..`
    /// resolved by their text.
    #[test]
    fn the_temporary_directory_is_named_by_its_real_path() {
        let base = std::env::temp_dir().join(format!("gro_cli_temp_dir_{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
        let expected = std::fs::canonicalize(&real).unwrap();
        let named = [
            real_temp_dir(&base.join("link")),
            real_temp_dir(&base.join("link/./")),
            real_temp_dir(&base.join("missing/./x/..")),
        ];
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(named, [expected.clone(), expected, base.join("missing")]);
    }

    /// A program's code passes as it is, however the run ended: the one
    /// it gave `exit`, and the one it set when it ran to completion; with
    /// none set, a run that ran to completion exits 0.
    #[test]
    fn a_program_s_code_passes_as_it_is() {
        assert_eq!(exit_code(Ended::Completed(None), false), 0);
        for code in [0, 1, 2, 3, 13, 101, 254, 255] {
            assert_eq!(exit_code(Ended::Completed(Some(code)), false), code);
            assert_eq!(exit_code(Ended::Exited(code), false), code);
        }
    }

    /// A run that failed exits with the failure's code: the code the
    /// program set is no part of how it ended.
    #[test]
    fn a_failed_run_exits_with_the_failure_s_code() {
        for code in [1, 2] {
            assert_eq!(exit_code(Ended::Failed(code), false), code);
            assert_eq!(exit_code(Ended::Failed(code), true), code);
        }
    }

    /// A refused `import()` turns 0 into 1 however the run ended, a
    /// program's `exit(0)` and a set code of 0 included, and leaves every
    /// other code as it is.
    #[test]
    fn a_refusal_fails_a_run_that_ended_with_0() {
        for ended in [
            Ended::Completed(None),
            Ended::Completed(Some(0)),
            Ended::Exited(0),
        ] {
            assert_eq!(exit_code(ended, true), EXIT_FAILURE, "{ended:?}");
            assert_eq!(exit_code(ended, false), 0, "{ended:?}");
        }
        for code in [1, 2, 3, 255] {
            for ended in [
                Ended::Completed(Some(code)),
                Ended::Exited(code),
                Ended::Failed(code),
            ] {
                assert_eq!(exit_code(ended, true), code, "{ended:?}");
                assert_eq!(exit_code(ended, false), code, "{ended:?}");
            }
        }
    }

    /// The script's arguments reach the runtime's options in order and as
    /// written, whatever they look like.
    #[test]
    fn the_runtime_receives_the_script_s_arguments() {
        let script_args = [
            "--help",
            "",
            "--",
            "-x",
            "b c",
            "--allow-env=A",
            "a.js",
            "é",
        ];
        let args = [&["run", "--", "a.js"][..], &script_args].concat();
        let Ok(Invocation::Run(run)) = parse(&args) else {
            panic!("{args:?} runs a.js");
        };
        assert_eq!(
            runtime_options(
                &spec(&run),
                &TestEnvironment::default(),
                GrantedDirs::default(),
                Path::new("/w")
            )
            .args,
            script_args
        );
        let Ok(Invocation::Run(run)) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert!(
            runtime_options(
                &spec(&run),
                &TestEnvironment::default(),
                GrantedDirs::default(),
                Path::new("/w")
            )
            .args
            .is_empty()
        );
    }

    /// The runtime is named as the build names itself.
    #[test]
    fn the_runtime_is_named_with_the_build_s_version() {
        let Ok(Invocation::Run(run)) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert_eq!(
            runtime_options(
                &spec(&run),
                &TestEnvironment::default(),
                GrantedDirs::default(),
                Path::new("/w")
            )
            .runtime_version,
            RUNTIME_VERSION
        );
    }

    /// `run` grants the args host module, the exit host module, whose
    /// step is the CLI's, the stdout and stderr host modules, and the fs
    /// host module, its root the project's tree to read, and nothing else,
    /// reading no environment variable.
    #[test]
    fn the_run_grants_args_exit_the_streams_and_fs() {
        let run = invocation(&["a.js"]);
        let environment = TestEnvironment::of(&[(b"HOME", b"/home/a")]);
        let options = runtime_options(
            &spec(&run),
            &environment,
            GrantedDirs::default(),
            Path::new("/w"),
        );
        assert_eq!(
            options.authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant {}),
                stdout: Some(gro_runtime::StdoutGrant {}),
                stderr: Some(gro_runtime::StderrGrant {}),
                env: None,
                fs: Some(FsGrant::default()),
                spawn: None,
            })
        );
        assert_eq!(options.env, gro_runtime::EnvVars::default());
        assert_eq!(options.spawn_env, gro_runtime::EnvVars::default());
        assert!(environment.asked.borrow().is_empty());
        assert!(!environment.listed.get());
        let cli_step: gro_runtime::ExitStep = exit_now;
        assert!(
            options
                .exit_step
                .is_some_and(|step| std::ptr::fn_addr_eq(step, cli_step))
        );
    }

    /// `--allow-env=` grants the env host module exactly the names its
    /// options list, together, and the run reads only those of the
    /// environment, each by name, a value that isn't UTF-8 decoded with
    /// U+FFFD, and an unset one left out.
    #[test]
    fn allow_env_grants_and_reads_exactly_its_names() {
        let run = invocation(&[
            "--allow-env=B,A",
            "--allow-env=C,A",
            "a.js",
            "--allow-env=D",
        ]);
        let environment =
            TestEnvironment::of(&[(b"A", b"1"), (b"B", b"b\xff"), (b"D", b"4"), (b"E", b"5")]);
        let options = runtime_options(
            &spec(&run),
            &environment,
            GrantedDirs::default(),
            Path::new("/w"),
        );
        assert_eq!(
            options.authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant {}),
                stdout: Some(gro_runtime::StdoutGrant {}),
                stderr: Some(gro_runtime::StderrGrant {}),
                env: Some(gro_runtime::EnvGrant {
                    names: ["A", "B", "C"].map(ToOwned::to_owned).into(),
                }),
                fs: Some(FsGrant::default()),
                spawn: None,
            })
        );
        assert_eq!(*environment.asked.borrow(), ["A", "B", "C"]);
        assert!(!environment.listed.get());
        let read: Vec<(&str, &str)> = options.env.iter().collect();
        assert_eq!(read, [("A", "1"), ("B", "b\u{fffd}")]);
        assert_eq!(options.args, ["--allow-env=D"]);
    }

    /// `-A` runs unconfined, its `--allow-env=` names checked and adding
    /// nothing, and the run reads every variable whose name the env host
    /// module could ask for, in order: a name that isn't UTF-8, an empty
    /// one, and one holding a `=` are left out, and a `*` is a name like
    /// any other.
    #[test]
    fn allow_all_runs_unconfined_and_reads_every_variable() {
        for args in [&["-A", "a.js"][..], &["--allow-env=A", "-A", "-A", "a.js"]] {
            let run = invocation(args);
            let environment = TestEnvironment::of(&[
                (b"B", b"2"),
                (b"N\xff", b"x"),
                (b"", b"empty"),
                (b"K=V", b"x"),
                (b"A*", b"glob"),
                (b"A", b"1"),
                (b"B", b"again"),
            ]);
            let options = runtime_options(
                &spec(&run),
                &environment,
                GrantedDirs::default(),
                Path::new("/w"),
            );
            assert_eq!(
                options.authority,
                gro_runtime::Authority::Unconfined,
                "{args:?}"
            );
            // asked by name only for the temporary directory, which `-A` names
            assert_eq!(*environment.asked.borrow(), ["TMPDIR", "TMP", "TEMP"]);
            assert!(environment.listed.get());
            let read: Vec<(&str, &str)> = options.env.iter().collect();
            assert_eq!(read, [("B", "2"), ("A*", "glob"), ("A", "1")], "{args:?}");
        }
    }

    /// The fs grant the directory flags make reaches a confined run's
    /// authority as it's given, and an unconfined run's holds none, `-A`
    /// reading and writing everything whatever the flags list.
    #[test]
    fn the_fs_grant_reaches_a_confined_run_only() {
        let id = gro_runtime::file_id::FileId {
            dev: 1,
            ino: 2,
            born: None,
        };
        let granted = |real: &str| GrantedDir {
            real: real.into(),
            id,
        };
        let fs = FsGrant {
            read: [("/r".into(), granted("/r"))].into(),
            write: [("/w".into(), granted("/w"))].into(),
            temp: false,
        };
        let run = invocation(&["--allow-read=/r", "a.js"]);
        let options = runtime_options(
            &spec(&run),
            &TestEnvironment::default(),
            GrantedDirs {
                fs: fs.clone(),
                ..GrantedDirs::default()
            },
            Path::new("/w"),
        );
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert_eq!(grants.fs, Some(fs.clone()));
        let run = invocation(&["--allow-read=/r", "-A", "a.js"]);
        let dirs = GrantedDirs {
            fs,
            ..GrantedDirs::default()
        };
        let options = runtime_options(
            &spec(&run),
            &TestEnvironment::default(),
            dirs,
            Path::new("/w"),
        );
        assert_eq!(options.authority, gro_runtime::Authority::Unconfined);
    }

    /// The hints say what grants each need, in the order first needed for
    /// env, one line a flag: env's first, naming the variables a shell
    /// passes as printed, `some of what` when it can't show every one, a
    /// need past the record's bound included, and no line when it can show
    /// none; then each fs line once, read, a symlink out of its root,
    /// write, then a handle to open, each fixed text naming no path.
    #[test]
    fn the_hints_name_env_s_variables_and_fs_s_kinds() {
        use gro_runtime::{FsNeed, NeededGrant, Refusals};

        use super::grant_hints;

        let hints = |needed: &[NeededGrant], more_names: bool| {
            grant_hints(
                &Refusals {
                    needed: needed.to_vec(),
                    more_names: if more_names {
                        vec![gro_runtime::NameKind::Env]
                    } else {
                        Vec::new()
                    },
                    ..Refusals::default()
                },
                None,
            )
        };
        let env = |name: &str| NeededGrant::Env(name.to_owned());
        let read = "hint: add --allow-read naming the directory to grant what fs refused";
        let write = "hint: add --allow-write naming the directory to grant what fs refused, \
                     then open it with fs.open_dir and write through that handle";
        let open_dir = "hint: the run grants what fs refused: \
                        open the directory with fs.open_dir and use that handle";
        let link = "hint: fs refused a symlink leaving its root \
                    (an absolute target, or a .. above the root, even one leading back in): \
                    make the link relative within its root, \
                    or open its target's directory by its own path with fs.open_dir";
        assert!(hints(&[], false).is_empty());
        assert_eq!(
            hints(
                &[
                    NeededGrant::Fs(FsNeed::OpenDir),
                    NeededGrant::Fs(FsNeed::Write),
                    NeededGrant::Fs(FsNeed::LinkTarget),
                    env("HOME"),
                    NeededGrant::Fs(FsNeed::Read),
                    env("A_1"),
                ],
                false
            ),
            [
                "hint: add --allow-env=HOME,A_1 to grant what env refused",
                read,
                link,
                write,
                open_dir,
            ]
        );
        assert_eq!(hints(&[NeededGrant::Fs(FsNeed::Read)], false), [read]);
        assert_eq!(hints(&[NeededGrant::Fs(FsNeed::LinkTarget)], false), [link]);
        assert_eq!(hints(&[NeededGrant::Fs(FsNeed::Write)], false), [write]);
        assert_eq!(
            hints(&[NeededGrant::Fs(FsNeed::OpenDir)], false),
            [open_dir]
        );
        assert_eq!(
            hints(&[NeededGrant::Fs(FsNeed::Read)], true),
            [read],
            "a variable past the bound makes no fs line"
        );
        assert_eq!(
            hints(&[env("A"), env("A B")], false),
            ["hint: add --allow-env=A to grant some of what env refused"]
        );
        assert_eq!(
            hints(&[env("A")], true),
            ["hint: add --allow-env=A to grant some of what env refused"]
        );
        assert_eq!(
            hints(&[env("A B")], false),
            Vec::<String>::new(),
            "an env name it can't show makes no line"
        );
    }

    /// Each free global's hint says what stands for it, in one shape.
    #[test]
    fn each_free_global_s_hint_says_what_stands_for_it() {
        use gro_runtime::FreeGlobal;

        use super::free_global_hint;

        let hints: Vec<String> = FreeGlobal::ALL.into_iter().map(free_global_hint).collect();
        let host = "a run reaches the system through Gro's host modules, each an import, like \
                    '@fuzdev/gro/fs.ts', '@fuzdev/gro/spawn.ts', and '@fuzdev/gro/env.ts'";
        assert_eq!(
            hints,
            [
                "hint: fgro has no global 'process': Gro's process modules are imports, and fgro \
                 has '@fuzdev/gro/env.ts', '@fuzdev/gro/args.ts', '@fuzdev/gro/exit.ts', \
                 '@fuzdev/gro/stdout.ts', '@fuzdev/gro/stderr.ts', '@fuzdev/gro/cwd.ts', \
                 '@fuzdev/gro/pid.ts', and '@fuzdev/gro/runtime_version.ts'"
                    .to_owned(),
                format!("hint: fgro has no global 'Deno': {host}"),
                format!("hint: fgro has no global 'Bun': {host}"),
                "hint: fgro has no global 'require': it loads ES modules only, so use import or \
                 import()"
                    .to_owned(),
                "hint: fgro has no global '__dirname': use import.meta.dirname".to_owned(),
                "hint: fgro has no global '__filename': use import.meta.filename".to_owned(),
                "hint: fgro has no global 'global': use globalThis".to_owned(),
                "hint: fgro has no global 'Buffer': use Uint8Array, with TextEncoder and \
                 TextDecoder for text"
                    .to_owned(),
                "hint: fgro has no global 'fetch': the network is an import, net.fetch from \
                 '@fuzdev/gro/net.ts', which isn't on fgro yet"
                    .to_owned(),
                "hint: fgro has no global 'crypto' yet: Web Crypto isn't built on it".to_owned(),
            ]
        );
    }

    /// A builtin module the run's grants lack the row of names the flag
    /// that grants it, first, once each, in the order each was needed,
    /// naming no name; a row `fgro run` always grants has no hint.
    #[test]
    fn the_module_hints_name_the_flag_first() {
        use gro_runtime::resolver::Capability;
        use gro_runtime::{NeededGrant, Refusals};

        use super::grant_hints;

        let hints = grant_hints(
            &Refusals {
                needed: vec![
                    NeededGrant::Env("HOME".to_owned()),
                    NeededGrant::Module(Capability::Spawn),
                    NeededGrant::Module(Capability::Fs),
                    NeededGrant::Module(Capability::Env),
                ],
                ..Refusals::default()
            },
            None,
        );
        assert_eq!(
            hints,
            [
                "hint: add --allow-run naming each program the run starts, or --allow-run-in \
                 naming a directory holding them, to grant the import of '@fuzdev/gro/spawn.ts'",
                "hint: add --allow-env naming each variable the run reads to grant the import of \
                 '@fuzdev/gro/env.ts'",
                "hint: add --allow-env=HOME to grant what env refused",
            ]
        );
    }

    /// The spawn hints name the programs a shell passes as printed, never
    /// a path or a name a shell or the CLI would read otherwise, and the
    /// variables to forward as env's rule names them, never a glob, each
    /// line `some of what` when it can't show every name of its kind, in
    /// the order env, programs, variables, then fs.
    #[test]
    fn the_spawn_hints_name_plain_programs_and_variables() {
        use gro_runtime::{FsNeed, NameKind, NeededGrant, Refusals};

        use super::grant_hints;

        let hints = |needed: &[NeededGrant], more_names: &[NameKind]| {
            grant_hints(
                &Refusals {
                    needed: needed.to_vec(),
                    more_names: more_names.to_vec(),
                    ..Refusals::default()
                },
                None,
            )
        };
        let run = |name: &str| NeededGrant::Run(name.to_owned());
        let run_env = |name: &str| NeededGrant::RunEnv(name.to_owned());
        assert_eq!(
            hints(
                &[
                    NeededGrant::Fs(FsNeed::Temp),
                    run_env("CARGO_HOME"),
                    run("git"),
                    NeededGrant::Env("HOME".to_owned()),
                    run("c++"),
                    run("x.py"),
                    run("a_b-1"),
                ],
                &[]
            ),
            [
                "hint: add --allow-env=HOME to grant what env refused",
                "hint: add --allow-run=git,c++,x.py,a_b-1 to grant what spawn refused",
                "hint: add --allow-run-env=CARGO_HOME to grant what spawn refused",
                "hint: add --allow-temp to grant what fs refused",
            ]
        );
        for unshown in [
            "./bin/tool",
            "-x",
            ".",
            "..",
            ".hidden",
            "_x",
            "+x",
            "a b",
            "$(x)",
            "a;b",
            "é",
            "a*",
        ] {
            assert_eq!(
                hints(&[run(unshown), run("git")], &[]),
                ["hint: add --allow-run=git to grant some of what spawn refused"],
                "{unshown}"
            );
            assert!(hints(&[run(unshown)], &[]).is_empty(), "{unshown}");
        }
        assert_eq!(
            hints(&[run_env("CARGO_*"), run_env("A")], &[]),
            ["hint: add --allow-run-env=A to grant some of what spawn refused"]
        );
        assert!(hints(&[run_env("CARGO_*")], &[]).is_empty());
        assert_eq!(
            hints(&[run("git"), run_env("A")], &[NameKind::Run]),
            [
                "hint: add --allow-run=git to grant some of what spawn refused",
                "hint: add --allow-run-env=A to grant what spawn refused",
            ]
        );
    }

    /// A listed refusal's line names what refused it, and for `fs` the
    /// kind of grant it needed in the words of that kind's hint, or that it
    /// needed none, its message escaped.
    #[test]
    fn a_listed_refusal_names_what_refused_it_and_an_fs_one_its_need() {
        use gro_runtime::{FsNeed, Refusal, RefusedBy};

        use super::refusal_line;

        let line = |by: RefusedBy, message: &str| {
            refusal_line(&Refusal {
                by,
                message: message.to_owned(),
            })
        };
        assert_eq!(
            line(RefusedBy::Fs(Some(FsNeed::Read)), "not_granted: /r"),
            "error: fs refused a read: not_granted: /r"
        );
        assert_eq!(
            line(RefusedBy::Fs(Some(FsNeed::LinkTarget)), "not_granted: /l"),
            "error: fs refused a symlink leaving its root: not_granted: /l"
        );
        assert_eq!(
            line(RefusedBy::Fs(Some(FsNeed::Write)), "not_granted: /w"),
            "error: fs refused a write: not_granted: /w"
        );
        assert_eq!(
            line(RefusedBy::Fs(Some(FsNeed::OpenDir)), "not_granted: /o"),
            "error: fs refused what the run grants: not_granted: /o"
        );
        assert_eq!(
            line(RefusedBy::Fs(None), "not_granted: /n\u{1b}[2K\u{2028}"),
            "error: fs refused what no grant allows: not_granted: /n\\u{1b}[2K\\u{2028}"
        );
        assert_eq!(
            line(RefusedBy::Env, "not_granted: HOME\n"),
            "error: env refused: not_granted: HOME\\u{a}"
        );
        assert_eq!(
            line(RefusedBy::Import, "cannot resolve 'ext:x'"),
            "error: import() refused: cannot resolve 'ext:x'"
        );
        assert_eq!(
            line(RefusedBy::Spawn, "not_granted: git\u{1b}[2K"),
            "error: spawn refused: not_granted: git\\u{1b}[2K"
        );
    }

    /// A flag's directory is noted when its real path isn't its text,
    /// resolved against the working directory with its `.` and `..`
    /// resolved by their text: through a symlink, a `..` after one
    /// included, and never for a path that is real already however it's
    /// spelled; the typed text and the real path printed escaped.
    #[test]
    fn a_flag_through_a_symlink_is_noted_with_its_real_path() {
        use std::path::Path;

        use super::grant_note;
        use crate::cli::ListFlag;

        let note = |flag: ListFlag, dir: &str, real: &str| {
            grant_note(flag, None, dir, &Path::new("/c").join(dir), Path::new(real))
        };
        for dir in [
            "d", "./d", "d/", "d/.", "x/../d", "/c/d", "/c//d/", "/../c/d",
        ] {
            assert_eq!(note(ListFlag::Read, dir, "/c/d"), None, "{dir}");
        }
        assert_eq!(
            note(ListFlag::Read, "l", "/elsewhere"),
            Some("note: --allow-read=l grants /elsewhere".to_owned())
        );
        assert_eq!(
            note(ListFlag::Write, "l/..", "/r"),
            Some("note: --allow-write=l/.. grants /r".to_owned())
        );
        assert_eq!(
            note(ListFlag::Read, "l\u{1b}[2K", "/r\u{202e}"),
            Some("note: --allow-read=l\\u{1b}[2K grants /r\\u{202e}".to_owned())
        ); // the write flag of one program names it, escaped too
        assert_eq!(
            grant_note(
                ListFlag::RunWrite,
                Some("v\u{1b}"),
                "l",
                Path::new("/c/l"),
                Path::new("/r")
            ),
            Some("note: --allow-run-write=v\\u{1b}:l grants /r".to_owned())
        );
    }

    /// For a command of Gro's, a directory any directory flag lists that
    /// lies inside the project by its text, relative or absolute, and
    /// leads out of it through a symlink is refused, naming the flag as
    /// written and the real path to type instead; one leading through a
    /// symlink that stays inside is noted, as is one typed outside; and
    /// `fgro run` notes each, the path being the person's.
    #[test]
    fn a_command_s_directory_leading_out_of_the_project_is_refused() {
        use std::collections::{BTreeMap, BTreeSet};

        use super::{DirCheck, GrantedDirs, granted_dirs};
        use crate::cli::{FlagDirs, ListFlag};
        use crate::error::CliError;

        let base = RemovedOnDrop::new("gro_cli_leaves_project");
        let base = &base.0;
        let root = base.join("project");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::os::unix::fs::symlink("../outside", root.join("out")).unwrap();
        std::os::unix::fs::symlink("src", root.join("in")).unwrap();
        std::os::unix::fs::symlink(".", root.join("self")).unwrap();
        std::os::unix::fs::symlink("outside", base.join("link")).unwrap();
        // the project by another path, a link to it outside, as a `$PWD`
        // through a symlinked directory names it
        std::os::unix::fs::symlink("project", base.join("alias")).unwrap();
        let outside = base.join("outside");
        let flags = |flag: ListFlag, dir: &str| {
            let mut dirs = FlagDirs::default();
            let dir = BTreeSet::from([dir.to_owned()]);
            match flag {
                ListFlag::Read => dirs.read = dir,
                ListFlag::Write => dirs.write = dir,
                ListFlag::RunIn => dirs.run_in = dir,
                ListFlag::RunWrite => dirs.run_write = BTreeMap::from([("vitest".to_owned(), dir)]),
                ListFlag::Env | ListFlag::Run | ListFlag::RunEnv => unreachable!(),
            }
            dirs
        };
        let refused = |flag: ListFlag, dir: &str| match granted_dirs(
            &flags(flag, dir),
            &root,
            DirCheck::Project(&root),
        ) {
            Err(error @ CliError::LeavesProject { .. }) => {
                assert_eq!(error.exit_code(), crate::EXIT_USAGE);
                error.to_string()
            }
            other => panic!("{flag} {dir}: {other:?}"),
        };
        let mut results = Vec::new();
        for flag in [
            ListFlag::Read,
            ListFlag::Write,
            ListFlag::RunIn,
            ListFlag::RunWrite,
        ] {
            let program = if flag == ListFlag::RunWrite {
                "vitest:"
            } else {
                ""
            };
            for dir in [
                "out",
                "./out/",
                "src/../out",
                &root.join("out").to_string_lossy(),
                "../alias/out",
                &base.join("alias/out").to_string_lossy(),
                "self/out",
            ] {
                results.push((
                    refused(flag, dir),
                    format!(
                        "{flag}={program}{dir} names {dir}, which leads out of the project to {} \
                         through a symlink: a directory typed inside the project must be inside \
                         it, so name {} itself to grant it",
                        outside.display(),
                        outside.display()
                    ),
                ));
            }
            // inside through a symlink, the root itself through one, and
            // outside by its text, noted
            for (dir, real) in [
                ("in", root.join("src")),
                ("self", root.clone()),
                ("../link", outside.clone()),
            ] {
                let granted =
                    granted_dirs(&flags(flag, dir), &root, DirCheck::Project(&root)).unwrap();
                assert_eq!(
                    granted.notes,
                    [format!(
                        "note: {flag}={program}{dir} grants {}",
                        real.display()
                    )],
                    "{flag} {dir}"
                );
            }
            // `fgro run` notes it
            let granted = granted_dirs(&flags(flag, "out"), &root, DirCheck::Noted).unwrap();
            assert_eq!(
                granted.notes,
                [format!(
                    "note: {flag}={program}out grants {}",
                    outside.display()
                )]
            );
        }
        // the run's write directories stay apart from every grant of the
        // realm's
        let granted = granted_dirs(
            &flags(ListFlag::RunWrite, "../outside"),
            &root,
            DirCheck::Project(&root),
        )
        .unwrap();
        assert_eq!(granted.dirs, GrantedDirs::default());
        assert!(granted.resolved.is_empty());
        assert_eq!(
            granted.run_writes,
            [gro_runtime::confine::plan::RunWriteDir {
                named: "vitest".to_owned(),
                dir: "../outside".to_owned(),
                real: outside,
            }]
        );
        for (found, expected) in results {
            assert_eq!(found, expected);
        }
    }

    /// A directory of a test's own under the system's temporary directory,
    /// by its real path, removed with what it holds when dropped, a failed
    /// case's included.
    struct RemovedOnDrop(PathBuf);

    impl RemovedOnDrop {
        fn new(name: &str) -> Self {
            let base = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("{name}_{}", std::process::id()));
            std::fs::create_dir_all(&base).unwrap();
            Self(base)
        }
    }

    impl Drop for RemovedOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// For a command of Gro's, a directory any directory flag lists whose
    /// real path lies in a top-level dot-entry or `node_modules` its text
    /// doesn't name is refused, naming that entry's path to type instead:
    /// through a link in a directory, a top-level link, a link into
    /// `node_modules`, and the project reached through a link outside it.
    /// The entry named outright is honored, by any spelling, as before; a
    /// link inside to an ordinary directory is noted; and `fgro run` notes
    /// the link into the entry.
    #[test]
    fn a_command_s_directory_leading_into_an_unnamed_entry_is_refused() {
        use std::collections::{BTreeMap, BTreeSet};

        use super::{DirCheck, granted_dirs};
        use crate::cli::{FlagDirs, ListFlag};
        use crate::error::CliError;

        let base = RemovedOnDrop::new("gro_cli_into_entry");
        let base = &base.0;
        let root = base.join("project");
        for dir in ["src/lib", ".git/hooks", "node_modules/x"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::os::unix::fs::symlink("../.git", root.join("src/fixtures")).unwrap();
        std::os::unix::fs::symlink(".git", root.join("out")).unwrap();
        std::os::unix::fs::symlink("../node_modules/x", root.join("src/nm")).unwrap();
        std::os::unix::fs::symlink("lib", root.join("src/in")).unwrap();
        std::os::unix::fs::symlink("project", base.join("alias")).unwrap();
        let flags = |flag: ListFlag, dir: &str| {
            let mut dirs = FlagDirs::default();
            let dir = BTreeSet::from([dir.to_owned()]);
            match flag {
                ListFlag::Read => dirs.read = dir,
                ListFlag::Write => dirs.write = dir,
                ListFlag::RunIn => dirs.run_in = dir,
                ListFlag::RunWrite => dirs.run_write = BTreeMap::from([("vitest".to_owned(), dir)]),
                ListFlag::Env | ListFlag::Run | ListFlag::RunEnv => unreachable!(),
            }
            dirs
        };
        let alias = base.join("alias");
        for flag in [
            ListFlag::Read,
            ListFlag::Write,
            ListFlag::RunIn,
            ListFlag::RunWrite,
        ] {
            let program = if flag == ListFlag::RunWrite {
                "vitest:"
            } else {
                ""
            };
            for (dir, entry) in [
                ("src/fixtures", ".git"),
                ("src/fixtures/hooks", ".git/hooks"),
                ("out", ".git"),
                ("./out/hooks", ".git/hooks"),
                ("src/nm", "node_modules/x"),
                (&alias.join("src/fixtures").to_string_lossy(), ".git"),
                ("../alias/out", ".git"),
            ] {
                match granted_dirs(&flags(flag, dir), &root, DirCheck::Project(&root)) {
                    Err(error @ CliError::LinksIntoEntry { .. }) => {
                        assert_eq!(error.exit_code(), crate::EXIT_USAGE);
                        assert_eq!(
                            error.to_string(),
                            format!(
                                "{flag}={program}{dir} names {dir}, which leads through a symlink \
                                 to {entry}: name {entry} itself to grant it"
                            )
                        );
                    }
                    other => panic!("{flag} {dir}: {other:?}"),
                }
            }
            // the entry named outright, by any spelling, as before
            for dir in [
                ".git",
                "./.git/hooks",
                &root.join(".git").to_string_lossy(),
                &alias.join(".git/hooks").to_string_lossy(),
                "node_modules/x",
                "src/../node_modules",
            ] {
                let granted = granted_dirs(&flags(flag, dir), &root, DirCheck::Project(&root))
                    .unwrap_or_else(|error| panic!("{flag} {dir}: {error}"));
                assert!(
                    granted.notes.is_empty() || dir.starts_with(&*alias.to_string_lossy()),
                    "{flag} {dir}: {:?}",
                    granted.notes
                );
            }
            // a link inside to an ordinary directory is noted
            let granted =
                granted_dirs(&flags(flag, "src/in"), &root, DirCheck::Project(&root)).unwrap();
            assert_eq!(
                granted.notes,
                [format!(
                    "note: {flag}={program}src/in grants {}",
                    root.join("src/lib").display()
                )]
            );
            // `fgro run` notes the link into the entry
            let granted =
                granted_dirs(&flags(flag, "src/fixtures"), &root, DirCheck::Noted).unwrap();
            assert_eq!(
                granted.notes,
                [format!(
                    "note: {flag}={program}src/fixtures grants {}",
                    root.join(".git").display()
                )]
            );
        }
    }

    /// A verb's run grants the env and spawn host modules though it names
    /// nothing for them, since its entry imports both; each table program
    /// by its policy, a flag's by the flag; a child's environment read
    /// whole when a program may be passed it, else the base set and what
    /// the table's lists and the flags name; and its condition.
    #[test]
    fn a_verb_s_run_grants_its_closure_s_programs_by_their_policies() {
        use std::collections::BTreeSet;

        use gro_runtime::{EnvPattern, PassEnvGrant, ProgramGrant};

        use crate::cli::{FlagDirs, RunGrant};
        use crate::verb::prepass::VerbRun;

        let vars: &[(&[u8], &[u8])] = &[(b"PATH", b"/bin"), (b"GIT_DIR", b"/g"), (b"SECRET", b"s")];
        let git = ProgramGrant::Table {
            pass_env: PassEnvGrant::Names(vec![EnvPattern::Prefix("GIT_".to_owned())]),
            env: BTreeMap::new(),
        };
        let vitest = ProgramGrant::Table {
            pass_env: PassEnvGrant::All {
                omit_env: BTreeSet::new(),
            },
            env: BTreeMap::new(),
        };
        let run = |programs: &[(&str, &ProgramGrant)], development| VerbRun {
            cli_args: vec!["check".to_owned()],
            grant_flags: Vec::new(),
            root: PathBuf::from("/w"),
            grant: RunGrant::Named {
                env: BTreeSet::new(),
                temp: false,
                run: programs
                    .iter()
                    .map(|(name, _)| (*name).to_owned())
                    .chain(["cargo".to_owned()])
                    .collect(),
                run_env: BTreeSet::new(),
            },
            dirs: FlagDirs::default(),
            flag_dirs: FlagDirs::default(),
            writes_root: false,
            tools: BTreeMap::new(),
            declared: BTreeSet::new(),
            policy: crate::verb::fallback::FallbackPolicy::All,
            gro: None,
            programs: programs
                .iter()
                .map(|(name, program)| ((*name).to_owned(), (*program).clone()))
                .collect(),
            run_writes: BTreeMap::new(),
            may_start: BTreeSet::new(),
            development,
            home: None,
            version: String::new(),
        };
        let options = |run: &VerbRun, environment: &TestEnvironment| {
            runtime_options(
                &super::start_spec(&super::RunStart::from(run.clone())),
                environment,
                GrantedDirs::default(),
                Path::new("/w"),
            )
        };
        let listed = TestEnvironment::of(vars);
        let git_only = options(&run(&[("git", &git)], true), &listed);
        let read: Vec<(&str, &str)> = git_only.spawn_env.iter().collect();
        assert_eq!(read, [("PATH", "/bin"), ("GIT_DIR", "/g")]);
        assert!(git_only.conditions.development);
        assert_eq!(git_only.args, ["check"]);
        let gro_runtime::Authority::Confined(grants) = git_only.authority else {
            panic!("confined");
        };
        assert_eq!(grants.env.map(|env| env.names), Some(BTreeSet::new()));
        let spawn = grants.spawn.unwrap();
        assert_eq!(spawn.programs["git"], git);
        assert_eq!(
            spawn.programs["cargo"],
            ProgramGrant::Flag {
                forward: Vec::new()
            }
        );
        let whole = TestEnvironment::of(vars);
        let with_vitest = options(
            &run(
                &[("git", &git), ("node_modules/.bin/vitest", &vitest)],
                false,
            ),
            &whole,
        );
        let read: Vec<(&str, &str)> = with_vitest.spawn_env.iter().collect();
        assert_eq!(read, [("PATH", "/bin"), ("GIT_DIR", "/g"), ("SECRET", "s")]);
        assert!(!with_vitest.conditions.development);
    }

    /// `--allow-run=` grants the spawn host module its programs, each
    /// forwarded what `--allow-run-env=` lists, and the run reads for
    /// children the base set and each exact name, by name, the whole
    /// environment's listing only when a glob is granted, keeping what a
    /// glob covers; a run granting no program reads none of it, and `-A`
    /// hands children what it reads for `env`, everything.
    #[test]
    fn allow_run_grants_its_programs_and_reads_only_what_children_may_get() {
        use gro_runtime::{EnvPattern, ProgramGrant, SpawnGrant};

        let vars: &[(&[u8], &[u8])] = &[
            (b"PATH", b"/bin"),
            (b"HOME", b"/h"),
            (b"CARGO_HOME", b"/c\xff"),
            (b"CARGO\xff", b"x"),
            (b"XCARGO_HOME", b"x"),
            (b"SECRET", b"s"),
            (b"TOKEN", b"t"),
        ];
        let run = invocation(&[
            "--allow-run=git,cargo",
            "--allow-run-env=TOKEN,UNSET",
            "a.js",
        ]);
        let environment = TestEnvironment::of(vars);
        let options = runtime_options(
            &spec(&run),
            &environment,
            GrantedDirs::default(),
            Path::new("/w"),
        );
        let forward = vec![
            EnvPattern::Exact("TOKEN".to_owned()),
            EnvPattern::Exact("UNSET".to_owned()),
        ];
        let program = ProgramGrant::Flag { forward };
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert_eq!(
            grants.spawn,
            Some(SpawnGrant {
                programs: [
                    ("cargo".to_owned(), program.clone()),
                    ("git".to_owned(), program),
                ]
                .into(),
                exec_roots: BTreeMap::new(),
            })
        );
        assert_eq!(
            *environment.asked.borrow(),
            [
                "PATH",
                "HOME",
                "TMPDIR",
                "TERM",
                "NO_COLOR",
                "FORCE_COLOR",
                "TOKEN",
                "UNSET"
            ]
        );
        assert!(!environment.listed.get());
        let read: Vec<(&str, &str)> = options.spawn_env.iter().collect();
        assert_eq!(read, [("PATH", "/bin"), ("HOME", "/h"), ("TOKEN", "t")]);
        assert_eq!(options.env, gro_runtime::EnvVars::default());

        let run = invocation(&["--allow-run=git", "--allow-run-env=CARGO_*", "a.js"]);
        let environment = TestEnvironment::of(vars);
        let options = runtime_options(
            &spec(&run),
            &environment,
            GrantedDirs::default(),
            Path::new("/w"),
        );
        assert!(environment.listed.get());
        let read: Vec<(&str, &str)> = options.spawn_env.iter().collect();
        assert_eq!(
            read,
            [
                ("PATH", "/bin"),
                ("HOME", "/h"),
                ("CARGO_HOME", "/c\u{fffd}")
            ]
        );

        // the variables' flag names nothing without a program, so a run
        // granting no program reads nothing for children
        let run = invocation(&["--allow-env=HOME", "a.js"]);
        let environment = TestEnvironment::of(vars);
        let options = runtime_options(
            &spec(&run),
            &environment,
            GrantedDirs::default(),
            Path::new("/w"),
        );
        assert_eq!(*environment.asked.borrow(), ["HOME"]);
        assert_eq!(options.spawn_env, gro_runtime::EnvVars::default());

        let run = invocation(&["--allow-run=git", "-A", "a.js"]);
        let environment = TestEnvironment::of(vars);
        let options = runtime_options(
            &spec(&run),
            &environment,
            GrantedDirs::default(),
            Path::new("/w"),
        );
        assert_eq!(options.authority, gro_runtime::Authority::Unconfined);
        assert_eq!(options.spawn_env, options.env);
        assert_eq!(options.spawn_env.iter().count(), 6);
    }

    /// `--allow-run-in=` grants `spawn` with its exec roots, each as the
    /// flags resolved it, passed what `--allow-run-env=` forwards, though
    /// no program is named, and reads what a child may be given; with
    /// neither flag, the run grants no `spawn` and reads nothing for
    /// children.
    #[test]
    fn allow_run_in_grants_exec_roots_and_reads_what_children_may_get() {
        let id = gro_runtime::file_id::FileId {
            dev: 1,
            ino: 2,
            born: None,
        };
        let dir = GrantedDir {
            real: "/real/x".into(),
            id,
        };
        let dirs = GrantedDirs {
            exec_roots: [(PathBuf::from("/x"), dir.clone())].into(),
            ..GrantedDirs::default()
        };
        let run = invocation(&["--allow-run-in=/x", "--allow-run-env=A", "a.js"]);
        assert!(super::grants_programs(&run.grant, &run.dirs));
        let environment = TestEnvironment::of(&[(b"A", b"1"), (b"PATH", b"/bin")]);
        let options = runtime_options(&spec(&run), &environment, dirs, Path::new("/w"));
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        let forward = vec![gro_runtime::EnvPattern::Exact("A".to_owned())];
        assert_eq!(
            grants.spawn,
            Some(gro_runtime::SpawnGrant {
                programs: BTreeMap::new(),
                exec_roots: [(
                    PathBuf::from("/x"),
                    gro_runtime::ExecRootGrant { dir, forward }
                )]
                .into(),
            })
        );
        assert_eq!(
            options.spawn_env.iter().collect::<Vec<_>>(),
            [("PATH", "/bin"), ("A", "1")]
        );
        let run = invocation(&["--allow-read=/x", "a.js"]);
        assert!(!super::grants_programs(&run.grant, &run.dirs));
        let environment = TestEnvironment::of(&[(b"PATH", b"/bin")]);
        let options = runtime_options(
            &spec(&run),
            &environment,
            GrantedDirs::default(),
            Path::new("/w"),
        );
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert_eq!(grants.spawn, None);
        assert!(environment.asked.borrow().is_empty());
    }

    /// The variable a child of [`a_refused_raise_of_the_open_files_limit_fails_nothing`]
    /// runs its half by.
    const READY_CHILD: &str = "GRO_CLI_TEST_READY_CHILD";

    /// A run that grants a program is made ready to start them though the
    /// kernel refuses to raise the soft limit on open descriptors, as Node
    /// goes on: the raise is best-effort. In a child process, since making
    /// ready marks every inherited descriptor close-on-exec.
    #[test]
    fn a_refused_raise_of_the_open_files_limit_fails_nothing() {
        if std::env::var_os(READY_CHILD).is_some() {
            let refused = || Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            assert!(super::ready_to_spawn(refused).is_ok());
            return;
        }
        let test = "run::tests::a_refused_raise_of_the_open_files_limit_fails_nothing";
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--nocapture", "--quiet"])
            .env(READY_CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // a filter that matches no test exits 0 as well
        assert!(String::from_utf8_lossy(&output.stdout).contains(" 1 passed;"));
    }

    /// The variables' `Debug` names them and prints no value.
    #[test]
    fn the_variables_debug_prints_no_value() {
        let run = invocation(&["--allow-env=TOKEN", "a.js"]);
        let environment = TestEnvironment::of(&[(b"TOKEN", b"secret-value")]);
        let options = runtime_options(
            &spec(&run),
            &environment,
            GrantedDirs::default(),
            Path::new("/w"),
        );
        let printed = format!("{options:?}");
        assert!(printed.contains("TOKEN"), "{printed}");
        assert!(!printed.contains("secret-value"), "{printed}");
    }
}
