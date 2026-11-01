//! `fgro`: gro on its own JS runtime, the binary's name while it's developed
//! beside the Node `gro`.
//!
//! `fgro run <file>` runs an ES module from disk (a path relative to the
//! working directory, or absolute) until its event loop drains, TypeScript
//! stripped of its types first. Its imports resolve strictly: files by their
//! full names, and packages in `node_modules` through their `"exports"`. It
//! takes only the entry file: there are no script arguments yet. Errors print
//! to stderr, prefixed `error:`; an uncaught error's report carries its stack
//! with source positions (`file:///path:line:column`).
//!
//! `fgro eval <module> [--driver -] [--export NAME] [--jit] [--timeout MS]
//! [--graph]` evaluates a module to JSON data and prints one envelope on stdout
//! (`gro_runtime::eval`): with no driver, the module's export (`default`
//! unless `--export` names another); with `--driver -`, a driver module read
//! from stdin, whose default export is called with the module's namespace.
//! It runs jitless unless `--jit`, with code generation from strings off,
//! the locale `en-US`, the clock and randomness denied, the heap capped, and
//! a timeout (`--timeout`, else `gro_runtime::eval::DEFAULT_TIMEOUT`), and
//! its console writes every level to stderr, uncolored. It clears its
//! environment but `TZ` before it starts a thread, so past exec no variable
//! configures it or a library it runs, and refuses a time zone other than
//! UTC, which ICU reads from `TZ`. Out of memory, the runtime writes a
//! prepared envelope through its own handle to stdout, duplicated here
//! before the run. With `--graph`, the envelope also carries the module
//! graph the run loaded, as `graph` (`gro_runtime::eval::GraphDump`).
//! Once eval starts, the envelope is always written, `ok: false` with an
//! error kind on failure, unless the process dies first (a panic, or memory
//! outside V8's heap running out); a usage error prints to stderr with no
//! envelope.
//!
//! Every command but `eval` runs on one thread with [`STACK_SIZE`] reserved,
//! not on the main thread, so a debug build stays inside the 16 MiB of stack
//! tsv's TypeScript strip is sized for (a release build needs 2 MiB), whatever
//! the platform's main thread has. A module nested or chained past tsv's
//! depth limits fails to load (exit `1`) with a message naming the limit,
//! rather than overflowing the stack.
//!
//! `eval` keeps its process confinable: the main thread parses the
//! arguments, closes every file descriptor above stderr the process
//! inherited (which the box, gating only new opens, wouldn't reach; failing
//! that, eval fails as `environment`), opens the phases file in a phases
//! build, clears the environment but `TZ` (failing that, as `environment`),
//! and reads the driver from stdin, the working directory, and the
//! out-of-memory handle; a thread with [`STACK_SIZE`] reserved resolves the
//! entry and computes the box (`gro_runtime::eval::prepass`) and is joined;
//! the main thread, then the process's only one, confines it
//! (`gro_runtime::confine`); and only then does a second thread with
//! [`STACK_SIZE`] reserved start the engine and evaluate, so every thread
//! the engine and the runtime start comes after the box. The box is Linux's,
//! and needs Landlock ABI 6; elsewhere, and without it, eval refuses. A build
//! with no snapshot, whose runtime would read its sources from the build
//! tree, outside the box, refuses once the entry resolves, before the box,
//! so the same on every host.
//!
//! `fgro --version` prints one line,
//! `fgro <version> (commit <sha>[-dirty], V8 <version>, snapshot)`: the git
//! commit it was built from, full, with `-dirty` when the working tree
//! differed from it (`commit unknown` when no build embedded one, as a plain
//! `cargo build` doesn't), and whether a startup snapshot is embedded
//! (`no snapshot` otherwise), then `, phases` in a phases build and
//! `, prices` in a priced build.
//!
//! Exit codes, the same for `run` and `eval`:
//!
//! - `0`: the program ran to completion (for `eval`, its value serialized),
//!   `--help`, or `--version`
//! - `1`: the program threw, rejected a promise no one handled, or failed to
//!   load a module (a TypeScript module the strip refuses included), or the
//!   runtime failed to start; for `eval`, any failure after the entry
//!   resolved, whose kind the envelope names
//! - `2`: a usage error, so the command line must change: an unknown
//!   subcommand or flag, a missing argument (including no arguments at all),
//!   an argument that isn't UTF-8, `eval`'s `--driver` other than `-` or
//!   with `--export` or its `--timeout` below 1, or an entry file that
//!   doesn't resolve
//!   (missing, not a `.js`, `.mjs`, `.ts`, or `.mts` file, or a `.js` file
//!   not under `"type": "module"`), or, for `eval`, one no project owns
//!   (inside `node_modules`, or with no project root above it)
//!
//! Built with the `phases` feature, which release and snapshot builds never
//! enable, `fgro` takes a timestamp at each phase boundary of `run` and `eval`
//! and writes them at exit to the file `GRO_PHASES_FILE` names
//! (`gro_runtime::phases`), for `cargo xtask bench --phases`; eval opens the
//! file before its box goes up. Without the feature it reads no such
//! variable.
//!
//! Built with the `prices` feature, which release and snapshot builds never
//! enable, `fgro` reads `GRO_PRICES_OFF`, a comma-separated list of eval's
//! defaults to turn off, one at a time, for the bench's prices suite
//! (`gro_runtime::prices`); a name it doesn't know is a usage error. Without
//! the feature it reads no such variable.
//!
//! Built with the `snapshot` feature, `fgro` starts from a startup snapshot
//! embedded at compile time, and runs off its build tree. `cargo xtask build`
//! makes that build in two stages: a plain `fgro` writes the snapshot with
//! `fgro make-snapshot <dir>` (internal to the xtask, so not in `--help`),
//! then `fgro` is rebuilt with the feature, reading the directory from
//! `GRO_SNAPSHOT_DIR`; that build also embeds the commit, from `GRO_COMMIT`.
//! Without the feature, `fgro` evaluates the runtime's sources at startup,
//! reading the web extensions' JS from the build tree.

mod color;

use std::ffi::OsString;
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::os::fd::AsFd as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use argh::{EarlyExit, FromArgs};
use gro_runtime::confine::{Confined, Confinement};
use gro_runtime::engine::{EngineOptions, init_confined_engine, init_engine};
use gro_runtime::eval::{Envelope, ErrorKind, EvalError, EvalOptions, Prepass, RuntimeInfo};
use gro_runtime::phases::{self, Phase};
use gro_runtime::prices::{self, EvalDefault};
use gro_runtime::snapshot::StartupSnapshot;
use gro_runtime::{RunError, RuntimeOptions, STACK_SIZE};

/// The snapshot this binary was built with: `snapshot.bin` and `residual.rs`
/// from `fgro make-snapshot`, in the directory `GRO_SNAPSHOT_DIR` names.
#[cfg(feature = "snapshot")]
mod embedded {
    pub static SNAPSHOT: &[u8] = include_bytes!(concat!(env!("GRO_SNAPSHOT_DIR"), "/snapshot.bin"));
    include!(concat!(env!("GRO_SNAPSHOT_DIR"), "/residual.rs"));
}

#[cfg(feature = "snapshot")]
const STARTUP_SNAPSHOT: Option<StartupSnapshot> = Some(StartupSnapshot {
    bytes: embedded::SNAPSHOT,
    residual_lazy_js: embedded::RESIDUAL_LAZY_JS,
});

#[cfg(not(feature = "snapshot"))]
const STARTUP_SNAPSHOT: Option<StartupSnapshot> = None;

/// The internal subcommand that writes the snapshot, handled before argh so
/// it stays out of `--help`.
const MAKE_SNAPSHOT: &str = "make-snapshot";

/// The caller must change the arguments before re-running.
const EXIT_USAGE: u8 = 2;

/// The program failed.
const EXIT_FAILURE: u8 = 1;

/// A panic, Rust's own exit code for one, restated because the panicking
/// thread is joined rather than unwinding out of `main`. Only a debug build
/// reaches it: the release profile aborts on a panic.
const EXIT_PANIC: u8 = 101;

/// gro on its own JS runtime (experimental)
#[derive(FromArgs)]
struct Args {
    /// print the version, the git commit it was built from, the V8 version,
    /// and whether a startup snapshot is embedded
    #[argh(switch)]
    version: bool,
    #[argh(subcommand)]
    command: Option<Command>,
}

/// The commit this binary was built from, read from `GRO_COMMIT` at compile
/// time, never at runtime: a full SHA, with `-dirty` appended when the working
/// tree differed from it. `cargo xtask build` sets it for its stage-two build,
/// and only that build's `snapshot` feature reads it, so a plain `cargo build`
/// has none whatever the shell sets; unset or empty, there is none either.
/// Cargo records the variable as an input, so a changed value rebuilds the
/// crate.
const COMMIT: Option<&str> = if cfg!(feature = "snapshot") {
    commit(option_env!("GRO_COMMIT"))
} else {
    None
};

/// What `--version` prints for a build without [`COMMIT`].
const COMMIT_UNKNOWN: &str = "unknown";

/// The suffix [`COMMIT`] carries when the working tree differed from the commit.
const DIRTY_SUFFIX: &str = "-dirty";

/// Validates a `GRO_COMMIT` value: `None` when it's unset or empty, else a
/// full SHA (40 or 64 lowercase hex digits), optionally followed by
/// [`DIRTY_SUFFIX`]. A malformed value panics, which, evaluated for
/// [`COMMIT`], fails the build rather than printing it.
const fn commit(value: Option<&'static str>) -> Option<&'static str> {
    let Some(value) = value else {
        return None;
    };
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let suffix = DIRTY_SUFFIX.as_bytes();
    let mut sha_len = bytes.len();
    if sha_len > suffix.len() {
        let start = sha_len - suffix.len();
        let mut i = 0;
        while i < suffix.len() && bytes[start + i] == suffix[i] {
            i += 1;
        }
        if i == suffix.len() {
            sha_len = start;
        }
    }
    assert!(
        sha_len == 40 || sha_len == 64,
        "GRO_COMMIT must be a full commit SHA, optionally followed by `-dirty`"
    );
    let mut i = 0;
    while i < sha_len {
        assert!(
            matches!(bytes[i], b'0'..=b'9' | b'a'..=b'f'),
            "GRO_COMMIT's SHA must be lowercase hex"
        );
        i += 1;
    }
    Some(value)
}

/// The `--version` line:
/// `fgro <version> (commit <sha>[-dirty], V8 <version>, snapshot)`, with
/// `commit unknown` for a build without [`COMMIT`] and `no snapshot` for a
/// build without one, and `, phases` before the parenthesis closes in a
/// build that records phases (`gro_runtime::phases::ENABLED`), and
/// `, prices` in one that reads the prices switches
/// (`gro_runtime::prices::ENABLED`).
fn version_line() -> String {
    format!(
        "fgro {} (commit {}, V8 {}, {}{}{})",
        env!("CARGO_PKG_VERSION"),
        COMMIT.unwrap_or(COMMIT_UNKNOWN),
        gro_runtime::v8_version(),
        if STARTUP_SNAPSHOT.is_some() {
            "snapshot"
        } else {
            "no snapshot"
        },
        if phases::ENABLED { ", phases" } else { "" },
        if prices::ENABLED { ", prices" } else { "" }
    )
}

#[derive(FromArgs)]
#[argh(subcommand)]
enum Command {
    Run(RunArgs),
    Eval(EvalArgs),
}

/// run an ES module (.js, .mjs, .ts, or .mts) from disk
#[derive(FromArgs)]
#[argh(subcommand, name = "run")]
struct RunArgs {
    /// the module to run: a path relative to the working directory, or absolute
    #[argh(positional)]
    file: PathBuf,
}

/// evaluate an ES module to JSON data, written as one envelope on stdout
#[derive(FromArgs)]
#[argh(subcommand, name = "eval")]
struct EvalArgs {
    /// the module to evaluate: a path relative to the working directory, or absolute
    #[argh(positional)]
    module: PathBuf,
    /// read a driver module from stdin (pass `-`), whose default export is
    /// called with the module's namespace and returns the value
    #[argh(option)]
    driver: Option<String>,
    /// the export to serialize, with no driver (default: default)
    #[argh(option)]
    export: Option<String>,
    /// turn V8's JIT on, and with it WebAssembly
    #[argh(switch)]
    jit: bool,
    /// stop the eval after this many milliseconds, at least 1 (default: 10000)
    #[argh(option)]
    timeout: Option<u64>,
    /// add the module graph to the envelope, as `graph`
    #[argh(switch)]
    graph: bool,
}

/// The `--driver` value that reads the driver from stdin, the only one so far.
const DRIVER_STDIN: &str = "-";

fn main() -> ExitCode {
    phases::start();
    let code = main_code();
    phases::finish();
    ExitCode::from(code)
}

/// [`main`]'s work, returning the exit code.
fn main_code() -> u8 {
    // a priced build's switches, refused before anything runs when one is
    // unknown; nothing without the `prices` feature
    if let Err(message) = prices::start() {
        eprintln!("error: {message}");
        return EXIT_USAGE;
    }
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
            return EXIT_USAGE;
        }
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // eval's process must keep one thread until it is confined, so it runs
    // here and starts its own
    if runs_on_main_thread(&args) {
        return run(&args);
    }
    run_on_sized_stack(&args)
}

/// Whether `args` parse as `fgro eval`, which confines its process and so
/// runs on the main thread: decided from what argh parses, not the first
/// argument, so every spelling it accepts (`fgro -- eval`) runs there.
fn runs_on_main_thread(args: &[&str]) -> bool {
    args.first() != Some(&MAKE_SNAPSHOT)
        && matches!(
            Args::from_args(&["fgro"], args),
            Ok(Args {
                version: false,
                command: Some(Command::Eval(_)),
            })
        )
}

/// Runs [`run`] on a thread named `fgro` with [`STACK_SIZE`] reserved, so the
/// runtime, the module loads, and the strip all share the one stated stack:
/// tsv's, so a debug build keeps inside its 16 MiB contract (a release build
/// needs 2 MiB), and the main thread's platform stack (commonly 8 MiB on
/// Linux, 1 MiB on Windows) never decides whether a module within tsv's depth
/// limits strips.
///
/// Not `tsv_ts::run_with_stack`: that names its thread `tsv`, and resumes a
/// panic rather than mapping it to [`EXIT_PANIC`], and drops its work when the
/// spawn is refused.
///
/// A refused spawn runs on this thread instead, and says so, since a module
/// could then overflow the inherited stack with nothing saying why. That
/// rescues only the commands that don't start V8 (`--version`, `--help`, a
/// usage error): V8 needs threads of its own, so a process that can't start
/// this one can't start the runtime either.
fn run_on_sized_stack(args: &[&str]) -> u8 {
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("fgro".to_owned())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || run(args));
        match spawned {
            // a panic has printed by the time it's joined
            Ok(handle) => handle.join().unwrap_or(EXIT_PANIC),
            Err(error) => {
                eprintln!(
                    "warning: could not start the fgro thread ({error}); running on the \
                     inherited stack"
                );
                run(args)
            }
        }
    })
}

/// Runs `fgro` with `args` (the program name excluded), returning the exit code.
fn run(args: &[&str]) -> u8 {
    if args.first() == Some(&MAKE_SNAPSHOT) {
        // no flags, so `--help` can't become a directory's name
        let [_, dir] = args else {
            eprintln!("usage: fgro {MAKE_SNAPSHOT} <dir>");
            return EXIT_USAGE;
        };
        if dir.starts_with('-') {
            eprintln!("usage: fgro {MAKE_SNAPSHOT} <dir> (it takes no flags)");
            return EXIT_USAGE;
        }
        return make_snapshot(Path::new(dir)).map_or_else(
            |error| {
                eprintln!("error: {error}");
                EXIT_FAILURE
            },
            |()| 0,
        );
    }
    match Args::from_args(&["fgro"], args) {
        Ok(Args {
            version: true,
            command: None,
        }) => {
            println!("{}", version_line());
            0
        }
        Ok(Args {
            version: true,
            command: Some(_),
        }) => {
            eprintln!("error: --version takes no subcommand");
            EXIT_USAGE
        }
        Ok(Args {
            version: false,
            command: None,
        }) => {
            eprintln!("error: a subcommand is required\nRun fgro --help for more information.");
            EXIT_USAGE
        }
        Ok(Args {
            version: false,
            command: Some(Command::Run(RunArgs { file })),
        }) => {
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
            let result = std::env::current_dir()
                .map_err(RunError::Start)
                .and_then(|cwd| gro_runtime::run_main(engine, &options, &file, &cwd));
            match result {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("error: {error}");
                    exit_code(&error)
                }
            }
        }
        Ok(Args {
            version: false,
            command: Some(Command::Eval(args)),
        }) => eval(args),
        Err(early) => early_exit(early),
    }
}

/// The exit code of argh's early exit, `--help` or a usage error, printed:
/// argh's `from_env` exits `1` on a usage error, so its result is mapped by
/// hand.
fn early_exit(EarlyExit { output, status }: EarlyExit) -> u8 {
    if status.is_ok() {
        println!("{}", output.trim_end());
        0
    } else {
        eprintln!("{}", output.trim_end());
        EXIT_USAGE
    }
}

/// Runs `fgro eval` on the main thread: [`confine_eval`], then evaluates on
/// eval's engine on another thread, and prints the envelope to stdout,
/// returning the exit code. A usage error prints to stderr with no envelope.
fn eval(args: EvalArgs) -> u8 {
    let jit = args.jit;
    let (prepass, confined) = match confine_eval(args) {
        Ok(confined) => confined,
        Err(code) => return code,
    };
    let unstarted = prepass.failed(EvalError::new(
        ErrorKind::Internal,
        "could not start the thread that evaluates",
    ));
    // every thread the engine starts comes after the box, spawned from here
    let evaluated = on_sized_thread("fgro", move || {
        match init_confined_engine(confined, &gro_runtime::eval::engine_options(jit)) {
            Ok(engine) => {
                phases::mark(Phase::Engine);
                gro_runtime::eval::eval(engine, prepass)
            }
            Err(error) => prepass.failed(EvalError::new(ErrorKind::Internal, error.to_string())),
        }
    });
    let code = match evaluated {
        Joined::Returned(envelope) => write_envelope(&envelope),
        Joined::Unstarted(error) => write_envelope(&with_error(unstarted, &error)),
        Joined::Panicked => EXIT_PANIC,
    };
    phases::mark(Phase::Write);
    code
}

/// `eval`'s path up to and including the box, on the main thread: checks the
/// flags, closes the descriptors the process inherited, clears its environment
/// but `TZ`, reads the working directory and, when asked, the driver from
/// stdin, resolves the entry and computes the box on a thread of its own,
/// refuses a build with no snapshot, and confines the process, returning the
/// pre-pass and the confinement.
///
/// # Errors
///
/// Returns the exit code of a failure, after printing it: a usage error to
/// stderr with no envelope, any other as its envelope on stdout.
fn confine_eval(args: EvalArgs) -> Result<(Prepass, Confined), u8> {
    let EvalArgs {
        module,
        driver,
        export,
        jit,
        timeout,
        graph,
    } = args;
    let timeout = match timeout {
        None => gro_runtime::eval::DEFAULT_TIMEOUT,
        Some(0) => {
            eprintln!("error: --timeout takes a number of milliseconds of at least 1");
            return Err(EXIT_USAGE);
        }
        Some(ms) => Duration::from_millis(ms),
    };
    match (driver.as_deref(), &export) {
        (Some(DRIVER_STDIN) | None, None) | (None, Some(_)) => {}
        (Some(DRIVER_STDIN), Some(_)) => {
            eprintln!(
                "error: --export reads an export with no driver; a driver is handed the whole \
                 namespace"
            );
            return Err(EXIT_USAGE);
        }
        (Some(path), _) => {
            eprintln!(
                "error: --driver takes `-`, to read the driver from stdin, not {path:?}: a \
                 driver from a file isn't supported yet"
            );
            return Err(EXIT_USAGE);
        }
    }
    // first, before the process opens anything: a descriptor it inherited
    // would reach past the box, which gates only new opens
    let closed = if prices::off(EvalDefault::CloseFds) {
        Ok(())
    } else {
        close_inherited()
    };
    // the phases file, under the `phases` feature, opens before the box
    phases::open();
    // then the environment goes but `TZ`, so no library eval starts reads a
    // variable; a phases or priced build has read its own by now
    let cleared = if prices::off(EvalDefault::ClearEnv) {
        Ok(())
    } else {
        clear_env()
    };
    let mut options = EvalOptions {
        entry: module,
        cwd: PathBuf::new(),
        export,
        driver: None,
        jit,
        snapshot: STARTUP_SNAPSHOT,
        runtime: RuntimeInfo::new(env!("CARGO_PKG_VERSION"), COMMIT),
        timeout,
        // the out-of-memory backstop writes through its own handle, never
        // through stdout's lock, which another thread may hold then; without
        // one, it exits with no envelope
        backstop: std::io::stdout()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .map(|fd| Arc::new(File::from(fd))),
        graph,
    };
    if let Err(error) = closed {
        let error = EvalError::new(
            ErrorKind::Environment,
            format!("cannot close the descriptors the eval inherited: {error}"),
        );
        return Err(write_envelope(&gro_runtime::eval::failed(&options, error)));
    }
    if let Err(error) = cleared {
        let error = EvalError::new(
            ErrorKind::Environment,
            format!("cannot clear the environment the eval inherited: {error}"),
        );
        return Err(write_envelope(&gro_runtime::eval::failed(&options, error)));
    }
    // read here, before the box, not in the runtime, which takes them as inputs
    let mut read = || -> Result<(), String> {
        options.cwd = std::env::current_dir()
            .map_err(|error| format!("failed to read the working directory: {error}"))?;
        if driver.is_some() {
            let mut source = Vec::new();
            std::io::stdin()
                .read_to_end(&mut source)
                .map_err(|error| format!("failed to read the driver from stdin: {error}"))?;
            options.driver = Some(source);
        }
        Ok(())
    };
    if let Err(message) = read() {
        let error = EvalError::new(ErrorKind::Internal, message);
        return Err(write_envelope(&gro_runtime::eval::failed(&options, error)));
    }
    let unstarted = gro_runtime::eval::failed(
        &options,
        EvalError::new(
            ErrorKind::Internal,
            "could not start the thread that resolves the eval",
        ),
    );
    let prepass = match on_sized_thread("fgro-prepass", || gro_runtime::eval::prepass(options)) {
        Joined::Returned(Ok(prepass)) => {
            phases::mark(Phase::Prepass);
            prepass
        }
        Joined::Returned(Err(envelope)) => return Err(write_envelope(&envelope)),
        Joined::Unstarted(error) => return Err(write_envelope(&with_error(unstarted, &error))),
        Joined::Panicked => return Err(EXIT_PANIC),
    };
    // a confined eval starts only from a snapshot, so a build with none is
    // refused here, the same on every host, before the box is attempted
    if !prepass.has_snapshot() {
        return Err(write_envelope(
            &prepass.failed(gro_runtime::eval::no_snapshot_error()),
        ));
    }
    // the box takes eval's JIT choice from the pre-pass's options, as the
    // engine does, so a jitless eval's box refuses executable mappings
    match prepass.confine() {
        Ok(confined) => {
            phases::mark(Phase::Confine);
            Ok((prepass, confined))
        }
        Err(error) => Err(write_envelope(
            &prepass.failed(gro_runtime::eval::confine_error(&error)),
        )),
    }
}

/// Closes every file descriptor above stderr the process inherited (a file,
/// a socket, a device its parent left open), which the box, gating only new
/// opens, wouldn't reach: `gro_sys::close_inherited_fds`, sound because it
/// runs first on the main thread, the only one, before anything in the
/// process opens a descriptor.
#[cfg(target_os = "linux")]
fn close_inherited() -> std::io::Result<()> {
    gro_sys::close_inherited_fds()
}

/// No box to escape elsewhere: eval refuses before it would start.
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature as Linux's, which can fail"
)]
const fn close_inherited() -> std::io::Result<()> {
    Ok(())
}

/// The variables eval keeps from its environment: `TZ` alone, which ICU
/// reads for the time zone eval then checks is UTC.
#[cfg(target_os = "linux")]
const KEPT_ENV: &[&str] = &["TZ"];

/// Clears the process's environment but [`KEPT_ENV`], so no library eval
/// starts (`deno_core`'s diagnostics and import graph file, ICU's locale,
/// std's backtrace and stack size, any other) reads a variable:
/// `gro_sys::clear_env_except`, sound because it runs on the main thread,
/// the only one, before anything that keeps a pointer into the environment
/// has run.
#[cfg(target_os = "linux")]
fn clear_env() -> std::io::Result<()> {
    gro_sys::clear_env_except(KEPT_ENV)
}

/// No box elsewhere: eval refuses before it would start.
#[cfg(not(target_os = "linux"))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the same signature as Linux's, which can fail"
)]
const fn clear_env() -> std::io::Result<()> {
    Ok(())
}

/// `envelope`, its error's message followed by `error`, why a thread didn't
/// start.
fn with_error(mut envelope: Envelope, error: &std::io::Error) -> Envelope {
    if let Err(failure) = &mut envelope.result {
        failure.message = format!("{} ({error})", failure.message);
    }
    envelope
}

/// How a thread [`on_sized_thread`] ran ended.
enum Joined<T> {
    /// Its work returned this.
    Returned(T),
    /// It couldn't start.
    Unstarted(std::io::Error),
    /// Its work panicked, and the panic has printed.
    Panicked,
}

/// Runs `work` on a thread named `name` with [`STACK_SIZE`] reserved, and
/// joins it.
fn on_sized_thread<T: Send>(name: &str, work: impl FnOnce() -> T + Send) -> Joined<T> {
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, work);
        match spawned {
            Ok(handle) => handle.join().map_or(Joined::Panicked, Joined::Returned),
            Err(error) => Joined::Unstarted(error),
        }
    })
}

/// Writes `envelope` to stdout, one line, returning the exit code its result
/// gives, or [`EXIT_FAILURE`] when it can't be written.
fn write_envelope(envelope: &Envelope) -> u8 {
    // written and flushed by hand: `println!` panics when stdout fails (a
    // closed pipe, a full disk)
    let written = envelope
        .to_json()
        .map_err(|error| error.to_string())
        .and_then(|json| {
            let mut stdout = std::io::stdout().lock();
            writeln!(stdout, "{json}")
                .and_then(|()| stdout.flush())
                .map_err(|error| error.to_string())
        });
    if let Err(error) = written {
        eprintln!("error: failed to write the envelope: {error}");
        return EXIT_FAILURE;
    }
    match &envelope.result {
        Ok(_) => 0,
        Err(error) if error.usage => EXIT_USAGE,
        Err(_) => EXIT_FAILURE,
    }
}

/// Writes `snapshot.bin` and `residual.rs` into `dir`, creating it first so an
/// unwritable directory fails before the snapshot is built.
fn make_snapshot(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    // the engine `fgro run` sets up
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default())?;
    // the xtask's stamp decides when to rerun
    gro_runtime::snapshot::make_snapshot(engine)?.write_to(dir)?;
    Ok(())
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
        assert_eq!(run(&["--version"]), 0);
        assert_eq!(run(&["run", "--help"]), 0);
        assert_eq!(run(&["eval", "--help"]), 0);
    }

    /// Every spelling argh parses as `eval` runs on the main thread, `--`
    /// before the subcommand included, and nothing else does.
    #[test]
    fn eval_alone_runs_on_the_main_thread() {
        for args in [
            &["eval", "a.js"][..],
            &["--", "eval", "a.js"],
            &["eval", "--jit", "a.js"],
        ] {
            assert!(runs_on_main_thread(args), "{args:?}");
        }
        for args in [
            &["run", "a.js"][..],
            &["--version"],
            &["eval"],
            &["--version", "eval", "a.js"],
            &["make-snapshot", "eval"],
            &[],
        ] {
            assert!(!runs_on_main_thread(args), "{args:?}");
        }
    }

    #[test]
    fn commit_accepts_a_full_sha_and_the_dirty_suffix() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(commit(None), None);
        assert_eq!(commit(Some("")), None);
        assert_eq!(commit(Some(sha)), Some(sha));
        let dirty = "0123456789abcdef0123456789abcdef01234567-dirty";
        assert_eq!(commit(Some(dirty)), Some(dirty));
        let sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(commit(Some(sha256)), Some(sha256));
    }

    #[test]
    fn commit_refuses_anything_else() {
        for value in [
            "unknown",
            "0123456",
            "0123456789abcdef0123456789abcdef01234567-",
            "0123456789abcdef0123456789abcdef01234567-clean",
            "0123456789ABCDEF0123456789ABCDEF01234567",
            "0123456789abcdef0123456789abcdef0123456g",
            "-dirty",
        ] {
            let result = std::panic::catch_unwind(|| commit(Some(value)));
            assert!(result.is_err(), "{value}");
        }
    }

    /// Only a `snapshot` build reads `GRO_COMMIT`, so the plain build
    /// `cargo test` runs has no commit, whatever the shell sets.
    #[test]
    fn version_line_names_the_commit() {
        let line = version_line();
        let expected = option_env!("GRO_COMMIT")
            .filter(|value| cfg!(feature = "snapshot") && !value.is_empty())
            .unwrap_or(COMMIT_UNKNOWN);
        assert!(
            line.starts_with(&format!(
                "fgro {} (commit {expected}, V8 ",
                env!("CARGO_PKG_VERSION")
            )),
            "{line}"
        );
    }

    #[test]
    fn usage_error_exits_2() {
        assert_eq!(run(&[]), EXIT_USAGE);
        assert_eq!(run(&["--unknown"]), EXIT_USAGE);
        assert_eq!(run(&["positional"]), EXIT_USAGE);
        assert_eq!(run(&["run"]), EXIT_USAGE);
        assert_eq!(run(&["run", "a.js", "b.js"]), EXIT_USAGE);
        assert_eq!(run(&["--version", "run", "a.js"]), EXIT_USAGE);
        assert_eq!(run(&["make-snapshot"]), EXIT_USAGE);
        assert_eq!(run(&["make-snapshot", "a", "b"]), EXIT_USAGE);
        assert_eq!(run(&["make-snapshot", "--help"]), EXIT_USAGE);
        assert_eq!(run(&["make-snapshot", "-x"]), EXIT_USAGE);
        assert_eq!(run(&["eval"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "b.js"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "--driver", "d.js"]), EXIT_USAGE);
        assert_eq!(
            run(&["eval", "a.js", "--driver", "-", "--export", "x"]),
            EXIT_USAGE
        );
        assert_eq!(run(&["eval", "a.js", "--timeout", "0"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "--timeout", "-1"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "--timeout", "1s"]), EXIT_USAGE);
        assert_eq!(run(&["eval", "a.js", "--unknown"]), EXIT_USAGE);
    }
}
