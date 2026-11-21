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
//!
//! Kind: the native parse of the trusted command line: it refuses a slip (a
//! misplaced or misspelled flag) rather than let it quietly become a task's
//! arg or a grant, and bounds nothing against the project, which doesn't
//! write the command line.

use gro_runtime::BINARY_NAME;
use gro_runtime::verb::closure::{RUN_TASK_NAME, is_task_path_input};
use thiserror::Error;

use crate::cli::{
    ALLOW_ALL, ALLOW_RUN, ALLOW_RUN_ENV, ALLOW_RUN_IN, ALLOW_TEMP, GrantFlag, ListFlag,
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

/// Why a command line's grant flags are refused. Each names the flag as
/// [`echo_argument`] prints it, up to its first `=`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GrantFlagsError {
    /// A grant flag before the command word, where it would be the CLI's
    /// arg, or Gro's.
    #[error(
        "{flag} comes before the command: grant flags go right after the command word, like \
         `{bin} {command} {flag}`",
        flag = echo_argument(.flag),
        command = .command.as_deref().map_or_else(|| "<command>".into(), escape_controls),
        bin = BINARY_NAME
    )]
    BeforeCommand {
        /// The flag, as written.
        flag: String,
        /// The first argument naming no flag before any `--`, which the
        /// hint puts first, or none.
        command: Option<String>,
    },
    /// `--allow-temp` given a value.
    #[error("{ALLOW_TEMP} takes no value: {}", echo_argument(.flag))]
    TakesNoValue {
        /// The flag, as written.
        flag: String,
    },
    /// A flag that lists, with no list or an empty one.
    #[error(
        "{name} takes a list after `=`, like {name}=a,b: {}",
        echo_argument(.flag),
        name = .list_flag.spelled()
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
/// come, then `run` given `--allow-run-env` alone.
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
            return Err(GrantFlagsError::BeforeCommand {
                flag: arg.clone(),
                command,
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

#[cfg(test)]
mod tests {
    use gro_runtime::BINARY_NAME;
    use serde_json::Value;

    use super::{GrantFlagsError, split_grant_flags};
    use crate::cli::ListFlag;
    use crate::verb::tests::{has_fields, read_fixture, strings};

    /// The refusal's kind as the fixture names it, the flag it names, and
    /// for `before_command` the hint's command.
    fn kind(error: &GrantFlagsError) -> (&'static str, Option<&str>, Option<&str>) {
        match error {
            GrantFlagsError::BeforeCommand { flag, command } => {
                ("before_command", Some(flag), command.as_deref())
            }
            GrantFlagsError::TakesNoValue { flag } => ("takes_no_value", Some(flag), None),
            GrantFlagsError::TakesAList { flag, .. } => ("takes_a_list", Some(flag), None),
            GrantFlagsError::NoGrantFlag { flag } => ("no_grant_flag", Some(flag), None),
            GrantFlagsError::RunEnvWithoutRun => ("run_env_without_run", None, None),
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
                    "run_env_without_run" => &["kind"],
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
            format!(
                "-A comes before the command: grant flags go right after the command word, like \
                 `{BINARY_NAME} <command> -A`"
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
            refused(&["check", "--allow-net=secret\u{1b}"]),
            "--allow-net=… is no grant flag: they're -A, --allow-temp, --allow-env, \
             --allow-read, --allow-write, --allow-run, --allow-run-in, --allow-run-env"
        );
        assert_eq!(
            refused(&["check", "--allow-\u{1b}[2J"]),
            "--allow-\\u{1b}[2J is no grant flag: they're -A, --allow-temp, --allow-env, \
             --allow-read, --allow-write, --allow-run, --allow-run-in, --allow-run-env"
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
}
