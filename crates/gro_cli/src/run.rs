//! `fgro run`: starts the engine and runs one module until its event loop
//! drains, and maps a failure to its exit code.

use std::path::Path;

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::phases::{self, Phase};
use gro_runtime::{RunError, RuntimeOptions};

use crate::snapshot::STARTUP_SNAPSHOT;
use crate::{EXIT_FAILURE, EXIT_USAGE, color};

/// Runs the module `file` (a path relative to the working directory, or
/// absolute), returning the exit code and printing a failure to stderr.
pub fn run_module(file: &Path) -> u8 {
    let engine = match init_engine(Confinement::Unconfined, &EngineOptions::default()) {
        Ok(engine) => engine,
        Err(error) => {
            eprintln!("error: {error}");
            return EXIT_FAILURE;
        }
    };
    phases::mark(Phase::Engine);
    let options = RuntimeOptions {
        snapshot: STARTUP_SNAPSHOT,
        console: color::console_options(),
        ..RuntimeOptions::default()
    };
    // read here, not in the runtime, which takes it as an input
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("error: cannot read the working directory: {error}");
            return EXIT_FAILURE;
        }
    };
    match gro_runtime::run_main(engine, &options, file, &cwd) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            exit_code(&error)
        }
    }
}

const fn exit_code(error: &RunError) -> u8 {
    match error {
        RunError::Entry(error) if error.kind.is_refusal() => EXIT_USAGE,
        _ => EXIT_FAILURE,
    }
}
