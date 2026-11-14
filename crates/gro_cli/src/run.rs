//! `fgro run`: starts the engine and runs one module until its event loop
//! drains, printing its failure, then each `import()` gro's rules refused,
//! which fail the run too.

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::phases::{self, Phase};
use gro_runtime::{ArgsGrant, Authority, Grants, RefusedImports, RunOutcome, RuntimeOptions};

use crate::EXIT_FAILURE;
use crate::cli::RunInvocation;
use crate::color;
use crate::error::CliError;
use crate::print::{escape_controls, stderr_line};
use crate::snapshot::STARTUP_SNAPSHOT;
use crate::version::RUNTIME_VERSION;

/// Runs the module `run.file` names (a path relative to the working
/// directory, or absolute), handing the runtime the script's arguments,
/// prints its failure if it failed, then the `import()`s gro's rules
/// refused, and returns the exit code: the failure's
/// ([`CliError::exit_code`]), else [`EXIT_FAILURE`] for a run refused an
/// `import()`, else 0.
pub fn run_module(run: &RunInvocation) -> u8 {
    let RunOutcome { result, refused } = match start_and_run(run) {
        Ok(outcome) => outcome,
        Err(error) => return error.report(),
    };
    let code = result.map_or_else(|error| CliError::from(error).report(), |()| 0);
    report_refused(&refused);
    exit_code(code, !refused.is_empty())
}

/// The exit code of a run that ended with `code`, 0 when the program ran to
/// completion, and that `refused` an `import()` by rule or didn't: a
/// refusal fails the run, whether or not the program caught it, so it turns
/// 0 into [`EXIT_FAILURE`], and any other code stands.
const fn exit_code(code: u8, refused: bool) -> u8 {
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
/// granted the args host module, the script's arguments, and the runtime's
/// name and version.
fn runtime_options(run: &RunInvocation) -> RuntimeOptions {
    RuntimeOptions {
        authority: Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
        }),
        snapshot: STARTUP_SNAPSHOT,
        console: color::console_options(),
        args: run.args.clone(),
        runtime_version: RUNTIME_VERSION.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{RUNTIME_VERSION, exit_code, runtime_options};
    use crate::cli::{Invocation, Task, parse};

    /// A refused `import()` turns a run's 0 into 1 and leaves every other
    /// code as it is.
    #[test]
    fn a_refusal_fails_a_run_that_ended_with_0() {
        assert_eq!(exit_code(0, false), 0);
        assert_eq!(exit_code(0, true), 1);
        for code in [1, 2, 3, 255] {
            assert_eq!(exit_code(code, true), code);
            assert_eq!(exit_code(code, false), code);
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

    /// `run` grants the args host module, and only it.
    #[test]
    fn the_run_grants_args() {
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert_eq!(
            runtime_options(&run).authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
            })
        );
    }
}
