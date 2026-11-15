//! `fgro run`: starts the engine and runs one module until its event loop
//! drains or the program exits, printing its failure, then each `import()`
//! and host op's call gro's rules refused, which fail the run too, and
//! leaves the runtime as it is, since the process exits next.
//! [`exit_code`] is the one place the run's exit code is decided, however
//! the run ended.

use std::ffi::OsString;

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::phases::{self, Phase};
use gro_runtime::{
    ArgsGrant, Authority, EnvGrant, EnvVars, Exit, ExitGrant, Grants, Refusals, RefusedBy,
    RunOutcome, RuntimeOptions, env_name_fault,
};

use crate::EXIT_FAILURE;
use crate::cli::{RunGrant, RunInvocation};
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
/// prints its failure if it failed, then the `import()`s and host ops'
/// calls gro's rules refused, and returns the exit code ([`exit_code`]). A
/// program that calls `exit` never gets here: [`exit_now`] ends the process
/// from inside the call.
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
/// op: prints what gro's rules refused so far, as a run that ends on its
/// own does, then ends the process with [`exit_code`]'s code,
/// by the step every command exits by.
fn exit_now(exit: &Exit) -> ! {
    report_refused(&exit.refused);
    crate::end(exit_code(
        Ended::Exited(exit.code),
        !exit.refused.is_empty(),
    ))
}

/// The exit code of a run that `ended` so, and in which gro's rules
/// `refused` an `import()` or a host op's call, or didn't: the one place
/// the rules meet.
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
    let options = runtime_options(run, &ProcessEnvironment);
    // read here, not in the runtime, which takes it as an input
    let cwd = std::env::current_dir().map_err(CliError::Cwd)?;
    Ok(gro_runtime::run_main(engine, &options, &run.file, &cwd))
}

/// Prints to stderr each refusal `refused` lists, one line each, naming
/// what refused it, an `import()` or a host module, and its message, its
/// control characters escaped, since the program chose the specifier or
/// the name, then one line counting the refusals it doesn't list.
fn report_refused(refused: &Refusals) {
    for refusal in &refused.listed {
        let message = escape_controls(&refusal.message);
        match refusal.by {
            RefusedBy::Import => stderr_line(format_args!("error: import() refused: {message}")),
            RefusedBy::Grant { grant, .. } => {
                stderr_line(format_args!("error: {grant} refused: {message}"));
            }
        }
    }
    match refused.unlisted {
        0 => {}
        1 => stderr_line("error: 1 more refusal, not listed"),
        more => stderr_line(format_args!("error: {more} more refusals, not listed")),
    }
}

/// Where `fgro run` reads the environment variables it hands the runtime:
/// the process's environment ([`ProcessEnvironment`]), or a test's.
trait Environment {
    /// The value of the variable `name`, if it's set.
    fn var(&self, name: &str) -> Option<OsString>;
    /// Every variable, in the order the environment holds them.
    fn vars(&self) -> Vec<(OsString, OsString)>;
}

/// The process's own environment.
struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn vars(&self) -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }
}

/// The options `run`'s runtime starts with: the authority its flags grant
/// ([`authority`]) and the environment variables that authority reaches,
/// read from `environment` now, once ([`env_vars`]); the exit step
/// [`exit_now`]; the script's arguments; and the runtime's name and
/// version.
fn runtime_options(run: &RunInvocation, environment: &impl Environment) -> RuntimeOptions {
    RuntimeOptions {
        authority: authority(&run.grant),
        env: env_vars(&run.grant, environment),
        exit_step: Some(exit_now),
        snapshot: STARTUP_SNAPSHOT,
        console: color::console_options(),
        args: run.args.clone(),
        runtime_version: RUNTIME_VERSION.to_owned(),
    }
}

/// The authority `grant` gives: unconfined for `-A`; else the project's
/// own modules granted the args host module, the exit host module, and,
/// when `--allow-env=` named any variable, the env host module with those
/// names.
fn authority(grant: &RunGrant) -> Authority {
    match grant {
        RunGrant::All => Authority::Unconfined,
        RunGrant::Named { env } => Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
            exit: Some(ExitGrant {}),
            env: (!env.is_empty()).then(|| EnvGrant { names: env.clone() }),
        }),
    }
}

/// The environment variables `grant` reaches, read from `environment`:
/// each name `--allow-env=` granted that's set, asked for by name and no
/// other read, or, under `-A`, every variable whose name the env host
/// module could ask for (UTF-8, as a JS string is, and neither empty nor
/// holding a `=`). A value that isn't UTF-8 is decoded with U+FFFD in place
/// of each invalid sequence, as Node reads one.
fn env_vars(grant: &RunGrant, environment: &impl Environment) -> EnvVars {
    let read = |(name, value): (String, OsString)| (name, value.to_string_lossy().into_owned());
    match grant {
        RunGrant::Named { env } => env
            .iter()
            .filter_map(|name| Some((name.clone(), environment.var(name)?)))
            .map(read)
            .collect(),
        RunGrant::All => environment
            .vars()
            .into_iter()
            .filter_map(|(name, value)| {
                let name = name.into_string().ok()?;
                env_name_fault(&name, true)
                    .is_none()
                    .then_some((name, value))
            })
            .map(read)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    use super::{
        EXIT_FAILURE, Ended, Environment, RUNTIME_VERSION, exit_code, exit_now, runtime_options,
    };
    use crate::cli::{Invocation, RunInvocation, Task, parse};

    /// An environment of a test's own, never the process's, which records
    /// what was read of it.
    #[derive(Default)]
    struct TestEnvironment {
        /// Its variables, in order.
        vars: Vec<(OsString, OsString)>,
        /// Each name asked for by [`Environment::var`].
        asked: RefCell<Vec<String>>,
        /// Whether [`Environment::vars`] was called.
        listed: Cell<bool>,
    }

    impl TestEnvironment {
        fn of(vars: &[(&[u8], &[u8])]) -> Self {
            Self {
                vars: vars
                    .iter()
                    .map(|(name, value)| {
                        (
                            OsString::from_vec(name.to_vec()),
                            OsString::from_vec(value.to_vec()),
                        )
                    })
                    .collect(),
                ..Self::default()
            }
        }
    }

    impl Environment for TestEnvironment {
        fn var(&self, name: &str) -> Option<OsString> {
            self.asked.borrow_mut().push(name.to_owned());
            self.vars
                .iter()
                .find(|(var, _)| var.as_encoded_bytes() == name.as_bytes())
                .map(|(_, value)| value.clone())
        }

        fn vars(&self) -> Vec<(OsString, OsString)> {
            self.listed.set(true);
            self.vars.clone()
        }
    }

    /// The run `args` (after `run`) ask for.
    fn invocation(args: &[&str]) -> RunInvocation {
        let args = [&["run"][..], args].concat();
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&args) else {
            panic!("{args:?} runs a file");
        };
        run
    }

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
        assert_eq!(
            runtime_options(&run, &TestEnvironment::default()).args,
            script_args
        );
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert!(
            runtime_options(&run, &TestEnvironment::default())
                .args
                .is_empty()
        );
    }

    /// The runtime is named as the build names itself.
    #[test]
    fn the_runtime_is_named_with_the_build_s_version() {
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert_eq!(
            runtime_options(&run, &TestEnvironment::default()).runtime_version,
            RUNTIME_VERSION
        );
    }

    /// `run` grants the args host module and the exit host module, whose
    /// step is the CLI's, and nothing else, reading no environment
    /// variable.
    #[test]
    fn the_run_grants_args_and_exit() {
        let run = invocation(&["a.js"]);
        let environment = TestEnvironment::of(&[(b"HOME", b"/home/a")]);
        let options = runtime_options(&run, &environment);
        assert_eq!(
            options.authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant {}),
                env: None,
            })
        );
        assert_eq!(options.env, gro_runtime::EnvVars::default());
        assert!(environment.asked.borrow().is_empty());
        assert!(!environment.listed.get());
        let cli_step: gro_runtime::ExitStep = exit_now;
        assert!(
            options
                .exit_step
                .is_some_and(|step| std::ptr::fn_addr_eq(step, cli_step))
        );
    }

    /// `--allow-env=` grants the env host module exactly the names its
    /// options list, together, and the run reads only those of the
    /// environment, each by name, a value that isn't UTF-8 decoded with
    /// U+FFFD, and an unset one left out.
    #[test]
    fn allow_env_grants_and_reads_exactly_its_names() {
        let run = invocation(&[
            "--allow-env=B,A",
            "--allow-env=C,A",
            "a.js",
            "--allow-env=D",
        ]);
        let environment =
            TestEnvironment::of(&[(b"A", b"1"), (b"B", b"b\xff"), (b"D", b"4"), (b"E", b"5")]);
        let options = runtime_options(&run, &environment);
        assert_eq!(
            options.authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant {}),
                env: Some(gro_runtime::EnvGrant {
                    names: ["A", "B", "C"].map(ToOwned::to_owned).into(),
                }),
            })
        );
        assert_eq!(*environment.asked.borrow(), ["A", "B", "C"]);
        assert!(!environment.listed.get());
        let read: Vec<(&str, &str)> = options.env.iter().collect();
        assert_eq!(read, [("A", "1"), ("B", "b\u{fffd}")]);
        assert_eq!(options.args, ["--allow-env=D"]);
    }

    /// `-A` runs unconfined, its `--allow-env=` names checked and adding
    /// nothing, and the run reads every variable whose name the env host
    /// module could ask for, in order: a name that isn't UTF-8, an empty
    /// one, and one holding a `=` are left out, and a `*` is a name like
    /// any other.
    #[test]
    fn allow_all_runs_unconfined_and_reads_every_variable() {
        for args in [&["-A", "a.js"][..], &["--allow-env=A", "-A", "-A", "a.js"]] {
            let run = invocation(args);
            let environment = TestEnvironment::of(&[
                (b"B", b"2"),
                (b"N\xff", b"x"),
                (b"", b"empty"),
                (b"K=V", b"x"),
                (b"A*", b"glob"),
                (b"A", b"1"),
                (b"B", b"again"),
            ]);
            let options = runtime_options(&run, &environment);
            assert_eq!(
                options.authority,
                gro_runtime::Authority::Unconfined,
                "{args:?}"
            );
            assert!(environment.asked.borrow().is_empty());
            assert!(environment.listed.get());
            let read: Vec<(&str, &str)> = options.env.iter().collect();
            assert_eq!(read, [("B", "2"), ("A*", "glob"), ("A", "1")], "{args:?}");
        }
    }

    /// The variables' `Debug` names them and prints no value.
    #[test]
    fn the_variables_debug_prints_no_value() {
        let run = invocation(&["--allow-env=TOKEN", "a.js"]);
        let environment = TestEnvironment::of(&[(b"TOKEN", b"secret-value")]);
        let options = runtime_options(&run, &environment);
        let printed = format!("{options:?}");
        assert!(printed.contains("TOKEN"), "{printed}");
        assert!(!printed.contains("secret-value"), "{printed}");
    }
}
