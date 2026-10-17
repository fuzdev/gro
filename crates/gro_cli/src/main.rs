//! `fgro`: gro on its own JS runtime, the binary's name while it's developed
//! beside the Node `gro`.
//!
//! `fgro run <file>` runs an ES module from disk (a path relative to the
//! working directory, or absolute) until its event loop drains. Errors print to
//! stderr, prefixed `error:`; an uncaught error's report carries its stack with
//! source positions (`file:///path:line:column`).
//!
//! Exit codes:
//!
//! - `0`: the program ran to completion, or `--help`
//! - `1`: the program threw, rejected a promise no one handled, or failed to
//!   load a module, or the runtime failed to start
//! - `2`: a usage error, so the command line must change: an unknown
//!   subcommand or flag, a missing argument (including no arguments at all),
//!   an argument that isn't UTF-8, or an entry file that doesn't resolve
//!   (missing, or not a `.js` or `.mjs` file)

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use argh::{EarlyExit, FromArgs};
use gro_runtime::{RunError, RuntimeOptions};

/// The caller must change the arguments before re-running.
const EXIT_USAGE: u8 = 2;

/// The program failed.
const EXIT_FAILURE: u8 = 1;

/// gro on its own JS runtime (experimental)
#[derive(FromArgs)]
struct Args {
    #[argh(subcommand)]
    command: Command,
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Run(RunArgs),
}

/// run an ES module (.js or .mjs) from disk
#[derive(FromArgs)]
#[argh(subcommand, name = "run")]
struct RunArgs {
    /// the module to run, relative to the working directory
    #[argh(positional)]
    file: PathBuf,
}

fn main() -> ExitCode {
    let args: Result<Vec<String>, OsString> = std::env::args_os()
        .skip(1)
        .map(OsString::into_string)
        .collect();
    let args = match args {
        Ok(args) => args,
        Err(arg) => {
            eprintln!(
                "error: an argument is not valid UTF-8: {}",
                arg.to_string_lossy()
            );
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    ExitCode::from(run(&args))
}

/// Runs `fgro` with `args` (the program name excluded), returning the exit code.
fn run(args: &[&str]) -> u8 {
    match Args::from_args(&["fgro"], args) {
        Ok(Args {
            command: Command::Run(RunArgs { file }),
        }) => match gro_runtime::run_main(&RuntimeOptions::default(), &file) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("error: {error}");
                exit_code(&error)
            }
        },
        // argh's `from_env` exits `1` on a usage error, so map its result by hand
        Err(EarlyExit {
            output,
            status: Ok(()),
        }) => {
            println!("{}", output.trim_end());
            0
        }
        Err(EarlyExit {
            output,
            status: Err(()),
        }) => {
            eprintln!("{}", output.trim_end());
            EXIT_USAGE
        }
    }
}

const fn exit_code(error: &RunError) -> u8 {
    match error {
        RunError::Entry(error) if error.kind.is_specifier_error() => EXIT_USAGE,
        _ => EXIT_FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_exits_0() {
        assert_eq!(run(&["--help"]), 0);
        assert_eq!(run(&["run", "--help"]), 0);
    }

    #[test]
    fn usage_error_exits_2() {
        assert_eq!(run(&[]), EXIT_USAGE);
        assert_eq!(run(&["--unknown"]), EXIT_USAGE);
        assert_eq!(run(&["positional"]), EXIT_USAGE);
        assert_eq!(run(&["run"]), EXIT_USAGE);
        assert_eq!(run(&["run", "a.js", "b.js"]), EXIT_USAGE);
    }
}
