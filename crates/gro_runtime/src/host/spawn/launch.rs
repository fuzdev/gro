//! A program's `run`, `out`, and `out_bytes`: the call's checks, the
//! launch, and the result its watch ([`watch`](super::watch)) settles it
//! with.
//!
//! A call is checked as the Node module checks it, in the same order and
//! with the same messages: the receiver, the arguments (an array, each
//! element its own data and text), the options (an object, each field its
//! own data: `cwd`, an fs directory handle, `timeout_ms`, and, for a
//! captured call, `max_bytes` and `stdin`), each a `TypeError` for what
//! isn't one. `signal` isn't built: given, it's a plain `Error`.
//!
//! Then the launch, on the isolate's thread, before the call returns its
//! promise, so a launch that fails rejects it, `launch_failed`:
//!
//! - The pin is checked ([`Pin::check_launch`](super::pins::Pin::check_launch)):
//!   a native binary's descriptor still the file pinned, its change time
//!   the one pinned, and a script's path still leading to its pin, with
//!   its change time.
//! - A slot of the signal handler's tables is reserved: its group's guard
//!   for a child that leads one, else a place among the children a
//!   `SIGTERM` passes on to; past `gro_sys`'s caps the launch fails, so no
//!   child runs unguarded, and so does one once a signal the handler
//!   guards against has begun ending the run. The handler is installed at
//!   the run's first launch, and one that arrives between a reservation
//!   and its arming waits for the arming, so the child is killed too.
//! - The child is started by std's `Command`, on `posix_spawn`, with no
//!   code run between its fork and its exec: a native binary by its pin's
//!   descriptor (`/proc/self/fd/N`), so the file pinned is the file run
//!   whatever its path leads to now, and a script by the path it was found
//!   at; its `argv[0]` the pin's; its working directory a descriptor too,
//!   `cwd`'s directory opened beneath its root, or the project's, opened
//!   as the run started; its environment exactly the program's, nothing
//!   inherited; and, for a captured call whose stdin isn't `'inherit'`, a
//!   process group of its own, as the Node module's child with no stream
//!   inherited leads one. The Node module's child leads a session too;
//!   this one stays in the terminal's, so a captured child that reads
//!   `/dev/tty` with `stdin: 'ignore'` stops (`SIGTTIN`) rather than
//!   failing the read, and an `out` without `timeout_ms` waits for it.
//! - Right after, before anything can reap it, the child is held by its
//!   pidfd, its slot armed, and it joins the run's table of children; a
//!   step that fails from there kills and reaps it before the launch
//!   fails.
//!
//! The result is a frozen null-prototype record, as the Node module's:
//! `kind` and `code` for `exited`, `signal` and `signal_number` for
//! `signaled`, or `code`, `signal`, and `signal_number` for `timed_out`,
//! with the code `null` or the signal and its number; and for a captured
//! call `stdout`, `stderr`, and `truncated`, `out`'s decoded as UTF-8 whole
//! with `U+FFFD` for what isn't, a byte order mark kept, and `out_bytes`'
//! each a `Uint8Array` of its own buffer, from the isolate's allocator. A
//! signal is named as Node names it, a real-time one as `SIGRTMIN+n`, and
//! one of the two the C library keeps below `SIGRTMIN` as `SIG32` or
//! `SIG33`, which the Node module reports as no signal.

use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use deno_core::{OpState, v8};
use gro_sys::child::{Ended, Pidfd, PidfdWatch};
use gro_sys::{
    FORWARDED_CHILDREN, ForwardReservation, GUARDED_GROUPS, GroupGuard, GroupReservation,
    TermForward, Unreserved,
};
use gro_v8::PendingArrayBuffer;
use tokio::net::unix::pipe;

use super::pins::{Pin, PinKind};
use super::watch::{Capture, Guard, Held, Outcome, Started, Stop, Watch};
use super::{
    Failure, MODULE, NOT_AN_FS_HANDLE, SpawnErrorKind, SpawnRun, array, checked_text, element,
    own_option, read_handle, run_state, throw,
};
use crate::BINARY_NAME;
use crate::host::error_class::DeclaredFamily;
use crate::host::fs::{HandleDirFault, open_handle_dir};
use crate::host::handle::{FS_READ_DIR, SPAWN_PROGRAM};
use crate::host::later::{self, Deferred};
use crate::host::{name, own_data, string, throw_range_error};

/// The most bytes a captured call keeps of each stream by default, the
/// Node module's `DEFAULT_MAX_BYTES`.
const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;

/// The longest `timeout_ms`, the Node module's `MAX_TIMEOUT_MS`.
const MAX_TIMEOUT_MS: f64 = 2_147_483_647.0;

/// The largest safe integer, `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// The most bytes `out_bytes` hands back in one buffer, the bound the fs
/// reads keep too, past which V8's allocator would end the process.
const MAX_BUFFER: usize = 2_147_483_647;

/// What a capture that couldn't grow throws, V8's message for an
/// `ArrayBuffer` it can't allocate.
const UNALLOCATED: &str = "Array buffer allocation failed";

/// What a call's result throws for text past V8's longest string.
const TOO_LONG: &str = "Invalid string length";

/// The child's stdin for a captured call.
#[derive(Debug)]
enum StdinOption {
    /// The host's.
    Inherit,
    /// `/dev/null`.
    Ignore,
    /// A pipe this data is written to, then closed.
    Data(Vec<u8>),
}

/// What a started child's stream is: `stdio`'s value for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// The host's.
    Inherit,
    /// A pipe to the `SpawnChild`.
    Pipe,
    /// `/dev/null`.
    Ignore,
}

/// Which options a call reads beside `cwd`, `timeout_ms`, and `signal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    /// None: `run`'s.
    Run,
    /// `max_bytes` and `stdin`: `out`'s and `out_bytes`'.
    Captured,
    /// `stdio`: `start`'s.
    Started,
}

/// A call's options, each read once.
#[derive(Debug)]
pub struct CallOptions {
    /// `cwd`'s directory, opened, or its path and why it couldn't be, which
    /// fails the launch; `None` for the project's.
    pub cwd: Option<Result<OwnedFd, (String, String)>>,
    /// `timeout_ms`.
    pub timeout: Option<Duration>,
    /// `max_bytes`.
    max_bytes: usize,
    /// `stdin`.
    stdin: StdinOption,
    /// `stdio`, each stream's.
    pub stdio: [Stream; 3],
}

/// A program handle, as a call reads the one it's called on.
#[derive(Debug)]
pub struct Program {
    /// The name it was asked for.
    name: String,
    /// Its pin's place in the run's table.
    pin: usize,
    /// Its children's environment.
    env: Vec<(String, String)>,
}

/// `program.run(args, options?)`, `out`, or `out_bytes`, as `capture`
/// says, called on `receiver`: checked, launched, and its promise deferred
/// to the child's watch, or `None` with the exception pending; `errors`,
/// `SpawnError`'s family as the member declares it.
pub fn call<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    capture: Capture,
    receiver: v8::Local<'s, v8::Value>,
    args: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<Deferred<'s>> {
    let started = (|| {
        let run = run_state(state)?;
        let program = read_program(scope, receiver)?;
        let argv = read_argv(scope, args)?;
        let reads = if capture == Capture::None {
            Reads::Run
        } else {
            Reads::Captured
        };
        let options = read_call_options(state, scope, reads, options)?;
        launch(run, &program, &argv, options, capture)
    })();
    match started {
        Ok(started) => later::defer_task(state, scope, finish, move |settler| {
            Watch::new(started, settler)
        }),
        Err(failure) => {
            throw(state, scope, errors, failure);
            None
        }
    }
}

/// The program `receiver` is, its name, pin, and children's environment
/// read from its own data and its state, the host's, or a `TypeError`.
pub(super) fn read_program<'s>(
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Result<Program, Failure> {
    let (object, held) = read_handle(scope, &SPAWN_PROGRAM, receiver)?;
    let field = |of: v8::Local<'s, v8::Object>, key: &str| {
        let key = name(scope, key).ok_or(Failure::Thrown)?;
        own_data(scope, of, key)
            .map_err(|_| Failure::Thrown)?
            .ok_or(Failure::Thrown)
    };
    let program_name = field(object, "name")?.to_rust_string_lossy(scope);
    let pin = v8::Local::<v8::Integer>::try_from(field(held, super::PIN)?)
        .ok()
        .and_then(|pin| usize::try_from(pin.value()).ok())
        .ok_or(Failure::Thrown)?;
    let record =
        v8::Local::<v8::Object>::try_from(field(held, super::ENV)?).map_err(|_| Failure::Thrown)?;
    let keys = record
        .get_own_property_names(scope, v8::GetPropertyNamesArgs::default())
        .ok_or(Failure::Thrown)?;
    let mut env = Vec::new();
    for index in 0..keys.length() {
        let key = keys.get_index(scope, index).ok_or(Failure::Thrown)?;
        let value = record.get(scope, key).ok_or(Failure::Thrown)?;
        env.push((
            key.to_rust_string_lossy(scope),
            value.to_rust_string_lossy(scope),
        ));
    }
    Ok(Program {
        name: program_name,
        pin,
        env,
    })
}

/// `args`, a call's arguments, as the Node module's `to_argv` reads them:
/// an array, no proxy, each element its own data and text.
pub(super) fn read_argv(
    scope: &v8::PinScope<'_, '_>,
    args: v8::Local<'_, v8::Value>,
) -> Result<Vec<String>, Failure> {
    let list = array(args)
        .ok_or_else(|| Failure::Type("spawn args are an array of strings".to_owned()))?;
    (0..list.length())
        .map(|index| {
            let element = element(scope, list, index)?;
            checked_text(scope, element, "a spawn argument")
        })
        .collect()
}

/// `value` as a number that's a safe integer, `None` for anything else.
fn safe_integer(value: v8::Local<'_, v8::Value>) -> Option<f64> {
    let number = v8::Local::<v8::Number>::try_from(value).ok()?.value();
    // an integer is its own truncation, exactly
    #[expect(clippy::float_cmp, reason = "an integer equals its truncation exactly")]
    let integer = number.trunc() == number;
    (number.is_finite() && integer && number.abs() <= MAX_SAFE_INTEGER).then_some(number)
}

/// A call's `options`, read and checked once, in the Node module's order:
/// `cwd`, `timeout_ms`, `signal`, then, as `reads` says, for a captured
/// call `max_bytes` and `stdin`, and for `start` `stdio`.
pub(super) fn read_call_options(
    state: &OpState,
    scope: &v8::PinScope<'_, '_>,
    reads: Reads,
    options: v8::Local<'_, v8::Value>,
) -> Result<CallOptions, Failure> {
    let mut read = CallOptions {
        cwd: None,
        timeout: None,
        max_bytes: DEFAULT_MAX_BYTES,
        stdin: StdinOption::Ignore,
        stdio: [Stream::Inherit; 3],
    };
    if options.is_undefined() {
        return Ok(read);
    }
    let Some(object) = v8::Local::<v8::Object>::try_from(options)
        .ok()
        .filter(|_| !options.is_function() && !options.is_array())
    else {
        return Err(Failure::Type("spawn call options are an object".to_owned()));
    };
    let given = |key: &str| {
        own_option(scope, object, key).map(|value| value.filter(|value| !value.is_undefined()))
    };
    if let Some(cwd) = given("cwd")? {
        read.cwd = Some(match open_handle_dir(state, scope, &FS_READ_DIR, cwd) {
            Ok((fd, _)) => Ok(fd),
            Err(HandleDirFault::NotAHandle) => {
                return Err(Failure::Type(NOT_AN_FS_HANDLE.to_owned()));
            }
            Err(HandleDirFault::Unopened(path, why)) => Err((path, why)),
        });
    }
    if let Some(timeout) = given("timeout_ms")? {
        let millis = safe_integer(timeout)
            .filter(|millis| *millis > 0.0 && *millis <= MAX_TIMEOUT_MS)
            .ok_or_else(|| {
                Failure::Type(
                    "`timeout_ms` is a positive safe integer of at most 2147483647".to_owned(),
                )
            })?;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a positive integer of at most 2147483647, exact as an f64"
        )]
        let millis = millis as u64;
        read.timeout = Some(Duration::from_millis(millis));
    }
    if given("signal")?.is_some() {
        return Err(Failure::Plain(format!(
            "the `signal` option of {MODULE} isn't built on {BINARY_NAME} yet"
        )));
    }
    match reads {
        Reads::Run => return Ok(read),
        Reads::Started => {
            if let Some(stdio) = given("stdio")? {
                read.stdio = read_stdio(scope, stdio)?;
            }
            return Ok(read);
        }
        Reads::Captured => {}
    }
    if let Some(max_bytes) = given("max_bytes")? {
        let max_bytes = safe_integer(max_bytes)
            .filter(|max_bytes| *max_bytes >= 0.0)
            .ok_or_else(|| {
                Failure::Type("`max_bytes` is a non-negative safe integer".to_owned())
            })?;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative safe integer, exact as an f64, saturating past usize"
        )]
        let max_bytes = max_bytes as usize;
        read.max_bytes = max_bytes;
    }
    if let Some(stdin) = given("stdin")? {
        let mode = stdin.is_string().then(|| stdin.to_rust_string_lossy(scope));
        read.stdin = match mode.as_deref() {
            Some("inherit") => StdinOption::Inherit,
            Some("ignore") => StdinOption::Ignore,
            _ => {
                let view = v8::Local::<v8::Uint8Array>::try_from(stdin).map_err(|_| {
                    Failure::Type("`stdin` is 'inherit', 'ignore', or a `Uint8Array`".to_owned())
                })?;
                // a copy, as the Node module's, so a later change to the
                // array changes nothing sent
                let mut data = vec![0; view.byte_length()];
                let copied = view.copy_contents(&mut data);
                data.truncate(copied);
                StdinOption::Data(data)
            }
        };
    }
    Ok(read)
}

/// A `stdio` option, read once: one value for all three streams, or the
/// three in order, an array of exactly three, each its own data.
fn read_stdio(
    scope: &v8::PinScope<'_, '_>,
    raw: v8::Local<'_, v8::Value>,
) -> Result<[Stream; 3], Failure> {
    let one = |value: v8::Local<'_, v8::Value>| {
        let mode = value.is_string().then(|| value.to_rust_string_lossy(scope));
        match mode.as_deref() {
            Some("inherit") => Ok(Stream::Inherit),
            Some("pipe") => Ok(Stream::Pipe),
            Some("ignore") => Ok(Stream::Ignore),
            _ => Err(Failure::Type(
                "a `stdio` value is 'inherit', 'pipe', or 'ignore'".to_owned(),
            )),
        }
    };
    if raw.is_string() {
        return one(raw).map(|stream| [stream; 3]);
    }
    let three = array(raw)
        .filter(|list| list.length() == 3)
        .ok_or_else(|| {
            Failure::Type("`stdio` is 'inherit', 'pipe', 'ignore', or three of them".to_owned())
        })?;
    Ok([
        one(element(scope, three, 0)?)?,
        one(element(scope, three, 1)?)?,
        one(element(scope, three, 2)?)?,
    ])
}

/// The slot of the signal handler's tables reserved for a child before it
/// starts, armed with its id once it has: its group's guard when it leads
/// one, else its place among the children a `SIGTERM` passes on to.
#[derive(Debug)]
enum Reserved {
    /// A group's guard.
    Group(GroupReservation),
    /// A place among the children a `SIGTERM` passes on to.
    Forward(ForwardReservation),
}

impl Reserved {
    /// The slot armed with `pid`, the child's, and the group's id when it
    /// leads one.
    fn arm(self, pid: u32) -> std::io::Result<Guard> {
        match self {
            Self::Group(reserved) => reserved.arm(pid).map(Guard::Group),
            Self::Forward(reserved) => reserved.arm(pid).map(Guard::Forward),
        }
    }
}

/// Why no slot of the signal handler's tables was reserved for a child,
/// `grouped` when it would lead a group: the run ending on a signal, or
/// the table's cap.
fn unreserved_why(unreserved: Unreserved, grouped: bool) -> String {
    match (unreserved, grouped) {
        (Unreserved::Ending, _) => "a signal is ending the run".to_owned(),
        (Unreserved::Full, true) => {
            format!("the run guards {GUARDED_GROUPS} process groups, the most it holds at once")
        }
        (Unreserved::Full, false) => format!(
            "the run holds {FORWARDED_CHILDREN} children in its own group, the most it holds at \
             once"
        ),
    }
}

/// `launch_failed` for `program`, found at `path`, which didn't start, as
/// `why` says.
fn not_started(program: &Program, path: &str, why: String) -> Failure {
    Failure::Spawn {
        kind: SpawnErrorKind::LaunchFailed,
        program: program.name.clone(),
        cause: format!("{} at {path} didn't start", program.name),
        inner: Some(why),
    }
}

/// A child launched: held, its pidfd watched, and its piped streams.
#[derive(Debug)]
pub struct Launched {
    /// The child's process id.
    pub pid: u32,
    /// The child.
    pub held: Held,
    /// Its pidfd, registered with the reactor.
    pub exit: PidfdWatch,
    /// Its stdin, when it's piped.
    pub stdin: Option<pipe::Sender>,
    /// Its stdout, when it's piped.
    pub stdout: Option<pipe::Receiver>,
    /// Its stderr, when it's piped.
    pub stderr: Option<pipe::Receiver>,
}

/// Launches `program` with `argv` and `options` for a call that makes
/// `capture` of its output, returning what its watch takes.
///
/// # Errors
///
/// Returns [`launch_child`]'s errors.
fn launch(
    run: &SpawnRun,
    program: &Program,
    argv: &[String],
    options: CallOptions,
    capture: Capture,
) -> Result<Started, Failure> {
    let stdio = match capture {
        Capture::None => [Stream::Inherit; 3],
        Capture::Text | Capture::Bytes => [
            match options.stdin {
                StdinOption::Data(_) => Stream::Pipe,
                StdinOption::Inherit => Stream::Inherit,
                StdinOption::Ignore => Stream::Ignore,
            },
            Stream::Pipe,
            Stream::Pipe,
        ],
    };
    let launched = launch_child(run, program, argv, options.cwd.as_ref(), stdio)?;
    let feed_data = match options.stdin {
        StdinOption::Data(data) => Some(data),
        StdinOption::Inherit | StdinOption::Ignore => None,
    };
    Ok(Started {
        held: launched.held,
        exit: launched.exit,
        capture,
        max_bytes: options.max_bytes,
        pipes: launched.stdout.zip(launched.stderr),
        feed: launched.stdin.zip(feed_data),
        timeout: options.timeout,
    })
}

/// Launches `program` with `argv`, in `cwd`'s directory or the project's,
/// each stream as `stdio` says, leading a process group of its own when it
/// inherits none, as the Node module's child does.
///
/// # Errors
///
/// Returns `launch_failed` when the pin no longer checks out, no slot of
/// the signal handler's tables is free or a signal is ending the run, the
/// working directory can't be opened, or the child can't be started or
/// held.
pub(super) fn launch_child(
    run: &SpawnRun,
    program: &Program,
    argv: &[String],
    cwd: Option<&Result<OwnedFd, (String, String)>>,
    stdio: [Stream; 3],
) -> Result<Launched, Failure> {
    let pins = run.pins.borrow();
    let pin = pins
        .get(program.pin)
        .ok_or_else(|| Failure::Plain("a program handle names no pin of the run".to_owned()))?;
    pin.check_launch(&program.name)
        .map_err(|unlaunched| Failure::Spawn {
            kind: SpawnErrorKind::LaunchFailed,
            program: program.name.clone(),
            cause: unlaunched.cause,
            inner: unlaunched.inner,
        })?;
    let path = pin.run_path.as_str();
    let grouped = !stdio.contains(&Stream::Inherit);
    let reserved = if grouped {
        GroupGuard::reserve().map(Reserved::Group)
    } else {
        TermForward::reserve().map(Reserved::Forward)
    };
    let reserved = reserved
        .map_err(|unreserved| not_started(program, path, unreserved_why(unreserved, grouped)))?;
    run.guard_on_signal()
        .map_err(|error| not_started(program, path, error))?;
    let cwd = match cwd {
        Some(Ok(fd)) => fd.as_raw_fd(),
        Some(Err((dir, why))) => {
            return Err(not_started(
                program,
                path,
                format!("its working directory {dir} can't be opened: {why}"),
            ));
        }
        None => run
            .project_dir()
            .map_err(|why| not_started(program, path, why))?,
    };
    // a `node_modules/.bin` script runs as its `node`'s argument, by its
    // real path, after the flags its line gives
    let (runs, script): (&Pin, _) = match &pin.kind {
        PinKind::NodeScript { node, flags } => (node, Some((flags, &pin.run_path))),
        PinKind::Native(_) | PinKind::Script => (pin, None),
    };
    let mut command = Command::new(match &runs.kind {
        PinKind::Native(held) => format!("/proc/self/fd/{}", held.as_raw_fd()),
        PinKind::Script | PinKind::NodeScript { .. } => runs.run_path.clone(),
    });
    if let Some((flags, script)) = script {
        command.args(flags).arg(script);
    }
    let to_stdio = |stream: Stream| match stream {
        Stream::Inherit => Stdio::inherit(),
        Stream::Pipe => Stdio::piped(),
        Stream::Ignore => Stdio::null(),
    };
    command
        .arg0(&runs.argv0)
        .args(argv)
        .env_clear()
        .envs(program.env.iter().map(|(key, value)| (key, value)))
        .current_dir(format!("/proc/self/fd/{cwd}"))
        .stdin(to_stdio(stdio[0]))
        .stdout(to_stdio(stdio[1]))
        .stderr(to_stdio(stdio[2]));
    if grouped {
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| not_started(program, path, error.to_string()))?;
    let pid = child.id();
    let pidfd = match Pidfd::open(pid) {
        Ok(pidfd) => Arc::new(pidfd),
        Err(error) => {
            // unreaped, so its pid is still its own
            let _ = child.kill();
            let _ = child.wait();
            return Err(not_started(
                program,
                path,
                format!("it couldn't be held by its pidfd: {error}"),
            ));
        }
    };
    let guard = match reserved.arm(pid) {
        Ok(guard) => guard,
        Err(error) => {
            // unreaped, so its pid is still its own
            let _ = child.kill();
            let _ = child.wait();
            return Err(not_started(
                program,
                path,
                format!("its slot of the signal handler's tables couldn't be armed: {error}"),
            ));
        }
    };
    let group = grouped.then_some(pid);
    let held = Held::track(Arc::clone(&pidfd), group, guard, &run.children);
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // std's handle never waits for it: its pidfd reaps it
    drop(child);
    let watched = (|| {
        let exit = pidfd.watch()?;
        let stdin = stdin
            .map(|stdin| pipe::Sender::from_owned_fd(OwnedFd::from(stdin)))
            .transpose()?;
        let stdout = stdout
            .map(|stdout| pipe::Receiver::from_owned_fd(OwnedFd::from(stdout)))
            .transpose()?;
        let stderr = stderr
            .map(|stderr| pipe::Receiver::from_owned_fd(OwnedFd::from(stderr)))
            .transpose()?;
        Ok::<_, std::io::Error>((exit, stdin, stdout, stderr))
    })();
    let (exit, stdin, stdout, stderr) = match watched {
        Ok(watched) => watched,
        Err(error) => {
            held.abandon();
            return Err(not_started(
                program,
                path,
                format!("it couldn't be watched: {error}"),
            ));
        }
    };
    Ok(Launched {
        pid,
        held,
        exit,
        stdin,
        stdout,
        stderr,
    })
}

/// The name of the signal numbered `signal`, as Node names it, a real-time
/// one as `SIGRTMIN+n`, and one of neither as `SIG` and its number.
pub fn signal_name(signal: libc::c_int) -> String {
    const NAMES: [&str; 31] = [
        "SIGHUP",
        "SIGINT",
        "SIGQUIT",
        "SIGILL",
        "SIGTRAP",
        "SIGABRT",
        "SIGBUS",
        "SIGFPE",
        "SIGKILL",
        "SIGUSR1",
        "SIGSEGV",
        "SIGUSR2",
        "SIGPIPE",
        "SIGALRM",
        "SIGTERM",
        "SIGSTKFLT",
        "SIGCHLD",
        "SIGCONT",
        "SIGSTOP",
        "SIGTSTP",
        "SIGTTIN",
        "SIGTTOU",
        "SIGURG",
        "SIGXCPU",
        "SIGXFSZ",
        "SIGVTALRM",
        "SIGPROF",
        "SIGWINCH",
        "SIGIO",
        "SIGPWR",
        "SIGSYS",
    ];
    let named = usize::try_from(signal)
        .ok()
        .and_then(|signal| signal.checked_sub(1))
        .and_then(|at| NAMES.get(at));
    if let Some(name) = named {
        return (*name).to_owned();
    }
    if (libc::SIGRTMIN()..=libc::SIGRTMAX()).contains(&signal) {
        return format!("SIGRTMIN+{}", signal - libc::SIGRTMIN());
    }
    format!("SIG{signal}")
}

/// The record of `outcome`, a call's result: `None` with an exception
/// pending when it can't be made, a `RangeError` for a capture that
/// couldn't grow or doesn't fit, a plain `Error` for a child whose end is
/// unknown.
pub(super) fn finish<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    outcome: Outcome,
) -> Option<v8::Local<'s, v8::Value>> {
    let Outcome {
        capture,
        ended,
        stop,
        stdout,
        stderr,
        truncated,
        unallocated,
    } = outcome;
    let ended = match ended {
        Ok(ended) => ended,
        Err(why) => {
            crate::host::throw_error(scope, &format!("the host lost track of a child: {why}"));
            return None;
        }
    };
    if unallocated {
        throw_range_error(scope, UNALLOCATED);
        return None;
    }
    let (code, signal) = match ended {
        Ended::Exited(code) => (Some(code), None),
        Ended::Killed { signal, .. } => (None, Some(signal)),
    };
    let mut entries: Vec<(&str, v8::Local<'s, v8::Value>)> = Vec::new();
    let code = code.map_or_else(
        || v8::null(scope).into(),
        |code| v8::Integer::new(scope, code).into(),
    );
    let (signal, signal_number): (v8::Local<v8::Value>, v8::Local<v8::Value>) = match signal {
        Some(signal) => (
            string(scope, &signal_name(signal))?.into(),
            v8::Integer::new(scope, signal).into(),
        ),
        None => (v8::null(scope).into(), v8::null(scope).into()),
    };
    match stop {
        Some(Stop::TimedOut) => {
            entries.push(("kind", string(scope, "timed_out")?.into()));
            entries.push(("code", code));
            entries.push(("signal", signal));
            entries.push(("signal_number", signal_number));
        }
        None if signal.is_null() => {
            entries.push(("kind", string(scope, "exited")?.into()));
            entries.push(("code", code));
        }
        None => {
            entries.push(("kind", string(scope, "signaled")?.into()));
            entries.push(("signal", signal));
            entries.push(("signal_number", signal_number));
        }
    }
    match capture {
        Capture::None => {}
        Capture::Text => {
            for (key, bytes) in [("stdout", &stdout), ("stderr", &stderr)] {
                let text = String::from_utf8_lossy(bytes);
                let Some(text) =
                    v8::String::new_from_utf8(scope, text.as_bytes(), v8::NewStringType::Normal)
                else {
                    throw_range_error(scope, TOO_LONG);
                    return None;
                };
                entries.push((key, text.into()));
            }
        }
        Capture::Bytes => {
            for (key, bytes) in [("stdout", &stdout), ("stderr", &stderr)] {
                if bytes.len() > MAX_BUFFER {
                    throw_range_error(scope, UNALLOCATED);
                    return None;
                }
                let mut buffer = PendingArrayBuffer::new(scope, bytes.len());
                buffer.bytes_mut().copy_from_slice(bytes);
                let length = buffer.len();
                let buffer = buffer.into_array_buffer(scope);
                entries.push((key, v8::Uint8Array::new(scope, buffer, 0, length)?.into()));
            }
        }
    }
    if capture != Capture::None {
        entries.push(("truncated", v8::Boolean::new(scope, truncated).into()));
    }
    super::frozen_record(scope, &entries).map(Into::into)
}

#[cfg(test)]
mod tests {
    use gro_sys::{FORWARDED_CHILDREN, GUARDED_GROUPS, Unreserved};

    use super::{signal_name, unreserved_why};

    /// A launch refused once a signal is ending the run says so, whichever
    /// table it would take a slot of, and one refused at a table's cap
    /// names the cap.
    #[test]
    fn a_launch_refused_as_the_run_ends_says_so_and_one_at_a_cap_names_it() {
        for grouped in [true, false] {
            assert_eq!(
                unreserved_why(Unreserved::Ending, grouped),
                "a signal is ending the run"
            );
        }
        assert_eq!(
            unreserved_why(Unreserved::Full, true),
            format!("the run guards {GUARDED_GROUPS} process groups, the most it holds at once")
        );
        assert_eq!(
            unreserved_why(Unreserved::Full, false),
            format!(
                "the run holds {FORWARDED_CHILDREN} children in its own group, the most it holds \
                 at once"
            )
        );
    }

    /// A signal is named as Node names it, a real-time one past Node's
    /// names by its place after `SIGRTMIN`, and one of neither by its
    /// number.
    #[test]
    fn a_signal_is_named_as_node_names_it_and_a_real_time_one_by_sigrtmin() {
        assert_eq!(signal_name(libc::SIGTERM), "SIGTERM");
        assert_eq!(signal_name(libc::SIGKILL), "SIGKILL");
        assert_eq!(signal_name(libc::SIGABRT), "SIGABRT");
        assert_eq!(signal_name(libc::SIGIO), "SIGIO");
        assert_eq!(signal_name(libc::SIGSYS), "SIGSYS");
        assert_eq!(signal_name(libc::SIGRTMIN()), "SIGRTMIN+0");
        assert_eq!(signal_name(libc::SIGRTMIN() + 1), "SIGRTMIN+1");
        assert_eq!(
            signal_name(libc::SIGRTMAX()),
            format!("SIGRTMIN+{}", libc::SIGRTMAX() - libc::SIGRTMIN())
        );
        assert_eq!(signal_name(32), "SIG32");
        assert_eq!(signal_name(0), "SIG0");
    }
}
