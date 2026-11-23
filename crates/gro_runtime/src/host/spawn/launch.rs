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
//! - The signal handler is installed at the run's first launch.
//! - A slot of the handler's tables is reserved: its group's guard for a
//!   child that leads one, else a place among the children a `SIGTERM`
//!   passes on to; past `gro_sys`'s caps the launch fails, so no child
//!   runs unguarded, and so does one once a signal the handler guards
//!   against has begun ending the run. A signal that arrives between a
//!   reservation and its arming waits for the arming, so the child is
//!   killed too.
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
//! A launch given a program's box takes the reservation, the spawn, the
//! pidfd, and the arming on a short-lived thread of its own that installs
//! the box on itself first, so the child starts in it ([`spawn_held`]),
//! and the isolate's thread then joins the child to the run's table and
//! watches it.
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
use std::process::{Child, Command, Stdio};
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
use crate::confine::program::{ProgramBox, ProgramBoxError};
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

/// A child started by [`spawn_held`]: std's handle, which never waits for
/// it, its pidfd, opened before anything could reap it, and its slot of
/// the signal handler's tables, armed with its id.
#[derive(Debug)]
pub struct Spawned {
    /// The child, its piped streams on it.
    pub child: Child,
    /// Its pidfd.
    pub pidfd: Arc<Pidfd>,
    /// Its slot, armed.
    pub guard: Guard,
}

/// Why [`spawn_held`] started no child, or started one and killed and
/// reaped it, as the text of the launch's `launch_failed`.
#[derive(Debug, thiserror::Error)]
pub enum Unspawned {
    /// No slot of the signal handler's tables was free, or a signal is
    /// ending the run; `grouped` when the child would lead a group.
    #[error("{}", unreserved_why(*.why, *.grouped))]
    Unreserved {
        /// Why.
        why: Unreserved,
        /// Whether the child would lead a group.
        grouped: bool,
    },
    /// The program's box couldn't be installed on the thread that starts
    /// the child.
    #[error(
        "its box couldn't be installed on the thread that starts it: {}",
        with_causes(.0)
    )]
    Restricted(ProgramBoxError),
    /// The kernel refuses the thread in the program's box a signal to the
    /// run's main thread, which the signal handler on that thread hands a
    /// signal ending the run on to, so none of the run's other children
    /// would be killed (`gro_sys::check_hand_off`).
    #[error(
        "the kernel refuses its box's thread a signal to the run's main thread, which a signal \
         ending the run is handed on to, so a child started there could outlive the run: {0}"
    )]
    HandOffRefused(std::io::Error),
    /// The thread that starts the child in its box couldn't be started.
    #[error("the thread that starts it in its box couldn't be started: {0}")]
    Thread(std::io::Error),
    /// `Command::spawn` failed.
    #[error("{0}")]
    Spawn(std::io::Error),
    /// The child couldn't be held by its pidfd; it was killed and reaped.
    #[error("it couldn't be held by its pidfd: {0}")]
    Pidfd(std::io::Error),
    /// Its slot couldn't be armed; it was killed and reaped.
    #[error("its slot of the signal handler's tables couldn't be armed: {0}")]
    Arm(std::io::Error),
}

/// `error`'s message, then each of its causes', joined by `: `.
fn with_causes(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(": ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

/// Starts `command`, held by its pidfd and guarded, in `program_box` when
/// given one.
///
/// It sets the command to lead a process group of its own when `grouped`,
/// so the slot and the group always agree, reserves the child's slot of
/// the signal handler's tables, spawns it, holds it by its pidfd, and arms
/// the slot with its id. With no `program_box`, on the calling thread.
/// Given one, on a thread of its own, which installs the box on itself
/// first ([`ProgramBox::restrict_thread`]), checks that it may hand a
/// signal on to the main thread (`gro_sys::check_hand_off`), starting
/// nothing where the kernel refuses it, then takes every step above
/// itself, so the child starts in the box and the slot is made, armed, or
/// freed on the thread that reserved it, then ends; the caller's thread
/// and the run's others keep the run's box. A panic on that thread is
/// Gro's own bug, raised again on the caller's.
///
/// The slot's steps stay on the one thread because of how the handler
/// waits: as it begins, it waits for each slot reserved to be armed or
/// freed, unless the thread it runs on holds one, which that thread can't
/// arm meanwhile. Every thread but the main one hands the signal on to the
/// main thread, so the handler runs there: a caller on the main thread
/// that reserved a slot and armed it once the box's thread was joined
/// would have the handler skip the wait, and the child that slot is for
/// would outlive the run, while a slot whose steps all run on the box's
/// thread is always waited for. The hand-off matters for the box's thread
/// besides: Landlock's signal scope lets it signal its own child alone,
/// each earlier child being in a sibling domain of its own, so the handler
/// kills only that child there, and the main thread, in the run's domain, kills
/// every guarded group and passes the `SIGTERM` on to every other child
/// (`gro_sys::guard_children_on_signal`).
///
/// Kind: a guardrail against Gro's own bug, and an availability measure
/// (children outliving the run), not a security boundary. Its adversary
/// is a change to the launch that splits the slot's steps across threads:
/// `gro_sys`'s reservation is neither `Send` nor `Sync`, so it's made and
/// dropped on one thread, but nothing makes the thread that arms it the
/// one that spawned the child, and that split compiles; the injection
/// tests of a launch in a box (`gro_cli`'s `boxed_launch`) are its review
/// signal. It's defeated by a `SIGKILL` of the run, which orphans every
/// child, and, for a child sharing the run's group, by a `SIGINT` from a
/// sender that isn't the terminal, which the handler doesn't pass on. A
/// kernel that refuses the hand-off doesn't defeat it: the check before
/// the reservation refuses the launch there instead.
///
/// # Errors
///
/// Returns [`Unspawned`]: no slot free, or the run ending; the box not
/// installed, its thread not started, or the hand-off refused; the spawn
/// failing; or the pidfd not opened or the slot not armed, the child
/// killed and reaped first.
///
/// # Panics
///
/// Raises again a panic of the box's thread, which is a bug of Gro's; a
/// child it had started may be left running.
pub fn spawn_held(
    command: &mut Command,
    grouped: bool,
    program_box: Option<&ProgramBox>,
) -> Result<Spawned, Unspawned> {
    if grouped {
        command.process_group(0);
    }
    let Some(program_box) = program_box else {
        return spawn_here(command, grouped);
    };
    std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .spawn_scoped(scope, || {
                program_box
                    .restrict_thread()
                    .map_err(Unspawned::Restricted)?;
                gro_sys::check_hand_off().map_err(Unspawned::HandOffRefused)?;
                #[cfg(feature = "testing")]
                testing::observe(LaunchStep::Restricted);
                spawn_here(command, grouped)
            })
            .map_err(Unspawned::Thread)?;
        thread
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// [`spawn_held`]'s steps on the calling thread.
fn spawn_here(command: &mut Command, grouped: bool) -> Result<Spawned, Unspawned> {
    let reserved = if grouped {
        GroupGuard::reserve().map(Reserved::Group)
    } else {
        TermForward::reserve().map(Reserved::Forward)
    };
    let reserved = reserved.map_err(|why| Unspawned::Unreserved { why, grouped })?;
    #[cfg(feature = "testing")]
    testing::observe(LaunchStep::Reserved);
    let mut child = command.spawn().map_err(Unspawned::Spawn)?;
    let pid = child.id();
    #[cfg(feature = "testing")]
    testing::observe(LaunchStep::Spawned(pid));
    let pidfd = match Pidfd::open(pid) {
        Ok(pidfd) => Arc::new(pidfd),
        Err(error) => {
            // unreaped, so its pid is still its own
            let _ = child.kill();
            let _ = child.wait();
            return Err(Unspawned::Pidfd(error));
        }
    };
    let guard = match reserved.arm(pid) {
        Ok(guard) => guard,
        Err(error) => {
            // unreaped, so its pid is still its own
            let _ = child.kill();
            let _ = child.wait();
            return Err(Unspawned::Arm(error));
        }
    };
    #[cfg(feature = "testing")]
    testing::observe(LaunchStep::Armed);
    Ok(Spawned {
        child,
        pidfd,
        guard,
    })
}

/// A step of [`spawn_held`], told to the observer a test sets
/// ([`observe_launch_steps`]) on the thread that took it, so the test can
/// signal a thread between two steps.
#[cfg(feature = "testing")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStep {
    /// The program's box installed on the launch's own thread, and the
    /// hand-off to the main thread checked.
    Restricted,
    /// The child's slot reserved.
    Reserved,
    /// The child spawned, by its pid, and its slot not armed yet.
    Spawned(u32),
    /// The child's slot armed.
    Armed,
}

/// Sets the observer every launch tells its steps to, once per process;
/// `false`, changing nothing, when one is set already.
#[cfg(feature = "testing")]
pub fn observe_launch_steps(observer: fn(LaunchStep)) -> bool {
    testing::OBSERVER.set(observer).is_ok()
}

/// The observer of a launch's steps.
#[cfg(feature = "testing")]
mod testing {
    use std::sync::OnceLock;

    use super::LaunchStep;

    /// The observer, set once.
    pub(super) static OBSERVER: OnceLock<fn(LaunchStep)> = OnceLock::new();

    /// Tells the observer, if one is set, of `step`.
    pub(super) fn observe(step: LaunchStep) {
        if let Some(observer) = OBSERVER.get() {
            observer(step);
        }
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
    let Spawned {
        mut child,
        pidfd,
        guard,
    } = spawn_held(&mut command, grouped, None)
        .map_err(|unspawned| not_started(program, path, unspawned.to_string()))?;
    let pid = child.id();
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

    use super::{Unspawned, signal_name, unreserved_why};
    use crate::confine::ConfineError;
    use crate::confine::program::ProgramBoxError;

    /// A launch that failed says why as its `launch_failed`'s text: the
    /// steps every launch takes as they've always said it, and a launch in
    /// a box, whose thread couldn't install the box, with each cause its
    /// message doesn't hold already, couldn't start, or panicked.
    #[test]
    fn a_launch_that_failed_says_which_step() {
        let os = std::io::Error::from_raw_os_error;
        assert_eq!(
            Unspawned::Unreserved {
                why: Unreserved::Ending,
                grouped: true
            }
            .to_string(),
            "a signal is ending the run"
        );
        assert_eq!(
            Unspawned::Spawn(os(libc::ENOENT)).to_string(),
            "No such file or directory (os error 2)"
        );
        assert_eq!(
            Unspawned::Pidfd(os(libc::EMFILE)).to_string(),
            "it couldn't be held by its pidfd: Too many open files (os error 24)"
        );
        assert_eq!(
            Unspawned::Arm(os(libc::EINVAL)).to_string(),
            "its slot of the signal handler's tables couldn't be armed: Invalid argument (os \
             error 22)"
        );
        assert_eq!(
            Unspawned::Restricted(ProgramBoxError::Duplicate(os(libc::EMFILE))).to_string(),
            "its box couldn't be installed on the thread that starts it: failed to duplicate \
             the program's box's ruleset: Too many open files (os error 24)"
        );
        assert_eq!(
            Unspawned::Restricted(ProgramBoxError::Landlock(ConfineError::LandlockPartial))
                .to_string(),
            "its box couldn't be installed on the thread that starts it: the kernel enforced \
             the box's Landlock rules only in part, so none can be relied on"
        );
        assert_eq!(
            Unspawned::Thread(os(libc::EAGAIN)).to_string(),
            "the thread that starts it in its box couldn't be started: Resource temporarily \
             unavailable (os error 11)"
        );
        assert_eq!(
            Unspawned::HandOffRefused(os(libc::EPERM)).to_string(),
            "the kernel refuses its box's thread a signal to the run's main thread, which a \
             signal ending the run is handed on to, so a child started there could outlive the \
             run: Operation not permitted (os error 1)"
        );
    }

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
