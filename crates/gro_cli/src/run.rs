//! `fgro run`: starts the engine and runs one module until its event loop
//! drains, printing its failure, then warning on stderr of each `import()`
//! gro's rules refused that the failure doesn't already show.

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::phases::{self, Phase};
use gro_runtime::{Authority, Grants, ProcessGrant, RefusedImport, RunOutcome, RuntimeOptions};

use crate::cli::RunInvocation;
use crate::color;
use crate::error::CliError;
use crate::print::{escape_controls, stderr_line};
use crate::snapshot::STARTUP_SNAPSHOT;

/// Runs the module `run.file` names (a path relative to the working
/// directory, or absolute), handing the runtime the script's arguments,
/// prints its failure if it failed, and returns the exit code
/// ([`CliError::exit_code`], 0 on success).
///
/// After the failure it warns of each `import()` refused by one of gro's
/// rules that isn't the run's uncaught error, one line each, its control
/// characters escaped, then of how many more times one was refused, not
/// listed. The warnings are the last thing the run writes, so no text the
/// program chose, its uncaught error's included, prints after them to move
/// the cursor back over them or erase them. The exit code doesn't change
/// for them.
pub fn run_module(run: &RunInvocation) -> u8 {
    let RunOutcome {
        result,
        refused_imports,
        unlisted_refusals,
        unlisted_host_module_refusals,
    } = match start_and_run(run) {
        Ok(outcome) => outcome,
        Err(error) => return error.report(),
    };
    let code = result.map_or_else(|error| CliError::from(error).report(), |()| 0);
    warn_of_refusals(
        &refused_imports,
        unlisted_host_module_refusals,
        unlisted_refusals - unlisted_host_module_refusals,
    );
    code
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

/// Warns on stderr of each of `refused`, its message's control characters
/// escaped, then of the `host_modules` more times an `import()` naming a
/// host module was refused, and the `unlisted` more times another was.
fn warn_of_refusals(refused: &[RefusedImport], host_modules: u64, unlisted: u64) {
    for refused in refused {
        stderr_line(format_args!(
            "warning: import() refused: {}",
            escape_controls(&refused.message)
        ));
    }
    match host_modules {
        0 => {}
        1 => stderr_line("warning: import() refused: 1 more time for a host module, not listed"),
        more => stderr_line(format_args!(
            "warning: import() refused: {more} more times for a host module, not listed"
        )),
    }
    match unlisted {
        0 => {}
        1 => stderr_line("warning: import() refused: 1 more time, not listed"),
        more => stderr_line(format_args!(
            "warning: import() refused: {more} more times, not listed"
        )),
    }
}

/// The options `run`'s runtime starts with: the project's own modules
/// granted the process host module, which reads only the script's
/// arguments so far.
fn runtime_options(run: &RunInvocation) -> RuntimeOptions {
    RuntimeOptions {
        authority: Authority::Confined(Grants {
            process: Some(ProcessGrant {}),
        }),
        snapshot: STARTUP_SNAPSHOT,
        console: color::console_options(),
        args: run.args.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::runtime_options;
    use crate::cli::{Invocation, Task, parse};

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

    /// `run` grants the process host module, and only it.
    #[test]
    fn the_run_grants_process() {
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert_eq!(
            runtime_options(&run).authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                process: Some(gro_runtime::ProcessGrant {}),
            })
        );
    }
}
