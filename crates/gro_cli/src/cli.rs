//! The command line's shape: the argh types for `fgro`'s flags and
//! subcommands, `run`'s arguments, which argh routes but never parses, the
//! internal `make-snapshot` and `make-embed` names argh never sees, every
//! other command line a command of gro's, which argh never sees either, and
//! [`parse`], which reads them once into an [`Invocation`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use argh::{CommandInfo, EarlyExit, FromArgs, SubCommand};
use gro_runtime::confine::Jit;
use gro_runtime::confine::plan::DirFlag;
use gro_runtime::{BINARY_NAME, EnvNameFault, EnvPattern, EnvPatternFault, env_name_fault};
use thiserror::Error;

use crate::error::CliError;
use crate::print::{echo_argument, escape_controls};

/// The internal subcommand that writes the snapshot, handled before argh so
/// it stays out of `--help`.
pub const MAKE_SNAPSHOT: &str = "make-snapshot";

/// The internal subcommand that writes the embed, handled before argh so it
/// stays out of `--help`.
pub const MAKE_EMBED: &str = "make-embed";

/// gro on its own JS runtime (experimental)
#[derive(FromArgs)]
#[argh(note = "{command_name} <command> runs a command of gro's, which {command_name} alone lists")]
pub struct Args {
    /// print the version, the git commit it was built from, the V8 version,
    /// and whether a startup snapshot is embedded
    #[argh(switch)]
    pub version: bool,
    #[argh(subcommand)]
    pub command: Option<Command>,
}

#[derive(FromArgs)]
#[argh(subcommand)]
pub enum Command {
    Run(RunArgs),
    Eval(EvalArgs),
}

/// `fgro run`'s arguments, every one after `run` as written, which argh
/// routes here without parsing them: an argument starting with `-` that
/// follows the file is the script's, where argh would read it as an option of
/// fgro's. [`RunInvocation::check`] parses them.
pub struct RunArgs {
    args: Vec<String>,
}

impl FromArgs for RunArgs {
    /// Takes `args` as they are, argh's help trigger included when `help` or
    /// `--help` came before `run` (`fgro help run`): argh puts `help` first.
    fn from_args(_command_name: &[&str], args: &[&str]) -> Result<Self, EarlyExit> {
        Ok(Self {
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
        })
    }
}

impl SubCommand for RunArgs {
    const COMMAND: &'static CommandInfo = &CommandInfo {
        name: RUN,
        short: &'\0',
        description: RUN_DESCRIPTION,
    };
}

/// `run`'s name.
pub const RUN: &str = "run";

/// `run`'s description, in `fgro --help`'s list of commands and at the top of
/// its own help.
const RUN_DESCRIPTION: &str = "run an ES module (.js, .mjs, .ts, or .mts) from disk";

/// The arguments before the file that ask for `run`'s help: argh's help
/// triggers, the same for every command.
const HELP_TRIGGERS: [&str; 2] = ["--help", "help"];

/// The first arguments the binary keeps for itself: its commands, its
/// version and help, and the internal subcommands. Every other first
/// argument, a flag, a path, or `--` among them, and none at all, is a
/// command of gro's ([`Invocation::Verb`]).
const BINARY_WORDS: [&str; 7] = [
    RUN,
    "eval",
    "--version",
    HELP_TRIGGERS[0],
    HELP_TRIGGERS[1],
    MAKE_SNAPSHOT,
    MAKE_EMBED,
];

/// The option that grants the env host module the names it lists.
pub const ALLOW_ENV: &str = "--allow-env";

/// The option that grants the fs host module the directories it lists, to
/// read.
pub const ALLOW_READ: &str = DirFlag::Read.spelled();

/// The option that grants the fs host module the directories it lists, to
/// write.
pub const ALLOW_WRITE: &str = DirFlag::Write.spelled();

/// The option that grants the fs host module fresh directories under the
/// OS's temporary directory, made by `fs.temp_dir`.
pub const ALLOW_TEMP: &str = "--allow-temp";

/// The option that grants the spawn host module the programs it lists.
pub const ALLOW_RUN: &str = "--allow-run";

/// The option that grants the spawn host module the directories it lists
/// as exec roots, beneath which `program_in` and `find_in` find a program.
pub const ALLOW_RUN_IN: &str = DirFlag::RunIn.spelled();

/// The option that forwards to the programs `--allow-run` grants, and those
/// found beneath what `--allow-run-in` grants, the environment variables it
/// lists, when a program's `pass_env` names them.
pub const ALLOW_RUN_ENV: &str = "--allow-run-env";

/// The option that lets one program the run grants write the directories
/// it lists, after the program's name and a `:`.
pub const ALLOW_RUN_WRITE: &str = "--allow-run-write";

/// The option that runs unconfined, every grant at its widest.
pub const ALLOW_ALL: &str = "-A";

/// A grant flag that lists what it grants after `=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFlag {
    /// `--allow-env`.
    Env,
    /// `--allow-read`.
    Read,
    /// `--allow-write`.
    Write,
    /// `--allow-run`.
    Run,
    /// `--allow-run-in`.
    RunIn,
    /// `--allow-run-env`.
    RunEnv,
    /// `--allow-run-write`.
    RunWrite,
}

impl ListFlag {
    /// Every one, in the order usage names them.
    pub const ALL: [Self; 7] = [
        Self::Env,
        Self::Read,
        Self::Write,
        Self::Run,
        Self::RunIn,
        Self::RunEnv,
        Self::RunWrite,
    ];

    /// The flag as it's spelled, without its list.
    pub const fn spelled(self) -> &'static str {
        match self {
            Self::Env => ALLOW_ENV,
            Self::Read => ALLOW_READ,
            Self::Write => ALLOW_WRITE,
            Self::Run => ALLOW_RUN,
            Self::RunIn => ALLOW_RUN_IN,
            Self::RunEnv => ALLOW_RUN_ENV,
            Self::RunWrite => ALLOW_RUN_WRITE,
        }
    }

    /// What follows the flag's `=` in an example of it, as a usage error
    /// shows one: `vitest:src` for `--allow-run-write`, which names a
    /// program first, and `a,b` for the rest.
    pub const fn example(self) -> &'static str {
        match self {
            Self::RunWrite => "vitest:src",
            Self::Env | Self::Read | Self::Write | Self::Run | Self::RunIn | Self::RunEnv => "a,b",
        }
    }
}

impl From<DirFlag> for ListFlag {
    fn from(flag: DirFlag) -> Self {
        match flag {
            DirFlag::Read => Self::Read,
            DirFlag::Write => Self::Write,
            DirFlag::RunIn => Self::RunIn,
        }
    }
}

impl std::fmt::Display for ListFlag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.spelled())
    }
}

/// A grant flag as it's spelled, what it lists not yet read: the spelling
/// `fgro run` takes before its file and the verb right after its command
/// word, one grammar for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantFlag<'a> {
    /// `-A`.
    All,
    /// `--allow-temp`.
    Temp,
    /// `--allow-temp=…`, a value given to the flag that takes none.
    TempWithValue,
    /// A flag that lists, with what follows its first `=`, which may be
    /// empty, or with no `=`.
    List(ListFlag, Option<&'a str>),
}

impl<'a> GrantFlag<'a> {
    /// The grant flag `arg` spells, `None` for any other argument: `-A`
    /// exactly, or a flag's name, up to any `=`, exactly.
    pub fn parse(arg: &'a str) -> Option<Self> {
        if arg == ALLOW_ALL {
            return Some(Self::All);
        }
        let (name, list) = match arg.split_once('=') {
            Some((name, list)) => (name, Some(list)),
            None => (arg, None),
        };
        if name == ALLOW_TEMP {
            return Some(if list.is_some() {
                Self::TempWithValue
            } else {
                Self::Temp
            });
        }
        ListFlag::ALL
            .into_iter()
            .find(|flag| flag.spelled() == name)
            .map(|flag| Self::List(flag, list))
    }
}

/// `fgro run --help`, laid out as argh lays out the other commands' help.
fn run_help() -> String {
    [
        &format!(
            "Usage: {BINARY_NAME} {RUN} [{ALLOW_ENV}=<names>] [{ALLOW_READ}=<dirs>] [{ALLOW_WRITE}=<dirs>] \
             [{ALLOW_TEMP}] [{ALLOW_RUN}=<names>] [{ALLOW_RUN_IN}=<dirs>] \
             [{ALLOW_RUN_ENV}=<names>] [{ALLOW_RUN_WRITE}=<program>:<dirs>] [{ALLOW_ALL}] [--] \
             <file> [<args...>]"
        ),
        "",
        RUN_DESCRIPTION,
        "",
        "Positional Arguments:",
        "  file              the module to run: a path relative to the working directory,",
        "                    or absolute",
        "  args              the script's arguments, every one after the file as written,",
        "                    flags and `--` included, read from `@fuzdev/gro/args.ts`",
        "",
        "Options:",
        "  --allow-env=<names>",
        "                    grant the project's modules `@fuzdev/gro/env.ts` with these",
        "                    environment variables, a comma-separated list of exact names;",
        "                    repeatable",
        "  --allow-read=<dirs>",
        "                    grant the project's modules `@fuzdev/gro/fs.ts` reading these",
        "                    directories, a comma-separated list of existing ones,",
        "                    relative to the working directory or absolute; repeatable;",
        "                    the project's root is read without it",
        "  --allow-write=<dirs>",
        "                    grant the project's modules `@fuzdev/gro/fs.ts` writing these",
        "                    directories, as --allow-read lists them; a directory only",
        "                    written isn't read",
        "  --allow-temp      grant the project's modules `@fuzdev/gro/fs.ts` making fresh",
        "                    directories under the OS's temporary directory, each read",
        "                    and written by the run, with fs.temp_dir; nothing else of",
        "                    that directory",
        "  --allow-run=<names>",
        "                    grant the project's modules `@fuzdev/gro/spawn.ts` making",
        "                    these programs, a comma-separated list of exact names, each",
        "                    found on PATH, or beneath the project's root when it holds",
        "                    `/`, and pinned as the run starts; repeatable. Until each",
        "                    child is boxed, granting any program is close to granting",
        "                    what -A grants: most can be made to run code by their",
        "                    arguments or environment, a shell, git, and cargo among",
        "                    them, and the root's env values set any variable; hand",
        "                    library code a narrowed root, which refuses env values",
        "  --allow-run-in=<dirs>",
        "                    grant the project's modules `@fuzdev/gro/spawn.ts` making",
        "                    programs found beneath these directories with program_in",
        "                    and find_in, as --allow-read lists them, each held as the",
        "                    run starts; repeatable. A directory also granted by",
        "                    --allow-write runs what the run writes there; as for",
        "                    --allow-run, granting one is close to granting what -A",
        "                    grants",
        "  --allow-run-env=<names>",
        "                    let the programs --allow-run grants, and those found",
        "                    beneath what --allow-run-in grants, be passed these",
        "                    environment variables, when a program's pass_env names",
        "                    them, a comma-separated list of exact names or PREFIX_*",
        "                    globs; repeatable; only beside --allow-run or",
        "                    --allow-run-in",
        "  --allow-run-write=<program>:<dirs>",
        "                    let one program --allow-run grants write these directories:",
        "                    its name, `:`, then directories as --allow-write lists them;",
        "                    repeatable, one program each, only beside --allow-run.",
        &format!(
            "                    `{BINARY_NAME} {RUN}` starts programs in no box, so here the flag is"
        ),
        "                    checked and holds nothing back",
        "  -A                run unconfined: every grant at its widest, to every module,",
        "                    every environment variable and every directory among them",
        "  --help, help      display usage information",
    ]
    .join("\n")
}

/// The authority a run's flags give it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunGrant {
    /// The project's modules granted `args`, `exit`, `stdout`, `stderr`,
    /// and `fs`, which reads the project's tree and the directories
    /// `--allow-read=` lists and writes those `--allow-write=` lists
    /// ([`FlagDirs`]), and `env` for the names `--allow-env=` listed, when
    /// it listed any, and fresh temporary directories when `--allow-temp`
    /// was given, and `spawn` for the programs `--allow-run=` listed and
    /// the exec roots `--allow-run-in=` listed ([`FlagDirs`]), when it
    /// listed any of either, each passed the variables `--allow-run-env=`
    /// lists.
    Named {
        /// The environment variables granted, each an exact name.
        env: BTreeSet<String>,
        /// Whether `fs.temp_dir` makes fresh directories under the OS's
        /// temporary directory.
        temp: bool,
        /// The programs granted, each by the name a program asks for.
        run: BTreeSet<String>,
        /// The variables each program granted may be passed.
        run_env: BTreeSet<EnvPattern>,
    },
    /// `-A`: unconfined, every grant at its widest, the one option that
    /// grants beyond what it names.
    All,
}

/// The directories `--allow-read=`, `--allow-write=`, `--allow-run-in=`,
/// and `--allow-run-write=` list, each as written, not yet resolved or
/// checked: a run checks each, under `-A` too, before it starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagDirs {
    /// The directories `--allow-read=` lists.
    pub read: BTreeSet<String>,
    /// The directories `--allow-write=` lists.
    pub write: BTreeSet<String>,
    /// The directories `--allow-run-in=` lists.
    pub run_in: BTreeSet<String>,
    /// The directories `--allow-run-write=` lists, by the program each
    /// flag names, as written.
    pub run_write: BTreeMap<String, BTreeSet<String>>,
}

/// `fgro run`'s arguments, checked.
pub struct RunInvocation {
    /// The module to run: relative to the working directory, or absolute.
    pub file: PathBuf,
    /// The script's arguments, every one after the file, as written.
    pub args: Vec<String>,
    /// What the flags grant.
    pub grant: RunGrant,
    /// The directories the flags list, which grant nothing more under `-A`.
    pub dirs: FlagDirs,
}

impl RunInvocation {
    /// What `args` ask for. fgro's own options are read up to the first
    /// argument that doesn't start with `-`, or the one after a bare `--`:
    /// that is the file, and every later argument is the script's, as
    /// written. The options are the help trigger (`--help`, or `help` as
    /// argh spells it for every command), which asks for the help whatever
    /// follows it; `--allow-env=<names>`, repeatable, its names one set;
    /// `--allow-read=<dirs>` and `--allow-write=<dirs>`, each repeatable,
    /// its directories one set; `--allow-temp`, which takes no value;
    /// `--allow-run=<names>`, `--allow-run-in=<dirs>`, and
    /// `--allow-run-env=<names>`, each repeatable, its names or directories
    /// one set, the last only beside one of the others;
    /// `--allow-run-write=<program>:<dirs>`, repeatable, its program one
    /// `--allow-run=` lists; and `-A`, which grants everything, so names,
    /// directories, and `--allow-temp` given beside it are checked and add
    /// nothing. A file starting with `-` comes after `--`. The grant flags
    /// are read by [`RunGrantBuilder`], as the verb's are.
    ///
    /// # Errors
    ///
    /// Returns the usage error, checked in the order the arguments come: an
    /// option before the file that isn't one of these, or a grant flag
    /// [`RunGrantBuilder::add`] refuses; then `--allow-run-env=` without
    /// `--allow-run=` or `--allow-run-in=`; then `--allow-run-write=`
    /// without `--allow-run=`, then naming a program no `--allow-run=`
    /// lists; then no file.
    fn check(args: RunArgs) -> Result<Task, CliError> {
        let mut help = false;
        let mut grant = RunGrantBuilder::default();
        let mut args = args.args.into_iter();
        let file = loop {
            let Some(arg) = args.next() else {
                break None;
            };
            if HELP_TRIGGERS.contains(&arg.as_str()) {
                help = true;
                continue;
            }
            if arg == "--" {
                break args.next();
            }
            match GrantFlag::parse(&arg) {
                Some(flag) => grant.add(flag)?,
                None if arg.starts_with('-') => return Err(CliError::RunOption(arg)),
                None => break Some(arg),
            }
        };
        if grant.run_env_alone() {
            return Err(CliError::AllowRunEnvAlone);
        }
        if grant.run_write_alone() {
            return Err(CliError::AllowRunWriteAlone);
        }
        if let Some(program) = grant.run_write_ungranted() {
            return Err(CliError::RunWriteUngranted(program.to_owned()));
        }
        match (help, file) {
            (true, _) => Ok(Task::Help(run_help())),
            (false, None) => Err(CliError::RunNoFile),
            (false, Some(file)) => {
                let (grant, dirs) = grant.build();
                Ok(Task::Run(Self {
                    file: PathBuf::from(file),
                    args: args.collect(),
                    grant,
                    dirs,
                }))
            }
        }
    }
}

/// The grant flags read one at a time into what they grant, each checked
/// as it's added: the one reading of a grant list, `fgro run`'s flags
/// before its file and the verb's after its command word
/// (`verb::grant_flags::to_run_grant`) alike.
#[derive(Debug, Default)]
pub struct RunGrantBuilder {
    /// Whether `-A` was given.
    all: bool,
    /// Whether `--allow-temp` was given.
    temp: bool,
    /// The names `--allow-env=` lists.
    env: BTreeSet<String>,
    /// The names `--allow-run=` lists.
    run: BTreeSet<String>,
    /// The names and globs `--allow-run-env=` lists.
    run_env: BTreeSet<EnvPattern>,
    /// The directories the directory flags list.
    dirs: FlagDirs,
}

impl RunGrantBuilder {
    /// Adds `flag`, what it lists checked: `-A`, which grants everything,
    /// so what's given beside it is checked and adds nothing; `--allow-temp`;
    /// or a flag that lists, its names or directories joining the ones
    /// listed before.
    ///
    /// # Errors
    ///
    /// Returns the usage error for a bare flag that lists, which grants
    /// nothing, `--allow-temp` given a value, a name `--allow-env=` lists
    /// that no variable can have ([`check_env_flag_names`]), a directory
    /// `--allow-read=`, `--allow-write=`, or `--allow-run-in=` lists that
    /// no path can be ([`check_dir_flag`]), a name `--allow-run=` lists
    /// that names no program it can grant ([`check_run_flag_names`]), one
    /// `--allow-run-env=` lists that names no variable or glob
    /// ([`check_run_env_flag_names`]), or an `--allow-run-write=` that
    /// names no one program and its directories ([`check_run_write_flag`]).
    pub fn add(&mut self, flag: GrantFlag<'_>) -> Result<(), GrantListError> {
        match flag {
            GrantFlag::All => self.all = true,
            GrantFlag::Temp => self.temp = true,
            GrantFlag::TempWithValue => return Err(GrantListError::TempValue),
            GrantFlag::List(flag, None) => {
                return Err(match flag {
                    ListFlag::Env => GrantListError::EnvBare,
                    ListFlag::Read | ListFlag::Write | ListFlag::RunIn => {
                        GrantListError::DirsBare(flag)
                    }
                    ListFlag::Run => GrantListError::RunBare,
                    ListFlag::RunEnv => GrantListError::RunEnvBare,
                    ListFlag::RunWrite => GrantListError::RunWriteBare,
                });
            }
            GrantFlag::List(flag, Some(listed)) => match flag {
                ListFlag::Env => self.env.extend(check_env_flag_names(listed)?),
                ListFlag::Read => self.dirs.read.extend(check_dir_flag(flag, listed)?),
                ListFlag::Write => self.dirs.write.extend(check_dir_flag(flag, listed)?),
                ListFlag::Run => self.run.extend(check_run_flag_names(listed)?),
                ListFlag::RunIn => self.dirs.run_in.extend(check_dir_flag(flag, listed)?),
                ListFlag::RunEnv => self.run_env.extend(check_run_env_flag_names(listed)?),
                ListFlag::RunWrite => {
                    let (program, dirs) = check_run_write_flag(listed)?;
                    self.dirs.run_write.entry(program).or_default().extend(dirs);
                }
            },
        }
        Ok(())
    }

    /// Whether `--allow-run-env=` listed variables with neither
    /// `--allow-run=` nor `--allow-run-in=` listing anything, so no
    /// program the flags grant is passed them: a usage error where the
    /// flags alone grant programs, as `fgro run`'s do.
    pub fn run_env_alone(&self) -> bool {
        !self.run_env.is_empty() && self.run.is_empty() && self.dirs.run_in.is_empty()
    }

    /// Whether `--allow-run-write=` named a program with `--allow-run=`
    /// listing none, so no program the flags grant writes what it names: a
    /// usage error where the flags alone grant programs, as `fgro run`'s
    /// do.
    pub fn run_write_alone(&self) -> bool {
        !self.dirs.run_write.is_empty() && self.run.is_empty()
    }

    /// The first program `--allow-run-write=` names, as written, that no
    /// `--allow-run=` lists: a usage error where the flags alone grant
    /// programs, as `fgro run`'s do.
    pub fn run_write_ungranted(&self) -> Option<&str> {
        self.dirs
            .run_write
            .keys()
            .find(|program| !self.run.contains(*program))
            .map(String::as_str)
    }

    /// What the flags added grant, and the directories they list.
    pub fn build(self) -> (RunGrant, FlagDirs) {
        let grant = if self.all {
            RunGrant::All
        } else {
            RunGrant::Named {
                env: self.env,
                temp: self.temp,
                run: self.run,
                run_env: self.run_env,
            }
        };
        (grant, self.dirs)
    }
}

/// Why a grant flag's list is refused, as [`RunGrantBuilder::add`] checks
/// it: `fgro run`'s usage error ([`CliError::GrantList`]), which adds its
/// help's hint, and the verb's ([`GrantFlagsError::List`]), which doesn't.
///
/// [`GrantFlagsError::List`]: crate::verb::grant_flags::GrantFlagsError::List
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GrantListError {
    /// `--allow-env` with no names, which grants none.
    #[error("{ALLOW_ENV} takes the names it grants, as {ALLOW_ENV}=A,B")]
    EnvBare,
    /// `--allow-env=` listing a name no variable can have.
    #[error("{0}")]
    EnvName(EnvFlagFault),
    /// `--allow-read`, `--allow-write`, or `--allow-run-in` with no
    /// directories, which grants none.
    #[error("{0} takes the directories it grants, as {0}=a,/b")]
    DirsBare(ListFlag),
    /// `--allow-temp=` with a value, which it takes none of; the value isn't
    /// echoed.
    #[error(
        "{ALLOW_TEMP} takes no value: it grants fresh directories under the OS's temporary \
         directory, as {ALLOW_TEMP}"
    )]
    TempValue,
    /// `--allow-run` with no names, which grants none.
    #[error("{ALLOW_RUN} takes the programs it grants, as {ALLOW_RUN}=git,cargo")]
    RunBare,
    /// `--allow-run=` listing a name it can't grant.
    #[error("{0}")]
    RunName(RunFlagFault),
    /// `--allow-run-env` with no names, which passes none.
    #[error(
        "{ALLOW_RUN_ENV} takes the variables it lets programs be passed, as \
         {ALLOW_RUN_ENV}=A,B_*"
    )]
    RunEnvBare,
    /// `--allow-run-env=` listing a name that names no variable or glob.
    #[error("{0}")]
    RunEnvName(RunEnvFlagFault),
    /// `--allow-run-write` with no program and directories, which grants
    /// none.
    #[error(
        "{ALLOW_RUN_WRITE} takes a program and the directories it may write, as \
         {ALLOW_RUN_WRITE}=vitest:src"
    )]
    RunWriteBare,
    /// `--allow-run-write=` naming no one program before a `:` with
    /// directories after it: no `:`, nothing before or after it, or a `,`
    /// before it, which would name two.
    #[error(
        "{ALLOW_RUN_WRITE} names one program, then `:`, then the directories it may write, like \
         {ALLOW_RUN_WRITE}=vitest:src,test"
    )]
    RunWriteShape,
    /// `--allow-run-write=` naming a program no `--allow-run=` name could
    /// be.
    #[error("{}", run_write_program_fault(.0))]
    RunWriteProgram(RunFlagFault),
    /// `--allow-read=`, `--allow-write=`, `--allow-run-in=`, or
    /// `--allow-run-write=` listing a path that names nothing.
    #[error(
        "{}",
        match .1 {
            DirFlagFault::Empty => format!(
                "a {0} directory is empty: {0}={1} lists directories separated by single \
                 commas",
                .0,
                dir_list_example(*.0)
            ),
            DirFlagFault::Nul => format!("a {} directory holds a NUL byte", .0),
        }
    )]
    Dir(ListFlag, DirFlagFault),
}

/// The example of a directory list [`GrantListError::Dir`] shows for
/// `flag`: `vitest:a,/b` for `--allow-run-write`, which names its program
/// first, and `a,/b` for the rest.
const fn dir_list_example(flag: ListFlag) -> &'static str {
    match flag {
        ListFlag::RunWrite => "vitest:a,/b",
        ListFlag::Env
        | ListFlag::Read
        | ListFlag::Write
        | ListFlag::Run
        | ListFlag::RunIn
        | ListFlag::RunEnv => "a,/b",
    }
}

/// [`GrantListError::RunWriteProgram`]'s message: `fault` for the program
/// `--allow-run-write=` names, which is named as the tool is called or as
/// `--allow-run=` names it, escaped ([`escape_controls`]). An empty name
/// is [`GrantListError::RunWriteShape`]'s.
fn run_write_program_fault(fault: &RunFlagFault) -> String {
    match fault {
        RunFlagFault::Empty => GrantListError::RunWriteShape.to_string(),
        RunFlagFault::Nul => format!("an {ALLOW_RUN_WRITE} program holds a NUL byte"),
        RunFlagFault::Glob(name) => format!(
            "an {ALLOW_RUN_WRITE} program is exact, never a glob: {}",
            escape_controls(name)
        ),
        RunFlagFault::Absolute(name) => format!(
            "an {ALLOW_RUN_WRITE} program is named as the tool is called or as {ALLOW_RUN} names \
             it, never by an absolute path: {}",
            escape_controls(name)
        ),
        RunFlagFault::Climb(name) => format!(
            "an {ALLOW_RUN_WRITE} program holding / resolves beneath the project's root, and this \
             one climbs above it: {}",
            escape_controls(name)
        ),
    }
}

/// The names `--allow-env=` lists in `names`, split at each comma, each
/// held to the rule the env host module's `narrow` holds a name to: not
/// empty, no NUL byte, no `=`, and no `*`, every name being exact.
///
/// # Errors
///
/// Returns [`GrantListError::EnvName`] for the first name that breaks it.
fn check_env_flag_names(names: &str) -> Result<Vec<String>, GrantListError> {
    names
        .split(',')
        .map(|name| {
            env_name_fault(name, false).map_or_else(
                || Ok(name.to_owned()),
                |fault| {
                    Err(GrantListError::EnvName(match fault {
                        EnvNameFault::Glob => EnvFlagFault::Glob(name.to_owned()),
                        EnvNameFault::Empty => EnvFlagFault::Empty,
                        EnvNameFault::Nul => EnvFlagFault::Nul,
                        EnvNameFault::Equals => EnvFlagFault::Equals,
                    }))
                },
            )
        })
        .collect()
}

/// Why a name `--allow-env=` lists can't be granted. Only a glob's name is
/// kept, echoed escaped ([`escape_controls`]): a name holding a `=` is
/// never echoed, since what follows it can be a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvFlagFault {
    /// An empty name: a list that's empty, or starts, ends, or doubles a
    /// comma.
    Empty,
    /// A name holding a NUL byte.
    Nul,
    /// A name holding a `=`.
    Equals,
    /// A name holding a `*`, which names no glob.
    Glob(String),
}

impl std::fmt::Display for EnvFlagFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str(
                "an --allow-env name is empty: --allow-env=A,B lists names separated by single \
                 commas",
            ),
            Self::Nul => f.write_str("an --allow-env name holds a NUL byte"),
            Self::Equals => {
                f.write_str("an --allow-env name holds a `=`, which no variable's name holds")
            }
            Self::Glob(name) => write!(
                f,
                "an --allow-env name is exact, never a glob: {}",
                escape_controls(name)
            ),
        }
    }
}

/// The names `--allow-run=` lists in `names`, split at each comma, each a
/// program's name as a program asks for it: not empty, no NUL byte, no
/// `*`, every name being exact, not absolute, and, holding `/`, not
/// climbing above the project's root by `..`, which a name holding `/`
/// resolves beneath. Whether each is found is decided as the run starts.
///
/// # Errors
///
/// Returns [`GrantListError::RunName`] for the first name that breaks it.
fn check_run_flag_names(names: &str) -> Result<Vec<String>, GrantListError> {
    names
        .split(',')
        .map(|name| {
            run_name_fault(name).map_or_else(
                || Ok(name.to_owned()),
                |fault| Err(GrantListError::RunName(fault)),
            )
        })
        .collect()
}

/// Why `name` can't be a program's name as a program asks for it, if it
/// can't: empty, holding a NUL byte or a `*`, absolute, or holding `/`
/// and climbing above the project's root by `..`.
fn run_name_fault(name: &str) -> Option<RunFlagFault> {
    if name.is_empty() {
        Some(RunFlagFault::Empty)
    } else if name.contains('\0') {
        Some(RunFlagFault::Nul)
    } else if name.contains('*') {
        Some(RunFlagFault::Glob(name.to_owned()))
    } else if name.starts_with('/') {
        Some(RunFlagFault::Absolute(name.to_owned()))
    } else if name.contains('/') && climbs(name) {
        Some(RunFlagFault::Climb(name.to_owned()))
    } else {
        None
    }
}

/// The program `--allow-run-write=` names in `listed` and the directories
/// it lists for it: the value split at its first `:`, the program before
/// it held to the rules an `--allow-run=` name is ([`run_name_fault`]),
/// one name and so no `,`, and the directories after it read as
/// [`check_dir_flag`] reads them, so a directory may hold a `:`.
///
/// # Errors
///
/// Returns [`GrantListError::RunWriteShape`] for a value with no `:`,
/// nothing before or after it, or a `,` before it;
/// [`GrantListError::RunWriteProgram`] for a program no `--allow-run=`
/// name could be; and [`GrantListError::Dir`] for the first directory
/// [`check_dir_flag`] refuses.
fn check_run_write_flag(listed: &str) -> Result<(String, Vec<String>), GrantListError> {
    let Some((program, dirs)) = listed.split_once(':') else {
        return Err(GrantListError::RunWriteShape);
    };
    if program.is_empty() || dirs.is_empty() || program.contains(',') {
        return Err(GrantListError::RunWriteShape);
    }
    if let Some(fault) = run_name_fault(program) {
        return Err(GrantListError::RunWriteProgram(fault));
    }
    Ok((
        program.to_owned(),
        check_dir_flag(ListFlag::RunWrite, dirs)?,
    ))
}

/// Whether `path`, relative, climbs above where it starts by a `..`, its
/// `.` and `..` resolved by their text.
fn climbs(path: &str) -> bool {
    let mut depth = 0_usize;
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => match depth.checked_sub(1) {
                Some(up) => depth = up,
                None => return true,
            },
            _ => depth += 1,
        }
    }
    false
}

/// Why a name `--allow-run=` lists can't be granted. A name is echoed,
/// escaped ([`escape_controls`]): it's a program's, never a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFlagFault {
    /// An empty name: a list that's empty, or starts, ends, or doubles a
    /// comma.
    Empty,
    /// A name holding a NUL byte.
    Nul,
    /// A name holding a `*`, which names no glob.
    Glob(String),
    /// An absolute name.
    Absolute(String),
    /// A name holding `/` that climbs above the project's root.
    Climb(String),
}

impl std::fmt::Display for RunFlagFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str(
                "an --allow-run name is empty: --allow-run=a,b lists names separated by single \
                 commas",
            ),
            Self::Nul => f.write_str("an --allow-run name holds a NUL byte"),
            Self::Glob(name) => write!(
                f,
                "an --allow-run name is exact, never a glob: {}",
                escape_controls(name)
            ),
            Self::Absolute(name) => write!(
                f,
                "an --allow-run name is found on PATH, or beneath the project's root when it \
                 holds /, never absolute: {}",
                escape_controls(name)
            ),
            Self::Climb(name) => write!(
                f,
                "an --allow-run name holding / resolves beneath the project's root, and this \
                 one climbs above it: {}",
                escape_controls(name)
            ),
        }
    }
}

/// The patterns `--allow-run-env=` lists in `names`, split at each comma,
/// each a variable's exact name or a `PREFIX_*` glob ([`EnvPattern`]).
///
/// # Errors
///
/// Returns [`GrantListError::RunEnvName`] for the first that names
/// neither.
fn check_run_env_flag_names(names: &str) -> Result<Vec<EnvPattern>, GrantListError> {
    names
        .split(',')
        .map(|name| {
            EnvPattern::parse(name).map_err(|fault| {
                GrantListError::RunEnvName(match fault {
                    EnvPatternFault::Name(EnvNameFault::Empty) => RunEnvFlagFault::Empty,
                    EnvPatternFault::Name(EnvNameFault::Nul) => RunEnvFlagFault::Nul,
                    EnvPatternFault::Name(EnvNameFault::Equals) => RunEnvFlagFault::Equals,
                    EnvPatternFault::Name(EnvNameFault::Glob) | EnvPatternFault::GlobNotLast => {
                        RunEnvFlagFault::GlobNotLast(name.to_owned())
                    }
                    EnvPatternFault::GlobAlone => RunEnvFlagFault::GlobAlone,
                })
            })
        })
        .collect()
}

/// Why a name `--allow-run-env=` lists can't be granted. Only a glob's name
/// is kept, echoed escaped ([`escape_controls`]): a name holding a `=` is
/// never echoed, since what follows it can be a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEnvFlagFault {
    /// An empty name: a list that's empty, or starts, ends, or doubles a
    /// comma.
    Empty,
    /// A name holding a NUL byte.
    Nul,
    /// A name holding a `=`.
    Equals,
    /// A `*` before a name's last character.
    GlobNotLast(String),
    /// A `*` alone, which would pass every variable without saying so.
    GlobAlone,
}

impl std::fmt::Display for RunEnvFlagFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str(
                "an --allow-run-env name is empty: --allow-run-env=A,B_* lists names separated \
                 by single commas",
            ),
            Self::Nul => f.write_str("an --allow-run-env name holds a NUL byte"),
            Self::Equals => {
                f.write_str("an --allow-run-env name holds a `=`, which no variable's name holds")
            }
            Self::GlobNotLast(name) => write!(
                f,
                "an --allow-run-env glob ends in its one *, after a prefix: {}",
                escape_controls(name)
            ),
            Self::GlobAlone => f.write_str(
                "an --allow-run-env glob needs a prefix: * alone would pass every variable",
            ),
        }
    }
}

/// The directories `flag` lists in `listed`, split at each comma, each as
/// written: not empty and holding no NUL byte. A path holding a comma
/// can't be listed. Whether each exists, and is a directory, a run checks
/// before it starts.
///
/// # Errors
///
/// Returns [`GrantListError::Dir`] for the first directory that breaks it.
fn check_dir_flag(flag: ListFlag, listed: &str) -> Result<Vec<String>, GrantListError> {
    listed
        .split(',')
        .map(|dir| {
            if dir.is_empty() {
                Err(GrantListError::Dir(flag, DirFlagFault::Empty))
            } else if dir.contains('\0') {
                Err(GrantListError::Dir(flag, DirFlagFault::Nul))
            } else {
                Ok(dir.to_owned())
            }
        })
        .collect()
}

/// Why a directory `--allow-read=`, `--allow-write=`, `--allow-run-in=`,
/// or `--allow-run-write=` lists can't be granted, before it's looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirFlagFault {
    /// An empty path: a list that's empty, or starts, ends, or doubles a
    /// comma.
    Empty,
    /// A path holding a NUL byte.
    Nul,
}

/// evaluate an ES module to JSON data, written as one envelope on stdout
#[derive(FromArgs)]
#[argh(subcommand, name = "eval")]
pub struct EvalArgs {
    /// the module to evaluate: a path relative to the working directory, or absolute
    #[argh(positional)]
    pub module: PathBuf,
    /// read a driver module from stdin (pass `-`), whose default export is
    /// called with the module's namespace and returns the value
    #[argh(option)]
    pub driver: Option<String>,
    /// the export to serialize, with no driver (default: default)
    #[argh(option)]
    pub export: Option<String>,
    /// turn V8's JIT on, and with it WebAssembly
    #[argh(switch)]
    pub jit: bool,
    /// stop the eval after this many milliseconds, at least 1 (default: 10000)
    #[argh(option)]
    pub timeout: Option<u64>,
    /// add the module graph to the envelope, as `graph`
    #[argh(switch)]
    pub graph: bool,
}

/// The `--driver` value that reads the driver from stdin, the only one so far.
pub const DRIVER_STDIN: &str = "-";

/// `fgro eval`'s arguments, checked.
pub struct EvalInvocation {
    /// The module to evaluate: relative to the working directory, or
    /// absolute.
    pub module: PathBuf,
    /// Where the value comes from.
    pub value: EvalValue,
    /// Whether V8's JIT is on.
    pub jit: Jit,
    /// How long the eval may run, at least a millisecond.
    pub timeout: Duration,
    /// Whether the envelope carries the module graph.
    pub graph: bool,
}

/// Where an eval's value comes from.
pub enum EvalValue {
    /// A driver read from stdin (`--driver -`), handed the namespace.
    StdinDriver,
    /// The export named, `default` when unset.
    Export(Option<String>),
}

impl EvalInvocation {
    /// The checked form of `args`.
    ///
    /// # Errors
    ///
    /// Returns the usage error: a `--timeout` of 0, `--export` with a
    /// driver, or a `--driver` other than [`DRIVER_STDIN`], checked in that
    /// order.
    fn check(args: EvalArgs) -> Result<Self, CliError> {
        let EvalArgs {
            module,
            driver,
            export,
            jit,
            timeout,
            graph,
        } = args;
        let timeout = match timeout {
            None => gro_runtime::eval::DEFAULT_TIMEOUT,
            Some(0) => return Err(CliError::TimeoutZero),
            Some(ms) => Duration::from_millis(ms),
        };
        let value = match (driver, export) {
            (None, export) => EvalValue::Export(export),
            (Some(driver), None) if driver == DRIVER_STDIN => EvalValue::StdinDriver,
            (Some(driver), Some(_)) if driver == DRIVER_STDIN => {
                return Err(CliError::DriverWithExport);
            }
            (Some(path), _) => return Err(CliError::DriverPath(path)),
        };
        Ok(Self {
            module,
            value,
            jit: if jit { Jit::On } else { Jit::Off },
            timeout,
            graph,
        })
    }
}

/// What the arguments ask for, parsed once.
pub enum Invocation {
    /// `fgro eval`, which confines its process and so runs on the main
    /// thread.
    Eval(EvalInvocation),
    /// A command of gro's, every argument as written: its pre-pass and its
    /// box's steps run on the main thread before any thread starts, then
    /// it runs on a thread with a stated stack, or on the Node Gro.
    Verb(Vec<String>),
    /// Every other command, run on a thread with a stated stack.
    Sized(Task),
}

/// A command that runs on a thread with a stated stack.
pub enum Task {
    /// `--version`.
    Version,
    /// `--help`, of `fgro` or a subcommand: argh's text, for stdout.
    Help(String),
    /// `make-snapshot <dir>`.
    MakeSnapshot(PathBuf),
    /// `make-embed <src> <dir>`: the checkout's `src/` and the directory
    /// to write into.
    MakeEmbed(PathBuf, PathBuf),
    /// `run <file> [<args...>]`.
    Run(RunInvocation),
}

/// The `N` paths an internal subcommand takes, `args` its name and them,
/// `usage` naming them.
///
/// # Errors
///
/// Returns [`CliError::InternalUsage`] without exactly `N`, and
/// [`CliError::InternalFlag`] for a flag, so `--help` can't become a
/// directory's name.
fn internal_args<const N: usize>(
    args: &[&str],
    name: &'static str,
    usage: &'static str,
) -> Result<[PathBuf; N], CliError> {
    let paths: &[&str; N] = args
        .get(1..)
        .and_then(|paths| paths.try_into().ok())
        .ok_or(CliError::InternalUsage(name, usage))?;
    if paths.iter().any(|path| path.starts_with('-')) {
        return Err(CliError::InternalFlag(name, usage));
    }
    Ok(paths.map(PathBuf::from))
}

/// Parses `args` (the program name excluded): a first argument that isn't
/// one of [`BINARY_WORDS`], or none, is a command of gro's, its arguments
/// kept as written for its pre-pass; `make-snapshot` and `make-embed` by
/// hand, so they stay out of `--help`; and everything else by argh, whose
/// early exits are mapped by hand, since its `from_env` exits `1` on a
/// usage error. Of argh's text only its help is printed, made of the
/// commands' own names and docs; a refusal is [`invalid_arguments`], made
/// of the arguments. argh routes `run`, and [`RunInvocation::check`] parses
/// its arguments. `eval` is decided from what argh parses, not the first
/// argument, so every spelling it accepts runs on the main thread.
///
/// # Errors
///
/// Returns the usage error the arguments make, run's and eval's checked
/// arguments included ([`RunInvocation::check`], [`EvalInvocation::check`]).
pub fn parse(args: &[&str]) -> Result<Invocation, CliError> {
    if args
        .first()
        .is_none_or(|first| !BINARY_WORDS.contains(first))
    {
        return Ok(Invocation::Verb(
            args.iter().map(|&arg| arg.to_owned()).collect(),
        ));
    }
    if args.first() == Some(&MAKE_SNAPSHOT) {
        let [dir] = internal_args(args, MAKE_SNAPSHOT, "<dir>")?;
        return Ok(Invocation::Sized(Task::MakeSnapshot(dir)));
    }
    if args.first() == Some(&MAKE_EMBED) {
        let [src, dir] = internal_args(args, MAKE_EMBED, "<src> <dir>")?;
        return Ok(Invocation::Sized(Task::MakeEmbed(src, dir)));
    }
    match Args::from_args(&[BINARY_NAME], args) {
        Ok(Args {
            version: true,
            command: None,
        }) => Ok(Invocation::Sized(Task::Version)),
        Ok(Args {
            version: true,
            command: Some(_),
        }) => Err(CliError::VersionWithCommand),
        // argh sees a first argument that names a command or a flag of the
        // binary's, so it never parses to nothing; were it to, the
        // arguments would be a command of gro's, as none at all are
        Ok(Args {
            version: false,
            command: None,
        }) => Ok(Invocation::Verb(
            args.iter().map(|&arg| arg.to_owned()).collect(),
        )),
        Ok(Args {
            version: false,
            command: Some(Command::Run(args)),
        }) => RunInvocation::check(args).map(Invocation::Sized),
        Ok(Args {
            version: false,
            command: Some(Command::Eval(args)),
        }) => EvalInvocation::check(args).map(Invocation::Eval),
        Err(EarlyExit {
            output,
            status: Ok(()),
        }) => Ok(Invocation::Sized(Task::Help(output))),
        // argh's text for a refusal can carry any part of an argument, so
        // the error is made of the arguments themselves
        Err(EarlyExit {
            status: Err(()), ..
        }) => Err(invalid_arguments(args)),
    }
}

/// The most arguments [`invalid_arguments`] shows; past these it counts
/// the rest.
pub const MAX_SHOWN_ARGUMENTS: usize = 16;

/// The most characters [`invalid_arguments`] shows of one argument, as
/// [`echo_argument`] prints it; past these it cuts it with `…`.
pub const MAX_SHOWN_ARGUMENT: usize = 80;

/// The usage error for arguments argh refused, made of `args` alone, never
/// of argh's text, which can carry any part of an argument: the command
/// they were given to, the binary's name followed by ` eval` when the first
/// argument that isn't an option is `eval`, else the name alone, and the first [`MAX_SHOWN_ARGUMENTS`]
/// arguments as given, each as [`echo_argument`] prints it (up to its
/// first `=`, escaped),
/// cut past [`MAX_SHOWN_ARGUMENT`] characters, an empty one as `""`, then
/// how many more there were.
fn invalid_arguments(args: &[&str]) -> CliError {
    let command = if args.iter().find(|arg| !arg.starts_with('-')) == Some(&"eval") {
        format!("{BINARY_NAME} eval")
    } else {
        BINARY_NAME.to_owned()
    };
    let mut shown: Vec<String> = args
        .iter()
        .take(MAX_SHOWN_ARGUMENTS)
        .map(|arg| {
            if arg.is_empty() {
                return "\"\"".to_owned();
            }
            let echoed = echo_argument(arg);
            if echoed.chars().count() > MAX_SHOWN_ARGUMENT {
                let cut: String = echoed.chars().take(MAX_SHOWN_ARGUMENT).collect();
                format!("{cut}…")
            } else {
                echoed
            }
        })
        .collect();
    if args.len() > MAX_SHOWN_ARGUMENTS {
        shown.push(format!("… ({} more)", args.len() - MAX_SHOWN_ARGUMENTS));
    }
    CliError::InvalidArguments {
        command,
        arguments: shown.join(" "),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use gro_runtime::BINARY_NAME;

    use super::{
        DirFlagFault, EnvFlagFault, FlagDirs, GrantListError, Invocation, ListFlag,
        RUN_DESCRIPTION, RunEnvFlagFault, RunFlagFault, RunGrant, Task, parse, run_help,
    };
    use crate::error::CliError;

    /// What `parse` makes of `fgro run`'s arguments, in a form to compare.
    #[derive(Debug, PartialEq)]
    enum Run {
        /// The file, then the script's arguments.
        Runs(String, Vec<String>),
        /// `run`'s help.
        Help,
        /// [`CliError::RunNoFile`].
        NoFile,
        /// [`CliError::RunOption`], naming the option.
        Option(String),
        /// [`GrantListError::EnvBare`].
        AllowEnvBare,
        /// [`GrantListError::EnvName`].
        AllowEnvName(EnvFlagFault),
        /// [`GrantListError::DirsBare`].
        AllowDirsBare(ListFlag),
        /// [`GrantListError::Dir`].
        AllowDir(ListFlag, DirFlagFault),
        /// [`GrantListError::TempValue`].
        AllowTempValue,
        /// [`GrantListError::RunBare`].
        AllowRunBare,
        /// [`GrantListError::RunName`].
        AllowRunName(RunFlagFault),
        /// [`GrantListError::RunEnvBare`].
        AllowRunEnvBare,
        /// [`GrantListError::RunEnvName`].
        AllowRunEnvName(RunEnvFlagFault),
        /// [`CliError::AllowRunEnvAlone`].
        AllowRunEnvAlone,
        /// [`GrantListError::RunWriteBare`].
        AllowRunWriteBare,
        /// [`GrantListError::RunWriteShape`].
        AllowRunWriteShape,
        /// [`GrantListError::RunWriteProgram`].
        AllowRunWriteProgram(RunFlagFault),
        /// [`CliError::AllowRunWriteAlone`].
        AllowRunWriteAlone,
        /// [`CliError::RunWriteUngranted`], naming the program.
        UngrantedRunWrite(String),
    }

    fn run_outcome(args: &[&str]) -> Run {
        match parse(args) {
            Ok(Invocation::Sized(Task::Run(run))) => {
                Run::Runs(run.file.to_str().unwrap().to_owned(), run.args)
            }
            Ok(Invocation::Sized(Task::Help(help))) if help == run_help() => Run::Help,
            Err(CliError::RunNoFile) => Run::NoFile,
            Err(CliError::RunOption(option)) => Run::Option(option),
            Err(CliError::GrantList(GrantListError::EnvBare)) => Run::AllowEnvBare,
            Err(CliError::GrantList(GrantListError::EnvName(fault))) => Run::AllowEnvName(fault),
            Err(CliError::GrantList(GrantListError::DirsBare(flag))) => Run::AllowDirsBare(flag),
            Err(CliError::GrantList(GrantListError::Dir(flag, fault))) => {
                Run::AllowDir(flag, fault)
            }
            Err(CliError::GrantList(GrantListError::TempValue)) => Run::AllowTempValue,
            Err(CliError::GrantList(GrantListError::RunBare)) => Run::AllowRunBare,
            Err(CliError::GrantList(GrantListError::RunName(fault))) => Run::AllowRunName(fault),
            Err(CliError::GrantList(GrantListError::RunEnvBare)) => Run::AllowRunEnvBare,
            Err(CliError::GrantList(GrantListError::RunEnvName(fault))) => {
                Run::AllowRunEnvName(fault)
            }
            Err(CliError::AllowRunEnvAlone) => Run::AllowRunEnvAlone,
            Err(CliError::GrantList(GrantListError::RunWriteBare)) => Run::AllowRunWriteBare,
            Err(CliError::GrantList(GrantListError::RunWriteShape)) => Run::AllowRunWriteShape,
            Err(CliError::GrantList(GrantListError::RunWriteProgram(fault))) => {
                Run::AllowRunWriteProgram(fault)
            }
            Err(CliError::AllowRunWriteAlone) => Run::AllowRunWriteAlone,
            Err(CliError::RunWriteUngranted(program)) => Run::UngrantedRunWrite(program),
            _ => panic!("{args:?} is not one of run's outcomes"),
        }
    }

    /// What `fgro run` with `args` (after `run`) grants.
    fn grant(args: &[&str]) -> RunGrant {
        let args = [&["run"][..], args].concat();
        match parse(&args) {
            Ok(Invocation::Sized(Task::Run(run))) => run.grant,
            _ => panic!("{args:?} runs a file"),
        }
    }

    fn env(names: &[&str]) -> RunGrant {
        RunGrant::Named {
            env: names.iter().map(|&name| name.to_owned()).collect(),
            temp: false,
            run: BTreeSet::new(),
            run_env: BTreeSet::new(),
        }
    }

    fn runs(file: &str, args: &[&str]) -> Run {
        Run::Runs(
            file.to_owned(),
            args.iter().map(|&arg| arg.to_owned()).collect(),
        )
    }

    fn option(option: &str) -> Run {
        Run::Option(option.to_owned())
    }

    /// `run` reads fgro's options up to the file, the first argument not
    /// starting with `-` or the one after `--`, and passes every later
    /// argument to the script as written.
    #[test]
    fn run_parses_its_options_up_to_the_file() {
        use Run::{AllowDir, AllowDirsBare, AllowEnvBare, AllowEnvName, Help, NoFile};
        let cases: Vec<(&[&str], Run)> = vec![
            // the file alone, and the file with the script's arguments
            (&["run", "a.js"], runs("a.js", &[])),
            (&["run", "a.js", "b.js", "c"], runs("a.js", &["b.js", "c"])),
            // arguments that look like fgro's options are the script's
            (
                &[
                    "run",
                    "a.js",
                    "--help",
                    "--x",
                    "-A",
                    "--allow-env=A",
                    "help",
                ],
                runs("a.js", &["--help", "--x", "-A", "--allow-env=A", "help"]),
            ),
            // `--` before the file ends fgro's options, after it is the script's
            (&["run", "--", "-a.js"], runs("-a.js", &[])),
            (&["run", "--", "--"], runs("--", &[])),
            (&["run", "--", "help"], runs("help", &[])),
            (
                &["run", "--", "a.js", "--", "x"],
                runs("a.js", &["--", "x"]),
            ),
            (&["run", "a.js", "--"], runs("a.js", &["--"])),
            // an empty string is an argument like any other
            (&["run", "a.js", "", "x", ""], runs("a.js", &["", "x", ""])),
            (&["run", ""], runs("", &[])),
            // the help trigger before the file, whatever follows it
            (&["run", "--help"], Help),
            (&["run", "help"], Help),
            (&["run", "--help", "a.js", "--x"], Help),
            (&["run", "--help", "--", "a.js"], Help),
            (&["run", "--help", "--"], Help),
            (&["help", "run"], Help),
            (&["--help", "run", "a.js"], Help),
            // no file
            (&["run"], NoFile),
            (&["run", "--"], NoFile),
            // the grant options before the file, the help trigger beside them
            (&["run", "-A", "a.js"], runs("a.js", &[])),
            (
                &["run", "--allow-env=A", "a.js", "-A"],
                runs("a.js", &["-A"]),
            ),
            (
                &["run", "--allow-env=A", "-A", "--", "-a.js", "--allow-env="],
                runs("-a.js", &["--allow-env="]),
            ),
            (&["run", "-A", "--help", "a.js"], Help),
            (&["run", "--allow-env=A", "help"], Help),
            // an option before the file, named as written
            (&["run", "--x", "a.js"], option("--x")),
            (&["run", "-AB", "a.js"], option("-AB")),
            (&["run", "-a", "a.js"], option("-a")),
            (&["run", "--allow-all", "a.js"], option("--allow-all")),
            (&["run", "--allow-envA", "a.js"], option("--allow-envA")),
            (
                &["run", "--allow-reads=.", "a.js"],
                option("--allow-reads=."),
            ),
            (&["run", "--allow-readx", "a.js"], option("--allow-readx")),
            (
                &["run", "--allow-writes=.", "a.js"],
                option("--allow-writes=."),
            ),
            (&["run", "--allow-writex", "a.js"], option("--allow-writex")),
            (&["run", "--allow-envx=A", "a.js"], option("--allow-envx=A")),
            (&["run", "--allow-net=x", "a.js"], option("--allow-net=x")),
            (&["run", "-"], option("-")),
            (&["run", "--help", "--x"], option("--x")),
            (&["run", "--x", "--help"], option("--x")),
            // `--allow-env` with no names, and a name no variable can have
            (&["run", "--allow-env", "a.js"], AllowEnvBare),
            (&["run", "--allow-env", "A", "a.js"], AllowEnvBare),
            (&["run", "--help", "--allow-env"], AllowEnvBare),
            (&["run", "-A", "--allow-env", "a.js"], AllowEnvBare),
            (
                &["run", "--allow-env=", "a.js"],
                AllowEnvName(EnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-env=A,", "a.js"],
                AllowEnvName(EnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-env=,A", "a.js"],
                AllowEnvName(EnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-env=A,,B", "a.js"],
                AllowEnvName(EnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-env=A=B", "a.js"],
                AllowEnvName(EnvFlagFault::Equals),
            ),
            (
                &["run", "--allow-env==", "a.js"],
                AllowEnvName(EnvFlagFault::Equals),
            ),
            (
                &["run", "--allow-env=A\0", "a.js"],
                AllowEnvName(EnvFlagFault::Nul),
            ),
            (
                &["run", "--allow-env=A,B*", "a.js"],
                AllowEnvName(EnvFlagFault::Glob("B*".to_owned())),
            ),
            (
                &["run", "-A", "--allow-env=*", "a.js"],
                AllowEnvName(EnvFlagFault::Glob("*".to_owned())),
            ),
            // the first fault in the order the arguments come
            (
                &["run", "--allow-env=A=1", "--x", "a.js"],
                AllowEnvName(EnvFlagFault::Equals),
            ),
            (&["run", "--x", "--allow-env=", "a.js"], option("--x")),
            (&["run", "--allow-env=A"], NoFile),
            // the directory flags, bare, and listing a path that names nothing
            (
                &["run", "--allow-read=.", "--allow-write=/x,y", "a.js", "-A"],
                runs("a.js", &["-A"]),
            ),
            (
                &["run", "--allow-read", "a.js"],
                AllowDirsBare(ListFlag::Read),
            ),
            (
                &["run", "--allow-write", "a.js"],
                AllowDirsBare(ListFlag::Write),
            ),
            (
                &["run", "-A", "--allow-write", "a.js"],
                AllowDirsBare(ListFlag::Write),
            ),
            (
                &["run", "--allow-run-in", "a.js"],
                AllowDirsBare(ListFlag::RunIn),
            ),
            // the temp flag: bare, never given a value, a script's after the file
            (&["run", "--allow-temp", "a.js"], runs("a.js", &[])),
            (
                &["run", "a.js", "--allow-temp"],
                runs("a.js", &["--allow-temp"]),
            ),
            (&["run", "--allow-temp=x", "a.js"], Run::AllowTempValue),
            (&["run", "--allow-temp=", "a.js"], Run::AllowTempValue),
            (
                &["run", "-A", "--allow-temp=1", "a.js"],
                Run::AllowTempValue,
            ),
            (&["run", "--allow-temps", "a.js"], option("--allow-temps")),
            (
                &["run", "--allow-read=", "a.js"],
                AllowDir(ListFlag::Read, DirFlagFault::Empty),
            ),
            (
                &["run", "--allow-write=a,", "a.js"],
                AllowDir(ListFlag::Write, DirFlagFault::Empty),
            ),
            (
                &["run", "--allow-read=,a", "a.js"],
                AllowDir(ListFlag::Read, DirFlagFault::Empty),
            ),
            (
                &["run", "--allow-read=a,,b", "a.js"],
                AllowDir(ListFlag::Read, DirFlagFault::Empty),
            ),
            (
                &["run", "--allow-write=a\0", "a.js"],
                AllowDir(ListFlag::Write, DirFlagFault::Nul),
            ),
            (
                &["run", "--allow-read=a", "--allow-write=", "--x", "a.js"],
                AllowDir(ListFlag::Write, DirFlagFault::Empty),
            ),
            (&["run", "--allow-write=a"], NoFile),
            // the run flags: bare, a name no program can be granted by, and
            // the variables' flag only beside the programs'
            (
                &[
                    "run",
                    "--allow-run=git",
                    "--allow-run-env=A,B_*",
                    "a.js",
                    "x",
                ],
                runs("a.js", &["x"]),
            ),
            (&["run", "--allow-run", "a.js"], Run::AllowRunBare),
            (&["run", "-A", "--allow-run", "a.js"], Run::AllowRunBare),
            (
                &["run", "--allow-run=git", "--allow-run-env", "a.js"],
                Run::AllowRunEnvBare,
            ),
            (
                &["run", "--allow-runs=git", "a.js"],
                option("--allow-runs=git"),
            ),
            (
                &["run", "--allow-run-envs=A", "a.js"],
                option("--allow-run-envs=A"),
            ),
            (
                &["run", "--allow-run=", "a.js"],
                Run::AllowRunName(RunFlagFault::Empty),
            ),
            (
                &["run", "--allow-run=a,", "a.js"],
                Run::AllowRunName(RunFlagFault::Empty),
            ),
            (
                &["run", "--allow-run=a\0", "a.js"],
                Run::AllowRunName(RunFlagFault::Nul),
            ),
            (
                &["run", "--allow-run=*", "a.js"],
                Run::AllowRunName(RunFlagFault::Glob("*".to_owned())),
            ),
            (
                &["run", "--allow-run=git*", "a.js"],
                Run::AllowRunName(RunFlagFault::Glob("git*".to_owned())),
            ),
            (
                &["run", "--allow-run=/usr/bin/git", "a.js"],
                Run::AllowRunName(RunFlagFault::Absolute("/usr/bin/git".to_owned())),
            ),
            (
                &["run", "--allow-run=../x/tool", "a.js"],
                Run::AllowRunName(RunFlagFault::Climb("../x/tool".to_owned())),
            ),
            (
                &["run", "--allow-run=bin/../../tool", "a.js"],
                Run::AllowRunName(RunFlagFault::Climb("bin/../../tool".to_owned())),
            ),
            (
                &["run", "--allow-run=bin/../tool,./b/./c,..x,a..", "a.js"],
                runs("a.js", &[]),
            ),
            (
                &["run", "--allow-run=git", "--allow-run-env=", "a.js"],
                Run::AllowRunEnvName(RunEnvFlagFault::Empty),
            ),
            (
                &["run", "--allow-run=git", "--allow-run-env=A=1", "a.js"],
                Run::AllowRunEnvName(RunEnvFlagFault::Equals),
            ),
            (
                &["run", "--allow-run=git", "--allow-run-env=A\0", "a.js"],
                Run::AllowRunEnvName(RunEnvFlagFault::Nul),
            ),
            (
                &["run", "--allow-run=git", "--allow-run-env=*", "a.js"],
                Run::AllowRunEnvName(RunEnvFlagFault::GlobAlone),
            ),
            (
                &["run", "--allow-run=git", "--allow-run-env=A*B", "a.js"],
                Run::AllowRunEnvName(RunEnvFlagFault::GlobNotLast("A*B".to_owned())),
            ),
            (
                &["run", "--allow-run=git", "--allow-run-env=A**", "a.js"],
                Run::AllowRunEnvName(RunEnvFlagFault::GlobNotLast("A**".to_owned())),
            ),
            (&["run", "--allow-run-env=A", "a.js"], Run::AllowRunEnvAlone),
            (
                &["run", "-A", "--allow-run-env=A", "a.js"],
                Run::AllowRunEnvAlone,
            ),
            (
                &["run", "--help", "--allow-run-env=A"],
                Run::AllowRunEnvAlone,
            ),
            (&["run", "--allow-run-env=A"], Run::AllowRunEnvAlone),
            (&["run", "--allow-run-env=A", "--allow-run=git"], NoFile),
            // the write flag of one program: its program then `:` then its
            // directories, only beside --allow-run naming the program
            (
                &[
                    "run",
                    "--allow-run=x",
                    "--allow-run-write=x:out,a:b",
                    "a.js",
                ],
                runs("a.js", &[]),
            ),
            (
                &["run", "--allow-run-write=x:out", "--allow-run=y,x", "a.js"],
                runs("a.js", &[]),
            ),
            (
                &["run", "a.js", "--allow-run-write=x:out"],
                runs("a.js", &["--allow-run-write=x:out"]),
            ),
            (
                &["run", "--allow-run-write", "a.js"],
                Run::AllowRunWriteBare,
            ),
            (
                &["run", "--allow-run=x", "--allow-run-writes=x:a", "a.js"],
                option("--allow-run-writes=x:a"),
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=", "a.js"],
                Run::AllowRunWriteShape,
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=out", "a.js"],
                Run::AllowRunWriteShape,
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=:out", "a.js"],
                Run::AllowRunWriteShape,
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=x:", "a.js"],
                Run::AllowRunWriteShape,
            ),
            (
                &[
                    "run",
                    "--allow-run=x,y",
                    "--allow-run-write=x,y:out",
                    "a.js",
                ],
                Run::AllowRunWriteShape,
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=x*:out", "a.js"],
                Run::AllowRunWriteProgram(RunFlagFault::Glob("x*".to_owned())),
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=/x:out", "a.js"],
                Run::AllowRunWriteProgram(RunFlagFault::Absolute("/x".to_owned())),
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=../x:out", "a.js"],
                Run::AllowRunWriteProgram(RunFlagFault::Climb("../x".to_owned())),
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=x\0:out", "a.js"],
                Run::AllowRunWriteProgram(RunFlagFault::Nul),
            ),
            (
                &["run", "--allow-run=x", "--allow-run-write=x:a,,b", "a.js"],
                AllowDir(ListFlag::RunWrite, DirFlagFault::Empty),
            ),
            (
                &["run", "--allow-run-write=x:out", "a.js"],
                Run::AllowRunWriteAlone,
            ),
            (
                &[
                    "run",
                    "--allow-run-in=bin",
                    "--allow-run-write=x:out",
                    "a.js",
                ],
                Run::AllowRunWriteAlone,
            ),
            (
                &["run", "-A", "--allow-run-write=x:out", "a.js"],
                Run::AllowRunWriteAlone,
            ),
            (
                &["run", "--allow-run=y", "--allow-run-write=x:out", "a.js"],
                Run::UngrantedRunWrite("x".to_owned()),
            ),
            (
                &[
                    "run",
                    "-A",
                    "--allow-run=y",
                    "--allow-run-write=x:out",
                    "a.js",
                ],
                Run::UngrantedRunWrite("x".to_owned()),
            ),
        ];
        for (args, expected) in cases {
            assert_eq!(run_outcome(args), expected, "{args:?}");
        }
    }

    /// With neither grant option `run` grants the env host module no name;
    /// `--allow-env=`, repeatable, grants the names it lists, one set,
    /// as written, case and all; `-A`, wherever it comes before the file,
    /// grants everything, and the names beside it add nothing.
    #[test]
    fn run_s_grant_options_name_what_they_grant() {
        assert_eq!(grant(&["a.js"]), env(&[]));
        assert_eq!(grant(&["a.js", "-A", "--allow-env=A"]), env(&[]));
        assert_eq!(grant(&["--allow-env=A", "a.js"]), env(&["A"]));
        assert_eq!(
            grant(&[
                "--allow-env=b,A,a",
                "--allow-env=A",
                "--allow-env=Ä, x",
                "a.js"
            ]),
            RunGrant::Named {
                env: BTreeSet::from(["A", "a", "b", "Ä", " x"].map(ToOwned::to_owned)),
                temp: false,
                run: BTreeSet::new(),
                run_env: BTreeSet::new(),
            }
        );
        assert_eq!(
            grant(&["--allow-temp", "--allow-env=A", "a.js"]),
            RunGrant::Named {
                env: BTreeSet::from(["A".to_owned()]),
                temp: true,
                run: BTreeSet::new(),
                run_env: BTreeSet::new(),
            }
        );
        for args in [
            &["-A", "a.js"][..],
            &["--allow-temp", "-A", "a.js"],
            &["--allow-env=A", "-A", "a.js"],
            &["-A", "--allow-env=A", "a.js"],
            &["-A", "-A", "--", "a.js"],
        ] {
            assert_eq!(grant(args), RunGrant::All, "{args:?}");
        }
    }

    /// `--allow-run=` and `--allow-run-env=`, each repeatable, grant the
    /// names they list, one set each, as written, the variables' as
    /// patterns; under `-A` they're checked and add nothing.
    #[test]
    fn the_run_flags_name_the_programs_and_variables_they_grant() {
        use gro_runtime::EnvPattern;
        assert_eq!(
            grant(&[
                "--allow-run=git,cargo",
                "--allow-run-env=CARGO_*,HOME",
                "--allow-run=./bin/tool,git",
                "--allow-run-env=RUSTUP_*",
                "a.js",
            ]),
            RunGrant::Named {
                env: BTreeSet::new(),
                temp: false,
                run: BTreeSet::from(["./bin/tool", "cargo", "git"].map(ToOwned::to_owned)),
                run_env: BTreeSet::from([
                    EnvPattern::Prefix("CARGO_".to_owned()),
                    EnvPattern::Exact("HOME".to_owned()),
                    EnvPattern::Prefix("RUSTUP_".to_owned()),
                ]),
            }
        );
        assert_eq!(
            grant(&["--allow-run=git", "-A", "--allow-run-env=A", "a.js"]),
            RunGrant::All
        );
    }

    /// The directory flags, each repeatable, list their directories as
    /// written, one set each, a path both read and written in both; under
    /// `-A` too, where the run still checks them.
    #[test]
    fn the_directory_flags_list_their_directories() {
        let dirs = |args: &[&str]| {
            let args = [&["run"][..], args].concat();
            match parse(&args) {
                Ok(Invocation::Sized(Task::Run(run))) => (run.grant, run.dirs),
                _ => panic!("{args:?} runs a file"),
            }
        };
        let set = |dirs: &[&str]| dirs.iter().map(|&dir| dir.to_owned()).collect();
        assert_eq!(dirs(&["a.js"]), (env(&[]), FlagDirs::default()));
        assert_eq!(
            dirs(&[
                "--allow-read=b,/a",
                "--allow-write=b",
                "--allow-read=./c, d,/a",
                "--allow-run-in=x,b",
                "--allow-run-in=x",
                "a.js",
                "--allow-read=e",
            ]),
            (
                RunGrant::Named {
                    env: BTreeSet::new(),
                    temp: false,
                    run: BTreeSet::new(),
                    run_env: BTreeSet::new(),
                },
                FlagDirs {
                    read: set(&["/a", " d", "./c", "b"]),
                    write: set(&["b"]),
                    run_in: set(&["b", "x"]),
                    run_write: BTreeMap::new(),
                }
            )
        );
        assert_eq!(
            dirs(&["--allow-write=w", "--allow-run-in=w", "-A", "a.js"]),
            (
                RunGrant::All,
                FlagDirs {
                    read: set(&[]),
                    write: set(&["w"]),
                    run_in: set(&["w"]),
                    run_write: BTreeMap::new(),
                }
            )
        );
        // the variables' flag passes on to programs an exec root finds
        assert!(matches!(
            parse(&["run", "--allow-run-in=x", "--allow-run-env=A", "a.js"]),
            Ok(Invocation::Sized(Task::Run(_)))
        ));
    }

    /// The usage errors of the grant options say what was wrong without
    /// echoing what follows a `=`, which can be a value, and point at the
    /// help; exit 2, as every usage error.
    #[test]
    fn the_grant_options_usage_errors_echo_no_value() {
        let message = |args: &[&str]| match parse(&[&["run"][..], args].concat()) {
            Err(error) => {
                assert_eq!(error.exit_code(), crate::EXIT_USAGE, "{args:?}");
                error.to_string()
            }
            Ok(_) => panic!("{args:?} is a usage error"),
        };
        let help = format!("\nRun {BINARY_NAME} run --help for more information.");
        assert_eq!(
            message(&["--allow-env", "a.js"]),
            format!("--allow-env takes the names it grants, as --allow-env=A,B{help}")
        );
        assert_eq!(
            message(&["--allow-env=TOKEN=hunter2", "a.js"]),
            format!("an --allow-env name holds a `=`, which no variable's name holds{help}")
        );
        assert_eq!(
            message(&["--allow-env=A,", "a.js"]),
            format!(
                "an --allow-env name is empty: --allow-env=A,B lists names separated by single \
                 commas{help}"
            )
        );
        assert_eq!(
            message(&["--allow-env=AWS_*", "a.js"]),
            format!("an --allow-env name is exact, never a glob: AWS_*{help}")
        );
        assert_eq!(
            message(&["--allow-env=\u{1b}[2J\u{202e}*", "a.js"]),
            format!("an --allow-env name is exact, never a glob: \\u{{1b}}[2J\\u{{202e}}*{help}")
        );
        assert_eq!(
            message(&["--allow-env=A\0", "a.js"]),
            format!("an --allow-env name holds a NUL byte{help}")
        );
        assert_eq!(
            message(&["--allow-read", "a.js"]),
            format!("--allow-read takes the directories it grants, as --allow-read=a,/b{help}")
        );
        assert_eq!(
            message(&["--allow-write", "a.js"]),
            format!("--allow-write takes the directories it grants, as --allow-write=a,/b{help}")
        );
        assert_eq!(
            message(&["--allow-write=a,", "a.js"]),
            format!(
                "a --allow-write directory is empty: --allow-write=a,/b lists directories \
                 separated by single commas{help}"
            )
        );
        assert_eq!(
            message(&["--allow-read=a\0", "a.js"]),
            format!("a --allow-read directory holds a NUL byte{help}")
        );
        assert_eq!(
            message(&["--allow-run", "a.js"]),
            format!("--allow-run takes the programs it grants, as --allow-run=git,cargo{help}")
        );
        assert_eq!(
            message(&["--allow-run=a,,b", "a.js"]),
            format!(
                "an --allow-run name is empty: --allow-run=a,b lists names separated by single \
                 commas{help}"
            )
        );
        assert_eq!(
            message(&["--allow-run=\u{1b}[2J*", "a.js"]),
            format!("an --allow-run name is exact, never a glob: \\u{{1b}}[2J*{help}")
        );
        assert_eq!(
            message(&["--allow-run=/bin/sh", "a.js"]),
            format!(
                "an --allow-run name is found on PATH, or beneath the project's root when it \
                 holds /, never absolute: /bin/sh{help}"
            )
        );
        assert_eq!(
            message(&["--allow-run=../sh", "a.js"]),
            format!(
                "an --allow-run name holding / resolves beneath the project's root, and this one \
                 climbs above it: ../sh{help}"
            )
        );
        assert_eq!(
            message(&["--allow-run=git", "--allow-run-env", "a.js"]),
            format!(
                "--allow-run-env takes the variables it lets programs be passed, as \
                 --allow-run-env=A,B_*{help}"
            )
        );
        assert_eq!(
            message(&["--allow-run=git", "--allow-run-env=TOKEN=hunter2", "a.js"]),
            format!("an --allow-run-env name holds a `=`, which no variable's name holds{help}")
        );
        assert_eq!(
            message(&["--allow-run=git", "--allow-run-env=*", "a.js"]),
            format!(
                "an --allow-run-env glob needs a prefix: * alone would pass every variable{help}"
            )
        );
        assert_eq!(
            message(&["--allow-run=git", "--allow-run-env=A*B", "a.js"]),
            format!("an --allow-run-env glob ends in its one *, after a prefix: A*B{help}")
        );
        assert_eq!(
            message(&["--allow-run-env=A", "a.js"]),
            format!(
                "--allow-run-env lets the programs --allow-run and --allow-run-in grant be passed \
                 variables, and neither grants one{help}"
            )
        );
        assert_eq!(
            message(&["--allow-run-in", "a.js"]),
            format!("--allow-run-in takes the directories it grants, as --allow-run-in=a,/b{help}")
        );
        assert_eq!(
            message(&["--allow-run-in=a,,b", "a.js"]),
            format!(
                "a --allow-run-in directory is empty: --allow-run-in=a,/b lists directories \
                 separated by single commas{help}"
            )
        );
    }

    /// `fgro run --help`, written by hand, reads as argh writes the other
    /// commands' help, and `fgro --help` lists `run` first, as described
    /// there.
    #[test]
    fn run_s_help_reads_as_argh_s() {
        const EXPECTED: &str = "\
Usage: {BINARY_NAME} run [--allow-env=<names>] [--allow-read=<dirs>] [--allow-write=<dirs>] [--allow-temp] [--allow-run=<names>] [--allow-run-in=<dirs>] [--allow-run-env=<names>] [--allow-run-write=<program>:<dirs>] [-A] [--] <file> [<args...>]

run an ES module (.js, .mjs, .ts, or .mts) from disk

Positional Arguments:
  file              the module to run: a path relative to the working directory,
                    or absolute
  args              the script's arguments, every one after the file as written,
                    flags and `--` included, read from `@fuzdev/gro/args.ts`

Options:
  --allow-env=<names>
                    grant the project's modules `@fuzdev/gro/env.ts` with these
                    environment variables, a comma-separated list of exact names;
                    repeatable
  --allow-read=<dirs>
                    grant the project's modules `@fuzdev/gro/fs.ts` reading these
                    directories, a comma-separated list of existing ones,
                    relative to the working directory or absolute; repeatable;
                    the project's root is read without it
  --allow-write=<dirs>
                    grant the project's modules `@fuzdev/gro/fs.ts` writing these
                    directories, as --allow-read lists them; a directory only
                    written isn't read
  --allow-temp      grant the project's modules `@fuzdev/gro/fs.ts` making fresh
                    directories under the OS's temporary directory, each read
                    and written by the run, with fs.temp_dir; nothing else of
                    that directory
  --allow-run=<names>
                    grant the project's modules `@fuzdev/gro/spawn.ts` making
                    these programs, a comma-separated list of exact names, each
                    found on PATH, or beneath the project's root when it holds
                    `/`, and pinned as the run starts; repeatable. Until each
                    child is boxed, granting any program is close to granting
                    what -A grants: most can be made to run code by their
                    arguments or environment, a shell, git, and cargo among
                    them, and the root's env values set any variable; hand
                    library code a narrowed root, which refuses env values
  --allow-run-in=<dirs>
                    grant the project's modules `@fuzdev/gro/spawn.ts` making
                    programs found beneath these directories with program_in
                    and find_in, as --allow-read lists them, each held as the
                    run starts; repeatable. A directory also granted by
                    --allow-write runs what the run writes there; as for
                    --allow-run, granting one is close to granting what -A
                    grants
  --allow-run-env=<names>
                    let the programs --allow-run grants, and those found
                    beneath what --allow-run-in grants, be passed these
                    environment variables, when a program's pass_env names
                    them, a comma-separated list of exact names or PREFIX_*
                    globs; repeatable; only beside --allow-run or
                    --allow-run-in
  --allow-run-write=<program>:<dirs>
                    let one program --allow-run grants write these directories:
                    its name, `:`, then directories as --allow-write lists them;
                    repeatable, one program each, only beside --allow-run.
                    `{BINARY_NAME} run` starts programs in no box, so here the flag is
                    checked and holds nothing back
  -A                run unconfined: every grant at its widest, to every module,
                    every environment variable and every directory among them
  --help, help      display usage information";
        assert_eq!(run_help(), EXPECTED.replace("{BINARY_NAME}", BINARY_NAME));
        let Ok(Invocation::Sized(Task::Help(eval))) = parse(&["eval", "--help"]) else {
            panic!("eval --help prints its help");
        };
        // argh's own lines for the help trigger and a positional's wrapping
        assert_eq!(eval.trim_end().lines().last(), run_help().lines().last());
        assert!(
            eval.contains("\nPositional Arguments:\n  module            the module to evaluate")
        );
        let Ok(Invocation::Sized(Task::Help(top))) = parse(&["--help"]) else {
            panic!("--help prints the help");
        };
        let run = top.find(&format!("\n  run               {RUN_DESCRIPTION}\n"));
        let eval = top.find("\n  eval              evaluate an ES module");
        assert!(run.is_some() && run < eval, "{top}");
    }

    /// A usage error argh refused prints no part of an argument past its
    /// first `=` and no raw character [`crate::print::is_escaped`] names,
    /// whatever the arguments: lists of every size up to thousands, of
    /// arguments empty to very long, of controls, line breaks, bidi marks,
    /// and non-ASCII, each value after a first `=` made of characters
    /// nothing else holds, so one shown anywhere is a value shown. Its one
    /// line break is gro's own, between its two lines, and its length is
    /// bounded.
    #[test]
    fn a_refused_argument_list_prints_no_value_and_no_raw_control() {
        use super::{MAX_SHOWN_ARGUMENT, MAX_SHOWN_ARGUMENTS, invalid_arguments};
        const PLAIN: &[char] = &[
            'a',
            'Z',
            '-',
            ' ',
            '\u{e9}',
            '\u{1f600}',
            '\u{1b}',
            '\n',
            '\r',
            '\t',
            '\0',
            '\u{7f}',
            '\u{9b}',
            '\u{202e}',
            '\u{2066}',
            '\u{200e}',
            '"',
            '\u{2026}',
            ':',
            '[',
        ];
        // what follows a first `=`: any of PLAIN, another `=`, and three
        // characters only a value holds
        const MARKS: &[char] = &['\u{5024}', '\u{79d8}', '\u{5bc6}'];
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = |bound: usize| -> usize {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % u64::try_from(bound).unwrap()).unwrap()
        };
        let text = |len: usize, value: bool, next: &mut dyn FnMut(usize) -> usize| -> String {
            (0..len)
                .map(|_| {
                    if value && next(3) == 0 {
                        MARKS[next(MARKS.len())]
                    } else if value && next(8) == 0 {
                        '='
                    } else {
                        PLAIN[next(PLAIN.len())]
                    }
                })
                .collect()
        };
        for round in 0..300 {
            let count = [0, 1, 2, 5, 16, 17, 40, 3000][round % 8];
            let args: Vec<String> = (0..count)
                .map(|_| {
                    let len = [0, 1, 3, 40, 200, 5000][next(6)];
                    let head = text(len, false, &mut next);
                    if next(2) == 0 {
                        let value_len = [0, 1, 20, 3000][next(4)];
                        // at least one mark, so the value is never all plain
                        format!("{head}={}{}", MARKS[0], text(value_len, true, &mut next))
                    } else {
                        head
                    }
                })
                .collect();
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let message = invalid_arguments(&args).to_string();
            assert!(
                !message.contains(MARKS),
                "{round}: a value was printed: {message:?}"
            );
            assert_eq!(message.matches('\n').count(), 1, "{round}: {message:?}");
            assert!(
                !message
                    .chars()
                    .any(|c| c != '\n' && crate::print::is_escaped(c)),
                "{round}: a raw control: {message:?}"
            );
            let bound = MAX_SHOWN_ARGUMENTS * (MAX_SHOWN_ARGUMENT + 2) + 200;
            assert!(
                message.chars().count() <= bound,
                "{round}: {}",
                message.len()
            );
        }
    }

    /// argh's refusals print gro's own line naming the command, then the
    /// arguments as [`echo_argument`] prints them, and the help pointer:
    /// an argument crafted to look like argh's own words, to run into the
    /// next argument, to be part of another, or to forge a line changes
    /// nothing of that.
    #[test]
    fn a_refusal_by_argh_is_made_of_the_arguments() {
        let message = |args: &[&str]| match parse(args) {
            Err(error @ CliError::InvalidArguments { .. }) => error.to_string(),
            _ => panic!("{args:?} is refused by argh"),
        };
        assert_eq!(
            message(&["eval", "--export", ": T=", "a.js", "T=hunter2"]),
            format!(
                "invalid arguments to {BINARY_NAME} eval: eval --export : T=\u{2026} a.js \
                 T=\u{2026}\nRun {BINARY_NAME} eval --help for more information."
            )
        );
        assert_eq!(
            message(&[
                "eval",
                "--export",
                "argument: a=b",
                "a.js",
                "a=b\nerror: fake second line"
            ]),
            format!(
                "invalid arguments to {BINARY_NAME} eval: eval --export argument: a=\u{2026} \
                 a.js a=\u{2026}\nRun {BINARY_NAME} eval --help for more information."
            )
        );
        assert_eq!(
            message(&["eval", "--export", "=", "a.js", "TOKEN=hunter2"]),
            format!(
                "invalid arguments to {BINARY_NAME} eval: eval --export =\u{2026} a.js \
                 TOKEN=\u{2026}\nRun {BINARY_NAME} eval --help for more information."
            )
        );
        assert_eq!(
            message(&["eval", "--export", "x=", "a.js", "x=\u{1b}[2Jsecret"]),
            format!(
                "invalid arguments to {BINARY_NAME} eval: eval --export x=\u{2026} a.js \
                 x=\u{2026}\nRun {BINARY_NAME} eval --help for more information."
            )
        );
        assert_eq!(
            message(&["--version", "--unknown", "-\u{1b}[2J", ""]),
            format!(
                "invalid arguments to {BINARY_NAME}: --version --unknown -\\u{{1b}}[2J \"\"\nRun \
                 {BINARY_NAME} --help for more information."
            )
        );
        let many: Vec<String> = std::iter::once("--version".to_owned())
            .chain((0..19).map(|i| format!("--x{i}")))
            .collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(
            message(&many).contains(" --x14 \u{2026} (4 more)\n"),
            "{}",
            message(&many)
        );
        let long = format!("--{}", "y".repeat(200));
        assert!(
            message(&["--version", &long]).contains(&format!("--{}\u{2026}\n", "y".repeat(78)))
        );
    }

    /// The binary's help names a command of gro's in its notes.
    #[test]
    fn the_binary_s_help_names_a_command_of_gro_s() {
        let Ok(Invocation::Sized(Task::Help(help))) = parse(&["--help"]) else {
            panic!("--help prints the help");
        };
        assert!(
            help.contains(&format!(
                "Notes:\n  {BINARY_NAME} <command> runs a command of gro's, which {BINARY_NAME} \
                 alone lists"
            )),
            "{help}"
        );
    }

    /// `--timeout`'s help names the default eval takes.
    #[test]
    fn the_timeout_s_help_names_the_default() {
        let Ok(Invocation::Sized(Task::Help(help))) = parse(&["eval", "--help"]) else {
            panic!("eval --help prints its help");
        };
        let default = format!(
            "(default: {})",
            gro_runtime::eval::DEFAULT_TIMEOUT.as_millis()
        );
        assert!(help.contains(&default), "{help}");
    }

    #[test]
    fn a_grant_flag_is_its_name_exactly_up_to_any_list() {
        use super::{GrantFlag, ListFlag};
        assert_eq!(GrantFlag::parse("-A"), Some(GrantFlag::All));
        assert_eq!(GrantFlag::parse("--allow-temp"), Some(GrantFlag::Temp));
        assert_eq!(
            GrantFlag::parse("--allow-temp="),
            Some(GrantFlag::TempWithValue)
        );
        assert_eq!(
            GrantFlag::parse("--allow-run"),
            Some(GrantFlag::List(ListFlag::Run, None))
        );
        assert_eq!(
            GrantFlag::parse("--allow-run-in="),
            Some(GrantFlag::List(ListFlag::RunIn, Some("")))
        );
        assert_eq!(
            GrantFlag::parse("--allow-run-env=A=B,C"),
            Some(GrantFlag::List(ListFlag::RunEnv, Some("A=B,C")))
        );
        for flag in ListFlag::ALL {
            let listed = format!("{}=x", flag.spelled());
            assert_eq!(
                GrantFlag::parse(&listed),
                Some(GrantFlag::List(flag, Some("x")))
            );
        }
        for other in [
            "-A=x",
            "-AA",
            "-a",
            "--allow-",
            "--allow-net",
            "--allow-READ",
            "--allowread",
            "--allow-runx=y",
            "-allow-read=.",
            " --allow-read=.",
            "--allow-read =.",
            "--allow-read\u{a0}=.",
            "--allow\u{2010}read=.",
            "--allow-re\u{301}ad=.",
            "--help",
            "x",
        ] {
            assert_eq!(GrantFlag::parse(other), None, "{other}");
        }
    }
}
