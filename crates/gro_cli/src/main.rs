//! `fgro`: gro on its own JS runtime, the binary's name while it's developed
//! beside the Node `gro`.
//!
//! Not functional yet: it parses its arguments and exits. Exit codes: `0` for
//! `--help`, `2` for a usage error, and `1` otherwise.

use std::process::ExitCode;

use argh::{EarlyExit, FromArgs};

/// The caller must change the arguments before re-running.
const EXIT_USAGE: u8 = 2;

/// gro on its own JS runtime (experimental, not functional yet)
#[derive(FromArgs)]
struct Args {}

fn main() -> ExitCode {
    // `args_os`, so a non-UTF-8 argument is a usage error, not a panic
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    ExitCode::from(run(&args))
}

/// Runs `fgro` with `args` (the program name excluded), returning the exit code.
fn run(args: &[&str]) -> u8 {
    match Args::from_args(&["fgro"], args) {
        Ok(Args {}) => {
            eprintln!("fgro is not functional yet");
            1
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_exits_0() {
        assert_eq!(run(&["--help"]), 0);
    }

    #[test]
    fn usage_error_exits_2() {
        assert_eq!(run(&["--unknown"]), EXIT_USAGE);
        assert_eq!(run(&["positional"]), EXIT_USAGE);
    }

    #[test]
    fn no_args_exits_1() {
        assert_eq!(run(&[]), 1);
    }
}
