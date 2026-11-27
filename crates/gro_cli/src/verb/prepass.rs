//! The verb's pre-pass: what a command line of Gro's does on the binary,
//! decided before any thread starts and before any JS runs.
//!
//! [`plan`] reads the command line, the values the caller read from the
//! process ([`VerbInputs`]), and the project at the working directory
//! through [`ProjectReads`], and decides, in this order:
//!
//! 1. `GRO_FALLBACK`'s policy ([`FallbackPolicy`]), refused before anything
//!    else when it names none;
//! 2. the grant flags split off ([`split_grant_flags`]);
//! 3. the project's root, the working directory, which must hold a
//!    `package.json` that isn't a type marker;
//! 4. the command's closure (`gro_runtime::verb::closure`), and why it runs
//!    on the Node Gro, if it does ([`fallback_reasons`]), with the project's
//!    installed Gro: a fallback the policy refuses is refused, one with no
//!    installed Gro to run is too, and any other is the plan
//!    ([`VerbPlan::Fallback`]);
//! 5. else a forwarded section that turns off the check of a builtin an
//!    invoke marked `check` runs is refused
//!    ([`VerbError::CheckTurnedOff`]), since the closure holds that builtin
//!    to its reads where Node runs it writing;
//! 6. else the run's grants, the closure's unioned with the flags', each
//!    flag's list checked as `fgro run` checks its own ([`to_run_grant`]),
//!    each closure program granted by the table's policy under the name
//!    Gro's TS asks for it by ([`table_programs`]), each program
//!    `--allow-run-write=` names resolved to one the run grants
//!    ([`resolve_run_writes`]), and the entry's text ([`main_entry`]):
//!    [`VerbPlan::Run`].
//!
//! A flag's list is checked only for a run: a fallback passes the command
//! line to the Node Gro as written, which takes the flags and warns.
//!
//! Kinds: the grants a run gets are a boundary, held natively from the
//! run's first module, computed before any project code runs (the closure's
//! module doc names the adversary and the asset), whose bound is their
//! union with the flags, which the command line chose; the root check and
//! the refusal of a table [`table_programs`] can't read are guardrails
//! against a mistake and against gro's own drift, and so is the refusal of a
//! forwarded section turning a check off, which keeps the command from
//! failing at its first write where Node writes (the closure's reads bound
//! it either way); `GRO_FALLBACK`'s policy is
//! [`FallbackPolicy`]'s.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gro_runtime::confine::plan::{ToolDir, ToolWrites};
use gro_runtime::resolver::{GRO_PACKAGE, InstalledPackage, ProjectPackage, ResolveErrorKind};
use gro_runtime::verb::closure::{
    ClosureFallback, CommandClosure, TASK_VERB, command_closure, command_word, is_task_path_input,
};
use gro_runtime::verb::table::{BuiltinGrantsTable, BuiltinProgramGrant, PassEnv};
use gro_runtime::{BINARY_NAME, EnvPattern, PassEnvGrant, ProgramGrant};
use thiserror::Error;

use super::entry::main_entry;
use super::fallback::{FallbackPolicy, FallbackPolicyError, FallbackReason, fallback_reasons};
use super::grant_flags::{GrantFlagsError, split_grant_flags, to_run_grant};
use crate::cli::{FlagDirs, RunGrant};
use crate::print::{escape_controls, shell_quote};
use crate::{EXIT_FAILURE, EXIT_USAGE};

/// The directory a project's own installs are found in, which a `local`
/// program of the grants table is named under, as Gro's TS names it
/// (`to_local_cli_path`).
const LOCAL_BIN_DIR: &str = "node_modules/.bin";

/// The `NODE_ENV` value under which a run doesn't match the `development`
/// condition, as the Node Gro's loader decides it.
const NODE_ENV_PRODUCTION: &str = "production";

/// What [`plan`] takes from the process, read once by the caller.
#[derive(Debug, Clone, Copy)]
pub struct VerbInputs<'a> {
    /// The args after the binary's own path, the command word first.
    pub cli_args: &'a [String],
    /// `GRO_FALLBACK`, if it's set.
    pub fallback: Option<&'a OsStr>,
    /// `NODE_ENV`, if it's set.
    pub node_env: Option<&'a OsStr>,
    /// `HOME`, if it's set.
    pub home: Option<&'a OsStr>,
    /// The binary's version, which the project's installed Gro must be to
    /// run on the binary.
    pub binary_version: &'a str,
    /// The working directory, a real path: the project's root.
    pub root: &'a Path,
}

/// The project's files [`plan`] reads, the root's `package.json` and the
/// Gro it installed, from the disk or a test's.
pub trait ProjectReads {
    /// The root's `package.json`, `None` when there's none.
    ///
    /// # Errors
    ///
    /// Returns why it can't be read, as resolution names it.
    fn package(&self) -> Result<Option<ProjectPackage>, ResolveErrorKind>;

    /// The Gro the project installed, `None` when it installed none.
    ///
    /// # Errors
    ///
    /// Returns why it can't be read, as resolution names it.
    fn installed_gro(&self) -> Result<Option<InstalledPackage>, ResolveErrorKind>;
}

/// What a command line does on the binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerbPlan {
    /// It runs on the binary.
    Run(Box<VerbRun>),
    /// It runs on the Node Gro the project installed.
    Fallback(VerbFallback),
}

/// A command's run on the binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbRun {
    /// The command line without its grant flags, the args the run's args
    /// host module hands Gro.
    pub cli_args: Vec<String>,
    /// The grant flags, as written, in order, for a message that names
    /// the whole command again.
    pub grant_flags: Vec<String>,
    /// The project's root, the working directory, a real path: the
    /// entry's root.
    pub root: PathBuf,
    /// What the run grants by the flags' shape: the closure's variables and
    /// programs joined to the flags' names, `-A` unconfined.
    pub grant: RunGrant,
    /// The directories it reads, writes, and finds programs beneath: the
    /// closure's, relative to the root, joined to the flags', relative to
    /// the working directory, which is the root.
    pub dirs: FlagDirs,
    /// The directories the grant flags list, apart from the closure's:
    /// what widens the run's box past its plan's own rules.
    pub flag_dirs: FlagDirs,
    /// Whether the closure writes the project's root, its check mode's
    /// writes dropped.
    pub writes_root: bool,
    /// The programs of the grants table the closure grants that write tool
    /// directories, by the name the run grants each as
    /// (`node_modules/.bin/vitest`), each with its table name and its
    /// `write`.
    pub tools: BTreeMap<String, ToolWrites>,
    /// The packages the project's `package.json` declares in
    /// `dependencies` or `devDependencies`, by name.
    pub declared: BTreeSet<String>,
    /// Which fallbacks `GRO_FALLBACK` takes, for one the host decides once
    /// the run's box is probed ([`floor_fallback`]).
    pub policy: FallbackPolicy,
    /// The Gro the project installed, at the binary's version, if it
    /// installed one: what runs in the binary's place on a host that can't
    /// hold the box.
    pub gro: Option<InstalledPackage>,
    /// The policy of each program the closure grants, by the name it's
    /// granted as; a program only a flag grants is in none.
    pub programs: BTreeMap<String, ProgramGrant>,
    /// Each program `--allow-run-write=` names, as written, and the name
    /// the run grants it as ([`resolve_run_writes`]).
    pub run_writes: BTreeMap<String, String>,
    /// The programs the command may start, each by its plain name, as
    /// `--allow-run-write=` takes it ([`may_start`]).
    pub may_start: BTreeSet<String>,
    /// Whether the run's packages match the `development` condition.
    pub development: bool,
    /// The entry's text ([`main_entry`]).
    pub entry: String,
}

/// A command's run on the Node Gro the project installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbFallback {
    /// The command word, none for the listing.
    pub command: Option<String>,
    /// Why, each once, in the order [`fallback_reasons`] gives.
    pub reasons: Vec<FallbackReason>,
    /// The installed Gro.
    pub gro: InstalledPackage,
}

/// Why a command line can't run, on the binary or the Node Gro, before
/// anything runs.
#[derive(Debug, Error)]
pub enum VerbError {
    /// `GRO_FALLBACK` names no policy.
    #[error(transparent)]
    FallbackPolicy(#[from] FallbackPolicyError),
    /// The grant flags are refused.
    #[error(transparent)]
    GrantFlags(#[from] GrantFlagsError),
    /// The working directory holds no `package.json`.
    #[error(
        "{} runs the project at the working directory, and {} holds no package.json{}",
        command_line(.command.as_deref()),
        escape_controls(&.root.to_string_lossy()),
        help_pointer(.command.as_deref())
    )]
    NoPackageJson {
        /// The command word, none for the listing.
        command: Option<String>,
        /// The working directory.
        root: PathBuf,
    },
    /// The working directory's `package.json` only marks a module type.
    #[error(
        "{} runs the project at the working directory, and {} only marks a module type, as no \
         project's package.json does",
        command_line(.command.as_deref()),
        escape_controls(&.path.to_string_lossy())
    )]
    TypeMarker {
        /// The command word, none for the listing.
        command: Option<String>,
        /// The `package.json`.
        path: PathBuf,
    },
    /// The working directory's `package.json`, or the installed Gro's,
    /// can't be read.
    #[error("{} can't read the project: {}", command_line(.command.as_deref()), escape_controls(&.kind.to_string()))]
    Project {
        /// The command word, none for the listing.
        command: Option<String>,
        /// Why, as resolution names it.
        kind: ResolveErrorKind,
    },
    /// `GRO_FALLBACK=command` refuses a fallback the project's files
    /// decide.
    #[error(
        "GRO_FALLBACK=command refuses running the project's own gro, which its files pick: {}. \
         Install {GRO_PACKAGE}@{binary} to run this one, or run {} for the project's, \
         unconfined",
        joined_reasons(.reasons),
        node_gro_command(.cli_args)
    )]
    Strict {
        /// The command line, its grant flags split off.
        cli_args: Vec<String>,
        /// Why it would run on the Node Gro.
        reasons: Vec<FallbackReason>,
        /// The binary's version.
        binary: String,
    },
    /// `GRO_FALLBACK=command` refuses a fallback the host decides.
    #[error(
        "GRO_FALLBACK=command refuses running the project's own gro, which confines nothing, in \
         the kernel box's place: {reason}. Run {} for the project's, unconfined",
        node_gro_command(.cli_args)
    )]
    StrictHost {
        /// The command line, its grant flags split off.
        cli_args: Vec<String>,
        /// Why the box can't go up.
        reason: FallbackReason,
    },
    /// The command runs on the Node Gro, and the project installed none.
    #[error(
        "{} runs on the Node gro, because {}, and this project installs none: npm i -D \
         {GRO_PACKAGE}",
        command_line(.command.as_deref()),
        joined_reasons(.reasons)
    )]
    NoNodeGro {
        /// The command word, none for the listing.
        command: Option<String>,
        /// Why it would run on the Node Gro.
        reasons: Vec<FallbackReason>,
    },
    /// A forwarded section turns off the check of a builtin an invoke
    /// marked `check` runs, a write the command's closure never grants.
    #[error(
        "{} runs {} in check mode, writing no source, and the forwarded {} {} that check off, a \
         write it never grants: run {} for the writes, or {} on the project's Node gro, \
         unconfined",
        command_line(.command.as_deref()),
        and_list(.builtins.iter().cloned()),
        and_list(.builtins.iter().map(|name| format!("`-- gro {name}`"))),
        if .builtins.len() == 1 { "turns" } else { "turn" },
        and_list(.builtins.iter().map(|name| command_line(Some(name)))),
        node_gro_command(.cli_args)
    )]
    CheckTurnedOff {
        /// The command word.
        command: Option<String>,
        /// The builtins whose check a forwarded section turns off, names
        /// of the grants table's.
        builtins: Vec<String>,
        /// The command line, its grant flags split off.
        cli_args: Vec<String>,
    },
    /// `--allow-run-write=` names a program the command may start by no
    /// name, the program echoed, escaped: a program's name, never a value.
    #[error(
        "--allow-run-write=… names {}, and {} may start no program{}",
        escape_controls(.program),
        command_line(.command.as_deref()),
        if .may_start.is_empty() {
            String::new()
        } else {
            format!(" by that name: it may start {}", prose_list(.may_start))
        }
    )]
    RunWriteNoProgram {
        /// The program, as written.
        program: String,
        /// The command word, none for the listing.
        command: Option<String>,
        /// The names the command may start a program by, sorted, each
        /// escaped.
        may_start: Vec<String>,
    },
    /// `--allow-run-write=` names a program by a plain name that is both
    /// the project's own tool's and one `--allow-run=` grants.
    #[error(
        "--allow-run-write=… could name the project's own {0} or the {0} --allow-run= grants: \
         write {LOCAL_BIN_DIR}/{0} for the project's own; the other takes no directory while \
         both are granted",
        escape_controls(.program)
    )]
    RunWriteAmbiguous {
        /// The program, as written.
        program: String,
    },
    /// The grants table can't be read into a run's grants: Gro's own
    /// drift, which its tests hold against.
    #[error("the grants table can't grant `{program}`: {fault}")]
    Table {
        /// The program's name, as granted.
        program: String,
        /// What's wrong.
        fault: TableFault,
    },
}

/// Why the grants table can't grant a program.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TableFault {
    /// Two builtins in one closure grant it by different policies or tool
    /// directories.
    #[error("two builtins in the command's closure grant it by different policies")]
    TwoPolicies,
    /// An `omit_env` beside a `pass_env` that isn't `'all'`, which Gro's TS
    /// types refuse.
    #[error("its omit_env is beside a pass_env that isn't 'all'")]
    OmitBesideList,
    /// A `pass_env` name that names no variable or glob.
    #[error("its pass_env names {0:?}, no variable or PREFIX_* glob")]
    Pattern(String),
}

impl VerbError {
    /// The exit code: [`EXIT_FAILURE`] for a table Gro's own drift made,
    /// else [`EXIT_USAGE`], since the caller must change the command line,
    /// the environment, or the project.
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Table { .. } => EXIT_FAILURE,
            Self::FallbackPolicy(_)
            | Self::GrantFlags(_)
            | Self::NoPackageJson { .. }
            | Self::TypeMarker { .. }
            | Self::Project { .. }
            | Self::Strict { .. }
            | Self::StrictHost { .. }
            | Self::CheckTurnedOff { .. }
            | Self::RunWriteNoProgram { .. }
            | Self::RunWriteAmbiguous { .. }
            | Self::NoNodeGro { .. } => EXIT_USAGE,
        }
    }
}

/// The command that runs the command line `cli_args`, its grant flags split
/// off, on the project's own Node Gro: `node_modules/.bin/gro` and each
/// argument as a shell reads it back ([`shell_quote`]), so a message names
/// the whole command, a check's `--check` included, and never another.
pub fn node_gro_command(cli_args: &[String]) -> String {
    std::iter::once(NODE_GRO.to_owned())
        .chain(cli_args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The command line `cli_args` on the binary with its grant flags
/// `grant_flags` and then `added`, flags already quoted for a shell, right
/// after its command word, every other argument as a shell reads it back
/// ([`shell_quote`]): the whole command a message tells someone to run
/// with flags added, so a paste can't put one where the command reads it
/// as an arg. None with no command word for grant flags to follow.
pub fn binary_command_with(
    cli_args: &[String],
    grant_flags: &[String],
    added: &[String],
) -> Option<String> {
    let (word, rest) = cli_args
        .split_first()
        .filter(|(word, _)| !word.starts_with('-') && !is_task_path_input(word))?;
    Some(
        std::iter::once(BINARY_NAME.to_owned())
            .chain(std::iter::once(shell_quote(word)))
            .chain(grant_flags.iter().map(|flag| shell_quote(flag)))
            .chain(added.iter().cloned())
            .chain(rest.iter().map(|arg| shell_quote(arg)))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The project's own Node Gro, as a command names it.
const NODE_GRO: &str = "node_modules/.bin/gro";

/// What a refusal adds when there's no command word, the listing, which
/// the bare binary runs: a pointer to the binary's own commands.
fn help_pointer(command: Option<&str>) -> String {
    command.map_or_else(
        || format!("; {BINARY_NAME} --help lists the binary's own commands"),
        |_| String::new(),
    )
}

/// The command word of `cli_args`, the command line without its grant
/// flags, as the closure reads it: none for the listing, or for a path,
/// which names no command.
pub fn command_of(cli_args: &[String]) -> Option<String> {
    command_word(cli_args).filter(|word| !is_task_path_input(word))
}

/// `fgro` and the command word, as a message names the command line.
pub fn command_line(command: Option<&str>) -> String {
    format!("{BINARY_NAME}{}", command_suffix(command))
}

/// ` <command>`, escaped, or nothing for the listing.
fn command_suffix(command: Option<&str>) -> String {
    command.map_or_else(String::new, |command| {
        format!(" {}", escape_controls(command))
    })
}

/// `items` joined as a list, `and` before the last.
fn and_list(items: impl Iterator<Item = String>) -> String {
    let items: Vec<String> = items.collect();
    match items.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// `items` joined for prose: `a`, `a and b`, `a, b, and c`.
fn prose_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// `reasons` joined as one clause, `; and ` between them.
pub fn joined_reasons(reasons: &[FallbackReason]) -> String {
    reasons
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; and ")
}

/// What the command line `inputs.cli_args` does on the binary, read from
/// `table`, the project's files read through `reads`.
///
/// # Errors
///
/// Returns why it can't run, in the order the module doc lists.
pub fn plan(
    inputs: &VerbInputs<'_>,
    table: &BuiltinGrantsTable,
    reads: &impl ProjectReads,
) -> Result<VerbPlan, VerbError> {
    let policy = FallbackPolicy::read(inputs.fallback)?;
    let split = split_grant_flags(inputs.cli_args)?;
    let command = command_of(&split.cli_args);
    let project_error = |kind| VerbError::Project {
        command: command.clone(),
        kind,
    };
    let declared = match reads.package().map_err(project_error)? {
        None => {
            return Err(VerbError::NoPackageJson {
                command,
                root: inputs.root.to_owned(),
            });
        }
        Some(package) if package.type_marker => {
            return Err(VerbError::TypeMarker {
                command,
                path: package.path,
            });
        }
        Some(package) => package.declared,
    };
    let installed = reads.installed_gro().map_err(project_error)?;
    let closure = command_closure(table, &split.cli_args);
    let mut reasons = fallback_reasons(&closure, installed.as_ref(), inputs.binary_version);
    if command.as_deref() == Some(TASK_VERB)
        && !closure.fallback.contains(&ClosureFallback::ProjectTask)
    {
        reasons.push(FallbackReason::TaskListing);
        reasons.sort();
    }
    if !reasons.is_empty() {
        if policy.refuses(&reasons) {
            return Err(VerbError::Strict {
                cli_args: split.cli_args,
                reasons,
                binary: inputs.binary_version.to_owned(),
            });
        }
        let Some(gro) = installed else {
            return Err(VerbError::NoNodeGro { command, reasons });
        };
        return Ok(VerbPlan::Fallback(VerbFallback {
            command,
            reasons,
            gro,
        }));
    }
    if !closure.check_turned_off.is_empty() {
        return Err(VerbError::CheckTurnedOff {
            command,
            builtins: closure.check_turned_off.into_iter().collect(),
            cli_args: split.cli_args,
        });
    }
    let (grant, flags) = to_run_grant(&split.grant_flags)?;
    let mut dirs = flags.clone();
    let no_flags = (BTreeSet::new(), BTreeSet::new());
    let (flag_programs, forwards) = match &grant {
        RunGrant::Named { run, run_env, .. } => (run, run_env),
        RunGrant::All => (&no_flags.0, &no_flags.1),
    };
    let (programs, tools) = table_programs(&closure, flag_programs, forwards)?;
    let run_writes = resolve_run_writes(
        &closure,
        flag_programs,
        flags.run_write.keys(),
        command.as_deref(),
    )?;
    let may_start = may_start(&closure, flag_programs);
    let grant = match grant {
        RunGrant::All => RunGrant::All,
        RunGrant::Named {
            mut env,
            temp,
            mut run,
            run_env,
        } => {
            env.extend(closure.grants.env.iter().cloned());
            run.extend(programs.keys().cloned());
            RunGrant::Named {
                env,
                temp,
                run,
                run_env,
            }
        }
    };
    dirs.read.extend(closure.grants.read.iter().cloned());
    dirs.write.extend(closure.grants.write.iter().cloned());
    let home = inputs.home.map(|home| home.to_string_lossy());
    Ok(VerbPlan::Run(Box::new(VerbRun {
        cli_args: split.cli_args,
        grant_flags: split.grant_flags,
        root: inputs.root.to_owned(),
        grant,
        dirs,
        flag_dirs: flags,
        writes_root: closure.grants.write.contains(ROOT_DIR),
        tools,
        declared,
        policy,
        gro: installed,
        programs,
        run_writes,
        may_start,
        development: inputs.node_env != Some(OsStr::new(NODE_ENV_PRODUCTION)),
        entry: main_entry(home.as_deref(), inputs.binary_version),
    })))
}

/// What runs in the binary's place for `run` when the host can't hold the
/// run's box, `reason` saying why: the project's installed Gro, run as a
/// fallback is ([`VerbPlan::Fallback`]).
///
/// # Errors
///
/// Returns [`VerbError::StrictHost`] when `GRO_FALLBACK=command` refuses a
/// fallback the host decides, and [`VerbError::NoNodeGro`] when the
/// project installed no Gro.
pub fn floor_fallback(run: VerbRun, reason: FallbackReason) -> Result<VerbFallback, VerbError> {
    if run.policy.refuses(std::slice::from_ref(&reason)) {
        return Err(VerbError::StrictHost {
            cli_args: run.cli_args,
            reason,
        });
    }
    let command = command_of(&run.cli_args);
    let reasons = vec![reason];
    let Some(gro) = run.gro else {
        return Err(VerbError::NoNodeGro { command, reasons });
    };
    Ok(VerbFallback {
        command,
        reasons,
        gro,
    })
}

/// The project's root as a closure's write set names it.
const ROOT_DIR: &str = ".";

/// The name the run grants each program of `named`, the programs
/// `--allow-run-write=` names as written, as: every name equal to it among
/// those the run grants (the closure's, as [`table_programs`] names them,
/// and `flag_programs`, those `--allow-run=` lists), joined with every
/// local program of `closure` whose table name equals it, so a tool is
/// named as it's called (`vitest`) or by its name in the run
/// (`node_modules/.bin/vitest`). The match is against what the run grants,
/// not what it starts: a program granted and never started is named
/// harmlessly. A program found beneath an exec root has no name as the run
/// starts, and isn't nameable.
///
/// Kind: a guardrail on the trusted command line, so a typo is refused
/// before anything runs rather than surfacing as a tool's `EACCES`.
///
/// # Errors
///
/// Returns [`VerbError::RunWriteNoProgram`] for the first that names no
/// program, listing those the command may start, and
/// [`VerbError::RunWriteAmbiguous`] for one that names two: the project's
/// own tool and one `--allow-run=` grants by the same plain name.
fn resolve_run_writes<'a>(
    closure: &CommandClosure,
    flag_programs: &BTreeSet<String>,
    named: impl Iterator<Item = &'a String>,
    command: Option<&str>,
) -> Result<BTreeMap<String, String>, VerbError> {
    let table: Vec<(String, &str)> = closure
        .grants
        .programs
        .iter()
        .map(|program| {
            let grant = &program.grant;
            let key = program_key(grant);
            (key, grant.name.as_str())
        })
        .collect();
    let mut resolved = BTreeMap::new();
    for program in named {
        let keys: BTreeSet<&str> = table
            .iter()
            .filter(|(key, name)| key == program || name == program)
            .map(|(key, _)| key.as_str())
            .chain(flag_programs.get(program).map(String::as_str))
            .collect();
        let mut keys = keys.into_iter();
        match (keys.next(), keys.next()) {
            (Some(key), None) => {
                resolved.insert(program.clone(), key.to_owned());
            }
            (Some(_), Some(_)) => {
                return Err(VerbError::RunWriteAmbiguous {
                    program: program.clone(),
                });
            }
            (None, _) => {
                return Err(VerbError::RunWriteNoProgram {
                    program: program.clone(),
                    command: command.map(str::to_owned),
                    may_start: may_start(closure, flag_programs)
                        .iter()
                        .map(|name| escape_controls(name).into_owned())
                        .collect(),
                });
            }
        }
    }
    Ok(resolved)
}

/// The programs a command whose closure is `closure` may start, given
/// `flag_programs`, those `--allow-run=` lists: each program of the closure
/// by its table name, as the tool is called, and each the flag lists as it
/// lists it.
fn may_start(closure: &CommandClosure, flag_programs: &BTreeSet<String>) -> BTreeSet<String> {
    closure
        .grants
        .programs
        .iter()
        .map(|program| program.grant.name.clone())
        .chain(flag_programs.iter().cloned())
        .collect()
}

/// What [`table_programs`] reads from a closure: the policy of each
/// program, and the tool directories of each that writes any, both by the
/// name the run grants it as.
type TablePrograms = (BTreeMap<String, ProgramGrant>, BTreeMap<String, ToolWrites>);

/// The name the run grants the table's program `grant` as, which Gro's TS
/// asks for it by: `node_modules/.bin/<name>` for a `local` one, else its
/// bare name.
fn program_key(grant: &BuiltinProgramGrant) -> String {
    if grant.local {
        format!("{LOCAL_BIN_DIR}/{}", grant.name)
    } else {
        grant.name.clone()
    }
}

/// The policy of each program `closure` grants, by the name Gro's TS asks
/// for it by: `node_modules/.bin/<name>` for a `local` one, else its bare
/// name, beside the tool directories each that writes any names, by the
/// same name. A `pass_env` list, or none, becomes the names it lists, joined to
/// `forwards`, what `--allow-run-env=` forwards, when `flag_programs`, the
/// programs `--allow-run=` names, names it too, a flag adding its forwards
/// to a listed program it grants as well, and to no other; `'all'` passes
/// the whole environment but its `omit_env`, the forwards adding nothing;
/// and its `env` values are the only ones it may be set.
///
/// # Errors
///
/// Returns [`VerbError::Table`] for a name two builtins grant by different
/// policies or tool directories, an `omit_env` beside a list, and a list name that names no
/// variable or glob, each a table Gro's TS refuses or its tests hold
/// against.
fn table_programs(
    closure: &CommandClosure,
    flag_programs: &BTreeSet<String>,
    forwards: &BTreeSet<EnvPattern>,
) -> Result<TablePrograms, VerbError> {
    let mut programs: BTreeMap<String, ProgramGrant> = BTreeMap::new();
    let mut writes: BTreeMap<String, &[ToolDir]> = BTreeMap::new();
    for program in &closure.grants.programs {
        let grant = &program.grant;
        let name = program_key(grant);
        let fault = |fault| VerbError::Table {
            program: name.clone(),
            fault,
        };
        let forwards = if flag_programs.contains(&name) {
            forwards.iter().cloned().collect()
        } else {
            BTreeSet::new()
        };
        let pass_env = match (&grant.pass_env, &grant.omit_env) {
            (Some(PassEnv::All(_)), omit_env) => PassEnvGrant::All {
                omit_env: omit_env.iter().flatten().cloned().collect(),
            },
            (_, Some(_)) => return Err(fault(TableFault::OmitBesideList)),
            (Some(PassEnv::Names(names)), None) => {
                let mut patterns = names
                    .iter()
                    .map(|name| {
                        EnvPattern::parse(name)
                            .map_err(|_| fault(TableFault::Pattern(name.clone())))
                    })
                    .collect::<Result<BTreeSet<_>, _>>()?;
                patterns.extend(forwards);
                PassEnvGrant::Names(patterns.into_iter().collect())
            }
            (None, None) => PassEnvGrant::Names(forwards.into_iter().collect()),
        };
        let table = ProgramGrant::Table {
            pass_env,
            env: grant.env.clone().unwrap_or_default(),
        };
        match programs.get(&name) {
            Some(granted)
                if *granted != table || writes.get(&name).copied() != Some(&grant.write) =>
            {
                return Err(fault(TableFault::TwoPolicies));
            }
            Some(_) => {}
            None => {
                writes.insert(name.clone(), &grant.write);
                programs.insert(name, table);
            }
        }
    }
    let tools = closure
        .grants
        .programs
        .iter()
        .filter(|program| !program.grant.write.is_empty())
        .map(|program| {
            let grant = &program.grant;
            let name = program_key(grant);
            let tool = ToolWrites {
                name: grant.name.clone(),
                tool_dirs: grant.write.iter().copied().collect(),
            };
            (name, tool)
        })
        .collect();
    Ok((programs, tools))
}

#[cfg(test)]
mod tests;
