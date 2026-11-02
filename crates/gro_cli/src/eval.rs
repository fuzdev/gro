//! `fgro eval`: the main thread's path to the box, then the evaluation on a
//! thread of its own, and the envelope written to stdout.

use std::fs::File;
use std::io::Read as _;
use std::os::fd::AsFd as _;
use std::path::PathBuf;
use std::sync::Arc;

use gro_runtime::confine::{BoxRoots, Confined};
use gro_runtime::engine::init_confined_engine;
use gro_runtime::eval::{
    Envelope, ErrorKind, EvalError, EvalOptions, Prepass, PrepassError, RuntimeInfo,
};
use gro_runtime::phases::{self, Phase};
use gro_runtime::prices::{self, EvalDefault};

use crate::cli::{EvalInvocation, EvalValue};
use crate::harden::{clear_env, close_inherited, limit_memory};
use crate::print::{stderr_line, stdout_line};
use crate::snapshot::STARTUP_SNAPSHOT;
use crate::thread::{Joined, on_sized_thread};
use crate::version::COMMIT;
use crate::{EXIT_FAILURE, EXIT_PANIC, EXIT_USAGE};

/// Runs `fgro eval` on the main thread: [`confine_eval`], then evaluates on
/// eval's engine on another thread, and prints the envelope to stdout,
/// returning the exit code. The flags were checked as they were parsed.
pub fn eval(args: EvalInvocation) -> u8 {
    let (prepass, confined) = match confine_eval(args) {
        Ok(confined) => confined,
        Err(code) => return code,
    };
    let unstarted = prepass.failed(EvalError::new(
        ErrorKind::Internal,
        "failed to start the thread that evaluates",
    ));
    // every thread the engine starts comes after the box, spawned from here
    let evaluated = on_sized_thread("fgro", move || {
        // the JIT the box was installed for, as the engine must have it
        let options = gro_runtime::eval::engine_options(confined.jit());
        match init_confined_engine(confined, &options) {
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

/// `eval`'s path up to and including the box, on the main thread, returning
/// the pre-pass and the confinement. This is the authoritative order; the
/// process must keep one thread until the box is up, since Landlock and
/// seccomp bind only the calling thread and the box refuses with more than
/// one alive:
///
/// 1. [`close_inherited`]: every descriptor above stderr the process
///    inherited, first, before anything opens one, since the box gates only
///    new opens
/// 2. `phases::open`: a phases build's file, opened before the box
/// 3. [`clear_env`]: the environment but `TZ`, so no library eval starts
///    reads a variable (a phases or priced build has read its own by now)
/// 4. [`limit_memory`]: the default thread stack lowered
///    (`set_default_thread_stack`), then `RLIMIT_DATA` capped, before any
///    thread starts, so every thread and allocation after is under it
/// 5. stdout duplicated for the out-of-memory backstop, the working
///    directory read, and with `--driver -` the driver read from stdin
/// 6. `gro_runtime::eval::prepass` on a sized thread of its own, joined:
///    the entry resolved and the box's roots computed
/// 7. a build with no snapshot refused, the same on every host
/// 8. the process confined (`Prepass::confine`), on this thread, by then
///    the only one
///
/// A failure of steps 1, 3, or 4 is `environment`, the first reported
/// alone. [`eval`] then starts the engine on a second sized thread, so every
/// thread V8 and tokio start comes after the box.
///
/// # Errors
///
/// Returns the exit code of a failure, after printing its envelope on
/// stdout.
fn confine_eval(args: EvalInvocation) -> Result<(Prepass<BoxRoots>, Confined), u8> {
    let EvalInvocation {
        module,
        value,
        jit,
        timeout,
        graph,
    } = args;
    let (stdin_driver, export) = match value {
        EvalValue::StdinDriver => (true, None),
        EvalValue::Export(export) => (false, export),
    };
    // steps 1 to 4, in the order the doc gives
    let closed = unless_off(EvalDefault::CloseFds, close_inherited);
    phases::open();
    let cleared = unless_off(EvalDefault::ClearEnv, clear_env);
    let limited = unless_off(EvalDefault::DataLimit, limit_memory);
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
    // reported in the order the steps ran, the first failure alone
    for (result, step) in [
        (closed, "failed to close the descriptors the eval inherited"),
        (
            cleared,
            "failed to clear the environment the eval inherited",
        ),
        (limited, "failed to cap the eval's memory"),
    ] {
        if let Err(error) = result {
            let error = EvalError::new(ErrorKind::Environment, format!("{step}: {error}"));
            return Err(write_envelope(&Envelope::failed(&options, error)));
        }
    }
    // read here, before the box, not in the runtime, which takes them as inputs
    let mut read = || -> Result<(), String> {
        options.cwd = std::env::current_dir()
            .map_err(|error| format!("failed to read the working directory: {error}"))?;
        if stdin_driver {
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
        return Err(write_envelope(&Envelope::failed(&options, error)));
    }
    let unstarted = Envelope::failed(
        &options,
        EvalError::new(
            ErrorKind::Internal,
            "failed to start the thread that resolves the eval",
        ),
    );
    let prepass = match on_sized_thread("fgro-prepass", || gro_runtime::eval::prepass(options)) {
        Joined::Returned(Ok(prepass)) => {
            phases::mark(Phase::Prepass);
            prepass
        }
        Joined::Returned(Err(PrepassError::Usage(envelope))) => {
            return Err(write_envelope_failed(&envelope, EXIT_USAGE));
        }
        Joined::Returned(Err(PrepassError::Failed(envelope))) => {
            return Err(write_envelope(&envelope));
        }
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
            Ok(confined)
        }
        Err(envelope) => Err(write_envelope(&envelope)),
    }
}

/// Runs `step`, one of eval's hardening steps, unless a priced build turns
/// `default` off (never without the `prices` feature).
fn unless_off(default: EvalDefault, step: fn() -> std::io::Result<()>) -> std::io::Result<()> {
    if prices::off(default) { Ok(()) } else { step() }
}

/// `envelope`, its error's message followed by `error`, why a thread didn't
/// start.
fn with_error(mut envelope: Envelope, error: &std::io::Error) -> Envelope {
    if let Err(failure) = &mut envelope.result {
        failure.message = format!("{} ({error})", failure.message);
    }
    envelope
}

/// Writes `envelope` to stdout, one line, returning the exit code its result
/// gives, or [`EXIT_FAILURE`] when it can't be written.
fn write_envelope(envelope: &Envelope) -> u8 {
    write_envelope_failed(envelope, EXIT_FAILURE)
}

/// Writes `envelope` to stdout, one line, returning 0 for a value,
/// `failure_code` for a failure, or [`EXIT_FAILURE`] when it can't be
/// written.
fn write_envelope_failed(envelope: &Envelope, failure_code: u8) -> u8 {
    let written = envelope
        .to_json()
        .map_err(|error| error.to_string())
        .and_then(|json| stdout_line(json).map_err(|error| error.to_string()));
    if let Err(error) = written {
        stderr_line(format_args!("error: failed to write the envelope: {error}"));
        return EXIT_FAILURE;
    }
    if envelope.result.is_ok() {
        0
    } else {
        failure_code
    }
}
