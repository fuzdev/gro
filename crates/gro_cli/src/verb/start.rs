//! A command of Gro's started in the run's kernel box: the steps its
//! process takes on the main thread, its only one, from the pre-pass's plan
//! to the box going up, then its run on a thread of its own
//! ([`run_boxed`]).
//!
//! The order, written once here; each step that changes the process takes
//! the token the one before it made (`gro_runtime::confine::run`):
//!
//! 1. A build with no snapshot is refused: started from source, the
//!    runtime reads `deno_web`'s JS and makes Gro's library from the build
//!    tree, which no rule of the box admits.
//! 2. `-A` is refused, naming the project's Node Gro, the unconfined
//!    spelling: no box holds it.
//! 3. The probes that change nothing: the kernel's Landlock held to the
//!    box's floor (`probe_floor`), where a kernel below it is a fallback
//!    the host decides ([`floor_fallback`]), which execs the project's Node
//!    Gro here, as the pre-pass's fallbacks exec it, with the process as
//!    it started, or is refused under `GRO_FALLBACK=command`; then stdin,
//!    stdout, and stderr held to what the box allows (`check_stdio`).
//! 4. The inherited descriptors closed: the first change, after which
//!    nothing execs in place of the run.
//! 5. A phases build's file opened, which the close would have taken.
//! 6. The run's network, its own namespace or the host's with every
//!    internet socket refused; then the process hardened.
//! 7. The OS's temporary directory opened by its real path, and the private
//!    directories of runs gone swept from it, unless it lies in the
//!    project, which no sweep reaches.
//! 8. What the run and its box both hold, resolved once: the directories
//!    the grant flags list (`granted_dirs`), one inside the project that
//!    leads out of it through a symlink, or through one into an entry the
//!    box leaves out that it doesn't name, refused, those
//!    `--allow-run-write=` lists the box's alone, and the programs the run
//!    grants, pinned ([`RunPins::pin`]), the working directory the
//!    project's root the pre-pass read.
//! 9. The box's plan, from its inputs ([`box_inputs`]) and the disk's facts
//!    read once; what it refuses refuses the run, and its note prints. A
//!    `node_modules` the plan follows to another checkout's gets a read
//!    root of the realm's there too ([`linked_read_root`]), so the realm
//!    reads through it what the box reads.
//! 10. What the box names made: the run's private directory, where it has
//!     one, refused when the OS's temporary directory lies in the project,
//!     since it would be an entry the box lets the run's tools write and
//!     later commands read, and, for a run that starts a program, when a
//!     rule of the plan a program's box would take holds it, since every
//!     program would read the others' temporary directories through it;
//!     then its tool directories.
//! 11. The run's environment, set in the process so the run, its tools,
//!     and the libraries that read the process's own see one: the
//!     temporary variables and `XDG_DATA_HOME`, `XDG_CACHE_HOME`, and
//!     `XDG_STATE_HOME` pointed into the private directory (`REDIRECTED`,
//!     which each program's launch points into its own directory there)
//!     and the vendored `deno_core`'s diagnostic variables removed
//!     ([`REMOVED`]); then `SIGCHLD` at its default, process-wide.
//! 12. The run's options read from the process, after the environment is
//!     set.
//! 13. The box's rules opened from its plan, then the box installed; where
//!     the network is the host's, a note says the run binds no internet
//!     socket.
//! 14. The run's own thread, sized, started once the box is up, so every
//!     thread V8 and tokio start inherits it, which runs the generated
//!     entry on a [`RunEngine`](gro_runtime::engine::RunEngine), the engine
//!     only a box makes.
//!
//! A priced build (`gro_runtime::prices`) can leave out step 6's namespace,
//! step 7's sweep, and step 13's Landlock or denylist, each to be timed
//! against the build with every step taken; without the `prices` feature, as
//! the binary that ships is built, nothing reads the switches and every step
//! is taken.
//!
//! What the box says once the run is going, each text built from the rules
//! it installed: a write through the fs host module that the kernel
//! refuses, for a path whose real destination no write rule holds, gains a
//! cause naming where the box lets the run write and what stays read-only
//! ([`denied_write_cause`]), since the run's grants allowed it; and a run
//! that fails after a tool it started ended other than by exiting 0, by no
//! signal of the run's, ends with a note naming the box's walls
//! ([`walls_note`]), since the tool's own error may be a wall it met.
//!
//! Exit codes ([`BoxStartError::exit_code`]): `2` for what the caller
//! changes, `-A`, a socket on stdio, a directory a grant flag lists inside
//! the project that leads out of it through a symlink, or through one into
//! a dot-entry or `node_modules` it doesn't name (`CliError::LeavesProject`,
//! `CliError::LinksIntoEntry`), what the plan refuses, a temporary
//! directory in the project or held by a rule a program's box would take,
//! and every path planted where the box opens a rule or makes a tool
//! directory; `1` for a build with no snapshot and a step that failed. A
//! fallback exits with the Node Gro's code.
//!
//! Kinds: the box is a boundary, the kernel's, against the project's code
//! and tests, the tools and their children, and an exploit of the engine;
//! its asset is what its rules leave out (the home directory, `.git`,
//! `node_modules`, the dot-entries, the root's entry set, the network
//! beyond the run's loopback), and what defeats it is its module docs'
//! (`gro_runtime::confine::run`, `gro_runtime::confine::plan`). The order's
//! tokens and the engine's are guardrails against gro's own bug, held by
//! the compiler. The floor's fallback is the host deciding to run the Node
//! Gro, which confines nothing: its note is a review signal, and
//! `GRO_FALLBACK=command` refusing it a boundary, a native check before
//! anything runs. Every note and refusal here is a review signal the run's
//! code could print as well.

use std::collections::BTreeSet;
use std::os::fd::AsFd as _;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use gro_runtime::confine::ConfineError;
use gro_runtime::confine::plan::{
    BoxFacts, BoxInputs, BoxPlan, DirFlag, GrantedFlagDir, PlanRefusal, RuleSource, RunWriteDir,
    RunWriteGrant,
};
use gro_runtime::confine::program::{ProgramRules, REDIRECTED, Redirect};
use gro_runtime::confine::rules::RuleError;
use gro_runtime::confine::run::{
    LandlockFloor, RunClosed, RunConfined, RunNetwork, RunPrivateDir, RunStepError, check_stdio,
    probe_floor,
};
use gro_runtime::confine::tool_dirs::ToolDirError;
use gro_runtime::engine::{EngineOptions, init_run_engine};
use gro_runtime::file_id::FileId;
use gro_runtime::phases::{self, Phase};
use gro_runtime::prices::{self, RunDefault};
use gro_runtime::{BINARY_NAME, DeniedWriteCause, GeneratedEntry, GrantedDir, RunPins};
use gro_sys::private_dir::PrivateDirError;
use thiserror::Error;

use super::fallback::{FallbackReason, KernelLandlock, exec_node_gro};
use super::prepass::{VerbRun, command_line, command_of, floor_fallback, node_gro_command};
use crate::cli::{FlagDirs, RunGrant};
use crate::error::CliError;
use crate::print::{escape_controls, stderr_line};
use crate::run::{
    Environment, NODE_ENV, ProcessEnvironment, authority, finish, granted_dirs, grants_programs,
    real_temp_dir, runtime_options, temp_dir, verb_spec,
};
use crate::snapshot::STARTUP_SNAPSHOT;
use crate::thread::{Joined, on_sized_thread};
use crate::{EXIT_FAILURE, EXIT_PANIC, EXIT_USAGE};

/// The vendored `deno_core`'s diagnostic variables, removed from the run's
/// environment: a phases file, a lazy-load log, and an import graph file
/// it appends every module's imports to.
const REMOVED: [&str; 3] = [
    "DENO_STARTUP_PHASES",
    "DENO_LOG_LAZY_LOAD",
    "DENO_SNAPSHOT_IMPORT_GRAPH",
];

/// Why a command of Gro's couldn't start in the run's box.
#[derive(Debug, Error)]
pub enum BoxStartError {
    /// The binary was built with no snapshot.
    #[error(
        "{} runs in a kernel box, which starts only from a startup snapshot embedded in the \
         binary, and this build has none: started from source, the runtime reads its JS from \
         the build tree, outside the box. Build {BINARY_NAME} with its snapshot: cargo xtask build",
        command_line(.command.as_deref())
    )]
    NoSnapshot {
        /// The command word, none for the listing.
        command: Option<String>,
    },
    /// `-A` after the command word.
    #[error("{} ({})", PlanRefusal::Unconfined, node_gro_command(.cli_args))]
    Unconfined {
        /// The command line, its grant flags split off.
        cli_args: Vec<String>,
    },
    /// A step of the box refused, or failed, before it went up.
    #[error("{}", with_causes(.0))]
    Confine(ConfineError),
    /// A step before the box failed.
    #[error("{}", with_causes(.0))]
    Step(RunStepError),
    /// The OS's temporary directory couldn't be opened for the run's
    /// private directory.
    #[error(
        "the kernel box makes the run's private directory in the OS's temporary directory, and \
         {} can't be opened: {source}",
        escape_controls(&.path.to_string_lossy())
    )]
    TempDir {
        /// Its real path, as the run resolved it.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The OS's temporary directory lies in the project, where the run's
    /// private directory would be an entry its tools write that the box
    /// otherwise keeps read-only, left behind for later commands to read.
    #[error(
        "the kernel box makes the run's private directory in the OS's temporary directory, and \
         {} is inside the project, where the run's tools could write what later commands read: \
         {}",
        escape_controls(&.path.to_string_lossy()),
        .variable.map_or_else(
            || "it's the default, so set TMPDIR outside the project".to_owned(),
            |variable| format!("point {variable} outside the project")
        )
    )]
    TempInProject {
        /// Its real path, as the run resolved it.
        path: PathBuf,
        /// The variable that named it, `TMPDIR`, `TMP`, or `TEMP`, or none
        /// for the default.
        variable: Option<&'static str>,
    },
    /// A rule of the box's plan that a program's box would take is on the
    /// OS's temporary directory or a directory holding it, where a run that
    /// starts a program makes its private directory: Landlock grants beneath
    /// a rule whatever its directory holds, so every program would read all
    /// of the private directory, its siblings' temporary directories among
    /// them.
    #[error("{}", temp_held_message(path, rule, holder, *variable))]
    TempHeldByRule {
        /// The temporary directory's real path, as the run resolved it.
        path: PathBuf,
        /// The rule's absolute path.
        rule: PathBuf,
        /// What put the rule there.
        holder: TempHolder,
        /// The variable that named the temporary directory, `TMPDIR`,
        /// `TMP`, or `TEMP`, or none for the default.
        variable: Option<&'static str>,
    },
    /// Where a symlinked `node_modules` leads couldn't be looked at, for the
    /// realm's read root there.
    #[error(
        "the kernel box follows the project's node_modules to {}, and it can't be looked at: \
         {source}",
        escape_controls(&.path.to_string_lossy())
    )]
    LinkedNodeModules {
        /// Where it leads.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The disk's facts the box's plan reads couldn't be read.
    #[error(
        "the kernel box reads the project at {} to plan its rules, and it can't be read: {source}",
        escape_controls(&.root.to_string_lossy())
    )]
    Facts {
        /// The project's root.
        root: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// The box's plan refuses the run.
    #[error("{}", escape_controls(&refusal.message(Some(&command_line(command.as_deref())))))]
    Plan {
        /// The refusal.
        refusal: PlanRefusal,
        /// The command word, none for the listing, which a refusal naming
        /// the command that writes the project names.
        command: Option<String>,
    },
    /// The run's private directory couldn't be made.
    #[error("the kernel box's private directory: {}", escape_controls(&with_causes(.0)))]
    PrivateDir(PrivateDirError),
    /// A tool directory couldn't be made.
    #[error("{}", escape_controls(&with_causes(.0)))]
    ToolDir(ToolDirError),
    /// The run's environment couldn't be set.
    #[error("failed to set the run's environment ({name}): {source}")]
    Environment {
        /// The variable.
        name: &'static str,
        /// Why.
        source: std::io::Error,
    },
    /// `SIGCHLD` couldn't be set to its default.
    #[error("failed to set SIGCHLD to its default for the run's children: {0}")]
    ChildSignal(std::io::Error),
    /// A rule of the box couldn't be opened.
    #[error("{}", escape_controls(&with_causes(.0)))]
    Rule(RuleError),
    /// The run's box held no rules for its programs' boxes, though the run
    /// may start a program: Gro's own bug, refused rather than starting a
    /// program in no box of its own.
    #[error(
        "the kernel box holds no rules for the boxes of the programs {BINARY_NAME} may start, \
         which is a bug of Gro's: no program starts outside a box of its own"
    )]
    ProgramRules,
    /// The run's thread couldn't start.
    #[error("failed to start the {BINARY_NAME} thread the run's engine runs on: {0}")]
    Thread(std::io::Error),
}

impl BoxStartError {
    /// The exit code: [`EXIT_USAGE`] for what the caller changes, a socket
    /// on stdio, `-A`, what the plan refuses, a temporary directory in the
    /// project, and every path planted where the box opens a rule or makes
    /// a tool directory (a symlink, something other than a directory, a
    /// path leaving its anchor or that isn't the one opened), whatever
    /// holds it, the project, the command line, or the system's own paths
    /// changing under the run; else [`EXIT_FAILURE`], a step that failed.
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Unconfined { .. }
            | Self::Confine(ConfineError::StdioSocket { .. })
            | Self::Plan { .. }
            | Self::TempInProject { .. }
            | Self::TempHeldByRule { .. }
            | Self::ToolDir(ToolDirError::NotADirectory { .. } | ToolDirError::Symlink { .. })
            | Self::Rule(
                RuleError::Symlink { .. } | RuleError::Escapes { .. } | RuleError::NotReal { .. },
            ) => EXIT_USAGE,
            Self::NoSnapshot { .. }
            | Self::Confine(_)
            | Self::Step(_)
            | Self::TempDir { .. }
            | Self::Facts { .. }
            | Self::LinkedNodeModules { .. }
            | Self::PrivateDir(_)
            | Self::ToolDir(ToolDirError::Make { .. })
            | Self::Environment { .. }
            | Self::ChildSignal(_)
            | Self::Rule(_)
            | Self::ProgramRules
            | Self::Thread(_) => EXIT_FAILURE,
        }
    }
}

/// What put a rule holding the OS's temporary directory in the run's box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TempHolder {
    /// A grant flag that lists directories.
    Flag(DirFlag),
    /// `--allow-run-write=`, for the program named so, as written.
    RunWrite {
        /// The program, as written.
        program: String,
    },
    /// A rule of the box's own.
    Rule,
}

impl TempHolder {
    /// What put the rule on `rule`, an absolute path, from `source`, in
    /// `plan`'s box: for `--allow-run-write=`, the program as the flag
    /// naming that directory spelled it, since two flags may name one
    /// program two ways.
    fn of(source: RuleSource, rule: &Path, plan: &BoxPlan) -> Self {
        match source {
            RuleSource::Flag(flag) => Self::Flag(flag),
            RuleSource::RunWrite(index) => plan
                .run_write(index)
                .and_then(|grant| {
                    grant
                        .dirs
                        .iter()
                        .find(|dir| dir.real == rule)
                        .or_else(|| grant.dirs.first())
                })
                .map_or(Self::Rule, |dir| Self::RunWrite {
                    program: dir.named.clone(),
                }),
            _ => Self::Rule,
        }
    }
}

/// [`BoxStartError::TempHeldByRule`]'s message: the temporary directory
/// `temp`, named by `variable` (none for the default), held by the rule on
/// `rule` that `holder` put there, and the fix, narrowing a flag's rule or
/// moving the temporary directory.
fn temp_held_message(
    temp: &Path,
    rule: &Path,
    holder: &TempHolder,
    variable: Option<&'static str>,
) -> String {
    let temp = escape_controls(&temp.to_string_lossy()).to_string();
    let rule = escape_controls(&rule.to_string_lossy()).to_string();
    let elsewhere = variable.map_or_else(
        || "it's the default, so set TMPDIR elsewhere".to_owned(),
        |variable| format!("point {variable} elsewhere"),
    );
    let start = format!(
        "the kernel box makes the run's private directory in the OS's temporary directory, {temp}"
    );
    match holder {
        TempHolder::Flag(flag) => format!(
            "{start}, and {flag}={rule} holds it, so every program the run starts could read the \
             others' temporary directories there: narrow {flag} to a directory that doesn't hold \
             {temp}, or {elsewhere}"
        ),
        TempHolder::RunWrite { program } => {
            let program = escape_controls(program);
            format!(
                "{start}, and --allow-run-write={program}:{rule} holds it, so {program} could read \
                 every other program's temporary directory there: narrow {program}'s directories \
                 to ones that don't hold {temp}, or {elsewhere}"
            )
        }
        TempHolder::Rule => format!(
            "{start}, and the box's rule on {rule}, which every program the run starts reads, \
             holds it, so each could read the others' temporary directories there: {elsewhere}"
        ),
    }
}

/// `error`'s message, then each of its causes', joined by `: `.
fn with_causes(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

/// Runs the command of Gro's the pre-pass planned as `run` in the run's
/// box, in the order the [module doc](self) writes, returning the exit
/// code, or, on a kernel below the box's floor, execs the project's Node
/// Gro over `cli_args`, the command line as written.
pub fn run_boxed(run: VerbRun, cli_args: &[String]) -> u8 {
    let environment = ProcessEnvironment;
    let floor = match probed(&run) {
        Ok(floor) => floor,
        Err(Probed::Refused(error)) => return CliError::from(error).report(),
        Err(Probed::BelowFloor(kernel)) => {
            return match floor_fallback(run, FallbackReason::KernelFloor(kernel)) {
                Ok(fallback) => exec_node_gro(
                    &fallback,
                    cli_args,
                    environment.var("PATH").as_deref(),
                    environment.var(NODE_ENV).as_deref(),
                )
                .report(),
                Err(error) => CliError::Verb(error).report(),
            };
        }
    };
    phases::mark(Phase::Prepass);
    match boxed(&run, floor, &environment) {
        Ok(boxed) => run_on_thread(&run, boxed),
        Err(error) => error.report(),
    }
}

/// What the steps before the first change found wrong.
enum Probed {
    /// The run is refused.
    Refused(BoxStartError),
    /// The kernel's Landlock is below the box's floor.
    BelowFloor(KernelLandlock),
}

/// Steps 1 to 3 of the [module doc](self): nothing in the process changes.
fn probed(run: &VerbRun) -> Result<LandlockFloor, Probed> {
    if STARTUP_SNAPSHOT.is_none() {
        return Err(Probed::Refused(BoxStartError::NoSnapshot {
            command: command_of(&run.cli_args),
        }));
    }
    if matches!(run.grant, RunGrant::All) {
        return Err(Probed::Refused(BoxStartError::Unconfined {
            cli_args: run.cli_args.clone(),
        }));
    }
    let floor = probe_floor().map_err(|error| {
        let kernel = KernelLandlock::from_floor_error(&error);
        kernel.map_or_else(
            || Probed::Refused(BoxStartError::Confine(error)),
            Probed::BelowFloor,
        )
    })?;
    check_stdio().map_err(|error| Probed::Refused(BoxStartError::Confine(error)))?;
    Ok(floor)
}

/// The run, boxed: what its thread takes.
struct Boxed {
    /// The box, up.
    confined: RunConfined,
    /// The run's options, read once the environment was set.
    options: gro_runtime::RuntimeOptions,
    /// The pins of the programs it grants, which the box grants too.
    pins: RunPins,
    /// The rules its programs' boxes are built from, for a run that may
    /// start a program.
    programs: Option<ProgramRules>,
    /// The note it prints when it fails after a tool did ([`walls_note`]).
    walls: String,
}

/// Steps 4 to 13 of the [module doc](self), from the probed `floor`.
///
/// # Errors
///
/// Returns the step's failure that stopped the run, a directory a grant
/// flag lists that isn't one ([`CliError::NoDir`]) among them.
fn boxed(
    run: &VerbRun,
    floor: LandlockFloor,
    environment: &ProcessEnvironment,
) -> Result<Boxed, CliError> {
    let closed = RunClosed::close_inherited(floor).map_err(BoxStartError::Step)?;
    phases::open();
    let networked = closed.enter_network().map_err(BoxStartError::Step)?;
    let hardened = networked.harden().map_err(BoxStartError::Step)?;
    phases::mark(Phase::Network);
    // read before the environment points it into the private directory
    let (temp_path, temp_variable) = temp_dir(environment);
    let temp_path = real_temp_dir(&run.root.join(temp_path));
    // the project's own tree, where no private directory goes and no
    // sweep reaches
    let in_project = lies_in(&temp_path, &run.root);
    let temp = gro_sys::beneath::open_root(&temp_path);
    // a priced build can leave the sweep out (`gro_runtime::prices`)
    if let Ok(temp) = &temp
        && !in_project
        && !prices::off(RunDefault::Sweep)
    {
        // availability alone: a sweep that fails leaves the leftovers
        let _ = gro_sys::private_dir::sweep_private_dirs(temp.as_fd());
    }
    phases::mark(Phase::Sweep);
    let spec = verb_spec(run);
    let mut granted = granted_dirs(&run.dirs, &run.root, false)?;
    for note in &granted.notes {
        stderr_line(note);
    }
    // the closure's `.` and a write flag naming the root, by any spelling
    let realm_writes_root = granted
        .resolved
        .iter()
        .any(|dir| dir.flag == DirFlag::Write && dir.real == run.root);
    let path = environment
        .var("PATH")
        .map(|path| path.to_string_lossy().into_owned());
    let pins = RunPins::pin(
        &authority(&spec, granted.dirs.clone()),
        path.as_deref(),
        &run.root,
    );
    let inputs = box_inputs(
        run,
        &granted.resolved,
        &granted.run_writes,
        &pins,
        environment,
    );
    let facts = BoxFacts::read(&inputs).map_err(|source| BoxStartError::Facts {
        root: run.root.clone(),
        source,
    })?;
    phases::mark(Phase::Facts);
    let plan = BoxPlan::for_run(&inputs, &facts).map_err(|refusal| BoxStartError::Plan {
        refusal,
        command: command_of(&run.cli_args),
    })?;
    phases::mark(Phase::Plan);
    if let Some(note) = plan.note() {
        stderr_line(escape_controls(&note));
    }
    if let Some(linked) = plan.node_modules_link() {
        let read = linked_read_root(linked).map_err(|source| BoxStartError::LinkedNodeModules {
            path: linked.to_owned(),
            source,
        })?;
        granted
            .dirs
            .fs
            .read
            // granted as the link's own path, which a path through it
            // names, its directory the link's target
            .entry(run.root.join("node_modules"))
            .or_insert(read);
    }
    let private = if plan.private_temp() {
        if in_project {
            return Err(BoxStartError::TempInProject {
                path: temp_path,
                variable: temp_variable,
            }
            .into());
        }
        if plan.spawns()
            && let Some((rule, from)) = plan.holding(&temp_path)
        {
            let holder = TempHolder::of(from, &rule, &plan);
            return Err(BoxStartError::TempHeldByRule {
                path: temp_path,
                rule,
                holder,
                variable: temp_variable,
            }
            .into());
        }
        let temp = temp.map_err(|source| BoxStartError::TempDir {
            path: temp_path.clone(),
            source,
        })?;
        let private = hardened
            .make_private_dir(temp.as_fd(), &temp_path)
            .map_err(BoxStartError::PrivateDir)?;
        Some(private)
    } else {
        None
    };
    hardened
        .make_tool_dirs(&plan)
        .map_err(BoxStartError::ToolDir)?;
    set_environment(private.as_ref())?;
    gro_sys::child::default_child_signal().map_err(BoxStartError::ChildSignal)?;
    let mut options = runtime_options(&spec, environment, granted.dirs, &run.root);
    let mut confined = hardened
        .prepare(&plan, private)
        .map_err(BoxStartError::Rule)?
        .confine()
        .map_err(BoxStartError::Confine)?;
    phases::mark(Phase::Confine);
    let programs = confined.take_program_rules();
    if plan.spawns() && programs.is_none() {
        return Err(BoxStartError::ProgramRules.into());
    }
    options.denied_write_cause = Some(denied_write_cause(
        run,
        &wall_rules(&confined),
        realm_writes_root,
    ));
    if let RunNetwork::InetRefused { cause, .. } = confined.network() {
        stderr_line(format_args!(
            "note: no network namespace here ({cause}), so the box refuses every internet socket \
             in its place: tests that bind 127.0.0.1 fail"
        ));
    }
    let walls = walls_note(
        run,
        &wall_rules(&confined),
        plan.run_writes(),
        confined.network(),
    );
    Ok(Boxed {
        confined,
        options,
        pins,
        programs,
        walls,
    })
}

/// The realm's read root on `linked`, the real path the box followed a
/// symlinked `node_modules` to, from that one resolution: the directory
/// there, by its identity now, granted as the project's `node_modules`, so
/// reads through it that the box lets through (`fs.real_path`,
/// `fs.open_dir`, Gro's own checks of what a sync made) aren't refused as a
/// symlink leaving the project's root. Read only: a write there is a write
/// flag's.
///
/// Kind: the realm's native fs boundary, widened to what the kernel box
/// already lets the run read (adversary: a committed `node_modules ->
/// ../victim/node_modules`; asset: that checkout), so it grants code in
/// the run nothing the box doesn't.
///
/// # Errors
///
/// Returns why the directory can't be looked at.
fn linked_read_root(linked: &Path) -> std::io::Result<GrantedDir> {
    let metadata = std::fs::metadata(linked)?;
    Ok(GrantedDir {
        real: linked.to_owned(),
        id: FileId::of(&metadata),
    })
}

/// Whether `path`, a real path, or the directory it names lies in `root`:
/// by its text, or by device and inode up its ancestors, so a bind mount
/// of the project, or a case-folding directory's other spelling, names it
/// too.
fn lies_in(path: &Path, root: &Path) -> bool {
    let id = |dir: &Path| {
        std::fs::metadata(dir)
            .ok()
            .map(|metadata| (metadata.dev(), metadata.ino()))
    };
    path.starts_with(root)
        || id(root).is_some_and(|root| path.ancestors().any(|dir| id(dir) == Some(root)))
}

/// The run's environment set, as the [module doc](self) says: each of
/// [`REDIRECTED`] pointed into `private`, when the run has a private
/// directory, and each of [`REMOVED`] removed.
fn set_environment(private: Option<&RunPrivateDir>) -> Result<(), BoxStartError> {
    let failed = |name| move |source| BoxStartError::Environment { name, source };
    if let Some(private) = private {
        for (name, beneath, redirect) in REDIRECTED {
            if redirect == Redirect::WhereSet && std::env::var_os(name).is_none() {
                continue;
            }
            let path = beneath.map_or_else(
                || private.path().to_owned(),
                |beneath| private.path().join(beneath),
            );
            gro_sys::set_env(name, path.as_os_str()).map_err(failed(name))?;
        }
    }
    for name in REMOVED {
        gro_sys::remove_env(name).map_err(failed(name))?;
    }
    Ok(())
}

/// The box's inputs for `run`: the pre-pass's plan, `resolved`, the
/// directories the run's flags resolved to, of which the grant flags' own
/// widen the box, `run_writes`, those `--allow-run-write=` lists, each
/// joined to the program the pre-pass resolved its name to, `pins`, the
/// programs the run pinned, and the variables git's home config is found
/// by, read from `environment`. `programs` names each file once.
pub fn box_inputs(
    run: &VerbRun,
    resolved: &[GrantedFlagDir],
    run_writes: &[RunWriteDir],
    pins: &RunPins,
    environment: &impl Environment,
) -> BoxInputs {
    let listed = |dir: &GrantedFlagDir| flag_list(&run.flag_dirs, dir.flag).contains(&dir.dir);
    let path = |name: &str| environment.var(name).map(PathBuf::from);
    BoxInputs {
        root: run.root.clone(),
        unconfined: matches!(run.grant, RunGrant::All),
        writes_root: run.writes_root,
        tools: run.tools.clone(),
        spawns: grants_programs(&run.grant, &run.dirs),
        declared: run.declared.clone(),
        programs: pins.files(),
        flags: resolved.iter().filter(|dir| listed(dir)).cloned().collect(),
        run_writes: run_write_grants(run, run_writes),
        temp: matches!(run.grant, RunGrant::Named { temp: true, .. }),
        home: path("HOME"),
        xdg_config_home: path("XDG_CONFIG_HOME"),
        git_config_global: path("GIT_CONFIG_GLOBAL"),
    }
}

/// `dirs`, the directories `--allow-run-write=` lists, by the program the
/// pre-pass resolved each flag's name to (`VerbRun::run_writes`), in the
/// order of those programs, a name it didn't resolve, which no run reaches,
/// left out.
fn run_write_grants(run: &VerbRun, dirs: &[RunWriteDir]) -> Vec<RunWriteGrant> {
    let mut grants: Vec<RunWriteGrant> = Vec::new();
    for dir in dirs {
        let Some(program) = run.run_writes.get(&dir.named) else {
            continue;
        };
        match grants.iter_mut().find(|grant| grant.program == *program) {
            Some(grant) => grant.dirs.push(dir.clone()),
            None => grants.push(RunWriteGrant {
                program: program.clone(),
                table: run.programs.contains_key(program),
                dirs: vec![dir.clone()],
            }),
        }
    }
    grants
}

/// The directories `flags` lists for `flag`.
const fn flag_list(flags: &FlagDirs, flag: DirFlag) -> &BTreeSet<String> {
    match flag {
        DirFlag::Read => &flags.read,
        DirFlag::Write => &flags.write,
        DirFlag::RunIn => &flags.run_in,
    }
}

/// A rule of the run's box as its messages read it: its path, why it's
/// there, and whether it writes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WallRule {
    /// Its absolute path when it opened.
    path: PathBuf,
    /// Why it's there.
    source: RuleSource,
    /// Whether it grants writing.
    writes: bool,
}

/// `confined`'s rules, as its messages read them.
fn wall_rules(confined: &RunConfined) -> Vec<WallRule> {
    confined
        .rules()
        .iter()
        .map(|rule| WallRule {
            path: rule.path().to_owned(),
            source: rule.source(),
            writes: rule.rights().writes(),
        })
        .collect()
}

/// `items` joined for prose: `a`, `a and b`, `a, b, and c`.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// `path` as a message names it: beneath `root` where it's there, else
/// whole, escaped.
fn shown(root: &Path, path: &Path) -> String {
    let shown = path.strip_prefix(root).unwrap_or(path);
    escape_controls(&shown.to_string_lossy()).into_owned()
}

/// The top-level entries the box writes, summarized.
const ENTRIES: &str = "the top-level entries the project had as it started (but .git, \
                       node_modules, the dot-entries, symlinks)";

/// Where `rules`, a box's for the project at `root`, let gro's own code
/// write, through the fs host module, as a clause: `write only to <a>, <b>,
/// and <c>`, or `write nothing`. The top-level entries the box writes are
/// summarized, counted only when gro's code writes the root, `writes_root`;
/// the tool and flag directories are named, then the run's private
/// directory; what the box grants programs alone is left out.
fn writes_clause(root: &Path, rules: &[WallRule], writes_root: bool) -> String {
    let writing = || rules.iter().filter(|rule| rule.writes);
    let mut named = Vec::new();
    if writes_root && writing().any(|rule| rule.source == RuleSource::Entry) {
        named.push(ENTRIES.to_owned());
    }
    named.extend(
        writing()
            .filter(|rule| matches!(rule.source, RuleSource::ToolDir(_) | RuleSource::Flag(_)))
            .map(|rule| shown(root, &rule.path)),
    );
    if writing().any(|rule| rule.source == RuleSource::PrivateTemp) {
        named.push("a temporary directory of the run's".to_owned());
    }
    if named.is_empty() {
        "write nothing".to_owned()
    } else {
        format!("write only to {}", listed(&named))
    }
}

/// Where `rules`, a box's for the project at `root`, let the tools write,
/// each in a box of its own, as a clause: a temporary directory of each
/// one's own, the tool directories for the grants table's tools, and what
/// `--allow-run-write=` names for each program it names, `run_writes`, by
/// the name the flag gives it, the top-level entries summarized for one
/// that names the root. What `--allow-write=` names is left out, as no
/// tool writes it.
fn tools_writes_clause(root: &Path, rules: &[WallRule], run_writes: &[RunWriteGrant]) -> String {
    let writing = || rules.iter().filter(|rule| rule.writes);
    let mut named = Vec::new();
    if writing().any(|rule| rule.source == RuleSource::PrivateTemp) {
        named.push("a temporary directory of their own".to_owned());
    }
    let tool_dirs: Vec<String> = writing()
        .filter(|rule| matches!(rule.source, RuleSource::ToolDir(_)))
        .map(|rule| shown(root, &rule.path))
        .collect();
    if !tool_dirs.is_empty() {
        named.push(format!(
            "the grants table's tools also to {}",
            listed(&tool_dirs)
        ));
    }
    for (index, grant) in run_writes.iter().enumerate() {
        let mut dirs = Vec::new();
        if grant.dirs.iter().any(|dir| dir.real == root)
            && writing().any(|rule| rule.source == RuleSource::Entry)
        {
            dirs.push(ENTRIES.to_owned());
        }
        dirs.extend(
            writing()
                .filter(|rule| rule.source == RuleSource::RunWrite(index))
                .map(|rule| shown(root, &rule.path)),
        );
        if let (Some(dir), false) = (grant.dirs.first(), dirs.is_empty()) {
            named.push(format!(
                "{} also to {}",
                escape_controls(&dir.named),
                listed(&dirs)
            ));
        }
    }
    if named.is_empty() {
        "write nothing".to_owned()
    } else {
        format!("write only to {}", listed(&named))
    }
}

/// What stays read-only to gro's own code in the project at `root` under
/// `rules`, though a run's grants may write it, as a clause: `.git` but
/// where a write rule of the realm's covers it, `node_modules` (but its
/// tool directories, where any is there) but where such a rule covers it,
/// the other dot-entries (than those named writable), and a new top-level
/// entry. A rule `--allow-run-write=` makes is a program's, never the
/// realm's, so it covers nothing here.
fn read_only_clause(root: &Path, rules: &[WallRule]) -> String {
    // a write rule covering `path`: on it or on a directory holding it
    let covered = |path: &Path| {
        rules
            .iter()
            .filter(|rule| {
                rule.writes
                    && !matches!(
                        rule.source,
                        RuleSource::ToolDir(_) | RuleSource::RunWrite(_)
                    )
            })
            .any(|rule| path.starts_with(&rule.path))
    };
    let node_modules: Vec<PathBuf> = std::iter::once(root.join("node_modules"))
        .chain(
            rules
                .iter()
                .filter(|rule| rule.source == RuleSource::NodeModules && !rule.writes)
                .map(|rule| rule.path.clone()),
        )
        .collect();
    let mut kept = Vec::new();
    if !covered(&root.join(".git")) {
        kept.push(".git".to_owned());
    }
    if !node_modules.iter().any(|path| covered(path)) {
        let tools = rules
            .iter()
            .any(|rule| matches!(rule.source, RuleSource::ToolDir(dir) if dir.in_node_modules()));
        kept.push(if tools {
            "node_modules but its tool directories".to_owned()
        } else {
            "node_modules".to_owned()
        });
    }
    kept.push("the other dot-entries".to_owned());
    kept.push("a new top-level entry".to_owned());
    format!("{} stay read-only", listed(&kept))
}

/// What a write through the fs host module that the kernel refuses gains,
/// in `run`, boxed by `rules` ([`gro_runtime::DeniedWriteCause`]), whose
/// own code writes the project's root when `writes_root` says so: where
/// the box lets gro's own code write and what stays read-only, added only
/// to the refusal of a path no write rule holds, and those rules' paths,
/// every rule that writes among them, a program's too, since the kernel
/// lets the run write beneath each.
///
/// Kind: a review signal; the box is the wall.
fn denied_write_cause(run: &VerbRun, rules: &[WallRule], writes_root: bool) -> DeniedWriteCause {
    let cause = format!(
        "the run's kernel box lets {} {}; {}",
        command_line(command_of(&run.cli_args).as_deref()),
        writes_clause(&run.root, rules, writes_root),
        read_only_clause(&run.root, rules)
    );
    let writable = rules
        .iter()
        .filter(|rule| rule.writes)
        .map(|rule| rule.path.clone())
        .collect();
    DeniedWriteCause { cause, writable }
}

/// The note a run boxed by `rules`, in `network`, prints when it failed and
/// a tool it started ended other than by exiting 0, by no signal of the
/// run's, which may be a wall the tool met: what each tool's box, nested in
/// the run's, lets it reach and write, `run_writes` the directories
/// `--allow-run-write=` names for each program, and which flag widens what,
/// naming the programs the command may start, and leaving
/// `--allow-run-write=` out where it may start none by a name.
///
/// Kind: a review signal the run's code can print as well, and set off by
/// starting a program that fails.
fn walls_note(
    run: &VerbRun,
    rules: &[WallRule],
    run_writes: &[RunWriteGrant],
    network: &RunNetwork,
) -> String {
    let network = match network {
        RunNetwork::Namespace => "reach no network but the run's own loopback",
        RunNetwork::InetRefused { .. } => "reach no network",
    };
    let names: Vec<String> = run
        .may_start
        .iter()
        .map(|name| escape_controls(name).into_owned())
        .collect();
    // a program found beneath an exec root has no name the flag takes, so
    // a run that may start no other names none
    let run_write = match names.as_slice() {
        [] => String::new(),
        [one] => {
            format!("--allow-run-write=<program>:<dirs> lets one of them ({one}) write more, ")
        }
        [first, second] => format!(
            "--allow-run-write=<program>:<dirs> lets one of them ({first} or {second}) write more, "
        ),
        [rest @ .., last] => format!(
            "--allow-run-write=<program>:<dirs> lets one of them ({}, or {last}) write more, ",
            rest.join(", ")
        ),
    };
    format!(
        "note: {}'s tools each ran in a kernel box of their own inside the run's, which lets them \
         {network} and {}: a tool that hits a wall fails with EACCES, EPERM, or ENOSYS; \
         {run_write}--allow-read= and --allow-run-in= widen every box, and --allow-write= widens \
         only what gro's own code writes",
        command_line(command_of(&run.cli_args).as_deref()),
        tools_writes_clause(&run.root, rules, run_writes)
    )
}

/// Step 14 of the [module doc](self): the run's thread, started once the
/// box is up, which runs the generated entry on the engine the box made,
/// returning the exit code.
fn run_on_thread(run: &VerbRun, boxed: Boxed) -> u8 {
    let Boxed {
        confined,
        options,
        pins,
        programs,
        walls,
    } = boxed;
    let entry = GeneratedEntry {
        source: run.entry.clone(),
        root: run.root.clone(),
    };
    let cwd = run.root.clone();
    let node_hint = format!(
        "hint: a node: specifier was refused in this run; the Node gro runs it, unconfined: {}",
        node_gro_command(&run.cli_args)
    );
    let ran = on_sized_thread(BINARY_NAME.to_owned(), move || {
        let started = init_run_engine(confined, &EngineOptions::default())
            .map_err(CliError::Engine)
            .map(|engine| {
                phases::mark(Phase::Engine);
                gro_runtime::run_generated(engine, &options, &entry, pins, programs, &cwd)
            });
        finish(started, Some(&node_hint), Some(&walls))
    });
    match ran {
        Joined::Returned(code) => code,
        Joined::Unstarted(error) => CliError::from(BoxStartError::Thread(error)).report(),
        Joined::Panicked => EXIT_PANIC,
    }
}

#[cfg(test)]
mod tests;
