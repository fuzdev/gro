//! `fgro run`: starts the engine and runs one module until its event loop
//! drains, returning a failure as a `CliError`.

use std::path::Path;

use gro_runtime::RuntimeOptions;
use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::phases::{self, Phase};

use crate::color;
use crate::error::CliError;
use crate::snapshot::STARTUP_SNAPSHOT;

/// Runs the module `file` (a path relative to the working directory, or
/// absolute).
///
/// # Errors
///
/// Returns why the engine, the working directory, or the run failed, the
/// exit code with it ([`CliError::exit_code`]).
pub fn run_module(file: &Path) -> Result<(), CliError> {
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default())?;
    phases::mark(Phase::Engine);
    let options = RuntimeOptions {
        snapshot: STARTUP_SNAPSHOT,
        console: color::console_options(),
        ..RuntimeOptions::default()
    };
    // read here, not in the runtime, which takes it as an input
    let cwd = std::env::current_dir().map_err(CliError::Cwd)?;
    gro_runtime::run_main(engine, &options, file, &cwd)?;
    Ok(())
}
