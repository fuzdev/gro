//! The verb's grant flags, split off its command line: the twin of Gro's
//! `to_grant_flags` (`src/lib/task_args.ts`), held to
//! `tests/fixtures/verb/grant_flags.json`, which that function writes.
//!
//! The flags are the run of them right after the command word, spelled as
//! `fgro run` spells them before its file ([`GrantFlag`]), one grammar for
//! both. Only their spelling is checked here: what each lists is the
//! grant's to check, as `fgro run` checks it. Anything after the run is the
//! command's, so a flag spelled `--allow-*` there is a task's arg, and
//! everything from the first `--` is forwarded as written. A `--help` in the
//! run is kept for the command, as `fgro run` takes it among its flags.
//! What the flags list is read and checked by [`to_run_grant`], through the
//! reader `fgro run`'s flags go through ([`RunGrantBuilder`]).
//!
//! Kind: the native parse of the trusted command line: it refuses a slip (a
//! misplaced or misspelled flag) rather than let it quietly become a task's
//! arg or a grant, and bounds nothing against the project, which doesn't
//! write the command line.

use gro_runtime::BINARY_NAME;
use gro_runtime::verb::closure::{RUN_TASK_NAME, is_task_path_input};
use thiserror::Error;

use super::prepass::node_gro_command;
use crate::cli::{
    ALLOW_ALL, ALLOW_RUN, ALLOW_RUN_ENV, ALLOW_RUN_IN, ALLOW_RUN_WRITE, ALLOW_TEMP, FlagDirs,
    GrantFlag, GrantListError, ListFlag, RUN, RunGrant, RunGrantBuilder,
};
use crate::print::{echo_argument, escape_controls};

/// A command line with its grant flags split off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantFlagsSplit {
    /// The grant flags, as written, in order.
    pub grant_flags: Vec<String>,
    /// The command line without them, everything else as written, a
    /// `--help` among them kept right after the command word.
    pub cli_args: Vec<String>,
}

/// The refusal of `flag` before the command word `command` (none when no
/// argument names one): where grant flags go, and for `-A` before a
/// command of Gro's, the spelling that runs unconfined, the whole command
/// line `rest` on the Node Gro; `run` takes `-A` after its word.
fn before_command(flag: &str, command: Option<&str>, rest: &[String]) -> String {
    let boxed = flag == "-A" && command != Some(RUN);
    if boxed {
        format!(
            "-A comes before the command, and the binary runs each command of Gro's in a kernel \
             box, which -A can't lift: the project's Node gro is the unconfined spelling ({})",
            node_gro_command(rest)
        )
    } else {
        let command = command.map_or_else(|| "<command>".into(), escape_controls);
        format!(
            "{} comes before the command: grant flags go right after the command word, like \
             `{BINARY_NAME} {command} {}`",
            echo_argument(flag),
            echo_argument(flag)
        )
    }
}

/// Why a command line's grant flags are refused. Each names the flag as
/// [`echo_argument`] prints it, up to its first `=`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GrantFlagsError {
    /// A grant flag before the command word, where it would be the CLI's
    /// arg, or Gro's. `-A` is named apart: right after the command word it
    /// is refused too, for every command the binary runs in its box, the
    /// project's Node Gro being the unconfined spelling.
    #[error(
        "{}",
        before_command(.flag, .command.as_deref(), .rest)
    )]
    BeforeCommand {
        /// The flag, as written.
        flag: String,
        /// The first argument naming no flag before any `--`, which the
        /// hint puts first, or none.
        command: Option<String>,
        /// The command line without the grant flags before the command
        /// word, which the hint for `-A` names on the Node Gro.
        rest: Vec<String>,
    },
    /// `--allow-temp` given a value.
    #[error("{ALLOW_TEMP} takes no value: {}", echo_argument(.flag))]
    TakesNoValue {
        /// The flag, as written.
        flag: String,
    },
    /// A flag that lists, with no list or an empty one.
    #[error(
        "{name} takes a list after `=`, like {name}={example}: {}",
        echo_argument(.flag),
        name = .list_flag.spelled(),
        example = .list_flag.example()
    )]
    TakesAList {
        /// The flag, as written.
        flag: String,
        /// Which flag it is.
        list_flag: ListFlag,
    },
    /// An argument spelled `--allow-*` in the run that's no grant flag.
    #[error("{} is no grant flag: they're {}", echo_argument(.flag), flag_names())]
    NoGrantFlag {
        /// The argument, as written.
        flag: String,
    },
    /// `run` given `--allow-run-env` with neither `--allow-run` nor
    /// `--allow-run-in`, whose programs alone it passes variables to.
    #[error(
        "{ALLOW_RUN_ENV} passes variables to the programs {ALLOW_RUN} or {ALLOW_RUN_IN} grants, \
         and `{BINARY_NAME} {RUN_TASK_NAME}` was given neither"
    )]
    RunEnvWithoutRun,
    /// `run` given `--allow-run-write` with no `--allow-run`, whose
    /// programs alone it lets write.
    #[error(
        "{ALLOW_RUN_WRITE} lets a program {ALLOW_RUN} grants write, and `{BINARY_NAME} \
         {RUN_TASK_NAME}` was given no {ALLOW_RUN}"
    )]
    RunWriteWithoutRun,
    /// A flag's list refused, as `fgro run` refuses it, without its help's
    /// hint.
    #[error(transparent)]
    List(#[from] GrantListError),
}

/// Every grant flag's name, as [`GrantFlagsError::NoGrantFlag`] lists them.
fn flag_names() -> String {
    [ALLOW_ALL, ALLOW_TEMP]
        .into_iter()
        .chain(ListFlag::ALL.map(ListFlag::spelled))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether `arg` is spelled as a grant flag, `-A` or one starting
/// `--allow-`, valid or not.
fn is_grant_flag_like(arg: &str) -> bool {
    arg == ALLOW_ALL || arg.starts_with("--allow-")
}

/// Splits the grant flags off `cli_args`, the args after the CLI's own
/// path, the command word first.
///
/// # Errors
///
/// Returns why the flags are refused: one before the command word, then in
/// the run, a flag spelled `--allow-*` that's no grant flag, one that lists
/// with no list, and `--allow-temp` with a value, each in the order they
/// come, then `run` given `--allow-run-env` alone, then `run` given
/// `--allow-run-write` with no `--allow-run`.
pub fn split_grant_flags(cli_args: &[String]) -> Result<GrantFlagsSplit, GrantFlagsError> {
    for arg in cli_args {
        if arg == "--" || !arg.starts_with('-') {
            break;
        }
        if is_grant_flag_like(arg) {
            let command = cli_args
                .iter()
                .take_while(|arg| *arg != "--")
                .find(|arg| !arg.starts_with('-'))
                .cloned();
            let mut before = true;
            let rest = cli_args
                .iter()
                .filter(|arg| {
                    before &= *arg != "--" && arg.starts_with('-');
                    !(before && is_grant_flag_like(arg))
                })
                .cloned()
                .collect();
            return Err(GrantFlagsError::BeforeCommand {
                flag: arg.clone(),
                command,
                rest,
            });
        }
    }
    let command = cli_args.first();
    // with no command word there's no run, the args starting with a flag, or
    // with a path the CLI refuses there
    let has_command =
        command.is_some_and(|command| !command.starts_with('-') && !is_task_path_input(command));
    let mut end = if has_command { 1 } else { cli_args.len() };
    let mut help_flags = Vec::new();
    let mut grant_flags = Vec::new();
    for arg in cli_args.get(end..).unwrap_or_default() {
        if arg == "--help" {
            help_flags.push(arg.clone());
        } else if is_grant_flag_like(arg) {
            match GrantFlag::parse(arg) {
                Some(GrantFlag::TempWithValue) => {
                    return Err(GrantFlagsError::TakesNoValue { flag: arg.clone() });
                }
                Some(GrantFlag::List(list_flag, None | Some(""))) => {
                    return Err(GrantFlagsError::TakesAList {
                        flag: arg.clone(),
                        list_flag,
                    });
                }
                Some(GrantFlag::All | GrantFlag::Temp | GrantFlag::List(_, Some(_))) => {}
                None => return Err(GrantFlagsError::NoGrantFlag { flag: arg.clone() }),
            }
            grant_flags.push(arg.clone());
        } else {
            break;
        }
        end += 1;
    }
    let lists = |flag: ListFlag| {
        grant_flags
            .iter()
            .any(|arg| matches!(GrantFlag::parse(arg), Some(GrantFlag::List(f, _)) if f == flag))
    };
    if command.is_some_and(|command| command == RUN_TASK_NAME)
        && lists(ListFlag::RunEnv)
        && !lists(ListFlag::Run)
        && !lists(ListFlag::RunIn)
    {
        return Err(GrantFlagsError::RunEnvWithoutRun);
    }
    if command.is_some_and(|command| command == RUN_TASK_NAME)
        && lists(ListFlag::RunWrite)
        && !lists(ListFlag::Run)
    {
        return Err(GrantFlagsError::RunWriteWithoutRun);
    }
    let cli_args = match command {
        Some(command) if !grant_flags.is_empty() => std::iter::once(command.clone())
            .chain(help_flags)
            .chain(cli_args[end..].iter().cloned())
            .collect(),
        _ => cli_args.to_vec(),
    };
    Ok(GrantFlagsSplit {
        grant_flags,
        cli_args,
    })
}

/// What `grant_flags`, the flags [`split_grant_flags`] split off, grant,
/// and the directories they list, each read and checked as `fgro run`
/// reads and checks its own ([`RunGrantBuilder`]). `--allow-run-env=` is
/// taken alone: the split refuses it so for `run`, and it forwards to the
/// programs a command's closure grants by a list. So is
/// `--allow-run-write=`, which names a program of the closure's as often
/// as one `--allow-run=` grants, resolved by the pre-pass.
///
/// # Errors
///
/// Returns [`GrantFlagsError::List`] for the first flag whose list `fgro
/// run` refuses, and [`GrantFlagsError::NoGrantFlag`] for an argument
/// that's no grant flag, which no split leaves.
pub fn to_run_grant(grant_flags: &[String]) -> Result<(RunGrant, FlagDirs), GrantFlagsError> {
    let mut grant = RunGrantBuilder::default();
    for flag in grant_flags {
        let parsed = GrantFlag::parse(flag)
            .ok_or_else(|| GrantFlagsError::NoGrantFlag { flag: flag.clone() })?;
        grant.add(parsed)?;
    }
    Ok(grant.build())
}

#[cfg(test)]
mod tests {
    use gro_runtime::BINARY_NAME;
    use serde_json::Value;

    use std::collections::BTreeSet;

    use gro_runtime::EnvPattern;

    use super::{GrantFlagsError, split_grant_flags, to_run_grant};
    use crate::cli::{DirFlagFault, FlagDirs, GrantListError, ListFlag, RunGrant};
    use crate::verb::tests::{has_fields, read_fixture, strings};

    /// The refusal's kind as the fixture names it, the flag it names, and
    /// for `before_command` the hint's command.
    fn kind(error: &GrantFlagsError) -> (&'static str, Option<&str>, Option<&str>) {
        match error {
            GrantFlagsError::BeforeCommand { flag, command, .. } => {
                ("before_command", Some(flag), command.as_deref())
            }
            GrantFlagsError::TakesNoValue { flag } => ("takes_no_value", Some(flag), None),
            GrantFlagsError::TakesAList { flag, .. } => ("takes_a_list", Some(flag), None),
            GrantFlagsError::NoGrantFlag { flag } => ("no_grant_flag", Some(flag), None),
            GrantFlagsError::RunEnvWithoutRun => ("run_env_without_run", None, None),
            GrantFlagsError::RunWriteWithoutRun => ("run_write_without_run", None, None),
            GrantFlagsError::List(error) => panic!("the split reads no list: {error}"),
        }
    }

    #[test]
    fn the_split_is_to_grant_flags_s() {
        let fixture = read_fixture("grant_flags.json");
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() > 1);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let result = split_grant_flags(&strings(&case["argv"]));
            if let Some(refusal) = case.get("refusal") {
                assert!(
                    has_fields(case, &["name", "argv", "refusal", "error"]),
                    "{name}"
                );
                let error = result.expect_err(name);
                let (kind, flag, command) = kind(&error);
                assert_eq!(kind, refusal["kind"], "{name}");
                assert_eq!(flag, refusal.get("flag").and_then(Value::as_str), "{name}");
                assert_eq!(
                    command,
                    refusal.get("command").and_then(Value::as_str),
                    "{name}"
                );
                let fields: &[&str] = match kind {
                    "before_command" => &["kind", "flag", "command"],
                    "run_env_without_run" | "run_write_without_run" => &["kind"],
                    _ => &["kind", "flag"],
                };
                assert!(has_fields(refusal, fields), "{name}");
            } else {
                assert!(
                    has_fields(case, &["name", "argv", "cli_args", "grant_flags"]),
                    "{name}"
                );
                let split = result.unwrap_or_else(|error| panic!("{name}: {error}"));
                assert_eq!(split.grant_flags, strings(&case["grant_flags"]), "{name}");
                assert_eq!(split.cli_args, strings(&case["cli_args"]), "{name}");
            }
        }
    }

    #[test]
    fn a_refusal_names_the_flag_up_to_its_value() {
        let refused = |argv: &[&str]| {
            let argv: Vec<String> = argv.iter().map(|&arg| arg.to_owned()).collect();
            split_grant_flags(&argv).unwrap_err().to_string()
        };
        assert_eq!(
            refused(&["--allow-env=TOKEN=secret", "check"]),
            format!(
                "--allow-env=… comes before the command: grant flags go right after the command \
                 word, like `{BINARY_NAME} check --allow-env=…`"
            )
        );
        assert_eq!(
            refused(&["-A"]),
            "-A comes before the command, and the binary runs each command of Gro's in a kernel \
             box, which -A can't lift: the project's Node gro is the unconfined spelling \
             (node_modules/.bin/gro)"
        );
        // the whole command line, the grant flags before the command word dropped
        assert_eq!(
            refused(&["-A", "--allow-read=.", "format", "--check", "--", "-A"]),
            "-A comes before the command, and the binary runs each command of Gro's in a kernel \
             box, which -A can't lift: the project's Node gro is the unconfined spelling \
             (node_modules/.bin/gro format --check -- -A)"
        );
        assert_eq!(
            refused(&["-A", "check"]),
            "-A comes before the command, and the binary runs each command of Gro's in a kernel \
             box, which -A can't lift: the project's Node gro is the unconfined spelling \
             (node_modules/.bin/gro check)"
        );
        assert_eq!(
            refused(&["-A", "run", "x.ts"]),
            format!(
                "-A comes before the command: grant flags go right after the command word, like \
                 `{BINARY_NAME} run -A`"
            )
        );
        assert_eq!(
            refused(&["check", "--allow-temp=x"]),
            "--allow-temp takes no value: --allow-temp=…"
        );
        assert_eq!(
            refused(&["check", "--allow-run-in"]),
            "--allow-run-in takes a list after `=`, like --allow-run-in=a,b: --allow-run-in"
        );
        assert_eq!(
            refused(&["test", "--allow-run-write=", "--", "-u"]),
            "--allow-run-write takes a list after `=`, like --allow-run-write=vitest:src: \
             --allow-run-write=…"
        );
        assert_eq!(
            refused(&["run", "--allow-run-write=x:secret", "x.ts"]),
            format!(
                "--allow-run-write lets a program --allow-run grants write, and \
                 `{BINARY_NAME} run` was given no --allow-run"
            )
        );
        assert_eq!(
            refused(&["check", "--allow-net=secret\u{1b}"]),
            "--allow-net=… is no grant flag: they're -A, --allow-temp, --allow-env, \
             --allow-read, --allow-write, --allow-run, --allow-run-in, --allow-run-env, \
             --allow-run-write"
        );
        assert_eq!(
            refused(&["check", "--allow-\u{1b}[2J"]),
            "--allow-\\u{1b}[2J is no grant flag: they're -A, --allow-temp, --allow-env, \
             --allow-read, --allow-write, --allow-run, --allow-run-in, --allow-run-env, \
             --allow-run-write"
        );
        assert_eq!(
            refused(&["run", "--allow-run-env=X", "x.ts"]),
            format!(
                "--allow-run-env passes variables to the programs --allow-run or --allow-run-in \
                 grants, and `{BINARY_NAME} run` was given neither"
            )
        );
        assert!(matches!(
            split_grant_flags(&["check".to_owned(), "--allow-write=".to_owned()]),
            Err(GrantFlagsError::TakesAList {
                list_flag: ListFlag::Write,
                ..
            })
        ));
    }

    /// The verb's grant flags are read by `fgro run`'s reader once the
    /// split leaves them: a list it refuses is refused, though the split
    /// passed its spelling; a flag given twice joins one set; `-A` grants
    /// everything beside it; and `--allow-run-env=` is taken alone.
    #[test]
    fn the_verb_s_grant_flags_are_read_as_fgro_run_reads_its_own() {
        let read = |argv: &[&str]| {
            let argv: Vec<String> = argv.iter().map(|&arg| arg.to_owned()).collect();
            to_run_grant(&split_grant_flags(&argv).unwrap().grant_flags)
        };
        let set = |items: &[&str]| -> BTreeSet<String> {
            items.iter().map(|&item| item.to_owned()).collect()
        };
        for (argv, flag) in [
            (["check", "--allow-read=,"], ListFlag::Read),
            (["check", "--allow-write=a,,b"], ListFlag::Write),
            (["check", "--allow-run-in=a,"], ListFlag::RunIn),
            (["check", "--allow-run-write=vitest:a,"], ListFlag::RunWrite),
        ] {
            assert!(
                matches!(read(&argv), Err(GrantFlagsError::List(GrantListError::Dir(refused, DirFlagFault::Empty))) if refused == flag),
                "{argv:?}"
            );
        }
        assert!(matches!(
            read(&["check", "--allow-env=A=B"]),
            Err(GrantFlagsError::List(GrantListError::EnvName(_)))
        ));
        assert!(matches!(
            read(&["check", "--allow-run=../x"]),
            Err(GrantFlagsError::List(GrantListError::RunName(_)))
        ));
        assert!(matches!(
            read(&["check", "--allow-run-env=*"]),
            Err(GrantFlagsError::List(GrantListError::RunEnvName(_)))
        ));
        assert_eq!(
            read(&["check", "--allow-read=a,,b"])
                .unwrap_err()
                .to_string(),
            "a --allow-read directory is empty: --allow-read=a,/b lists directories separated \
             by single commas"
        );
        assert!(matches!(
            to_run_grant(&["--bogus".to_owned()]),
            Err(GrantFlagsError::NoGrantFlag { flag }) if flag == "--bogus"
        ));
        let (grant, dirs) = read(&[
            "check",
            "--allow-env=A",
            "--allow-env=B,A",
            "--allow-read=x",
            "--allow-run-env=GIT_*",
            "--allow-temp",
            "--",
            "--allow-read=,",
        ])
        .unwrap();
        assert_eq!(
            grant,
            RunGrant::Named {
                env: set(&["A", "B"]),
                temp: true,
                run: BTreeSet::new(),
                run_env: BTreeSet::from([EnvPattern::Prefix("GIT_".to_owned())]),
            }
        );
        assert_eq!(
            dirs,
            FlagDirs {
                read: set(&["x"]),
                ..FlagDirs::default()
            }
        );
        assert_eq!(
            read(&["check", "--allow-env=A", "-A", "--allow-read=y"])
                .unwrap()
                .0,
            RunGrant::All
        );
    }
}
