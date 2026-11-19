//! `fgro run`: starts the engine and runs one module until its event loop
//! drains or the program exits, printing its failure, then each `import()`
//! and host function's call gro's rules refused, which fail the run too, and
//! leaves the runtime as it is, since the process exits next.
//! [`exit_code`] is the one place the run's exit code is decided, however
//! the run ended.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use gro_runtime::confine::Confinement;
use gro_runtime::engine::{EngineOptions, init_engine};
use gro_runtime::file_id::FileId;
use gro_runtime::phases::{self, Phase};
use gro_runtime::resolver::normalize;
use gro_runtime::{
    ArgsGrant, Authority, CHILD_BASE_ENV, EnvGrant, EnvPattern, EnvVars, ExecRootGrant, Exit,
    ExitGrant, FsGrant, FsNeed, GrantedDir, Grants, NameKind, NeededGrant, ProgramGrant, Refusal,
    Refusals, RefusedBy, RunOutcome, RuntimeOptions, SpawnGrant, env_name_fault,
};

use crate::EXIT_FAILURE;
use crate::cli::{
    ALLOW_ENV, ALLOW_READ, ALLOW_RUN, ALLOW_RUN_ENV, ALLOW_RUN_IN, ALLOW_TEMP, ALLOW_WRITE,
    DirFlag, FlagDirs, RunGrant, RunInvocation,
};
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
/// ([`granted_dirs`]), or the engine failed, before anything ran.
fn start_and_run(run: &RunInvocation) -> Result<RunOutcome, CliError> {
    // read here, not in the runtime, which takes it as an input
    let cwd = std::env::current_dir().map_err(CliError::Cwd)?;
    let (dirs, notes) = granted_dirs(&run.dirs, &cwd)?;
    // a directory `-A` names grants nothing more, so there's nothing to note
    if !matches!(run.grant, RunGrant::All) {
        for note in notes {
            stderr_line(note);
        }
    }
    if grants_programs(&run.grant, &run.dirs) {
        ready_to_spawn(gro_sys::raise_open_files_limit).map_err(CliError::SpawnReady)?;
    }
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default())?;
    phases::mark(Phase::Engine);
    let options = runtime_options(run, &ProcessEnvironment, dirs, &cwd);
    Ok(gro_runtime::run_main(engine, &options, &run.file, &cwd))
}

/// Whether the run may start a program: `-A`, a program `--allow-run=`
/// names in `grant`, or an exec root `--allow-run-in=` lists in `dirs`.
fn grants_programs(grant: &RunGrant, dirs: &FlagDirs) -> bool {
    match grant {
        RunGrant::All => true,
        RunGrant::Named { run, .. } => !run.is_empty() || !dirs.run_in.is_empty(),
    }
}

/// Makes the process ready to start programs, as a run that grants one
/// does before anything runs: every descriptor it inherited above stderr
/// marked close-on-exec, so a child is handed only stdin, stdout, and
/// stderr, and `SIGCHLD` at its default where it was inherited ignored, so
/// the kernel leaves each child for its pidfd to reap; then the soft limit
/// on open descriptors raised to the hard one by `raise`, as Node raises
/// it, so a run holding many children, each with its pipes, isn't refused
/// at a low soft limit. The raise is best-effort, as Node's is: one the
/// kernel refuses leaves the limit as it was and the run going.
///
/// # Errors
///
/// Returns the error of the step that failed, but the raise's.
fn ready_to_spawn(raise: fn() -> std::io::Result<u64>) -> std::io::Result<()> {
    gro_sys::cloexec_inherited_fds()?;
    gro_sys::child::default_child_signal()?;
    // a refused raise leaves the soft limit, which the run starts with as
    // a run that grants no program does
    let _ = raise();
    Ok(())
}

/// The directories the flags grant: the fs grant's, read and written, and
/// the spawn grant's exec roots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GrantedDirs {
    /// The fs grant of the directories `--allow-read=` and
    /// `--allow-write=` list.
    fs: FsGrant,
    /// The exec roots `--allow-run-in=` lists, each by the path it was
    /// granted as.
    exec_roots: BTreeMap<PathBuf, GrantedDir>,
}

/// The grants of the directories `dirs` lists, each resolved against `cwd`
/// when it's relative and held to be a directory now: its real path read,
/// then the identity of the directory there, by one `stat`, which the
/// grant keeps ([`GrantedDir`]), the read ones first, then the written
/// ones, then the exec roots. The runtime opens each fs root by that real
/// path when the run first imports `fs`, and each exec root as the run
/// starts, and holds it to that identity, so a directory moved or replaced
/// between, like one gone by then, fails every op through it, and the fs
/// roots nest as these real paths do. With them, the notes
/// ([`grant_note`]) for the directories whose real path isn't their text.
///
/// # Errors
///
/// Returns [`CliError::NoDir`] for the first that's missing, isn't a
/// directory, or can't be looked at.
fn granted_dirs(dirs: &FlagDirs, cwd: &Path) -> Result<(GrantedDirs, Vec<String>), CliError> {
    let mut notes = Vec::new();
    let mut check = |flag: DirFlag, dir: &String| {
        let path = cwd.join(dir);
        let found = std::fs::canonicalize(&path)
            .and_then(|real| std::fs::metadata(&real).map(|metadata| (real, metadata)));
        let fault = match found {
            Ok((real, metadata)) if metadata.is_dir() => {
                let id = FileId::of(&metadata);
                notes.extend(grant_note(flag, dir, &path, &real));
                return Ok((path, GrantedDir { real, id }));
            }
            Ok(_) => DirFault::NotADirectory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => DirFault::Missing,
            Err(error) if error.kind() == std::io::ErrorKind::NotADirectory => {
                DirFault::NotADirectory
            }
            Err(error) => DirFault::Unreadable(error),
        };
        Err(CliError::NoDir { flag, path, fault })
    };
    let fs = FsGrant {
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
        temp: false,
    };
    let exec_roots = dirs
        .run_in
        .iter()
        .map(|dir| check(DirFlag::RunIn, dir))
        .collect::<Result<_, _>>()?;
    Ok((GrantedDirs { fs, exec_roots }, notes))
}

/// The note, if one is due, that `flag` listing `dir`, the text typed,
/// which is `path` resolved against the working directory, grants `real`,
/// its real path as the run starts: `note: --allow-read=<dir> grants
/// <real>`, each escaped ([`escape_controls`]), when `real` isn't `path`
/// with its `.` and `..` resolved by their text, so the path passes through
/// a symlink. A refusal names a path the program chose, and one copied into
/// a flag can lead through a symlink to another directory than it names.
///
/// Every symlink is noted, one in a directory above the working directory
/// too, as a symlinked home is: which symlinks the person made is past
/// knowing, and the note is fixed by typing the real path, or a path
/// relative to the working directory, which is real already.
fn grant_note(flag: DirFlag, dir: &str, path: &Path, real: &Path) -> Option<String> {
    (normalize(path) != real).then(|| {
        format!(
            "note: {flag}={} grants {}",
            escape_controls(dir),
            escape_controls(&real.to_string_lossy())
        )
    })
}

/// What follows a run's refusals on stderr, however many and however the
/// run ended: a refusal fails the run, and a program can't catch its way
/// out of one.
const REFUSALS_FAIL: &str = "error: a refusal fails the run, whether or not the program caught it";

/// Prints to stderr each refusal `refused` lists, one line each
/// ([`refusal_line`]), then one line counting the refusals it doesn't
/// list, then, when there were any, [`REFUSALS_FAIL`]; then the hints
/// saying what grants the refused calls' needs ([`grant_hints`]), if any
/// does, which the refusal that is the run's uncaught error, not listed,
/// has too.
fn report_refused(refused: &Refusals) {
    for refusal in &refused.listed {
        stderr_line(refusal_line(refusal));
    }
    match refused.unlisted {
        0 => {}
        1 => stderr_line("error: 1 more refusal, not listed"),
        more => stderr_line(format_args!("error: {more} more refusals, not listed")),
    }
    if !refused.is_empty() {
        stderr_line(REFUSALS_FAIL);
    }
    for hint in grant_hints(refused) {
        stderr_line(hint);
    }
}

/// A listed refusal's line, escaped ([`escape_controls`]): `error:
/// import() refused: <message>`, `error: env refused: <message>`, `error:
/// spawn refused: <message>`, or `error: fs refused <need>: <message>`,
/// the need in its hint's words.
/// `src/docs/run.md` §Errors and exit codes states the rules.
fn refusal_line(refusal: &Refusal) -> String {
    let message = escape_controls(&refusal.message);
    match refusal.by {
        RefusedBy::Import => format!("error: import() refused: {message}"),
        RefusedBy::Env => format!("error: env refused: {message}"),
        RefusedBy::Spawn => format!("error: spawn refused: {message}"),
        RefusedBy::Fs(need) => {
            let need = match need {
                Some(FsNeed::Read) => "a read",
                Some(FsNeed::LinkTarget) => "a symlink leaving its root",
                Some(FsNeed::Write) => "a write",
                Some(FsNeed::OpenDir) => "what the run grants",
                Some(FsNeed::Temp) => "a temporary directory",
                None => "what no grant allows",
            };
            format!("error: fs refused {need}: {message}")
        }
    }
}

/// The hint lines saying what grants the refused calls' needs
/// (`refused.needed`), made of that data, never of a listed message: one
/// for `--allow-env=` naming each variable that's a [`hint_name`], one for
/// `--allow-run=` naming each program that's a [`hint_program`], and one
/// for `--allow-run-env=` naming each variable that's a [`hint_name`], a
/// glob never, each saying `some of what` when it can't show every name
/// of its kind; then each fs line once, then the exec root's, fixed text
/// naming no path.
/// `src/docs/run.md` §Errors and exit codes states the rules.
fn grant_hints(refused: &Refusals) -> Vec<String> {
    let mut names: Vec<&str> = Vec::new();
    let mut programs: Vec<&str> = Vec::new();
    let mut forwarded: Vec<&str> = Vec::new();
    let more = |kind: NameKind| refused.more_names.contains(&kind);
    let (mut some_env, mut some_run, mut some_run_env) = (
        more(NameKind::Env),
        more(NameKind::Run),
        more(NameKind::RunEnv),
    );
    let (mut read, mut link, mut write, mut open_dir, mut temp, mut run_in) =
        (false, false, false, false, false, false);
    for needed in &refused.needed {
        match needed {
            NeededGrant::Env(name) if hint_name(name) => names.push(name),
            NeededGrant::Env(_) => some_env = true,
            NeededGrant::Run(name) if hint_program(name) => programs.push(name),
            NeededGrant::Run(_) => some_run = true,
            NeededGrant::RunEnv(name) if hint_name(name) => forwarded.push(name),
            NeededGrant::RunEnv(_) => some_run_env = true,
            NeededGrant::Fs(FsNeed::Read) => read = true,
            NeededGrant::Fs(FsNeed::LinkTarget) => link = true,
            NeededGrant::Fs(FsNeed::Write) => write = true,
            NeededGrant::Fs(FsNeed::OpenDir) => open_dir = true,
            NeededGrant::Fs(FsNeed::Temp) => temp = true,
            NeededGrant::RunIn => run_in = true,
        }
    }
    let mut hints = Vec::new();
    for (flag, listed, some, refused_by) in [
        (ALLOW_ENV, names, some_env, "env"),
        (ALLOW_RUN, programs, some_run, "spawn"),
        (ALLOW_RUN_ENV, forwarded, some_run_env, "spawn"),
    ] {
        if !listed.is_empty() {
            let what = if some { "some of what" } else { "what" };
            hints.push(format!(
                "hint: add {flag}={} to grant {what} {refused_by} refused",
                listed.join(",")
            ));
        }
    }
    if read {
        hints.push(format!(
            "hint: add {ALLOW_READ} naming the directory to grant what fs refused"
        ));
    }
    if link {
        hints.push(
            "hint: fs refused a symlink leaving its root \
             (an absolute target, or a .. above the root, even one leading back in): \
             make the link relative within its root, \
             or open its target's directory by its own path with fs.open_dir"
                .to_owned(),
        );
    }
    if write {
        hints.push(format!(
            "hint: add {ALLOW_WRITE} naming the directory to grant what fs refused, \
             then open it with fs.open_dir and write through that handle"
        ));
    }
    if open_dir {
        hints.push(
            "hint: the run grants what fs refused: \
             open the directory with fs.open_dir and use that handle"
                .to_owned(),
        );
    }
    if temp {
        hints.push(format!("hint: add {ALLOW_TEMP} to grant what fs refused"));
    }
    if run_in {
        hints.push(format!(
            "hint: add {ALLOW_RUN_IN} naming the directory to grant what spawn refused"
        ));
    }
    hints
}

/// Whether the hint names `name`: ASCII letters, digits, and `_` only
/// (`src/docs/run.md` §Errors and exit codes has why).
fn hint_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Whether the hint names the program `name`: a plain name, of ASCII
/// letters, digits, `.`, `_`, `+`, and `-`, starting with a letter or a
/// digit, so a shell passes it as printed, it's never taken for a flag,
/// and it's never `.` or `..`; a name holding `/` is a path, which the
/// hint leaves to its reader.
fn hint_program(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
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
/// authority reaches, read from `environment` now, once ([`env_vars`]),
/// and those a child may be passed ([`spawn_env`]);
/// the OS's temporary directory when the authority makes temporary
/// directories, read from `environment` too, resolved against `cwd`
/// ([`temp_dir`]), by its real path now ([`real_temp_dir`]); the exit step
/// [`exit_now`]; the script's arguments; and the runtime's name and
/// version.
fn runtime_options(
    run: &RunInvocation,
    environment: &impl Environment,
    dirs: GrantedDirs,
    cwd: &Path,
) -> RuntimeOptions {
    let temp = matches!(
        run.grant,
        RunGrant::All | RunGrant::Named { temp: true, .. }
    );
    let env = env_vars(&run.grant, environment);
    let spawn_env = match &run.grant {
        // the same read, every variable
        RunGrant::All => env.clone(),
        RunGrant::Named { run, run_env, .. } => {
            let granted = !run.is_empty() || !dirs.exec_roots.is_empty();
            spawn_env(granted, run_env, environment)
        }
    };
    RuntimeOptions {
        authority: authority(&run.grant, dirs),
        env,
        spawn_env,
        temp_dir: temp.then(|| real_temp_dir(&cwd.join(temp_dir(environment)))),
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
/// directories `dirs` grants as it grants them, and, when `--allow-env=`
/// named any variable, the env host module with those names, and, when
/// `--allow-run=` named any program or `--allow-run-in=` any exec root,
/// the spawn host module with those programs and exec roots, each
/// forwarded what `--allow-run-env=` lists.
fn authority(grant: &RunGrant, dirs: GrantedDirs) -> Authority {
    let GrantedDirs { fs, exec_roots } = dirs;
    match grant {
        RunGrant::All => Authority::Unconfined,
        RunGrant::Named {
            env,
            temp,
            run,
            run_env,
        } => Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
            exit: Some(ExitGrant {}),
            env: (!env.is_empty()).then(|| EnvGrant { names: env.clone() }),
            fs: Some(FsGrant { temp: *temp, ..fs }),
            spawn: (!run.is_empty() || !exec_roots.is_empty()).then(|| SpawnGrant {
                programs: run
                    .iter()
                    .map(|name| {
                        let program = ProgramGrant {
                            forward: run_env.iter().cloned().collect(),
                        };
                        (name.clone(), program)
                    })
                    .collect(),
                exec_roots: exec_roots
                    .into_iter()
                    .map(|(path, dir)| {
                        let forward = run_env.iter().cloned().collect();
                        (path, ExecRootGrant { dir, forward })
                    })
                    .collect(),
            }),
        }),
    }
}

/// The environment variables a confined run's children may be passed,
/// read from `environment`: none when the run grants no program and no
/// exec root, as `granted` says; else the
/// base set every child is given ([`CHILD_BASE_ENV`]), then each exact
/// name `run_env` lists, each asked for by name, and, only when it lists
/// a glob, every variable a glob covers, from the whole environment's
/// listing, each whose name the env host module could ask for. A value
/// that isn't UTF-8 is decoded with U+FFFD in place of each invalid
/// sequence, as Node reads one.
fn spawn_env(
    granted: bool,
    run_env: &BTreeSet<EnvPattern>,
    environment: &impl Environment,
) -> EnvVars {
    if !granted {
        return EnvVars::default();
    }
    let exact = run_env.iter().filter_map(|pattern| match pattern {
        EnvPattern::Exact(name) => Some(name.as_str()),
        EnvPattern::Prefix(_) => None,
    });
    let mut read: Vec<(String, OsString)> = CHILD_BASE_ENV
        .into_iter()
        .chain(exact)
        .filter_map(|name| Some((name.to_owned(), environment.var(name)?)))
        .collect();
    if run_env
        .iter()
        .any(|pattern| matches!(pattern, EnvPattern::Prefix(_)))
    {
        read.extend(environment.vars().into_iter().filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            let globbed = run_env
                .iter()
                .any(|pattern| matches!(pattern, EnvPattern::Prefix(_)) && pattern.covers(&name));
            (globbed && env_name_fault(&name, true).is_none()).then_some((name, value))
        }));
    }
    read.into_iter()
        .map(|(name, value)| (name, value.to_string_lossy().into_owned()))
        .collect()
}

/// The OS's temporary directory, as Node's `os.tmpdir()` names it, read
/// from `environment`: the first of `TMPDIR`, `TMP`, and `TEMP` that's set
/// and not empty, else `/tmp`, a trailing `/` dropped but from `/` itself.
/// A value that isn't UTF-8 is decoded with U+FFFD in place of each invalid
/// sequence, as Node reads one.
fn temp_dir(environment: &impl Environment) -> PathBuf {
    let set = ["TMPDIR", "TMP", "TEMP"]
        .into_iter()
        .filter_map(|name| environment.var(name))
        .map(|value| value.to_string_lossy().into_owned())
        .find(|value| !value.is_empty());
    let mut dir = set.unwrap_or_else(|| "/tmp".to_owned());
    if dir.len() > 1 && dir.ends_with('/') {
        dir.pop();
    }
    PathBuf::from(dir)
}

/// `dir`, the OS's temporary directory, an absolute path, by its real path
/// as the run starts, so `fs.temp_dir`, which refuses a symlink on it,
/// makes its directories where it led then and never where a symlink the
/// run made since leads. With nothing there now, it's `dir` with its `.`
/// and `..` resolved by their text ([`normalize`]): `fs.temp_dir` fails
/// on it until the run makes it, by no symlink, and always when a symlink
/// is on it.
fn real_temp_dir(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| normalize(dir))
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
        RunGrant::Named { env, .. } => env
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
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    use std::path::{Path, PathBuf};

    use super::{
        EXIT_FAILURE, Ended, Environment, FsGrant, GrantedDir, GrantedDirs, RUNTIME_VERSION,
        exit_code, exit_now, real_temp_dir, runtime_options,
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

    /// `--allow-temp` grants `fs.temp_dir` and names the OS's temporary
    /// directory as Node's `os.tmpdir()` does: the first of `TMPDIR`, `TMP`,
    /// and `TEMP` set and not empty, else `/tmp`, a trailing `/` dropped but
    /// from `/`, a relative one resolved against the working directory; `-A`
    /// names it too; a run granting neither names none, and reads no
    /// variable for it.
    #[test]
    fn the_temp_flag_names_the_temporary_directory() {
        let named = |args: &[&str], vars: &[(&[u8], &[u8])]| {
            let environment = TestEnvironment::of(vars);
            let options = runtime_options(
                &invocation(args),
                &environment,
                GrantedDirs::default(),
                Path::new("/w"),
            );
            (options, environment.asked.take())
        };
        for (vars, expected) in [
            (&[][..], "/tmp"),
            (&[(&b"TMPDIR"[..], &b"/t/"[..])], "/t"),
            (&[(b"TMPDIR", b""), (b"TMP", b"/x"), (b"TEMP", b"/y")], "/x"),
            (&[(b"TEMP", b"rel/")], "/w/rel"),
            (&[(b"TMPDIR", b"/")], "/"),
        ] {
            let (options, _) = named(&["--allow-temp", "a.js"], vars);
            assert_eq!(options.temp_dir, Some(PathBuf::from(expected)), "{vars:?}");
            let gro_runtime::Authority::Confined(grants) = options.authority else {
                panic!("a run without -A is confined");
            };
            assert!(grants.fs.is_some_and(|fs| fs.temp));
            let (options, _) = named(&["-A", "a.js"], vars);
            assert_eq!(options.temp_dir, Some(PathBuf::from(expected)), "{vars:?}");
        }
        let (options, asked) = named(&["a.js"], &[(b"TMPDIR", b"/t")]);
        assert_eq!(options.temp_dir, None);
        assert_eq!(asked, Vec::<String>::new());
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert!(grants.fs.is_some_and(|fs| !fs.temp));
    }

    /// The temporary directory is named by its real path as the run
    /// starts, a symlink on it resolved, so `fs.temp_dir` refuses one made
    /// since; a path with nothing there passes with its `.` and `..`
    /// resolved by their text.
    #[test]
    fn the_temporary_directory_is_named_by_its_real_path() {
        let base = std::env::temp_dir().join(format!("gro_cli_temp_dir_{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
        let expected = std::fs::canonicalize(&real).unwrap();
        let named = [
            real_temp_dir(&base.join("link")),
            real_temp_dir(&base.join("link/./")),
            real_temp_dir(&base.join("missing/./x/..")),
        ];
        std::fs::remove_dir_all(&base).unwrap();
        assert_eq!(named, [expected.clone(), expected, base.join("missing")]);
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
            runtime_options(
                &run,
                &TestEnvironment::default(),
                GrantedDirs::default(),
                Path::new("/w")
            )
            .args,
            script_args
        );
        let Ok(Invocation::Sized(Task::Run(run))) = parse(&["run", "a.js"]) else {
            panic!("runs a.js");
        };
        assert!(
            runtime_options(
                &run,
                &TestEnvironment::default(),
                GrantedDirs::default(),
                Path::new("/w")
            )
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
            runtime_options(
                &run,
                &TestEnvironment::default(),
                GrantedDirs::default(),
                Path::new("/w")
            )
            .runtime_version,
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
        let options = runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
        assert_eq!(
            options.authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant {}),
                env: None,
                fs: Some(FsGrant::default()),
                spawn: None,
            })
        );
        assert_eq!(options.env, gro_runtime::EnvVars::default());
        assert_eq!(options.spawn_env, gro_runtime::EnvVars::default());
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
        let options = runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
        assert_eq!(
            options.authority,
            gro_runtime::Authority::Confined(gro_runtime::Grants {
                args: Some(gro_runtime::ArgsGrant {}),
                exit: Some(gro_runtime::ExitGrant {}),
                env: Some(gro_runtime::EnvGrant {
                    names: ["A", "B", "C"].map(ToOwned::to_owned).into(),
                }),
                fs: Some(FsGrant::default()),
                spawn: None,
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
            let options =
                runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
            assert_eq!(
                options.authority,
                gro_runtime::Authority::Unconfined,
                "{args:?}"
            );
            // asked by name only for the temporary directory, which `-A` names
            assert_eq!(*environment.asked.borrow(), ["TMPDIR", "TMP", "TEMP"]);
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
        let id = gro_runtime::file_id::FileId {
            dev: 1,
            ino: 2,
            born: None,
        };
        let granted = |real: &str| GrantedDir {
            real: real.into(),
            id,
        };
        let fs = FsGrant {
            read: [("/r".into(), granted("/r"))].into(),
            write: [("/w".into(), granted("/w"))].into(),
            temp: false,
        };
        let run = invocation(&["--allow-read=/r", "a.js"]);
        let options = runtime_options(
            &run,
            &TestEnvironment::default(),
            GrantedDirs {
                fs: fs.clone(),
                ..GrantedDirs::default()
            },
            Path::new("/w"),
        );
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert_eq!(grants.fs, Some(fs.clone()));
        let run = invocation(&["--allow-read=/r", "-A", "a.js"]);
        let dirs = GrantedDirs {
            fs,
            ..GrantedDirs::default()
        };
        let options = runtime_options(&run, &TestEnvironment::default(), dirs, Path::new("/w"));
        assert_eq!(options.authority, gro_runtime::Authority::Unconfined);
    }

    /// The hints say what grants each need, in the order first needed for
    /// env, one line a flag: env's first, naming the variables a shell
    /// passes as printed, `some of what` when it can't show every one, a
    /// need past the record's bound included, and no line when it can show
    /// none; then each fs line once, read, a symlink out of its root,
    /// write, then a handle to open, each fixed text naming no path.
    #[test]
    fn the_hints_name_env_s_variables_and_fs_s_kinds() {
        use gro_runtime::{FsNeed, NeededGrant, Refusals};

        use super::grant_hints;

        let hints = |needed: &[NeededGrant], more_names: bool| {
            grant_hints(&Refusals {
                needed: needed.to_vec(),
                more_names: if more_names {
                    vec![gro_runtime::NameKind::Env]
                } else {
                    Vec::new()
                },
                ..Refusals::default()
            })
        };
        let env = |name: &str| NeededGrant::Env(name.to_owned());
        let read = "hint: add --allow-read naming the directory to grant what fs refused";
        let write = "hint: add --allow-write naming the directory to grant what fs refused, \
                     then open it with fs.open_dir and write through that handle";
        let open_dir = "hint: the run grants what fs refused: \
                        open the directory with fs.open_dir and use that handle";
        let link = "hint: fs refused a symlink leaving its root \
                    (an absolute target, or a .. above the root, even one leading back in): \
                    make the link relative within its root, \
                    or open its target's directory by its own path with fs.open_dir";
        assert!(hints(&[], false).is_empty());
        assert_eq!(
            hints(
                &[
                    NeededGrant::Fs(FsNeed::OpenDir),
                    NeededGrant::Fs(FsNeed::Write),
                    NeededGrant::Fs(FsNeed::LinkTarget),
                    env("HOME"),
                    NeededGrant::Fs(FsNeed::Read),
                    env("A_1"),
                ],
                false
            ),
            [
                "hint: add --allow-env=HOME,A_1 to grant what env refused",
                read,
                link,
                write,
                open_dir,
            ]
        );
        assert_eq!(hints(&[NeededGrant::Fs(FsNeed::Read)], false), [read]);
        assert_eq!(hints(&[NeededGrant::Fs(FsNeed::LinkTarget)], false), [link]);
        assert_eq!(hints(&[NeededGrant::Fs(FsNeed::Write)], false), [write]);
        assert_eq!(
            hints(&[NeededGrant::Fs(FsNeed::OpenDir)], false),
            [open_dir]
        );
        assert_eq!(
            hints(&[NeededGrant::Fs(FsNeed::Read)], true),
            [read],
            "a variable past the bound makes no fs line"
        );
        assert_eq!(
            hints(&[env("A"), env("A B")], false),
            ["hint: add --allow-env=A to grant some of what env refused"]
        );
        assert_eq!(
            hints(&[env("A")], true),
            ["hint: add --allow-env=A to grant some of what env refused"]
        );
        assert_eq!(
            hints(&[env("A B")], false),
            Vec::<String>::new(),
            "an env name it can't show makes no line"
        );
    }

    /// The spawn hints name the programs a shell passes as printed, never
    /// a path or a name a shell or the CLI would read otherwise, and the
    /// variables to forward as env's rule names them, never a glob, each
    /// line `some of what` when it can't show every name of its kind, in
    /// the order env, programs, variables, then fs.
    #[test]
    fn the_spawn_hints_name_plain_programs_and_variables() {
        use gro_runtime::{FsNeed, NameKind, NeededGrant, Refusals};

        use super::grant_hints;

        let hints = |needed: &[NeededGrant], more_names: &[NameKind]| {
            grant_hints(&Refusals {
                needed: needed.to_vec(),
                more_names: more_names.to_vec(),
                ..Refusals::default()
            })
        };
        let run = |name: &str| NeededGrant::Run(name.to_owned());
        let run_env = |name: &str| NeededGrant::RunEnv(name.to_owned());
        assert_eq!(
            hints(
                &[
                    NeededGrant::Fs(FsNeed::Temp),
                    run_env("CARGO_HOME"),
                    run("git"),
                    NeededGrant::Env("HOME".to_owned()),
                    run("c++"),
                    run("x.py"),
                    run("a_b-1"),
                ],
                &[]
            ),
            [
                "hint: add --allow-env=HOME to grant what env refused",
                "hint: add --allow-run=git,c++,x.py,a_b-1 to grant what spawn refused",
                "hint: add --allow-run-env=CARGO_HOME to grant what spawn refused",
                "hint: add --allow-temp to grant what fs refused",
            ]
        );
        for unshown in [
            "./bin/tool",
            "-x",
            ".",
            "..",
            ".hidden",
            "_x",
            "+x",
            "a b",
            "$(x)",
            "a;b",
            "é",
            "a*",
        ] {
            assert_eq!(
                hints(&[run(unshown), run("git")], &[]),
                ["hint: add --allow-run=git to grant some of what spawn refused"],
                "{unshown}"
            );
            assert!(hints(&[run(unshown)], &[]).is_empty(), "{unshown}");
        }
        assert_eq!(
            hints(&[run_env("CARGO_*"), run_env("A")], &[]),
            ["hint: add --allow-run-env=A to grant some of what spawn refused"]
        );
        assert!(hints(&[run_env("CARGO_*")], &[]).is_empty());
        assert_eq!(
            hints(&[run("git"), run_env("A")], &[NameKind::Run]),
            [
                "hint: add --allow-run=git to grant some of what spawn refused",
                "hint: add --allow-run-env=A to grant what spawn refused",
            ]
        );
    }

    /// A listed refusal's line names what refused it, and for `fs` the
    /// kind of grant it needed in the words of that kind's hint, or that it
    /// needed none, its message escaped.
    #[test]
    fn a_listed_refusal_names_what_refused_it_and_an_fs_one_its_need() {
        use gro_runtime::{FsNeed, Refusal, RefusedBy};

        use super::refusal_line;

        let line = |by: RefusedBy, message: &str| {
            refusal_line(&Refusal {
                by,
                message: message.to_owned(),
            })
        };
        assert_eq!(
            line(RefusedBy::Fs(Some(FsNeed::Read)), "not_granted: /r"),
            "error: fs refused a read: not_granted: /r"
        );
        assert_eq!(
            line(RefusedBy::Fs(Some(FsNeed::LinkTarget)), "not_granted: /l"),
            "error: fs refused a symlink leaving its root: not_granted: /l"
        );
        assert_eq!(
            line(RefusedBy::Fs(Some(FsNeed::Write)), "not_granted: /w"),
            "error: fs refused a write: not_granted: /w"
        );
        assert_eq!(
            line(RefusedBy::Fs(Some(FsNeed::OpenDir)), "not_granted: /o"),
            "error: fs refused what the run grants: not_granted: /o"
        );
        assert_eq!(
            line(RefusedBy::Fs(None), "not_granted: /n\u{1b}[2K\u{2028}"),
            "error: fs refused what no grant allows: not_granted: /n\\u{1b}[2K\\u{2028}"
        );
        assert_eq!(
            line(RefusedBy::Env, "not_granted: HOME\n"),
            "error: env refused: not_granted: HOME\\u{a}"
        );
        assert_eq!(
            line(RefusedBy::Import, "cannot resolve 'ext:x'"),
            "error: import() refused: cannot resolve 'ext:x'"
        );
        assert_eq!(
            line(RefusedBy::Spawn, "not_granted: git\u{1b}[2K"),
            "error: spawn refused: not_granted: git\\u{1b}[2K"
        );
    }

    /// A flag's directory is noted when its real path isn't its text,
    /// resolved against the working directory with its `.` and `..`
    /// resolved by their text: through a symlink, a `..` after one
    /// included, and never for a path that is real already however it's
    /// spelled; the typed text and the real path printed escaped.
    #[test]
    fn a_flag_through_a_symlink_is_noted_with_its_real_path() {
        use std::path::Path;

        use super::grant_note;
        use crate::cli::DirFlag;

        let note = |flag: DirFlag, dir: &str, real: &str| {
            grant_note(flag, dir, &Path::new("/c").join(dir), Path::new(real))
        };
        for dir in [
            "d", "./d", "d/", "d/.", "x/../d", "/c/d", "/c//d/", "/../c/d",
        ] {
            assert_eq!(note(DirFlag::Read, dir, "/c/d"), None, "{dir}");
        }
        assert_eq!(
            note(DirFlag::Read, "l", "/elsewhere"),
            Some("note: --allow-read=l grants /elsewhere".to_owned())
        );
        assert_eq!(
            note(DirFlag::Write, "l/..", "/r"),
            Some("note: --allow-write=l/.. grants /r".to_owned())
        );
        assert_eq!(
            note(DirFlag::Read, "l\u{1b}[2K", "/r\u{202e}"),
            Some("note: --allow-read=l\\u{1b}[2K grants /r\\u{202e}".to_owned())
        );
    }

    /// `--allow-run=` grants the spawn host module its programs, each
    /// forwarded what `--allow-run-env=` lists, and the run reads for
    /// children the base set and each exact name, by name, the whole
    /// environment's listing only when a glob is granted, keeping what a
    /// glob covers; a run granting no program reads none of it, and `-A`
    /// hands children what it reads for `env`, everything.
    #[test]
    fn allow_run_grants_its_programs_and_reads_only_what_children_may_get() {
        use gro_runtime::{EnvPattern, ProgramGrant, SpawnGrant};

        let vars: &[(&[u8], &[u8])] = &[
            (b"PATH", b"/bin"),
            (b"HOME", b"/h"),
            (b"CARGO_HOME", b"/c\xff"),
            (b"CARGO\xff", b"x"),
            (b"XCARGO_HOME", b"x"),
            (b"SECRET", b"s"),
            (b"TOKEN", b"t"),
        ];
        let run = invocation(&[
            "--allow-run=git,cargo",
            "--allow-run-env=TOKEN,UNSET",
            "a.js",
        ]);
        let environment = TestEnvironment::of(vars);
        let options = runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
        let forward = vec![
            EnvPattern::Exact("TOKEN".to_owned()),
            EnvPattern::Exact("UNSET".to_owned()),
        ];
        let program = ProgramGrant { forward };
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert_eq!(
            grants.spawn,
            Some(SpawnGrant {
                programs: [
                    ("cargo".to_owned(), program.clone()),
                    ("git".to_owned(), program),
                ]
                .into(),
                exec_roots: BTreeMap::new(),
            })
        );
        assert_eq!(
            *environment.asked.borrow(),
            [
                "PATH",
                "HOME",
                "TMPDIR",
                "TERM",
                "NO_COLOR",
                "FORCE_COLOR",
                "TOKEN",
                "UNSET"
            ]
        );
        assert!(!environment.listed.get());
        let read: Vec<(&str, &str)> = options.spawn_env.iter().collect();
        assert_eq!(read, [("PATH", "/bin"), ("HOME", "/h"), ("TOKEN", "t")]);
        assert_eq!(options.env, gro_runtime::EnvVars::default());

        let run = invocation(&["--allow-run=git", "--allow-run-env=CARGO_*", "a.js"]);
        let environment = TestEnvironment::of(vars);
        let options = runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
        assert!(environment.listed.get());
        let read: Vec<(&str, &str)> = options.spawn_env.iter().collect();
        assert_eq!(
            read,
            [
                ("PATH", "/bin"),
                ("HOME", "/h"),
                ("CARGO_HOME", "/c\u{fffd}")
            ]
        );

        // the variables' flag names nothing without a program, so a run
        // granting no program reads nothing for children
        let run = invocation(&["--allow-env=HOME", "a.js"]);
        let environment = TestEnvironment::of(vars);
        let options = runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
        assert_eq!(*environment.asked.borrow(), ["HOME"]);
        assert_eq!(options.spawn_env, gro_runtime::EnvVars::default());

        let run = invocation(&["--allow-run=git", "-A", "a.js"]);
        let environment = TestEnvironment::of(vars);
        let options = runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
        assert_eq!(options.authority, gro_runtime::Authority::Unconfined);
        assert_eq!(options.spawn_env, options.env);
        assert_eq!(options.spawn_env.iter().count(), 6);
    }

    /// `--allow-run-in=` grants `spawn` with its exec roots, each as the
    /// flags resolved it, passed what `--allow-run-env=` forwards, though
    /// no program is named, and reads what a child may be given; with
    /// neither flag, the run grants no `spawn` and reads nothing for
    /// children.
    #[test]
    fn allow_run_in_grants_exec_roots_and_reads_what_children_may_get() {
        let id = gro_runtime::file_id::FileId {
            dev: 1,
            ino: 2,
            born: None,
        };
        let dir = GrantedDir {
            real: "/real/x".into(),
            id,
        };
        let dirs = GrantedDirs {
            exec_roots: [(PathBuf::from("/x"), dir.clone())].into(),
            ..GrantedDirs::default()
        };
        let run = invocation(&["--allow-run-in=/x", "--allow-run-env=A", "a.js"]);
        assert!(super::grants_programs(&run.grant, &run.dirs));
        let environment = TestEnvironment::of(&[(b"A", b"1"), (b"PATH", b"/bin")]);
        let options = runtime_options(&run, &environment, dirs, Path::new("/w"));
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        let forward = vec![gro_runtime::EnvPattern::Exact("A".to_owned())];
        assert_eq!(
            grants.spawn,
            Some(gro_runtime::SpawnGrant {
                programs: BTreeMap::new(),
                exec_roots: [(
                    PathBuf::from("/x"),
                    gro_runtime::ExecRootGrant { dir, forward }
                )]
                .into(),
            })
        );
        assert_eq!(
            options.spawn_env.iter().collect::<Vec<_>>(),
            [("PATH", "/bin"), ("A", "1")]
        );
        let run = invocation(&["--allow-read=/x", "a.js"]);
        assert!(!super::grants_programs(&run.grant, &run.dirs));
        let environment = TestEnvironment::of(&[(b"PATH", b"/bin")]);
        let options = runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
        let gro_runtime::Authority::Confined(grants) = options.authority else {
            panic!("a run without -A is confined");
        };
        assert_eq!(grants.spawn, None);
        assert!(environment.asked.borrow().is_empty());
    }

    /// The variable a child of [`a_refused_raise_of_the_open_files_limit_fails_nothing`]
    /// runs its half by.
    const READY_CHILD: &str = "GRO_CLI_TEST_READY_CHILD";

    /// A run that grants a program is made ready to start them though the
    /// kernel refuses to raise the soft limit on open descriptors, as Node
    /// goes on: the raise is best-effort. In a child process, since making
    /// ready marks every inherited descriptor close-on-exec.
    #[test]
    fn a_refused_raise_of_the_open_files_limit_fails_nothing() {
        if std::env::var_os(READY_CHILD).is_some() {
            let refused = || Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            assert!(super::ready_to_spawn(refused).is_ok());
            return;
        }
        let test = "run::tests::a_refused_raise_of_the_open_files_limit_fails_nothing";
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--nocapture", "--quiet"])
            .env(READY_CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // a filter that matches no test exits 0 as well
        assert!(String::from_utf8_lossy(&output.stdout).contains(" 1 passed;"));
    }

    /// The variables' `Debug` names them and prints no value.
    #[test]
    fn the_variables_debug_prints_no_value() {
        let run = invocation(&["--allow-env=TOKEN", "a.js"]);
        let environment = TestEnvironment::of(&[(b"TOKEN", b"secret-value")]);
        let options = runtime_options(&run, &environment, GrantedDirs::default(), Path::new("/w"));
        let printed = format!("{options:?}");
        assert!(printed.contains("TOKEN"), "{printed}");
        assert!(!printed.contains("secret-value"), "{printed}");
    }
}
