//! `fgro run`: starts the engine and runs one module until its event loop
//! drains, returning a failure as a `CliError`.

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::phases::{self, Phase};
use gro_runtime::{Authority, Grants, ProcessGrant, RuntimeOptions};

use crate::cli::RunInvocation;
use crate::color;
use crate::error::CliError;
use crate::snapshot::STARTUP_SNAPSHOT;

/// Runs the module `run.file` names (a path relative to the working
/// directory, or absolute), handing the runtime the script's arguments.
///
/// # Errors
///
/// Returns why the engine, the working directory, or the run failed, the
/// exit code with it ([`CliError::exit_code`]).
pub fn run_module(run: &RunInvocation) -> Result<(), CliError> {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default())?;
    phases::mark(Phase::Engine);
    let options = runtime_options(run);
    // read here, not in the runtime, which takes it as an input
    let cwd = std::env::current_dir().map_err(CliError::Cwd)?;
    gro_runtime::run_main(engine, &options, &run.file, &cwd)?;
    Ok(())
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
