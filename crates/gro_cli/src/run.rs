//! `fgro run`: starts the engine and runs one module until its event loop
//! drains or the program exits, printing its failure, then each `import()`
//! gro's rules refused, which fail the run too, and leaves the runtime as
//! it is, since the process exits next. [`exit_code`] is the one place the
//! run's exit code is decided, however the run ended.

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::phases::{self, Phase};
use gro_runtime::{
    ArgsGrant, Authority, Exit, ExitGrant, Grants, RefusedImports, RunOutcome, RuntimeOptions,
};

use crate::EXIT_FAILURE;
use crate::cli::RunInvocation;
use crate::color;
use crate::error::CliError;
use crate::print::{escape_controls, stderr_line};
use crate::snapshot::STARTUP_SNAPSHOT;
use crate::version::RUNTIME_VERSION;

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

/// Runs the module `run.file` names (a path relative to the working
/// directory, or absolute), handing the runtime the script's arguments,
/// prints its failure if it failed, then the `import()`s gro's rules
/// refused, and returns the exit code ([`exit_code`]). A program that
/// calls `exit` never gets here: [`exit_now`] ends the process from inside
/// the call.
pub fn run_module(run: &RunInvocation) -> u8 {
    let RunOutcome {
        result,
        refused,
        exit_code: set,
        runtime,
    } = match start_and_run(run) {
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
    report_refused(&refused);
    exit_code(ended, !refused.is_empty())
}

/// The step a program's `exit` ends the process by, called inside its host
/// op: prints the `import()`s gro's rules refused so far, as a run that
/// ends on its own does, then ends the process with [`exit_code`]'s code,
/// by the step every command exits by.
fn exit_now(exit: &Exit) -> ! {
    report_refused(&exit.refused);
    crate::end(exit_code(
        Ended::Exited(exit.code),
        !exit.refused.is_empty(),
    ))
}

/// The exit code of a run that `ended` so, and that `refused` an
/// `import()` by rule or didn't: the one place the rules meet.
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

/// Starts the engine and runs `run`'s module, returning how the run ended.
///
/// # Errors
///
/// Returns why the engine or the working directory failed, before anything
/// ran.
fn start_and_run(run: &RunInvocation) -> Result<RunOutcome, CliError> {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default())?;
    phases::mark(Phase::Engine);
    let options = runtime_options(run);
    // read here, not in the runtime, which takes it as an input
    let cwd = std::env::current_dir().map_err(CliError::Cwd)?;
    Ok(gro_runtime::run_main(engine, &options, &run.file, &cwd))
}

/// Prints to stderr each `import()` `refused` lists, one line each, its
/// message's control characters escaped, since the program chose its
/// specifier, then one line counting the refusals it doesn't list.
fn report_refused(refused: &RefusedImports) {
    for message in &refused.listed {
        stderr_line(format_args!(
            "error: import() refused: {}",
            escape_controls(message)
        ));
    }
    match refused.unlisted {
        0 => {}
        1 => stderr_line("error: import() refused: 1 more time, not listed"),
        more => stderr_line(format_args!(
            "error: import() refused: {more} more times, not listed"
        )),
    }
}

/// The options `run`'s runtime starts with: the project's own modules
/// granted the args host module and the exit host module, whose step is
/// [`exit_now`], the script's arguments, and the runtime's name and
/// version.
fn runtime_options(run: &RunInvocation) -> RuntimeOptions {
    RuntimeOptions {
        authority: Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
            exit: Some(ExitGrant { end: exit_now }),
        }),
        snapshot: STARTUP_SNAPSHOT,
        console: color::console_options(),
        args: run.args.clone(),
        runtime_version: RUNTIME_VERSION.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{EXIT_FAILURE, Ended, RUNTIME_VERSION, exit_code, exit_now, runtime_options};
    use crate::cli::{Invocation, Task, parse};

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
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&args) else {
            panic!("{args:?} runs a.js");
        };
        assert_eq!(runtime_options(&run).args, script_args);
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert!(runtime_options(&run).args.is_empty());
    }

    /// The runtime is named as the build names itself.
    #[test]
    fn the_runtime_is_named_with_the_build_s_version() {
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert_eq!(runtime_options(&run).runtime_version, RUNTIME_VERSION);
    }

    /// `run` grants the args host module and the exit host module, whose
    /// step is the CLI's, and nothing else.
    #[test]
    fn the_run_grants_args_and_exit() {
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert_eq!(
            runtime_options(&run).authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant { end: exit_now }),
            })
        );
    }
}
