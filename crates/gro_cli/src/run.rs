//! `fgro run`: starts the engine and runs one module until its event loop
//! drains or the program exits, printing its failure, then each `import()`
//! and host function's call gro's rules refused, which fail the run too, and
//! leaves the runtime as it is, since the process exits next.
//! [`exit_code`] is the one place the run's exit code is decided, however
//! the run ended.

use std::ffi::OsString;
use std::path::Path;

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::phases::{self, Phase};
use gro_runtime::{
    ArgsGrant, Authority, EnvGrant, EnvVars, Exit, ExitGrant, FsGrant, Grants, NeededGrant,
    Refusals, RefusedBy, RunOutcome, RuntimeOptions, env_name_fault,
};

use crate::EXIT_FAILURE;
use crate::cli::{ALLOW_ENV, ALLOW_READ, ALLOW_WRITE, DirFlag, FlagDirs, RunGrant, RunInvocation};
use crate::color;
use crate::error::{CliError, DirFault};
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
/// prints its failure if it failed, then the `import()`s and host
/// functions' calls gro's rules refused, and returns the exit code ([`exit_code`]). A
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
/// `refused` an `import()` or a host function's call, or didn't: the one place
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
/// Returns why the working directory, a directory a flag grants
/// ([`fs_grant`]), or the engine failed, before anything ran.
fn start_and_run(run: &RunInvocation) -> Result<RunOutcome, CliError> {
    // read here, not in the runtime, which takes it as an input
    let cwd = std::env::current_dir().map_err(CliError::Cwd)?;
    let fs = fs_grant(&run.dirs, &cwd)?;
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default())?;
    phases::mark(Phase::Engine);
    let options = runtime_options(run, &ProcessEnvironment, fs);
    Ok(gro_runtime::run_main(engine, &options, &run.file, &cwd))
}

/// The fs grant of the directories `dirs` lists, each resolved against
/// `cwd` when it's relative and held to be a directory now, by one `stat`
/// that follows symlinks: the read ones first, then the written ones. The
/// runtime opens each when the run first imports `fs`, so a directory
/// replaced between is whatever directory is at its path then, and one
/// gone by then fails every op through it.
///
/// # Errors
///
/// Returns [`CliError::NoDir`] for the first that's missing, isn't a
/// directory, or can't be looked at.
fn fs_grant(dirs: &FlagDirs, cwd: &Path) -> Result<FsGrant, CliError> {
    let check = |flag: DirFlag, dir: &String| {
        let path = cwd.join(dir);
        let fault = match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => return Ok(path),
            Ok(_) => DirFault::NotADirectory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => DirFault::Missing,
            Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {
                DirFault::NotADirectory
            }
            Err(error) => DirFault::Unreadable(error),
        };
        Err(CliError::NoDir { flag, path, fault })
    };
    Ok(FsGrant {
        read: dirs
            .read
            .iter()
            .map(|dir| check(DirFlag::Read, dir))
            .collect::<Result<_, _>>()?,
        write: dirs
            .write
            .iter()
            .map(|dir| check(DirFlag::Write, dir))
            .collect::<Result<_, _>>()?,
    })
}

/// What follows a run's refusals on stderr, however many and however the
/// run ended: a refusal fails the run, and a program can't catch its way
/// out of one.
const REFUSALS_FAIL: &str = "error: a refusal fails the run, whether or not the program caught it";

/// Prints to stderr each refusal `refused` lists, one line each, naming
/// what refused it, an `import()` or a host module, and its message, its
/// control characters escaped, since the program chose the specifier or
/// the name, then one line counting the refusals it doesn't list, then,
/// when there were any, [`REFUSALS_FAIL`]; then the hints of the flags
/// that grant what the refused calls needed, and of the handles to open
/// for what a root already grants ([`grant_hints`]), if any does, which
/// the refusal that is the run's uncaught error, not listed, has too.
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
    if !refused.is_empty() {
        stderr_line(REFUSALS_FAIL);
    }
    for hint in grant_hints(refused, is_real_dir) {
        stderr_line(hint);
    }
}

/// Whether `path` is the real path of a directory: one is there, and no
/// symlink, `.`, `..`, or doubled or trailing `/` is on its text, so a
/// flag naming it grants that directory, and a root of it holds a path by
/// that text.
fn is_real_dir(path: &str) -> bool {
    std::fs::canonicalize(path).is_ok_and(|real| real.as_os_str() == path && real.is_dir())
}

/// The lines naming the flags that grant the refused calls' needed grants
/// (`refused.needed`), made of that data, never of a listed message, one
/// for each flag, `--allow-env`, then `--allow-read`, then `--allow-write`,
/// then one for each directory a handle on it would reach:
///
/// - `hint: add --allow-env=A,B to grant what env refused`, of each name
///   that's a [`hint_name`], or `... to grant some of what env refused`
///   when a call needed a name the line doesn't show, and no line when it
///   shows none;
/// - `hint: add --allow-read=/a,/b to grant what fs refused`, of each path
///   that's a [`hint_path`] and, as `real_dir` tells, the real path of a
///   directory, so pasting the line grants what it shows and the walk
///   that refused finds it by that text: a path through a symlink, which a
///   program chooses, grants another directory than it names
///   (`/proc/self/root` grants `/`), and a file or nothing grants nothing.
///   Else `... some of what fs refused` as for env, and when it shows no
///   path,
///   `hint: add --allow-read naming the directory to grant what fs
///   refused`, the flag without a path, which a shell reads as it's
///   printed, as for a step out of its root through a symlink, which only
///   a grant of a path through the symlink lets through; the same for
///   `--allow-write`;
/// - `hint: the run grants what fs refused in /a: open it with fs.open_dir
///   and use that handle`, of each such path that a root already grants as
///   the refused op needed, since a handle's scope never widens; and `...
///   in a directory not named here: ...` when one isn't shown.
///
/// A need past the record's bounds (`more_needed`) is one the flags' lines
/// don't show, of whichever flag. None for a refusal by rule or by a
/// handle the program narrowed, which need none, or under `-A`, which
/// refuses no grant.
fn grant_hints(refused: &Refusals, real_dir: impl Fn(&str) -> bool) -> Vec<String> {
    let shown = |path: &str| hint_path(path) && real_dir(path);
    let mut names: Vec<&str> = Vec::new();
    let mut reads: Vec<&str> = Vec::new();
    let mut writes: Vec<&str> = Vec::new();
    let mut handles: Vec<&str> = Vec::new();
    let (mut some_env, mut some_read, mut some_write, mut some_handle) =
        (false, false, false, false);
    for needed in &refused.needed {
        match needed {
            NeededGrant::Env(name) if hint_name(name) => names.push(name),
            NeededGrant::Env(_) => some_env = true,
            NeededGrant::Read(path) if shown(path) => reads.push(path),
            NeededGrant::Read(_) | NeededGrant::ReadThroughSymlink => some_read = true,
            NeededGrant::Write(path) if shown(path) => writes.push(path),
            NeededGrant::Write(_) => some_write = true,
            NeededGrant::OpenDir(path) if shown(path) => handles.push(path),
            NeededGrant::OpenDir(_) => some_handle = true,
        }
    }
    let more = refused.more_needed;
    let what = |some: bool| if some || more { "some of what" } else { "what" };
    let mut hints = Vec::new();
    if !names.is_empty() {
        hints.push(format!(
            "hint: add {ALLOW_ENV}={} to grant {} env refused",
            names.join(","),
            what(some_env)
        ));
    }
    for (flag, paths, some) in [
        (ALLOW_READ, reads, some_read),
        (ALLOW_WRITE, writes, some_write),
    ] {
        if !paths.is_empty() {
            hints.push(format!(
                "hint: add {flag}={} to grant {} fs refused",
                paths.join(","),
                what(some)
            ));
        } else if some {
            hints.push(format!(
                "hint: add {flag} naming the directory to grant what fs refused"
            ));
        }
    }
    let handle = |dir: &str| {
        format!(
            "hint: the run grants what fs refused in {dir}: open it with fs.open_dir and use that handle"
        )
    };
    hints.extend(handles.into_iter().map(handle));
    if some_handle {
        hints.push(handle("a directory not named here"));
    }
    hints
}

/// Whether the hint names `name`: ASCII letters, digits, and `_` only, a
/// word every shell passes as it's printed, so pasting the hint grants the
/// names it shows and runs nothing. A program chooses the names it asks
/// for, and a valid name can hold a space, a comma, a quote, `$(…)`, a
/// control character, or a lookalike letter.
fn hint_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether the hint names `path`: an absolute path of ASCII letters,
/// digits, `_`, `.`, `/`, and `-` only, which neither bash nor fish reads
/// as anything but the word it's printed as, so pasting the hint runs
/// nothing. A program chooses the paths it asks for, and a path
/// can hold a space, a comma, which separates the flag's directories, a
/// quote, `$(…)`, a `~`, a control character, or a lookalike letter.
fn hint_path(path: &str) -> bool {
    path.starts_with('/')
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-'))
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
/// ([`authority`]), `fs` among them, and the environment variables that
/// authority reaches, read from `environment` now, once ([`env_vars`]);
/// the exit step [`exit_now`]; the script's arguments; and the runtime's
/// name and version.
fn runtime_options(
    run: &RunInvocation,
    environment: &impl Environment,
    fs: FsGrant,
) -> RuntimeOptions {
    RuntimeOptions {
        authority: authority(&run.grant, fs),
        env: env_vars(&run.grant, environment),
        exit_step: Some(exit_now),
        snapshot: STARTUP_SNAPSHOT,
        console: color::console_options(),
        args: run.args.clone(),
        runtime_version: RUNTIME_VERSION.to_owned(),
    }
}

/// The authority `grant` gives: unconfined for `-A`, which reads and
/// writes every directory, `fs`'s directories adding nothing; else the
/// project's own modules granted the args host module, the exit host
/// module, the fs host module, reading the project's tree and the
/// directories `fs` grants as it grants them, and, when `--allow-env=`
/// named any variable, the env host module with those names.
fn authority(grant: &RunGrant, fs: FsGrant) -> Authority {
    match grant {
        RunGrant::All => Authority::Unconfined,
        RunGrant::Named { env } => Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
            exit: Some(ExitGrant {}),
            env: (!env.is_empty()).then(|| EnvGrant { names: env.clone() }),
            fs: Some(fs),
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
        EXIT_FAILURE, Ended, Environment, FsGrant, RUNTIME_VERSION, exit_code, exit_now,
        runtime_options,
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
            runtime_options(&run, &TestEnvironment::default(), FsGrant::default()).args,
            script_args
        );
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert!(
            runtime_options(&run, &TestEnvironment::default(), FsGrant::default())
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
            runtime_options(&run, &TestEnvironment::default(), FsGrant::default()).runtime_version,
            RUNTIME_VERSION
        );
    }

    /// `run` grants the args host module, the exit host module, whose
    /// step is the CLI's, and the fs host module, its root the project's
    /// tree to read, and nothing else, reading no environment variable.
    #[test]
    fn the_run_grants_args_exit_and_fs() {
        let run = invocation(&["a.js"]);
        let environment = TestEnvironment::of(&[(b"HOME", b"/home/a")]);
        let options = runtime_options(&run, &environment, FsGrant::default());
        assert_eq!(
            options.authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant {}),
                env: None,
                fs: Some(FsGrant::default()),
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
        let options = runtime_options(&run, &environment, FsGrant::default());
        assert_eq!(
            options.authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant {}),
                env: Some(gro_runtime::EnvGrant {
                    names: ["A", "B", "C"].map(ToOwned::to_owned).into(),
                }),
                fs: Some(FsGrant::default()),
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
            let options = runtime_options(&run, &environment, FsGrant::default());
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

    /// The fs grant the directory flags make reaches a confined run's
    /// authority as it's given, and an unconfined run's holds none, `-A`
    /// reading and writing everything whatever the flags list.
    #[test]
    fn the_fs_grant_reaches_a_confined_run_only() {
        let fs = FsGrant {
            read: ["/r".into()].into(),
            write: ["/w".into()].into(),
        };
        let run = invocation(&["--allow-read=/r", "a.js"]);
        let options = runtime_options(&run, &TestEnvironment::default(), fs.clone());
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert_eq!(grants.fs, Some(fs.clone()));
        let run = invocation(&["--allow-read=/r", "-A", "a.js"]);
        let options = runtime_options(&run, &TestEnvironment::default(), fs);
        assert_eq!(options.authority, gro_runtime::Authority::Unconfined);
    }

    /// The hints name each flag's values a shell passes as printed, in the
    /// order first needed, one line a flag, env's first: a path shows only
    /// when it's absolute and of letters, digits, `_`, `.`, `/`, and `-`; a
    /// line that can't show every value says `some of what`, and one that
    /// can show none names its flag without a value; a need past the
    /// record's bounds is one no line shows.
    #[test]
    fn the_hints_show_only_what_a_shell_passes_as_printed() {
        use gro_runtime::{NeededGrant, Refusals};

        use super::grant_hints;

        // every path a directory's real path, so only the shell decides
        let hints = |needed: &[NeededGrant], more_needed: bool| {
            grant_hints(
                &Refusals {
                    needed: needed.to_vec(),
                    more_needed,
                    ..Refusals::default()
                },
                |_| true,
            )
        };
        let read = |path: &str| NeededGrant::Read(path.to_owned());
        let write = |path: &str| NeededGrant::Write(path.to_owned());
        let open_dir = |path: &str| NeededGrant::OpenDir(path.to_owned());
        let env = |name: &str| NeededGrant::Env(name.to_owned());
        assert!(hints(&[], false).is_empty());
        assert_eq!(
            hints(
                &[
                    write("/p"),
                    read("/a/b-c_d.e"),
                    env("HOME"),
                    read("/x"),
                    env("A_1"),
                ],
                false
            ),
            [
                "hint: add --allow-env=HOME,A_1 to grant what env refused",
                "hint: add --allow-read=/a/b-c_d.e,/x to grant what fs refused",
                "hint: add --allow-write=/p to grant what fs refused",
            ]
        );
        let unsafe_paths = [
            "relative/dir",
            "/a b",
            "/a'b",
            "/a\"b",
            "/a$b",
            "/a$(touch x)",
            "/a`b`",
            "/a\nb",
            "/a\u{1b}[2J",
            "/a\u{202e}b",
            "/a,b",
            "/a*",
            "/~",
            "/a;b",
            "/a\\b",
            "/\u{e9}",
            "/a\u{ff0f}b",
            "",
        ];
        for path in unsafe_paths {
            assert_eq!(
                hints(&[read(path)], false),
                ["hint: add --allow-read naming the directory to grant what fs refused"],
                "{path:?}"
            );
            assert_eq!(
                hints(&[write("/ok"), write(path)], false),
                ["hint: add --allow-write=/ok to grant some of what fs refused"],
                "{path:?}"
            );
            assert_eq!(
                hints(&[open_dir(path)], false),
                [
                    "hint: the run grants what fs refused in a directory not named here: \
                     open it with fs.open_dir and use that handle"
                ],
                "{path:?}"
            );
        }
        assert_eq!(
            hints(&[read("/ok"), env("A")], true),
            [
                "hint: add --allow-env=A to grant some of what env refused",
                "hint: add --allow-read=/ok to grant some of what fs refused",
            ]
        );
        assert_eq!(
            hints(&[env("A B")], false),
            Vec::<String>::new(),
            "an env name it can't show makes no line"
        );
    }

    /// A path a refusal needed shows only when it's the real path of a
    /// directory, so a flag naming it grants it and the walk that refused
    /// finds it by that text: never a symlink, inside a root or
    /// `/proc/self/root`, which grants another directory than it names,
    /// nor a file or nothing, which grants nothing; a step out of its root
    /// through a symlink shows the flag alone. A directory a root already
    /// grants shows as a handle to open on it, never a flag, after the
    /// flags' lines.
    #[test]
    fn the_hints_show_only_real_directories() {
        use gro_runtime::{NeededGrant, Refusals};

        use super::{grant_hints, is_real_dir};

        let hints = |needed: &[NeededGrant]| {
            grant_hints(
                &Refusals {
                    needed: needed.to_vec(),
                    ..Refusals::default()
                },
                |path| matches!(path, "/p" | "/p/out" | "/r"),
            )
        };
        let read = |path: &str| NeededGrant::Read(path.to_owned());
        let write = |path: &str| NeededGrant::Write(path.to_owned());
        let open_dir = |path: &str| NeededGrant::OpenDir(path.to_owned());
        assert_eq!(
            hints(&[read("/p/docs"), read("/proc/self/root"), read("/r")]),
            ["hint: add --allow-read=/r to grant some of what fs refused"]
        );
        assert_eq!(
            hints(&[NeededGrant::ReadThroughSymlink, write("/p/x.txt")]),
            [
                "hint: add --allow-read naming the directory to grant what fs refused",
                "hint: add --allow-write naming the directory to grant what fs refused",
            ]
        );
        assert_eq!(
            hints(&[open_dir("/p/out"), write("/p"), open_dir("/p/gone")]),
            [
                "hint: add --allow-write=/p to grant what fs refused",
                "hint: the run grants what fs refused in /p/out: \
                 open it with fs.open_dir and use that handle",
                "hint: the run grants what fs refused in a directory not named here: \
                 open it with fs.open_dir and use that handle",
            ]
        );
        let real = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .display()
            .to_string();
        let at = |name: &str| format!("{real}/{name}");
        let shown: Vec<String> = [
            at("src"),
            at("src/../src"),
            at("./src"),
            format!("{real}//src"),
            at("src/"),
            at("Cargo.toml"),
            at("no_such_dir"),
            "/proc/self/root".to_owned(),
            "/proc/self/cwd".to_owned(),
            "/".to_owned(),
        ]
        .into_iter()
        .filter(|path| is_real_dir(path))
        .collect();
        assert_eq!(shown, [at("src"), "/".to_owned()]);
    }

    /// The variables' `Debug` names them and prints no value.
    #[test]
    fn the_variables_debug_prints_no_value() {
        let run = invocation(&["--allow-env=TOKEN", "a.js"]);
        let environment = TestEnvironment::of(&[(b"TOKEN", b"secret-value")]);
        let options = runtime_options(&run, &environment, FsGrant::default());
        let printed = format!("{options:?}");
        assert!(printed.contains("TOKEN"), "{printed}");
        assert!(!printed.contains("secret-value"), "{printed}");
    }
}
