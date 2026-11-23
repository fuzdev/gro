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
//! 5. else the run's grants, the closure's unioned with the flags', each
//!    flag's list checked as `fgro run` checks its own ([`to_run_grant`]),
//!    each closure program granted by the table's policy under the name
//!    Gro's TS asks for it by ([`table_programs`]), and the entry's text
//!    ([`main_entry`]): [`VerbPlan::Run`].
//!
//! A flag's list is checked only for a run: a fallback passes the command
//! line to the Node Gro as written, which takes the flags and warns.
//!
//! Kinds: the grants a run gets are a boundary, held natively from the
//! run's first module, computed before any project code runs (the closure's
//! module doc names the adversary and the asset), whose bound is their
//! union with the flags, which the command line chose; the root check and
//! the refusal of a table [`table_programs`] can't read are guardrails
//! against a mistake and against gro's own drift; `GRO_FALLBACK`'s policy is
//! [`FallbackPolicy`]'s.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gro_runtime::resolver::{GRO_PACKAGE, InstalledPackage, ProjectPackage, ResolveErrorKind};
use gro_runtime::verb::closure::{
    ClosureFallback, CommandClosure, TASK_VERB, command_closure, command_word, is_task_path_input,
};
use gro_runtime::verb::table::{BuiltinGrantsTable, PassEnv};
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
    /// The project's local programs the closure grants, each by its bare
    /// name (`vitest`, `svelte-kit`).
    pub tools: BTreeSet<String>,
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
    /// Two builtins in one closure grant it by different policies.
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
    let (grant, flags) = to_run_grant(&split.grant_flags)?;
    let mut dirs = flags.clone();
    let no_flags = (BTreeSet::new(), BTreeSet::new());
    let (flag_programs, forwards) = match &grant {
        RunGrant::Named { run, run_env, .. } => (run, run_env),
        RunGrant::All => (&no_flags.0, &no_flags.1),
    };
    let programs = table_programs(&closure, flag_programs, forwards)?;
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
    let tools = closure
        .grants
        .programs
        .iter()
        .filter(|program| program.grant.local)
        .map(|program| program.grant.name.clone())
        .collect();
    Ok(VerbPlan::Run(Box::new(VerbRun {
        cli_args: split.cli_args,
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

/// The policy of each program `closure` grants, by the name Gro's TS asks
/// for it by: `node_modules/.bin/<name>` for a `local` one, else its bare
/// name. A `pass_env` list, or none, becomes the names it lists, joined to
/// `forwards`, what `--allow-run-env=` forwards, when `flag_programs`, the
/// programs `--allow-run=` names, names it too, a flag adding its forwards
/// to a listed program it grants as well, and to no other; `'all'` passes
/// the whole environment but its `omit_env`, the forwards adding nothing;
/// and its `env` values are the only ones it may be set.
///
/// # Errors
///
/// Returns [`VerbError::Table`] for a name two builtins grant by different
/// policies, an `omit_env` beside a list, and a list name that names no
/// variable or glob, each a table Gro's TS refuses or its tests hold
/// against.
fn table_programs(
    closure: &CommandClosure,
    flag_programs: &BTreeSet<String>,
    forwards: &BTreeSet<EnvPattern>,
) -> Result<BTreeMap<String, ProgramGrant>, VerbError> {
    let mut programs: BTreeMap<String, ProgramGrant> = BTreeMap::new();
    for program in &closure.grants.programs {
        let grant = &program.grant;
        let name = if grant.local {
            format!("{LOCAL_BIN_DIR}/{}", grant.name)
        } else {
            grant.name.clone()
        };
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
            Some(granted) if *granted != table => return Err(fault(TableFault::TwoPolicies)),
            Some(_) => {}
            None => {
                programs.insert(name, table);
            }
        }
    }
    Ok(programs)
}

#[cfg(test)]
mod tests;
