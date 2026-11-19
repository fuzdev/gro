//! `start` and what it hands back: a `SpawnChild`, its `stdin`, and its
//! output streams and their iterators, each a handle (`handle.rs`) whose
//! methods reach the started child's state ([`Child`]) through the run's
//! table of started children ([`StartedChildren`]).
//!
//! `start` is checked and launched as `run` is (`launch.rs`), each stream
//! as `stdio` says, a child inheriting none leading a group of its own;
//! then its handles are made, its task spawned on the event loop's own
//! runtime holding its `closed` ([`ChildTask`]), and its promise settled
//! in a later turn with the `SpawnChild`, as the Node module's settles
//! once the child is running. A launch that fails rejects it.
//!
//! A child's handles share one token, a frozen record of the host's that
//! names the child's number in the table, which every handle's state holds
//! and no JS can reach: the child's and its stdin's state is the token,
//! and an output stream's and its iterator's a frozen record of the token
//! and the stream. The table holds each child beside a weak reference to
//! its token, whose finalizer takes the child out once no handle of it can
//! be reached, so a stream read on after its `SpawnChild` is dropped still
//! finds it, and nothing outlives them all but the task, which holds the
//! child until it's done with it. No borrow of the table is held across a
//! call into V8.
//!
//! An output stream is an async iterable once (`[Symbol.asyncIterator]`,
//! a second call a `TypeError`), its iterator's `next()` a `later` member
//! the child's task settles with a chunk, a `Uint8Array` of its own
//! buffer, or done; `return()` drops the rest, as the Node module's early
//! exit from a `for await` does, and `throw(error)` drops the rest and
//! rejects with `error`, as the Node module's does.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use deno_core::{JsRuntime, OpState, v8};
use gro_sys::child::Ended;
use gro_v8::PendingArrayBuffer;

use super::child::{Child, ChildTask, Chunk, Closed, StartedChild};
use super::launch::{self, Reads, Stream};
use super::watch::{Capture, Outcome};
use super::{Failure, HostData, frozen_record, read_handle, run_state, throw};
use crate::host::error_class::DeclaredFamily;
use crate::host::handle::{
    self, HandleKind, SPAWN_CHILD, SPAWN_OUTPUT, SPAWN_OUTPUT_ITERATOR, SPAWN_STDIN,
};
use crate::host::later::{self, Deferred};
use crate::host::{name, native, throw_error, throw_type_error};

/// What a stream's second iteration throws, the Node module's message.
const ONE_READER: &str = "a child output stream has one reader, once";

/// What `kill` throws for a signal it doesn't send, the Node module's
/// message.
const NOT_A_SIGNAL: &str = "a signal is one of the `SpawnSignal` names";

/// What a member throws when the run's table holds no child its handle
/// names, which no handle the host made meets.
const NO_CHILD: &str = "this spawn handle names no child the run holds";

/// What stdin's `write` throws for data that's neither text nor bytes.
const NOT_DATA: &str = "stdin data is a string or a `Uint8Array`";

/// What stdin's `write` throws for text that isn't well-formed.
const NOT_WELL_FORMED: &str = "stdin data isn't well-formed Unicode";

/// The signals `kill` sends, by the names `SpawnSignal` allows.
const SIGNALS: [(&str, libc::c_int); 7] = [
    ("SIGTERM", libc::SIGTERM),
    ("SIGKILL", libc::SIGKILL),
    ("SIGINT", libc::SIGINT),
    ("SIGHUP", libc::SIGHUP),
    ("SIGQUIT", libc::SIGQUIT),
    ("SIGUSR1", libc::SIGUSR1),
    ("SIGUSR2", libc::SIGUSR2),
];

/// The keys of a stream's state.
const TOKEN: &str = "token";
const STREAM: &str = "stream";

/// The key of a token's number.
const ID: &str = "id";

/// The run's started children, each by the number its handles' token
/// holds, touched only on the isolate's thread.
#[derive(Default)]
pub struct StartedChildren {
    /// Each child, beside the weak reference to its token whose finalizer
    /// removes it, and, until its `start` settles, its `SpawnChild`.
    table: RefCell<HashMap<u64, Entry>>,
    /// The number the next child takes, exact as a JS number.
    next: Cell<u64>,
}

impl std::fmt::Debug for StartedChildren {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartedChildren")
            .field("children", &self.table.borrow().len())
            .finish_non_exhaustive()
    }
}

/// A started child in the table.
struct Entry {
    /// Its state.
    child: Arc<Child>,
    /// Its `SpawnChild`, until `start`'s promise settles with it.
    pending: Option<v8::Global<v8::Object>>,
    /// Its token, weakly: dropped with the entry, which cancels the
    /// finalizer, and the finalizer drops the entry once the token is
    /// collected.
    _token: v8::Weak<v8::Object>,
}

impl StartedChildren {
    /// The child numbered `id`, when the table holds it.
    fn child(&self, id: u64) -> Option<Arc<Child>> {
        self.table
            .try_borrow()
            .ok()?
            .get(&id)
            .map(|entry| Arc::clone(&entry.child))
    }

    /// Takes the child numbered `id` out, when the table is free, which it
    /// always is when a finalizer runs.
    fn remove(&self, id: u64) {
        let removed = self
            .table
            .try_borrow_mut()
            .ok()
            .and_then(|mut table| table.remove(&id));
        // dropped once the table's borrow ended: dropping its weak
        // reference reaches the isolate
        drop(removed);
    }
}

/// `program.start(args, options?)` called on `receiver`: checked as `run`
/// is, `stdio` read beside the call's options, launched, its handles made
/// and its task spawned, and its promise deferred, settled with the
/// `SpawnChild` in a later turn; `None` with the exception pending when a
/// step threw; `errors`, `SpawnError`'s family as the member declares it.
pub fn start<'s>(
    state: &OpState,
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    args: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<Deferred<'s>> {
    let launched = (|| {
        let run = run_state(state)?;
        let program = launch::read_program(scope, receiver)?;
        let argv = launch::read_argv(scope, args)?;
        let options = launch::read_call_options(state, scope, Reads::Started, options)?;
        let launched =
            launch::launch_child(run, &program, &argv, options.cwd.as_ref(), options.stdio)?;
        Ok((launched, options.stdio, options.timeout))
    })();
    let (launched, stdio, timeout) = match launched {
        Ok(launched) => launched,
        Err(failure) => {
            throw(state, scope, errors, failure);
            return None;
        }
    };
    let pid = launched.pid;
    let child = Arc::new(Child::new(StartedChild {
        held: launched.held,
        exit: launched.exit,
        stdin: launched.stdin,
        outputs: [launched.stdout, launched.stderr],
        timeout,
    }));
    // the task holds `closed`'s settler from now, so the child is watched
    // whatever making its handles meets
    let task_child = Arc::clone(&child);
    let Some(closed) = later::defer_task(state, scope, finish_closed, move |settler| {
        ChildTask::new(task_child, settler)
    }) else {
        child.abandon();
        return None;
    };
    let made = make_child(state, scope, &child, pid, closed.promise(), stdio);
    let Some((id, made)) = made else {
        child.abandon();
        return None;
    };
    let Ok(run) = run_state(state) else {
        child.abandon();
        throw_error(scope, super::NO_SPAWN_STATE);
        return None;
    };
    if let Some(entry) = run.started.table.borrow_mut().get_mut(&id) {
        entry.pending = Some(v8::Global::new(scope, made));
    }
    later::defer(state, scope, finish_start, |settler| settler.settle(id))
}

/// Makes `child`'s handles: its token, registered in the run's table under
/// a new number, its stdin and output streams for those `stdio` pipes, and
/// the `SpawnChild` of them, `pid`, and `closed`, frozen. The number and the
/// `SpawnChild`, or `None` with an exception pending.
fn make_child<'s>(
    state: &OpState,
    scope: &mut v8::PinScope<'s, '_>,
    child: &Arc<Child>,
    pid: u32,
    closed: v8::Local<'s, v8::Promise>,
    stdio: [Stream; 3],
) -> Option<(u64, v8::Local<'s, v8::Object>)> {
    let Ok(run) = run_state(state) else {
        throw_error(scope, super::NO_SPAWN_STATE);
        return None;
    };
    let started = &run.started;
    let id = started.next.get();
    started.next.set(id + 1);
    #[expect(
        clippy::cast_precision_loss,
        reason = "a count of children started, far below 2 ** 53"
    )]
    let number: v8::Local<v8::Value> = v8::Number::new(scope, id as f64).into();
    let token = frozen_record(scope, &[(ID, number)])?;
    let prototype = |kind: &HandleKind| {
        let made = native::prototype(state, scope, kind);
        if made.is_none() {
            throw_error(scope, super::NO_SPAWN_STATE);
        }
        made
    };
    let null: v8::Local<v8::Value> = v8::null(scope).into();
    let stdin = if stdio[0] == Stream::Pipe {
        handle::make(
            scope,
            &SPAWN_STDIN,
            prototype(&SPAWN_STDIN)?,
            token.into(),
            &[],
        )?
        .into()
    } else {
        null
    };
    let mut outputs = [null, null];
    for (at, output) in outputs.iter_mut().enumerate() {
        if stdio[at + 1] == Stream::Pipe {
            let stream = u32::try_from(at).ok()?;
            let stream = v8::Integer::new_from_unsigned(scope, stream).into();
            let held = frozen_record(scope, &[(TOKEN, token.into()), (STREAM, stream)])?;
            *output = handle::make(
                scope,
                &SPAWN_OUTPUT,
                prototype(&SPAWN_OUTPUT)?,
                held.into(),
                &[],
            )?
            .into();
        }
    }
    // frozen, as the Node module's is, so no holder of the child gives it
    // a `then` of its own for the next to await
    closed.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    let own = [
        v8::Integer::new_from_unsigned(scope, pid).into(),
        closed.into(),
        outputs[0],
        outputs[1],
        stdin,
    ];
    let made = handle::make(
        scope,
        &SPAWN_CHILD,
        prototype(&SPAWN_CHILD)?,
        token.into(),
        &own,
    )?;
    let table = Rc::clone(&run.started);
    let weak = v8::Weak::with_finalizer(
        scope,
        token,
        Box::new(move |_: &mut v8::Isolate| table.remove(id)),
    );
    started.table.borrow_mut().insert(
        id,
        Entry {
            child: Arc::clone(child),
            pending: None,
            _token: weak,
        },
    );
    Some((id, made))
}

/// `start`'s promise's value, made on the isolate's thread: the
/// `SpawnChild` of the child numbered `id`, taken from the table.
fn finish_start<'s>(scope: &mut v8::PinScope<'s, '_>, id: u64) -> Option<v8::Local<'s, v8::Value>> {
    let state = JsRuntime::op_state_from(scope);
    let pending = state.try_borrow().ok().and_then(|state| {
        let data = state.try_borrow::<HostData>()?;
        let mut table = data.spawn.as_deref()?.started.table.try_borrow_mut().ok()?;
        table.get_mut(&id)?.pending.take()
    });
    drop(state);
    let Some(made) = pending else {
        throw_error(scope, NO_CHILD);
        return None;
    };
    Some(v8::Local::new(scope, made).into())
}

/// `closed`'s value: how the child ended, a frozen null-prototype record
/// as `run`'s result is. It never rejects: a child whose end couldn't be
/// read was killed (`SIGKILL`) before its reap failed, so that's the end
/// it's given.
fn finish_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    closed: Closed,
) -> Option<v8::Local<'s, v8::Value>> {
    let ended = closed.ended.unwrap_or(Ended::Killed {
        signal: libc::SIGKILL,
        dumped: false,
    });
    launch::finish(
        scope,
        Outcome {
            capture: Capture::None,
            ended: Ok(ended),
            stop: closed.stop,
            stdout: Vec::new(),
            stderr: Vec::new(),
            truncated: false,
            unallocated: false,
        },
    )
}

/// The child the handle `receiver`, of `kind`, names, and the stream its
/// state names for a stream's or an iterator's, or a `TypeError` when it's
/// no handle of `kind`.
fn read_child<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
    receiver: v8::Local<'s, v8::Value>,
) -> Result<(Arc<Child>, usize), Failure> {
    let run = run_state(state)?;
    let (_, held) = read_handle(scope, kind, receiver)?;
    // the host's own records, which no JS holds
    let field = |of: v8::Local<'s, v8::Object>, key: &str| {
        name(scope, key)
            .and_then(|key| of.get(scope, key.into()))
            .ok_or(Failure::Thrown)
    };
    let (token, stream) = if kind == &SPAWN_OUTPUT || kind == &SPAWN_OUTPUT_ITERATOR {
        let token =
            v8::Local::<v8::Object>::try_from(field(held, TOKEN)?).map_err(|_| Failure::Thrown)?;
        let stream = field(held, STREAM)?
            .uint32_value(scope)
            .and_then(|stream| usize::try_from(stream).ok())
            .ok_or(Failure::Thrown)?;
        (token, stream)
    } else {
        (held, 0)
    };
    let id = field(token, ID)?
        .number_value(scope)
        .ok_or(Failure::Thrown)?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a number the host made of a count, exact as an f64"
    )]
    let id = id as u64;
    let child = run
        .started
        .child(id)
        .ok_or_else(|| Failure::Plain(NO_CHILD.to_owned()))?;
    Ok((child, stream))
}

/// `child.kill(signal?)` called on `receiver`: `signal`, `SIGTERM` when
/// it's `undefined`, sent once to the child or the group it leads, `true`,
/// or `false` once it has exited; a `TypeError` for a receiver that's no
/// `SpawnChild` or a signal `SpawnSignal` doesn't name.
pub fn kill<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    signal: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (child, _) = read_child(state, scope, &SPAWN_CHILD, receiver)?;
        let number = if signal.is_undefined() {
            libc::SIGTERM
        } else {
            let named = signal
                .is_string()
                .then(|| signal.to_rust_string_lossy(scope));
            SIGNALS
                .iter()
                .find(|(name, _)| named.as_deref() == Some(*name))
                .map(|(_, number)| *number)
                .ok_or_else(|| Failure::Type(NOT_A_SIGNAL.to_owned()))?
        };
        Ok(v8::Boolean::new(scope, child.kill(number)).into())
    })();
    plain_finish(scope, result)
}

/// A child's `[Symbol.asyncDispose]()` called on `receiver`: the child
/// stopped, `SIGTERM` then `SIGKILL` after the grace, unless it has ended,
/// and the promise settled once `closed` has.
pub fn dispose<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<Deferred<'s>> {
    let child = match read_child(state, scope, &SPAWN_CHILD, receiver) {
        Ok((child, _)) => child,
        Err(failure) => return plain_throw(scope, failure),
    };
    later::defer(state, scope, finish_nothing, |settler| {
        child.dispose(settler);
    })
}

/// `stdin.write(data)` called on `receiver`: `data`, a string as UTF-8 or
/// a `Uint8Array`, copied now, written in order, the promise settled
/// `true` once the pipe took it, or `false` when it went nowhere; a
/// `TypeError` rejection for data that's neither, as the Node module's.
pub fn write<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
) -> Option<Deferred<'s>> {
    let read = (|| {
        let (child, _) = read_child(state, scope, &SPAWN_STDIN, receiver)?;
        let bytes = if let Ok(text) = v8::Local::<v8::String>::try_from(data) {
            let mut units = vec![0; text.length()];
            text.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
            String::from_utf16(&units)
                .map_err(|_| Failure::Type(NOT_WELL_FORMED.to_owned()))?
                .into_bytes()
        } else if let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(data) {
            // a copy, so a later change to the array changes nothing sent
            let mut bytes = vec![0; view.byte_length()];
            let copied = view.copy_contents(&mut bytes);
            bytes.truncate(copied);
            bytes
        } else {
            return Err(Failure::Type(NOT_DATA.to_owned()));
        };
        Ok((child, bytes))
    })();
    let (child, bytes) = match read {
        Ok(read) => read,
        Err(failure) => return plain_throw(scope, failure),
    };
    later::defer(state, scope, finish_bool, move |settler| {
        child.write(bytes, settler);
    })
}

/// `stdin.close()` called on `receiver`: the pipe closed once every write
/// before it is done, the promise settled then; closing twice is harmless.
pub fn close<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<Deferred<'s>> {
    let child = match read_child(state, scope, &SPAWN_STDIN, receiver) {
        Ok((child, _)) => child,
        Err(failure) => return plain_throw(scope, failure),
    };
    later::defer(state, scope, finish_nothing, |settler| child.close(settler))
}

/// A stream's `[Symbol.asyncIterator]()` called on `receiver`: its one
/// reader's iterator, a handle of the stream's state, or a `TypeError` for
/// a second call.
pub fn iterate<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (child, stream) = read_child(state, scope, &SPAWN_OUTPUT, receiver)?;
        if !child.claim(stream) {
            return Err(Failure::Type(ONE_READER.to_owned()));
        }
        let (_, held) = read_handle(scope, &SPAWN_OUTPUT, receiver)?;
        let prototype = native::prototype(state, scope, &SPAWN_OUTPUT_ITERATOR)
            .ok_or_else(|| Failure::Plain(super::NO_SPAWN_STATE.to_owned()))?;
        handle::make(scope, &SPAWN_OUTPUT_ITERATOR, prototype, held.into(), &[])
            .map(Into::into)
            .ok_or(Failure::Thrown)
    })();
    plain_finish(scope, result)
}

/// An iterator's `next()` called on `receiver`: its stream's next chunk,
/// `{ done: false, value }`, or `{ done: true, value: undefined }` once
/// it's done, settled in a later turn.
pub fn next<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<Deferred<'s>> {
    let (child, stream) = match read_child(state, scope, &SPAWN_OUTPUT_ITERATOR, receiver) {
        Ok(read) => read,
        Err(failure) => return plain_throw(scope, failure),
    };
    later::defer(state, scope, finish_chunk, move |settler| {
        child.next(stream, settler);
    })
}

/// An iterator's `return()` called on `receiver`: the rest of its stream
/// dropped, `{ done: true, value: undefined }`.
pub fn iterator_return<'s>(
    state: &OpState,
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result =
        read_child(state, scope, &SPAWN_OUTPUT_ITERATOR, receiver).and_then(|(child, stream)| {
            child.discard(stream);
            chunk_result(scope, None).ok_or(Failure::Thrown)
        });
    plain_finish(scope, result)
}

/// An iterator's `throw(error)` called on `receiver`: the rest of its
/// stream dropped, and `error` thrown, as an async generator thrown into
/// where nothing catches.
pub fn iterator_throw<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    error: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    match read_child(state, scope, &SPAWN_OUTPUT_ITERATOR, receiver) {
        Ok((child, stream)) => {
            child.discard(stream);
            scope.throw_exception(error);
            None
        }
        Err(failure) => plain_finish(scope, Err(failure)),
    }
}

/// `result` as a member that throws no `SpawnError` returns it.
fn plain_finish<'s>(
    scope: &v8::PinScope<'s, '_>,
    result: Result<v8::Local<'s, v8::Value>, Failure>,
) -> Option<v8::Local<'s, v8::Value>> {
    match result {
        Ok(value) => Some(value),
        Err(failure) => {
            plain_throw::<()>(scope, failure);
            None
        }
    }
}

/// Throws `failure`, of a member that throws no `SpawnError`: a
/// `TypeError` or a plain `Error`.
fn plain_throw<T>(scope: &v8::PinScope<'_, '_>, failure: Failure) -> Option<T> {
    match failure {
        Failure::Type(message) => throw_type_error(scope, &message),
        Failure::Plain(message) => throw_error(scope, &message),
        Failure::Thrown => {}
        // no member here makes a `SpawnError`
        Failure::Spawn { cause, .. } | Failure::NotGranted { cause, .. } => {
            throw_error(scope, &cause);
        }
    }
    None
}

/// A settlement's value of nothing: `undefined`.
const fn finish_nothing<'s>(
    _: &mut v8::PinScope<'s, '_>,
    (): (),
) -> Option<v8::Local<'s, v8::Value>> {
    None
}

/// A write's value: whether the pipe took it.
#[expect(
    clippy::unnecessary_wraps,
    reason = "a settlement's `Finish` returns an option, `None` when it threw"
)]
fn finish_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    taken: bool,
) -> Option<v8::Local<'s, v8::Value>> {
    Some(v8::Boolean::new(scope, taken).into())
}

/// A `next()`'s value: its chunk's iterator result.
fn finish_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    chunk: Chunk,
) -> Option<v8::Local<'s, v8::Value>> {
    chunk_result(scope, chunk)
}

/// An iterator's result for `chunk`, a null-prototype record, so a promise
/// settled with it finds no `then` of the realm's: `{ done: false, value }`
/// with the chunk a `Uint8Array` of its own buffer, from the isolate's
/// allocator, or `{ done: true, value: undefined }` for none.
fn chunk_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    chunk: Chunk,
) -> Option<v8::Local<'s, v8::Value>> {
    let (done, value): (bool, v8::Local<v8::Value>) = match chunk {
        None => (true, v8::undefined(scope).into()),
        Some(bytes) => {
            let mut buffer = PendingArrayBuffer::new(scope, bytes.len());
            buffer.bytes_mut().copy_from_slice(&bytes);
            let length = buffer.len();
            let buffer = buffer.into_array_buffer(scope);
            (false, v8::Uint8Array::new(scope, buffer, 0, length)?.into())
        }
    };
    let keys = [name(scope, "done")?, name(scope, "value")?];
    let values = [v8::Boolean::new(scope, done).into(), value];
    let null = v8::null(scope).into();
    Some(v8::Object::with_prototype_and_properties(scope, null, &keys, &values).into())
}
