//! The native half of the fs host module: the run's roots, the
//! directories its handles name beneath them, and every check of a call.
//!
//! The roots are the directories the run reads, each opened once, as a
//! path only (`O_PATH | O_DIRECTORY`), by its real path with no symlink on
//! it, the first time a run makes the fs namespace ([`FsState`]): the
//! project's root, the entry's, is the first and, for now, the only one.
//! Only a root holds a descriptor. A handle, an `FsReadDir` or an `FsDir`
//! (`handle.rs`), holds plain data under its brand: its root's place in
//! the table, its directory's real path beneath the root, and whether it
//! may write, which none may yet; and its directory's absolute path, as
//! its own `path`. So no handle owns a descriptor, none needs closing, and
//! a handle follows its path as the Node module's does: a directory moved
//! away isn't followed, and one moved in is what the handle names.
//!
//! An op on a handle opens the handle's directory beneath its root's
//! descriptor, refusing any symlink on the way, since the path it holds is
//! real (`ELOOP` once one is swapped in, as the Node module's `outside_root`
//! for a moved handle), then resolves its target beneath that by
//! `openat2` with `RESOLVE_BENEATH` (`gro_sys::beneath`): two opens and a
//! close, a handle on its root's own directory skipping the first. A
//! path's `..` segments resolve by its text first, as the Node module's
//! do: a climb above the handle is `outside_root` before any call, and an
//! emptied path is the handle's own directory, `.`. What the kernel then
//! refuses as leaving the handle's directory, an absolute symlink or one
//! that climbs out, a magic link among them, is `outside_root` too, naming
//! the path as requested.
//!
//! `fs.open_dir` and `fs.real_path` take an absolute path and walk it as
//! Node's `realpath` does, a `..` after the symlink before it ([`walk`]):
//! the names before a `..` open as a directory beneath the longest root
//! that holds them by their text, the `..` takes a name off that
//! directory's real path, and what's left walks on, so a lookup always
//! starts at a root, and a `..` above one is text. The entry's real path is
//! always beneath its root: one that left the root while it was opened is
//! `io_error`, its cause naming no path outside. A path no root holds is
//! `not_granted`, refused whether or not anything is there, and so is one
//! through a symlink that leaves its root by its target's text: an absolute
//! symlink, even to a path inside the root, or a relative one whose target
//! climbs above the root, even back into it, both of which Node follows. A
//! root holds a path by its own real path's text, so a path that reaches it
//! through a symlink outside it is held by none.
//! Such a refusal is recorded in the run's record of refusals as it's
//! thrown, which fails the run whatever the program does with the error.
//!
//! A whole-file read opens its target beneath the handle without waiting
//! for it, and `gro_sys` refuses anything but a regular file by the opened
//! descriptor before a byte is read: a directory is `is_a_directory`, as
//! the Node module's, and a FIFO, a socket, or a device `io_error`, where
//! the Node module's read would wait on a FIFO or read a device without
//! end. A regular file is read to its end, whatever size it reports.
//!
//! `list` returns a listing at once, reading nothing, not even its
//! receiver or its path, as the Node module's async generator does: its
//! first `next()` checks them, reads the whole directory, and sorts it by
//! the names' bytes, the directory's descriptor closed before that call
//! settles. So a listing, a handle whose state is the host's record under
//! its brand, holds no descriptor between calls, only its entries, which
//! a collection frees with it, however it ends.
//!
//! Every member's first step past its grant's check is the check that the
//! kernel serves `openat2` ([`FsState::ready`]), so a kernel below Linux 5.6
//! fails every op alike, as a plain `Error`, never only the ones that
//! resolve a path of more than one name. A root that couldn't be opened
//! (its path isn't UTF-8, or it's gone) is held as why, and every op
//! through it fails with `io_error` naming that in its `cause`: a value
//! the namespace holds is made without throwing.
//!
//! An argument is read as the value it is, as `env`'s are: a path is a
//! string, never converted, and an options record's `follow` is its data,
//! so a getter or a proxy is a `TypeError` where the Node module would run
//! it. A failure's error is an `FsError` whose `kind` and `path` are what
//! the Node module's would be, its `message` exactly `kind: path`, and its
//! `cause`, when the OS said something, an `Error` whose message is what
//! the OS said, which no case compares.
//!
//! Each op makes its calls inline and returns, for an `async` member, a
//! promise the host already settled. Its I/O ([`sys`]) takes owned
//! arguments, a root's descriptor shared by count and the paths as Rust
//! text, and borrows nothing of V8's, so it can move to a blocking pool
//! with no change to what it's given.

use std::cell::{Cell, OnceCell};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt as _;
use std::path::Path;
use std::sync::Arc;

use deno_core::{OpState, v8};

use super::error_class::{DeclaredFamily, ErrorClasses};
use super::handle::{self, FS_DIR, FS_LISTING, FS_READ_DIR, HandleKind};
use super::{HostData, NO_ERROR_CLASSES, name, native, string, throw_error, throw_type_error};
use crate::RefusedBy;
use crate::loader::RefusalLog;
use crate::resolver::Capability;

/// The fs namespace's specifier.
const MODULE: &str = "@fuzdev/gro/fs.ts";

/// What a member throws in a runtime started without the run's fs state,
/// which only a runtime made outside a run's start is.
const NO_FS_STATE: &str = "this runtime was started without the fs host module's state";

/// Why the root table's one root isn't open in a runtime given no
/// project, which only a runtime made outside a run's start is.
const NO_PROJECT: &str = "this runtime was started without a project";

/// What a method called on anything but a handle of its kind throws.
const NOT_A_HANDLE: &str = "fs's methods are called on an fs handle of their kind, not another \
                            object";

/// What a refused reach throws in a runtime started without the run's
/// record of refusals, which only a runtime made outside a run's start is.
const NO_REFUSAL_STATE: &str = "fs refused a path, but this runtime was started without the \
                                run's record of refusals, so the refusal can't fail the run";

/// The kind of the error a reach outside the run's roots throws.
const NOT_GRANTED: &str = "not_granted";

/// The keys of a handle's state.
const ROOT: &str = "root";
const BENEATH: &str = "beneath";
const WRITE: &str = "write";

/// The run's fs state, put in its `OpState` when it starts, empty: its
/// roots join when the fs namespace is first made.
#[derive(Debug, Default)]
pub struct FsState {
    /// The run's roots, each opened once, by its place in the table.
    roots: OnceCell<Vec<Root>>,
    /// Whether the kernel was found to serve `openat2`. `gro_sys` holds a
    /// failed check for the process, so this holds the other answer for the
    /// run, sparing every op after the first its probe.
    served: Cell<bool>,
}

/// A root of the run: a directory whose tree it reads.
#[derive(Debug)]
struct Root {
    /// Its absolute real path, which the paths of its handles extend; lossy
    /// only when its path isn't UTF-8, when it isn't open.
    path: String,
    /// Its descriptor, a path only, shared with any op under way, or why it
    /// isn't open, which every op through it reports.
    fd: Result<Arc<OwnedFd>, String>,
}

impl FsState {
    /// The run's roots, opened now if they weren't: the project's, from the
    /// run's data.
    fn roots(&self, data: &HostData) -> &[Root] {
        self.roots
            .get_or_init(|| vec![open_root(data.project.as_deref())])
    }

    /// That the kernel serves `openat2`, asked once a run while it does,
    /// else a plain `Error` saying what's missing.
    fn ready(&self) -> Result<(), Failure> {
        if !self.served.get() {
            sys::check().map_err(|error| Failure::Plain(error.to_string()))?;
            self.served.set(true);
        }
        Ok(())
    }
}

/// The root of `dir`, the project's root, opened, or held as why it isn't.
fn open_root(dir: Option<&Path>) -> Root {
    let Some(dir) = dir else {
        return Root {
            path: String::new(),
            fd: Err(NO_PROJECT.to_owned()),
        };
    };
    let path = dir.to_string_lossy().into_owned();
    let fd = if dir.to_str().is_none() {
        Err(format!(
            "the project's path {path} isn't UTF-8, so no string names it"
        ))
    } else {
        sys::open_root(dir)
            .map(Arc::new)
            .map_err(|error| format!("the project's directory {path} can't be opened: {error}"))
    };
    Root { path, fd }
}

/// Why a call failed, before it's thrown.
enum Failure {
    /// A `TypeError` of this message: an argument or receiver that's none.
    Type(String),
    /// An `FsError` of `kind` about `path`, with the OS's account of it as
    /// its `cause` when there is one.
    Fs {
        kind: &'static str,
        path: String,
        cause: Option<String>,
    },
    /// `not_granted` about `path`, recorded as it's thrown.
    NotGranted(String),
    /// A plain `Error` of this message: what isn't the call's to fix.
    Plain(String),
    /// What V8 threw, already pending.
    Thrown,
}

impl Failure {
    /// An `FsError` of `kind` about `path`, its cause `error`.
    fn of(kind: &'static str, path: &str, error: &std::io::Error) -> Self {
        Self::Fs {
            kind,
            path: path.to_owned(),
            cause: Some(error.to_string()),
        }
    }

    /// `outside_root` about `requested`, the path as given, `why` its cause.
    fn outside(requested: &str, why: String) -> Self {
        Self::Fs {
            kind: "outside_root",
            path: requested.to_owned(),
            cause: Some(why),
        }
    }
}

/// Throws `failure` through `scope`: `errors`, `FsError`'s family as the
/// member declares it, for an `FsError`; a refusal recorded in the run's
/// record first.
fn throw(
    state: &OpState,
    scope: &v8::PinScope<'_, '_>,
    errors: Option<DeclaredFamily>,
    failure: Failure,
) {
    let (kind, path, cause) = match failure {
        Failure::Thrown => return,
        Failure::Type(message) => return throw_type_error(scope, &message),
        Failure::Plain(message) => return throw_error(scope, &message),
        Failure::Fs { kind, path, cause } => (kind, path, cause),
        Failure::NotGranted(path) => {
            let Some(log) = state.try_borrow::<RefusalLog>() else {
                return throw_error(scope, NO_REFUSAL_STATE);
            };
            // recorded as it's thrown, so the run fails whatever the
            // program does with the error
            log.record(
                RefusedBy::Grant {
                    grant: Capability::Fs,
                    error: super::error_class::FS_ERROR.name,
                },
                &format!("{NOT_GRANTED}: {path}"),
                None,
            );
            (NOT_GRANTED, path, None)
        }
    };
    let (Some(errors), Some(classes)) = (errors, state.try_borrow::<ErrorClasses>()) else {
        return throw_error(scope, NO_ERROR_CLASSES);
    };
    let Some(target) = string(scope, &path) else {
        return;
    };
    let cause = match cause {
        // a `RangeError` when it's longer than V8's longest string, so the
        // failure is never lost
        Some(cause) => match string(scope, &cause) {
            Some(cause) => Some(v8::Exception::error(scope, cause)),
            None => return,
        },
        None => None,
    };
    classes.throw(scope, errors, kind, target, cause);
}

/// `result` as a member returns it, its failure thrown.
fn finish<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    errors: Option<DeclaredFamily>,
    result: Result<v8::Local<'s, v8::Value>, Failure>,
) -> Option<v8::Local<'s, v8::Value>> {
    result
        .map_err(|failure| throw(state, scope, errors, failure))
        .ok()
}

/// The run's fs state and data, or a plain `Error` in a runtime made
/// outside a run's start.
fn run_state(state: &OpState) -> Result<(&FsState, &HostData), Failure> {
    match (
        state.try_borrow::<FsState>(),
        state.try_borrow::<HostData>(),
    ) {
        (Some(fs), Some(data)) => Ok((fs, data)),
        _ => Err(Failure::Plain(NO_FS_STATE.to_owned())),
    }
}

/// `fs.project`: an `FsDir` on the project's root, made of `prototype`, the
/// `FsDir`s' prototype the namespace is being made with, opening the
/// run's roots if they weren't. It throws nothing for a reason of the
/// run's, as a value the namespace holds mustn't: a root that couldn't
/// open, or, in a runtime made outside a run's start, no root table, is
/// reported by every op through the handle. `None` with an exception
/// pending only when V8 failed.
pub fn project<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let path = run_state(state).map_or(String::new(), |(fs, data)| fs.roots(data)[0].path.clone());
    make_dir(scope, &FS_DIR, prototype, 0, &path, "", false).map(Into::into)
}

/// `fs.open_dir(path)`: an `FsDir` on the directory at `path`, an absolute
/// path inside a root, `not_granted` outside every root, also when nothing
/// is there, or when it leads out of its root by a `..` or a symlink;
/// `errors`, `FsError`'s family as the member declares it.
pub fn open_dir<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (fs, data) = run_state(state)?;
        fs.ready()?;
        let path = path_text(scope, path)?;
        let (index, root, beneath) =
            reach(fs.roots(data), &path, "fs.open_dir", sys::Opened::Directory)?;
        let prototype = native::prototype(state, scope, &FS_DIR).ok_or(Failure::Thrown)?;
        make_dir(
            scope, &FS_DIR, prototype, index, &root.path, &beneath, false,
        )
        .map(Into::into)
        .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `fs.real_path(path)`: the real path of the entry at `path`, an absolute
/// path inside a root, every symlink followed beneath it, always a path
/// inside its root; refused as `open_dir` refuses one, and `io_error` when
/// the entry left its root while it was opened.
pub fn real_path<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (fs, data) = run_state(state)?;
        fs.ready()?;
        let path = path_text(scope, path)?;
        let (_, root, beneath) = reach(fs.roots(data), &path, "fs.real_path", sys::Opened::Entry)?;
        string(scope, &join(&root.path, &beneath))
            .map(Into::into)
            .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `dir(path)` called on `receiver`, a handle of `kind`, the method's: a
/// handle of `kind` on the subdirectory at `path` beneath it, which must
/// exist, writing only when `kind` writes and the receiver may.
pub fn dir<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    kind: &'static HandleKind,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (fs, data) = run_state(state)?;
        fs.ready()?;
        let handle = read_handle(scope, kind, receiver)?;
        let requested = path_text(scope, path)?;
        let roots = fs.roots(data);
        let (root, handle_path) = handle_root(roots, &handle)?;
        let lexical = to_lexical(&requested, &handle_path)?;
        let target = lexical.path_beneath(&handle_path);
        let fd = root_fd(root, &target)?;
        let real = sys::open_dir(fd, handle.beneath.clone(), lexical.rel())
            .map_err(|error| op_failure(error, &requested, &handle_path, &target))?;
        let beneath = beneath_of(&root.path, real).map_err(|why| {
            Failure::outside(
                &requested,
                format!("{requested} beneath {handle_path}: {why}"),
            )
        })?;
        let prototype = native::prototype(state, scope, kind).ok_or(Failure::Thrown)?;
        let write = handle.write && *kind == FS_DIR;
        make_dir(
            scope,
            kind,
            prototype,
            handle.root,
            &root.path,
            &beneath,
            write,
        )
        .map(Into::into)
        .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `readonly()` called on `receiver`, an `FsDir`: an `FsReadDir` on the same
/// directory, which no op writes through.
pub fn readonly<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (fs, data) = run_state(state)?;
        fs.ready()?;
        let handle = read_handle(scope, &FS_DIR, receiver)?;
        let (root, _) = handle_root(fs.roots(data), &handle)?;
        let prototype = native::prototype(state, scope, &FS_READ_DIR).ok_or(Failure::Thrown)?;
        make_dir(
            scope,
            &FS_READ_DIR,
            prototype,
            handle.root,
            &root.path,
            &handle.beneath,
            false,
        )
        .map(Into::into)
        .ok_or(Failure::Thrown)
    })();
    finish(state, scope, None, result)
}

/// What `stat` and `exists` answer: the entry's info, or none when nothing
/// is there.
enum Found {
    /// The entry, as `stat` reported it.
    Entry(sys::Info),
    /// Nothing at the path.
    Missing,
}

/// `stat(path, options?)` called on `receiver`: the entry's info, a
/// null-prototype record of its `kind`, `size`, `mtime_ms`, and `mode`, or
/// `null` when nothing is at `path`, following a final symlink unless
/// `options` says `follow: false`, which a trailing `/` overrides, as in the
/// kernel.
pub fn stat<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = find(state, scope, receiver, path, Some(options)).and_then(|found| match found {
        Found::Entry(info) => file_info(scope, &info).ok_or(Failure::Thrown),
        Found::Missing => Ok(v8::null(scope).into()),
    });
    finish(state, scope, Some(errors), result)
}

/// `exists(path)` called on `receiver`: whether an entry is at `path`,
/// following a final symlink; any failure but a missing entry throws.
pub fn exists<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = find(state, scope, receiver, path, None)
        .map(|found| v8::Boolean::new(scope, matches!(found, Found::Entry(_))).into());
    finish(state, scope, Some(errors), result)
}

/// The entry at `path` beneath `receiver`, a handle of `FsReadDir`'s kind
/// or one it extends, as `stat` reads it with `options`, or as `exists`
/// does with none.
fn find<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    options: Option<v8::Local<'s, v8::Value>>,
) -> Result<Found, Failure> {
    let (fs, data) = run_state(state)?;
    fs.ready()?;
    let handle = read_handle(scope, &FS_READ_DIR, receiver)?;
    let requested = path_text(scope, path)?;
    let follow = match options {
        Some(options) => follow_option(scope, options)?,
        None => true,
    };
    let op = beneath(fs.roots(data), handle, requested)?;
    // a trailing `/` follows a final symlink, as in the kernel
    let follow = follow || op.lexical.dir;
    let (fd, dir, rel) = op.call();
    match sys::stat(fd, dir, rel, follow) {
        Ok(info) => Ok(Found::Entry(info)),
        Err(error) if is_missing(&error) => Ok(Found::Missing),
        Err(error) => Err(op.failure(error)),
    }
}

/// Whether an op's `error` is nothing at its path, the handle's directory
/// or the target.
fn is_missing(error: &sys::OpError) -> bool {
    matches!(error, sys::OpError::Handle(error) | sys::OpError::Target(error)
        if error.kind() == std::io::ErrorKind::NotFound)
}

/// An op on a path beneath a handle, resolved and checked, ready for its
/// calls.
struct Beneath {
    /// The descriptor of the handle's root.
    fd: Arc<OwnedFd>,
    /// The handle.
    handle: Handle,
    /// The path as requested.
    requested: String,
    /// It, resolved by its text beneath the handle.
    lexical: Lexical,
    /// The handle's directory's absolute path.
    handle_path: String,
    /// The absolute path the op names, which its errors name.
    target: String,
}

impl Beneath {
    /// What the op's call takes, owned: the root's descriptor, the
    /// handle's directory's real path beneath it, and the path beneath the
    /// handle the kernel resolves.
    fn call(&self) -> (Arc<OwnedFd>, String, String) {
        (
            Arc::clone(&self.fd),
            self.handle.beneath.clone(),
            self.lexical.rel(),
        )
    }

    /// What the op's failure, `error`, throws ([`op_failure`]).
    fn failure(&self, error: sys::OpError) -> Failure {
        op_failure(error, &self.requested, &self.handle_path, &self.target)
    }
}

/// The op on `requested` beneath `handle`, of the run's `roots`: its path
/// resolved by its text ([`to_lexical`]), `outside_root` for a climb above
/// the handle, and `io_error` when the handle's root isn't open.
fn beneath(roots: &[Root], handle: Handle, requested: String) -> Result<Beneath, Failure> {
    let (root, handle_path) = handle_root(roots, &handle)?;
    let lexical = to_lexical(&requested, &handle_path)?;
    let target = lexical.path_beneath(&handle_path);
    let fd = root_fd(root, &target)?;
    Ok(Beneath {
        fd,
        handle,
        requested,
        lexical,
        handle_path,
        target,
    })
}

/// What a whole-file read returns.
#[derive(Clone, Copy)]
pub enum Contents {
    /// The bytes, a `Uint8Array` of a buffer of its own.
    Bytes,
    /// The text, decoded from UTF-8 as Node's `readFile` with `'utf8'`
    /// decodes it: each malformed sequence a U+FFFD, a BOM kept.
    Text,
}

/// `read(path)`, `read_text(path)`, and their kin, called on `receiver`:
/// the contents of the regular file at `path`, read to its end, as
/// `contents` says; with `if_exists`, `null` when nothing is at `path`,
/// where any other failure still throws. A directory is `is_a_directory`,
/// and anything else that isn't a regular file, a FIFO, a socket, or a
/// device, `io_error`, refused before any byte is read, where the Node
/// module would wait on a FIFO or read a device without end.
pub fn read<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    contents: Contents,
    if_exists: bool,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (fs, data) = run_state(state)?;
        fs.ready()?;
        let handle = read_handle(scope, &FS_READ_DIR, receiver)?;
        let requested = path_text(scope, path)?;
        let op = beneath(fs.roots(data), handle, requested)?;
        let (fd, dir, rel) = op.call();
        let bytes = match sys::read(fd, dir, rel) {
            Ok(bytes) => bytes,
            Err(error) if if_exists && is_missing(&error) => return Ok(v8::null(scope).into()),
            Err(error) => return Err(op.failure(error)),
        };
        match contents {
            Contents::Bytes => to_bytes(scope, bytes),
            Contents::Text => v8::String::new_from_utf8(scope, &bytes, v8::NewStringType::Normal)
                .map(Into::into)
                .ok_or_else(|| Failure::Fs {
                    kind: "io_error",
                    path: op.target.clone(),
                    cause: Some(format!(
                        "{} holds {} bytes, more text than a string holds",
                        op.target,
                        bytes.len()
                    )),
                }),
        }
    })();
    finish(state, scope, Some(errors), result)
}

/// `bytes` as a `Uint8Array` of a buffer of its own, which takes the bytes
/// over without a copy.
fn to_bytes<'s>(
    scope: &v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Result<v8::Local<'s, v8::Value>, Failure> {
    let length = bytes.len();
    let buffer = if length == 0 {
        v8::ArrayBuffer::new(scope, 0)
    } else {
        let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
        v8::ArrayBuffer::with_backing_store(scope, &store)
    };
    v8::Uint8Array::new(scope, buffer, 0, length)
        .map(Into::into)
        .ok_or_else(|| Failure::Plain(format!("V8 made no Uint8Array of {length} bytes")))
}

/// `read_link(path)` called on `receiver`: the target of the symlink at
/// `path`, as stored, never followed, so it may be absolute or lead out of
/// the handle, which reading it reaches nothing of; with a trailing `/` the
/// link is followed, as in the kernel, and what it reaches is no symlink.
/// An entry that isn't a symlink is `io_error`, and so is a target that
/// isn't UTF-8, since no string spells it.
pub fn read_link<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (fs, data) = run_state(state)?;
        fs.ready()?;
        let handle = read_handle(scope, &FS_READ_DIR, receiver)?;
        let requested = path_text(scope, path)?;
        let op = beneath(fs.roots(data), handle, requested)?;
        let (fd, dir, rel) = op.call();
        let target =
            sys::read_link(fd, dir, rel, op.lexical.dir).map_err(|error| op.failure(error))?;
        let target = to_utf8(target.into_vec(), &op.target, "is a symlink to")?;
        string(scope, &target)
            .map(Into::into)
            .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `bytes`, a name the kernel gave for `path`, as text, or `io_error`
/// about `path` when they aren't UTF-8, its cause saying `what` they are
/// with each byte that isn't printable ASCII escaped, as the Node module
/// says it.
fn to_utf8(bytes: Vec<u8>, path: &str, what: &str) -> Result<String, Failure> {
    String::from_utf8(bytes).map_err(|error| not_utf8(error.as_bytes(), path, what))
}

/// `io_error` about `path` for `bytes`, a name the kernel gave for it
/// that isn't UTF-8, as [`to_utf8`] fails.
fn not_utf8(bytes: &[u8], path: &str, what: &str) -> Failure {
    Failure::Fs {
        kind: "io_error",
        path: path.to_owned(),
        cause: Some(format!(
            "{path} {what} {}, which isn't UTF-8",
            escaped(bytes)
        )),
    }
}

/// `bytes` with printable ASCII as itself and every other byte, a `\`
/// among them, as `\xNN`.
fn escaped(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for &byte in bytes {
        if (0x20..0x7f).contains(&byte) && byte != b'\\' {
            text.push(char::from(byte));
        } else {
            let _ = write!(text, "\\x{byte:02x}");
        }
    }
    text
}

/// The keys of a listing's state.
const LISTED_DIR: &str = "dir";
const LISTED_PATH: &str = "path";
const ENTRIES: &str = "entries";
const AT: &str = "at";
const DONE: &str = "done";

/// What a listing's method called on anything but a listing throws.
const NOT_A_LISTING: &str = "a listing's methods are called on a listing `list` returned, not \
                             another object";

/// `list(path?)` called on `receiver`: a listing, an async iterator of the
/// entries of the directory at `path`, the handle's own for none, whose
/// work waits for its first `next()`, as the Node module's async
/// generator's does: the call reads nothing, not even its receiver or its
/// path, which the first `next()` checks, then reads the whole directory,
/// sorted by the names' UTF-8 bytes, before it settles. So a listing holds
/// no descriptor between calls, only its entries, and one abandoned,
/// returned from, or thrown into holds nothing a collection doesn't free.
pub fn list<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let prototype = native::prototype(state, scope, &FS_LISTING).ok_or_else(|| {
            Failure::Plain("the fs namespace made no listing prototype".to_owned())
        })?;
        let keys = [
            name(scope, LISTED_DIR),
            name(scope, LISTED_PATH),
            name(scope, ENTRIES),
            name(scope, AT),
            name(scope, DONE),
        ];
        let keys = keys
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(Failure::Thrown)?;
        let values = [
            receiver,
            path,
            v8::undefined(scope).into(),
            v8::Integer::new(scope, 0).into(),
            v8::Boolean::new(scope, false).into(),
        ];
        let null = v8::null(scope).into();
        // the listing's state, which no JS holds, changed by its methods
        let held = v8::Object::with_prototype_and_properties(scope, null, &keys, &values);
        handle::make(scope, &FS_LISTING, prototype, held.into(), &[])
            .map(Into::into)
            .ok_or(Failure::Thrown)
    })();
    finish(state, scope, None, result)
}

/// A listing's state, as its methods read it from the listing `receiver`,
/// or a `TypeError` when it's none.
fn read_listing<'s>(
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, Failure> {
    let not_a_listing = || Failure::Type(NOT_A_LISTING.to_owned());
    let (_, held) = handle::read(scope, &FS_LISTING, receiver).ok_or_else(not_a_listing)?;
    // the host's own record, which no JS holds
    v8::Local::<v8::Object>::try_from(held).map_err(|_| not_a_listing())
}

/// The value of `key` in a listing's state, `held`, an own data property
/// of a null-prototype record, read running no code.
fn held_value<'s>(
    scope: &v8::PinScope<'s, '_>,
    held: v8::Local<'s, v8::Object>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, Failure> {
    name(scope, key)
        .and_then(|key| held.get(scope, key.into()))
        .ok_or(Failure::Thrown)
}

/// Sets `key` in a listing's state, `held`, to `value`, a definition, which
/// runs no code.
fn set_held<'s>(
    scope: &v8::PinScope<'s, '_>,
    held: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), Failure> {
    let key = name(scope, key).ok_or(Failure::Thrown)?;
    held.create_data_property(scope, key, value)
        .filter(|defined| *defined)
        .map(|_| ())
        .ok_or(Failure::Thrown)
}

/// Ends the listing whose state is `held`: every later `next()` is done,
/// and what it held, its handle, path, and entries, is let go.
fn end_listing<'s>(
    scope: &v8::PinScope<'s, '_>,
    held: v8::Local<'s, v8::Object>,
) -> Result<(), Failure> {
    let undefined = v8::undefined(scope).into();
    for key in [LISTED_DIR, LISTED_PATH, ENTRIES] {
        set_held(scope, held, key, undefined)?;
    }
    set_held(scope, held, DONE, v8::Boolean::new(scope, true).into())
}

/// An iterator's result, `{ value, done }`, a null-prototype record, so a
/// promise settled with it finds no `then` of the realm's.
fn iterator_result<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    done: bool,
) -> Result<v8::Local<'s, v8::Value>, Failure> {
    let keys = [
        name(scope, "value").ok_or(Failure::Thrown)?,
        name(scope, "done").ok_or(Failure::Thrown)?,
    ];
    let values = [value, v8::Boolean::new(scope, done).into()];
    let null = v8::null(scope).into();
    Ok(v8::Object::with_prototype_and_properties(scope, null, &keys, &values).into())
}

/// `next()` called on `receiver`, a listing: the next entry, `{ value,
/// done: false }`, the first call reading the whole directory, or `{ value:
/// undefined, done: true }` once there's none, or once the listing ended;
/// the first call's failure rejects, and ends the listing, as an async
/// generator's body throwing ends it.
pub fn listing_next<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let held = read_listing(scope, receiver)?;
        if held_value(scope, held, DONE)?.is_true() {
            return iterator_result(scope, v8::undefined(scope).into(), true);
        }
        let entries = held_value(scope, held, ENTRIES)?;
        let entries = if let Ok(entries) = v8::Local::<v8::Array>::try_from(entries) {
            entries
        } else {
            let dir = held_value(scope, held, LISTED_DIR)?;
            let path = held_value(scope, held, LISTED_PATH)?;
            let entries = match listed(state, scope, dir, path) {
                Ok(entries) => entries,
                Err(failure) => {
                    end_listing(scope, held)?;
                    return Err(failure);
                }
            };
            set_held(scope, held, ENTRIES, entries.into())?;
            // the handle and the path are read, so they're let go
            let undefined = v8::undefined(scope).into();
            set_held(scope, held, LISTED_DIR, undefined)?;
            set_held(scope, held, LISTED_PATH, undefined)?;
            entries
        };
        let at = held_value(scope, held, AT)?
            .uint32_value(scope)
            .ok_or(Failure::Thrown)?;
        if at >= entries.length() {
            end_listing(scope, held)?;
            return iterator_result(scope, v8::undefined(scope).into(), true);
        }
        let entry = entries.get_index(scope, at).ok_or(Failure::Thrown)?;
        set_held(
            scope,
            held,
            AT,
            v8::Integer::new_from_unsigned(scope, at + 1).into(),
        )?;
        iterator_result(scope, entry, false)
    })();
    finish(state, scope, Some(errors), result)
}

/// `return(value)` called on `receiver`, a listing: ends it, so every later
/// `next()` is done, and settles to `{ value, done: true }`, `value` as
/// given, never awaited, so no `then` of it is read.
pub fn listing_return<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = read_listing(scope, receiver).and_then(|held| {
        end_listing(scope, held)?;
        iterator_result(scope, value, true)
    });
    finish(state, scope, None, result)
}

/// `throw(error)` called on `receiver`, a listing: ends it, so every later
/// `next()` is done, and rejects with `error`, as an async generator thrown
/// into where nothing catches.
pub fn listing_throw<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    error: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = read_listing(scope, receiver).and_then(|held| {
        end_listing(scope, held)?;
        scope.throw_exception(error);
        Err::<v8::Local<'s, v8::Value>, _>(Failure::Thrown)
    });
    finish(state, scope, None, result)
}

/// The entries of the directory at `path` beneath `dir`, a handle of
/// `FsReadDir`'s kind or one it extends, the handle's own directory for an
/// undefined `path`: an array of `{ name, kind }` records of the realm's
/// `Object.prototype`, as the Node module's are, sorted by the names' UTF-8
/// bytes, a symlink's kind `symlink`, never followed. A name that isn't
/// UTF-8 fails the whole listing with `io_error`, naming the least such
/// name by its bytes in its cause, as the Node module's does.
fn listed<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    dir: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Array>, Failure> {
    let (fs, data) = run_state(state)?;
    fs.ready()?;
    let handle = read_handle(scope, &FS_READ_DIR, dir)?;
    let requested = if path.is_undefined() {
        String::new()
    } else {
        path_text(scope, path)?
    };
    let op = beneath(fs.roots(data), handle, requested)?;
    let (fd, dir, rel) = op.call();
    let listed = sys::list(fd, dir, rel).map_err(|error| op.failure(error))?;
    let mut entries = Vec::with_capacity(listed.len());
    let mut invalid: Option<Vec<u8>> = None;
    for entry in listed {
        match String::from_utf8(entry.name.into_vec()) {
            Ok(name) => entries.push((name, entry.kind)),
            Err(error) => {
                let bytes = error.into_bytes();
                if invalid.as_ref().is_none_or(|least| bytes < *least) {
                    invalid = Some(bytes);
                }
            }
        }
    }
    if let Some(bytes) = invalid {
        return Err(not_utf8(&bytes, &op.target, "holds an entry named"));
    }
    // a `String`'s order is its UTF-8 bytes'
    entries.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
    let object_prototype = v8::Object::new(scope)
        .get_prototype(scope)
        .ok_or(Failure::Thrown)?;
    let keys = [
        name(scope, "name").ok_or(Failure::Thrown)?,
        name(scope, "kind").ok_or(Failure::Thrown)?,
    ];
    let mut records = Vec::with_capacity(entries.len());
    for (entry_name, kind) in entries {
        let values = [
            string(scope, &entry_name).ok_or(Failure::Thrown)?.into(),
            string(scope, kind).ok_or(Failure::Thrown)?.into(),
        ];
        records.push(
            v8::Object::with_prototype_and_properties(scope, object_prototype, &keys, &values)
                .into(),
        );
    }
    Ok(v8::Array::new_with_elements(scope, &records))
}

/// A write op called on `receiver`, an `FsDir`, with `paths`, each checked
/// in order as the op would check it: no handle writes yet, so each is
/// refused `not_granted`, naming the first path, recorded as it's thrown.
pub fn refuse_write<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    paths: &[v8::Local<'s, v8::Value>],
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (fs, data) = run_state(state)?;
        fs.ready()?;
        let handle = read_handle(scope, &FS_DIR, receiver)?;
        let (_, handle_path) = handle_root(fs.roots(data), &handle)?;
        let mut targets = Vec::with_capacity(paths.len());
        for path in paths {
            let requested = path_text(scope, *path)?;
            targets.push(to_lexical(&requested, &handle_path)?.path_beneath(&handle_path));
        }
        let first = targets.into_iter().next().unwrap_or(handle_path);
        Err::<v8::Local<'s, v8::Value>, _>(Failure::NotGranted(first))
    })();
    finish(state, scope, Some(errors), result)
}

/// A member not built on fgro yet, `name`, called on `receiver`, a handle of
/// `kind`, or on none for the root's: a plain `Error` saying so, once the
/// receiver is checked, since no kind of `FsError` is a missing feature.
pub fn unbuilt<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    name: &str,
    on: Option<(&'static HandleKind, v8::Local<'s, v8::Value>)>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let (fs, _) = run_state(state)?;
        fs.ready()?;
        if let Some((kind, receiver)) = on {
            read_handle(scope, kind, receiver)?;
        }
        Err::<v8::Local<'s, v8::Value>, _>(Failure::Plain(format!(
            "{name} of {MODULE} isn't built on fgro yet"
        )))
    })();
    finish(state, scope, None, result)
}

/// A handle, as an op reads the one it's handed.
struct Handle {
    /// Its root's place in the table.
    root: usize,
    /// Its directory's real path beneath the root, `""` for the root's own.
    beneath: String,
    /// Whether it may write.
    write: bool,
}

/// The handle `receiver` is, of `kind` or a kind extending it, or a
/// `TypeError` when it's none.
fn read_handle<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
    receiver: v8::Local<'s, v8::Value>,
) -> Result<Handle, Failure> {
    let not_a_handle = || Failure::Type(NOT_A_HANDLE.to_owned());
    let (_, held) = handle::read(scope, kind, receiver).ok_or_else(not_a_handle)?;
    // the host's own record, which no JS holds
    let held = v8::Local::<v8::Object>::try_from(held).map_err(|_| not_a_handle())?;
    let field = |key: &str| {
        name(scope, key)
            .and_then(|key| held.get(scope, key.into()))
            .ok_or(Failure::Thrown)
    };
    let root = v8::Local::<v8::Integer>::try_from(field(ROOT)?).map_err(|_| not_a_handle())?;
    let beneath = field(BENEATH)?.to_rust_string_lossy(scope);
    let write = field(WRITE)?.is_true();
    Ok(Handle {
        root: usize::try_from(root.value()).map_err(|_| not_a_handle())?,
        beneath,
        write,
    })
}

/// Makes a handle of `kind` of `prototype` on the directory `beneath` the
/// root at `index`, whose path is `root_path`, writing as `write` says.
fn make_dir<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &'static HandleKind,
    prototype: v8::Local<'s, v8::Object>,
    index: usize,
    root_path: &str,
    beneath: &str,
    write: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    let index = u32::try_from(index).ok()?;
    let keys = [
        name(scope, ROOT)?,
        name(scope, BENEATH)?,
        name(scope, WRITE)?,
    ];
    let values = [
        v8::Integer::new_from_unsigned(scope, index).into(),
        string(scope, beneath)?.into(),
        v8::Boolean::new(scope, write).into(),
    ];
    let null = v8::null(scope).into();
    let held = v8::Object::with_prototype_and_properties(scope, null, &keys, &values);
    held.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    let path = string(scope, &join(root_path, beneath))?;
    handle::make(scope, kind, prototype, held.into(), &[path.into()])
}

/// The root `handle` is beneath and its directory's absolute path.
fn handle_root<'r>(roots: &'r [Root], handle: &Handle) -> Result<(&'r Root, String), Failure> {
    let root = roots
        .get(handle.root)
        .ok_or_else(|| Failure::Plain("an fs handle names no root of the run".to_owned()))?;
    let path = join(&root.path, &handle.beneath);
    Ok((root, path))
}

/// `root`'s descriptor, or `io_error` about `path` naming why it isn't
/// open.
fn root_fd(root: &Root, path: &str) -> Result<Arc<OwnedFd>, Failure> {
    root.fd.clone().map_err(|why| Failure::Fs {
        kind: "io_error",
        path: path.to_owned(),
        cause: Some(why),
    })
}

/// `beneath` joined to the directory `dir`, an absolute path, as text.
fn join(dir: &str, beneath: &str) -> String {
    match (dir, beneath) {
        (_, "") => dir.to_owned(),
        ("/", _) => format!("/{beneath}"),
        _ => format!("{dir}/{beneath}"),
    }
}

/// `real`, the real path of an entry opened beneath the root at
/// `root_path`, as its path beneath the root, or why it's none: it's
/// outside the root's path, which only the entry or the root moved since
/// it opened makes, a path no root holds and so the reason doesn't name;
/// or it isn't UTF-8.
fn beneath_of(root_path: &str, real: OsString) -> Result<String, String> {
    let prefix = if root_path == "/" {
        "/".to_owned()
    } else {
        format!("{root_path}/")
    };
    let left = || format!("it left the root {root_path} while it was opened, or the root moved");
    let bytes = real.as_encoded_bytes();
    if bytes != root_path.as_bytes() && !bytes.starts_with(prefix.as_bytes()) {
        return Err(left());
    }
    let real = real
        .into_string()
        .map_err(|real| format!("the real path {} isn't UTF-8", real.display()))?;
    if real == root_path {
        return Ok(String::new());
    }
    real.strip_prefix(&prefix)
        .map(ToOwned::to_owned)
        .ok_or_else(left)
}

/// `value` as a path, as the Node module checks one: a string, no NUL
/// byte, well-formed Unicode, else a `TypeError`.
fn path_text(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<String, Failure> {
    let Ok(text) = v8::Local::<v8::String>::try_from(value) else {
        return Err(Failure::Type("fs paths are strings".to_owned()));
    };
    let mut units = vec![0; text.length()];
    text.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    if units.contains(&0) {
        return Err(Failure::Type("fs paths hold no NUL byte".to_owned()));
    }
    String::from_utf16(&units)
        .map_err(|_| Failure::Type("fs paths are well-formed Unicode".to_owned()))
}

/// The `follow` of `options`, as the Node module reads `options?.follow ??
/// true`: `true` for none, a missing or nullish `follow`, or a primitive's,
/// else the value's truth; a proxy or an accessor on the way is a
/// `TypeError`, since an op runs no code of a caller's.
fn follow_option(
    scope: &v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Value>,
) -> Result<bool, Failure> {
    let Ok(mut object) = v8::Local::<v8::Object>::try_from(options) else {
        return Ok(true);
    };
    let plain =
        || Failure::Type("fs options are plain data, read without running code: follow".to_owned());
    let key = name(scope, "follow").ok_or(Failure::Thrown)?;
    let value_key = name(scope, "value").ok_or(Failure::Thrown)?;
    loop {
        if object.is_proxy() {
            return Err(plain());
        }
        let descriptor = object
            .get_own_property_descriptor(scope, key)
            .ok_or(Failure::Thrown)?;
        if let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) {
            // a descriptor V8 made is an ordinary object of its fields
            if !descriptor
                .has_own_property(scope, value_key)
                .ok_or(Failure::Thrown)?
            {
                return Err(plain());
            }
            let value = descriptor
                .get(scope, value_key.into())
                .ok_or(Failure::Thrown)?;
            return Ok(value.is_null_or_undefined() || value.boolean_value(scope));
        }
        match object
            .get_prototype(scope)
            .and_then(|prototype| v8::Local::<v8::Object>::try_from(prototype).ok())
        {
            Some(prototype) => object = prototype,
            None => return Ok(true),
        }
    }
}

/// A path requested beneath a handle, its `..` resolved by its text.
struct Lexical {
    /// The names beneath the handle, none of them `.` or `..`.
    segments: Vec<String>,
    /// Whether the request ended in `/`, `/.`, or `/..`, so the entry must
    /// be a directory, as the kernel reads a trailing `/`.
    dir: bool,
}

impl Lexical {
    /// The path the kernel resolves beneath the handle's directory: the
    /// names joined, `.` for none, and a trailing `/` for a directory.
    fn rel(&self) -> String {
        let mut rel = if self.segments.is_empty() {
            ".".to_owned()
        } else {
            self.segments.join("/")
        };
        if self.dir {
            rel.push('/');
        }
        rel
    }

    /// The absolute path it names beneath `dir`, the handle's, which an
    /// error names.
    fn path_beneath(&self, dir: &str) -> String {
        join(dir, &self.segments.join("/"))
    }
}

/// `requested` resolved by its text beneath the handle at `handle_path`, as
/// the Node module's `to_lexical_path` resolves it: an absolute path, or one
/// whose `..` climbs above the handle, is `outside_root`.
fn to_lexical(requested: &str, handle_path: &str) -> Result<Lexical, Failure> {
    if requested.starts_with('/') {
        return Err(Failure::outside(
            requested,
            format!("absolute path {requested}"),
        ));
    }
    let parts: Vec<&str> = requested.split('/').collect();
    let mut segments: Vec<String> = Vec::new();
    for part in &parts {
        match *part {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(Failure::outside(
                        requested,
                        format!("{requested} climbs above {handle_path}"),
                    ));
                }
            }
            name => segments.push(name.to_owned()),
        }
    }
    let dir = !segments.is_empty() && matches!(parts.last(), Some(&("" | "." | "..")));
    Ok(Lexical { segments, dir })
}

/// The entry at `path`, an absolute path inside a root, opened as `opened`,
/// as [`walk`] reaches it, each step opened beneath its root by `openat2`:
/// its root's place, the root, and its real path beneath the root.
fn reach<'r>(
    roots: &'r [Root],
    path: &str,
    what: &str,
    opened: sys::Opened,
) -> Result<(usize, &'r Root, String), Failure> {
    walk(roots, path, what, opened, |root, rel, step| {
        let fd = root_fd(root, path)?;
        let real = sys::open_in_root(fd, rel, step).map_err(|error| {
            if sys::is_escape(&error) {
                Failure::NotGranted(path.to_owned())
            } else if matches!(step, sys::Opened::Directory)
                && error.kind() == std::io::ErrorKind::NotADirectory
            {
                // the Node module names no cause for a path that is no
                // directory
                Failure::Fs {
                    kind: "not_a_directory",
                    path: path.to_owned(),
                    cause: None,
                }
            } else {
                Failure::of(kind_of(&error), path, &error)
            }
        })?;
        beneath_of(&root.path, real).map_err(|why| Failure::Fs {
            kind: "io_error",
            path: path.to_owned(),
            cause: Some(why),
        })
    })
}

/// The entry at `path`, an absolute path, reached as Node's `realpath`
/// reaches it, `..` after the symlink before it: the names up to each `..`
/// that follows a name are opened as a directory beneath the root that
/// holds them by their text, each `..` takes a name off that directory's
/// real path, and the names after it walk on from what's left, until the
/// whole is opened as `opened`. `open` opens a path beneath a root as it's
/// told and answers its real path beneath the root. So a name before a `..`
/// must be a directory, a `..` above a root is taken off the text of a real
/// path, never looked up, and every lookup starts at a root that holds its
/// names by their text. Each step costs the depth of the path it opens,
/// never the length of what's left of the path.
///
/// The root's place, the root, and the entry's real path beneath it; a
/// `TypeError` naming `what` for a path that isn't absolute, and
/// `not_granted` for one whose names no root holds at any step.
fn walk<'r>(
    roots: &'r [Root],
    path: &str,
    what: &str,
    opened: sys::Opened,
    mut open: impl FnMut(&'r Root, String, sys::Opened) -> Result<String, Failure>,
) -> Result<(usize, &'r Root, String), Failure> {
    if !path.starts_with('/') {
        return Err(Failure::Type(format!(
            "{what} takes an absolute path: {path}"
        )));
    }
    // a trailing `/` or `/.` asks for a directory, as the kernel reads it
    let dir = path.ends_with('/') || path.ends_with("/.");
    // the names left to walk, the next one last
    let mut left: Vec<&str> = path
        .split('/')
        .rev()
        .filter(|name| !name.is_empty() && *name != ".")
        .collect();
    // the names walked: a real path's, then those after it, none a `..`
    let mut names: Vec<String> = Vec::new();
    while let Some(name) = left.pop() {
        if name != ".." {
            names.push(name.to_owned());
            continue;
        }
        // `/..` is `/`, as the kernel reads it
        if !names.is_empty() {
            let (_, root, rel) = find_root(roots, &names, false, path)?;
            let beneath = open(root, rel, sys::Opened::Directory)?;
            names = join(&root.path, &beneath)
                .split('/')
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }
        names.pop();
        while left.last() == Some(&"..") {
            left.pop();
            names.pop();
        }
    }
    let (index, root, rel) = find_root(roots, &names, dir, path)?;
    let beneath = open(root, rel, opened)?;
    Ok((index, root, beneath))
}

/// The root of `roots` that holds the path of `names`, none of them `.` or
/// `..`, by their text, the longest, with its place and the path beneath it
/// for the kernel to open: `.` for the root's own directory, and a trailing
/// `/` when `dir` asks for a directory, but for the root's own, as a
/// handle's isn't asked for; `not_granted` about `path`, the path as
/// requested, when no root holds it.
fn find_root<'r>(
    roots: &'r [Root],
    names: &[String],
    dir: bool,
    path: &str,
) -> Result<(usize, &'r Root, String), Failure> {
    let mut found: Option<(usize, &Root, usize)> = None;
    for (index, root) in roots.iter().enumerate() {
        if root.path.is_empty() {
            continue;
        }
        let held: Vec<&str> = root
            .path
            .split('/')
            .filter(|name| !name.is_empty())
            .collect();
        let holds = names.len() >= held.len()
            && names
                .iter()
                .zip(&held)
                .all(|(name, root_name)| name == root_name);
        if holds && found.is_none_or(|(_, _, longest)| held.len() > longest) {
            found = Some((index, root, held.len()));
        }
    }
    let Some((index, root, depth)) = found else {
        return Err(Failure::NotGranted(path.to_owned()));
    };
    let beneath = names[depth..].join("/");
    let rel = if beneath.is_empty() {
        ".".to_owned()
    } else if dir {
        format!("{beneath}/")
    } else {
        beneath
    };
    Ok((index, root, rel))
}

/// What a failed op beneath a handle at `handle_path` throws: leaving the
/// handle's directory, or its directory now reached through a symlink, is
/// `outside_root` about `requested`; anything else is the kind the OS
/// said, about `target`, the absolute path.
fn op_failure(error: sys::OpError, requested: &str, handle_path: &str, target: &str) -> Failure {
    match error {
        sys::OpError::Handle(error)
            if sys::is_escape(&error) || sys::is_symlink_refused(&error) =>
        {
            Failure::outside(
                requested,
                format!("{handle_path} is reached through a symlink now: {error}"),
            )
        }
        sys::OpError::Target(error) if sys::is_escape(&error) => Failure::outside(
            requested,
            format!("{requested} resolves outside {handle_path}: {error}"),
        ),
        sys::OpError::Handle(error) | sys::OpError::Target(error) => {
            Failure::of(kind_of(&error), target, &error)
        }
    }
}

/// The `FsError` kind of `error`, as the Node module maps an OS code.
fn kind_of(error: &std::io::Error) -> &'static str {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => "not_found",
        ErrorKind::AlreadyExists => "already_exists",
        ErrorKind::PermissionDenied => "permission_denied",
        ErrorKind::NotADirectory => "not_a_directory",
        ErrorKind::IsADirectory => "is_a_directory",
        ErrorKind::DirectoryNotEmpty => "not_empty",
        _ => "io_error",
    }
}

/// `info` as `stat` returns it: a null-prototype record of the entry's
/// `kind`, `size`, `mtime_ms`, and `mode`, its own to change, as the Node
/// module's plain object is; null-prototype, so a promise resolved with it
/// finds no `then` of the realm's.
fn file_info<'s>(
    scope: &v8::PinScope<'s, '_>,
    info: &sys::Info,
) -> Option<v8::Local<'s, v8::Value>> {
    let keys = [
        name(scope, "kind")?,
        name(scope, "size")?,
        name(scope, "mtime_ms")?,
        name(scope, "mode")?,
    ];
    let values = [
        string(scope, info.kind)?.into(),
        v8::Number::new(scope, info.size).into(),
        v8::Number::new(scope, info.mtime_ms).into(),
        v8::Integer::new_from_unsigned(scope, info.mode).into(),
    ];
    let null = v8::null(scope).into();
    Some(v8::Object::with_prototype_and_properties(scope, null, &keys, &values).into())
}

/// The calls each op makes, over owned arguments: a root's descriptor,
/// shared by count, and the paths as Rust text, so none borrows V8's heap
/// and any can move to another thread.
mod sys {
    use std::io;

    /// What `stat` reported of an entry.
    pub struct Info {
        /// `file`, `directory`, `symlink`, or `other`.
        pub kind: &'static str,
        /// Its size in bytes.
        pub size: f64,
        /// Its modification time in milliseconds since the epoch, as Node's
        /// `mtimeMs`.
        pub mtime_ms: f64,
        /// Its permission bits, without the file type's.
        pub mode: u32,
    }

    /// An entry of a listed directory.
    pub struct Listed {
        /// Its name, the kernel's bytes.
        pub name: std::ffi::OsString,
        /// `file`, `directory`, `symlink`, or `other`, a symlink never
        /// followed.
        pub kind: &'static str,
    }

    /// An op's failure: opening the handle's directory beneath its root, or
    /// resolving the target beneath that.
    pub enum OpError {
        /// The handle's directory's open failed.
        Handle(io::Error),
        /// The target's resolution failed.
        Target(io::Error),
    }

    /// What [`open_in_root`] opens a path as.
    #[derive(Clone, Copy)]
    pub enum Opened {
        /// A directory, anything else failing with `ENOTDIR`.
        Directory,
        /// An entry of any kind.
        Entry,
    }

    #[cfg(target_os = "linux")]
    pub use linux::*;

    #[cfg(target_os = "linux")]
    #[expect(
        clippy::needless_pass_by_value,
        reason = "each op owns what it's given, so it can move to another thread as it is"
    )]
    mod linux {
        use std::ffi::OsString;
        use std::io;
        use std::os::fd::{AsFd as _, OwnedFd};
        use std::path::Path;
        use std::sync::Arc;

        use gro_sys::beneath::{self, FileKind, FinalLink, Links, OpenAs};

        use super::{Info, Listed, OpError, Opened};

        /// That the kernel serves `openat2`.
        pub fn check() -> io::Result<()> {
            beneath::check_openat2()
        }

        /// A root's descriptor, opened by its real path.
        pub fn open_root(dir: &Path) -> io::Result<OwnedFd> {
            beneath::open_root(dir)
        }

        /// Whether `error` is a path leaving the directory it resolved
        /// beneath.
        pub fn is_escape(error: &io::Error) -> bool {
            beneath::is_escape(error)
        }

        /// Whether `error` is a symlink refused on a path resolved with
        /// none allowed.
        pub fn is_symlink_refused(error: &io::Error) -> bool {
            error.raw_os_error() == Some(libc::ELOOP)
        }

        /// The handle's directory at `beneath` its root, opened, or none
        /// for the root's own: its path is real, so a symlink on it, one
        /// swapped in since, is refused.
        fn handle_dir(root: &OwnedFd, beneath: &str) -> Result<Option<OwnedFd>, OpError> {
            if beneath.is_empty() {
                return Ok(None);
            }
            beneath::open(
                root.as_fd(),
                Path::new(beneath),
                OpenAs::Directory,
                Links::Refused,
            )
            .map(Some)
            .map_err(OpError::Handle)
        }

        /// The entry at `rel` beneath the handle's directory, at `beneath`
        /// the root, its final symlink followed when `follow` says.
        pub fn stat(
            root: Arc<OwnedFd>,
            beneath: String,
            rel: String,
            follow: bool,
        ) -> Result<Info, OpError> {
            let held = handle_dir(&root, &beneath)?;
            let dir = held
                .as_ref()
                .map_or_else(|| root.as_fd(), |held| held.as_fd());
            let final_link = if follow {
                FinalLink::Follow
            } else {
                FinalLink::Keep
            };
            let stat = beneath::stat(dir, Path::new(&rel), final_link).map_err(OpError::Target)?;
            #[expect(
                clippy::cast_precision_loss,
                reason = "Node's `size` is the same double"
            )]
            let size = stat.size as f64;
            Ok(Info {
                kind: kind_name(stat.kind),
                size,
                mtime_ms: stat.mtime_ms(),
                mode: stat.mode,
            })
        }

        /// The name of an entry's kind.
        const fn kind_name(kind: FileKind) -> &'static str {
            match kind {
                FileKind::File => "file",
                FileKind::Directory => "directory",
                FileKind::Symlink => "symlink",
                FileKind::Other => "other",
            }
        }

        /// The bytes of the regular file at `rel` beneath the handle's
        /// directory, at `beneath` the root, read to its end; anything else
        /// is refused before a byte is read.
        pub fn read(root: Arc<OwnedFd>, beneath: String, rel: String) -> Result<Vec<u8>, OpError> {
            let held = handle_dir(&root, &beneath)?;
            let dir = held
                .as_ref()
                .map_or_else(|| root.as_fd(), |held| held.as_fd());
            beneath::read(dir, Path::new(&rel)).map_err(OpError::Target)
        }

        /// The target of the symlink at `rel` beneath the handle's
        /// directory, at `beneath` the root, as stored; a final symlink
        /// followed when `follow` says, which leaves no symlink to read.
        pub fn read_link(
            root: Arc<OwnedFd>,
            beneath: String,
            rel: String,
            follow: bool,
        ) -> Result<OsString, OpError> {
            let held = handle_dir(&root, &beneath)?;
            let dir = held
                .as_ref()
                .map_or_else(|| root.as_fd(), |held| held.as_fd());
            let final_link = if follow {
                FinalLink::Follow
            } else {
                FinalLink::Keep
            };
            beneath::read_link(dir, Path::new(&rel), final_link).map_err(OpError::Target)
        }

        /// The entries of the directory at `rel` beneath the handle's
        /// directory, at `beneath` the root, a final symlink followed, in
        /// the kernel's order: each one's kind as the directory records it,
        /// or, where it records none, as its own metadata says, a symlink
        /// not followed. The directory's descriptor is closed before this
        /// returns, whatever it returns.
        pub fn list(
            root: Arc<OwnedFd>,
            beneath: String,
            rel: String,
        ) -> Result<Vec<Listed>, OpError> {
            let held = handle_dir(&root, &beneath)?;
            let dir = held
                .as_ref()
                .map_or_else(|| root.as_fd(), |held| held.as_fd());
            let entries = beneath::read_dir(dir, Path::new(&rel)).map_err(OpError::Target)?;
            let mut listed = Vec::with_capacity(entries.len());
            for entry in entries {
                let kind = if let Some(kind) = entry.kind {
                    kind
                } else {
                    let named = Path::new(&rel).join(&entry.name);
                    beneath::stat(dir, &named, FinalLink::Keep)
                        .map_err(OpError::Target)?
                        .kind
                };
                listed.push(Listed {
                    name: entry.name,
                    kind: kind_name(kind),
                });
            }
            Ok(listed)
        }

        /// The real path of the directory at `rel` beneath the handle's
        /// directory, at `beneath` the root.
        pub fn open_dir(
            root: Arc<OwnedFd>,
            beneath: String,
            rel: String,
        ) -> Result<OsString, OpError> {
            let held = handle_dir(&root, &beneath)?;
            let dir = held
                .as_ref()
                .map_or_else(|| root.as_fd(), |held| held.as_fd());
            let opened = beneath::open(dir, Path::new(&rel), OpenAs::Directory, Links::Beneath)
                .map_err(OpError::Target)?;
            beneath::real_path(opened.as_fd()).map_err(OpError::Target)
        }

        /// The real path of what `rel` names beneath `root`, opened as
        /// `opened` says.
        pub fn open_in_root(
            root: Arc<OwnedFd>,
            rel: String,
            opened: Opened,
        ) -> io::Result<OsString> {
            let how = match opened {
                Opened::Directory => OpenAs::Directory,
                Opened::Entry => OpenAs::Path,
            };
            let fd = beneath::open(root.as_fd(), Path::new(&rel), how, Links::Beneath)?;
            beneath::real_path(fd.as_fd())
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub use other::*;

    /// Every call fails where `gro_sys` isn't built: fs on fgro takes
    /// Linux.
    #[cfg(not(target_os = "linux"))]
    mod other {
        use std::ffi::OsString;
        use std::io;
        use std::os::fd::OwnedFd;
        use std::path::Path;
        use std::sync::Arc;

        use super::{Info, Listed, OpError, Opened};

        fn unsupported() -> io::Error {
            io::Error::new(io::ErrorKind::Unsupported, "fs on fgro takes Linux")
        }

        pub fn check() -> io::Result<()> {
            Err(unsupported())
        }

        pub fn open_root(_dir: &Path) -> io::Result<OwnedFd> {
            Err(unsupported())
        }

        pub const fn is_escape(_error: &io::Error) -> bool {
            false
        }

        pub const fn is_symlink_refused(_error: &io::Error) -> bool {
            false
        }

        pub fn stat(
            _root: Arc<OwnedFd>,
            _beneath: String,
            _rel: String,
            _follow: bool,
        ) -> Result<Info, OpError> {
            Err(OpError::Target(unsupported()))
        }

        pub fn open_dir(
            _root: Arc<OwnedFd>,
            _beneath: String,
            _rel: String,
        ) -> Result<OsString, OpError> {
            Err(OpError::Target(unsupported()))
        }

        pub fn read(
            _root: Arc<OwnedFd>,
            _beneath: String,
            _rel: String,
        ) -> Result<Vec<u8>, OpError> {
            Err(OpError::Target(unsupported()))
        }

        pub fn read_link(
            _root: Arc<OwnedFd>,
            _beneath: String,
            _rel: String,
            _follow: bool,
        ) -> Result<OsString, OpError> {
            Err(OpError::Target(unsupported()))
        }

        pub fn list(
            _root: Arc<OwnedFd>,
            _beneath: String,
            _rel: String,
        ) -> Result<Vec<Listed>, OpError> {
            Err(OpError::Target(unsupported()))
        }

        pub fn open_in_root(
            _root: Arc<OwnedFd>,
            _rel: String,
            _opened: Opened,
        ) -> io::Result<OsString> {
            Err(unsupported())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Failure, Root, beneath_of, kind_of, op_failure, sys, to_lexical, walk};

    /// What `failure` is, as data: its kind of error, and an `FsError`'s
    /// kind and path, a `TypeError`'s message.
    fn shown(failure: Failure) -> (String, String) {
        match failure {
            Failure::Type(message) => ("TypeError".to_owned(), message),
            Failure::Fs { kind, path, .. } => (kind.to_owned(), path),
            Failure::NotGranted(path) => ("not_granted".to_owned(), path),
            Failure::Plain(message) => ("Error".to_owned(), message),
            Failure::Thrown => ("thrown".to_owned(), String::new()),
        }
    }

    /// A root at `path`, never opened.
    fn root(path: &str) -> Root {
        Root {
            path: path.to_owned(),
            fd: Err("not opened".to_owned()),
        }
    }

    /// A path's `..` resolves by its text, as the Node module's
    /// `to_lexical_path` resolves it: a climb above the handle, or an
    /// absolute path, is `outside_root` naming the path as requested; an
    /// emptied path is the handle's own directory, `.`; a trailing `/`,
    /// `/.`, or `/..` asks for a directory, but not of the handle's own.
    #[test]
    fn a_path_resolves_by_its_text_beneath_its_handle() {
        let rel = |requested: &str| {
            to_lexical(requested, "/h")
                .map(|lexical| (lexical.rel(), lexical.path_beneath("/h")))
                .map_err(shown)
        };
        for (requested, expected) in [
            ("a", ("a", "/h/a")),
            ("a/b", ("a/b", "/h/a/b")),
            ("", (".", "/h")),
            (".", (".", "/h")),
            ("a/..", (".", "/h")),
            ("./a//b/.", ("a/b/", "/h/a/b")),
            ("a/", ("a/", "/h/a")),
            ("a/b/..", ("a/", "/h/a")),
            ("a/../b", ("b", "/h/b")),
        ] {
            assert_eq!(
                rel(requested),
                Ok((expected.0.to_owned(), expected.1.to_owned())),
                "{requested}"
            );
        }
        for requested in ["..", "../h/a", "a/../..", "/h/a", "/"] {
            assert_eq!(
                rel(requested),
                Err(("outside_root".to_owned(), requested.to_owned())),
                "{requested}"
            );
        }
        assert_eq!(
            to_lexical("a", "/")
                .map(|lexical| lexical.path_beneath("/"))
                .ok(),
            Some("/a".to_owned())
        );
    }

    /// An absolute path walks as Node's `realpath` walks it, over a tree of
    /// roots `/p` and `/p/inner` where `link` is a symlink to `a/b`, `f` a
    /// file, `missing` nothing, and `out` a symlink out of its root: a `..`
    /// follows the symlink before it and needs a directory there, a `..`
    /// above a root is taken off the text and can come back into one, the
    /// longest root that holds a step's names opens it, and the kernel is
    /// never handed a `..`.
    #[test]
    fn a_path_walks_its_dot_dots_after_the_symlinks_before_them() {
        let roots = [root("/p"), root("/p/inner"), root("")];
        let walked = |path: &str, opened: sys::Opened| {
            let mut opens = Vec::new();
            let result = walk(&roots, path, "fs.open_dir", opened, |root, rel, step| {
                opens.push(format!("{}:{rel}", root.path));
                let names: Vec<&str> = rel.split('/').filter(|name| !name.is_empty()).collect();
                assert!(!names.contains(&".."), "{path} handed the kernel {rel}");
                let mut real: Vec<&str> = root
                    .path
                    .split('/')
                    .filter(|name| !name.is_empty())
                    .collect();
                for (at, name) in names.iter().enumerate() {
                    let last = at + 1 == names.len();
                    match *name {
                        "." => {}
                        "link" => real.extend(["a", "b"]),
                        "missing" => {
                            return Err(Failure::Fs {
                                kind: "not_found",
                                path: path.to_owned(),
                                cause: None,
                            });
                        }
                        "f" if !last
                            || rel.ends_with('/')
                            || matches!(step, sys::Opened::Directory) =>
                        {
                            return Err(Failure::Fs {
                                kind: "not_a_directory",
                                path: path.to_owned(),
                                cause: None,
                            });
                        }
                        "out" => return Err(Failure::NotGranted(path.to_owned())),
                        name => real.push(name),
                    }
                }
                let real = format!("/{}", real.join("/"));
                Ok(beneath_of(&root.path, real.into()).expect("the fake stays beneath its root"))
            });
            (
                result
                    .map(|(index, _, beneath)| (index, beneath))
                    .map_err(shown),
                opens,
            )
        };
        for (path, expected, opens) in [
            ("/p", (0, ""), &["/p:."][..]),
            ("/p/", (0, ""), &["/p:."]),
            ("/p/a/b", (0, "a/b"), &["/p:a/b"]),
            ("/p/a/./b/", (0, "a/b"), &["/p:a/b/"]),
            ("/p/a/..", (0, ""), &["/p:a", "/p:."]),
            ("/p/link/..", (0, "a"), &["/p:link", "/p:a"]),
            ("/p/link/../c/", (0, "a/c"), &["/p:link", "/p:a/c/"]),
            ("/p/a/b/../../c", (0, "c"), &["/p:a/b", "/p:c"]),
            ("/p/a/../link/..", (0, "a"), &["/p:a", "/p:link", "/p:a"]),
            ("/p/a/../../p/x", (0, "x"), &["/p:a", "/p:x"]),
            ("/p/inner/x", (1, "x"), &["/p/inner:x"]),
            ("/p/inner", (1, ""), &["/p/inner:."]),
            ("/p/inner/../x", (0, "x"), &["/p/inner:.", "/p:x"]),
            ("/../p/a", (0, "a"), &["/p:a"]),
            ("/p/f", (0, "f"), &["/p:f"]),
        ] {
            assert_eq!(
                walked(path, sys::Opened::Entry),
                (
                    Ok((expected.0, expected.1.to_owned())),
                    opens.iter().map(|open| (*open).to_owned()).collect()
                ),
                "{path}"
            );
        }
        for (path, kind) in [
            ("/", "not_granted"),
            ("/q", "not_granted"),
            ("/pp", "not_granted"),
            ("/pinner", "not_granted"),
            ("/p/..", "not_granted"),
            ("/q/../p/a", "not_granted"),
            ("/p/a/../../q", "not_granted"),
            ("/p/out/..", "not_granted"),
            ("/p/missing/..", "not_found"),
            ("/p/f/..", "not_a_directory"),
            ("/p/f/", "not_a_directory"),
        ] {
            assert_eq!(
                walked(path, sys::Opened::Entry).0,
                Err((kind.to_owned(), path.to_owned())),
                "{path}"
            );
        }
        // a step whose names no root holds is refused before anything opens
        assert_eq!(
            walked("/q/../p/a", sys::Opened::Entry).1,
            Vec::<String>::new()
        );
        assert_eq!(
            walked("/p/f", sys::Opened::Directory).0,
            Err(("not_a_directory".to_owned(), "/p/f".to_owned()))
        );
        assert_eq!(
            walked("p/a", sys::Opened::Entry).0,
            Err((
                "TypeError".to_owned(),
                "fs.open_dir takes an absolute path: p/a".to_owned()
            ))
        );
    }

    /// A real path is beneath its root's when it extends the root's own
    /// path, and none otherwise: a root moved since it opened, or a path
    /// that isn't UTF-8.
    #[test]
    fn a_real_path_is_named_beneath_its_root() {
        use std::os::unix::ffi::OsStringExt as _;
        assert_eq!(beneath_of("/p", "/p".into()), Ok(String::new()));
        assert_eq!(beneath_of("/p", "/p/a/b".into()), Ok("a/b".to_owned()));
        assert_eq!(beneath_of("/", "/a".into()), Ok("a".to_owned()));
        // a path outside the root is no root's to name
        for outside in [&b"/pq/a"[..], b"/moved/a", b"/moved/\xff"] {
            let why = beneath_of("/p", std::ffi::OsString::from_vec(outside.to_vec())).unwrap_err();
            assert!(!why.contains("/moved") && !why.contains("/pq"), "{why}");
        }
        assert!(beneath_of("/p", std::ffi::OsString::from_vec(b"/p/\xff".to_vec())).is_err());
    }

    /// A failure whose text is longer than V8's longest string still throws,
    /// a `RangeError` as the language throws one, where V8 would make no
    /// string and throw nothing, and the call would end with no exception
    /// pending, its promise fulfilled: an `FsError`'s cause, and a
    /// `TypeError`'s or an `Error`'s message. Each text is a zeroed
    /// allocation never written, so it costs no memory to speak of.
    #[test]
    fn a_failure_longer_than_a_string_still_throws() {
        use deno_core::{JsRuntime, RuntimeOptions, v8};

        use super::super::error_class::{DeclaredFamily, ErrorClasses, FS_ERROR};
        use super::throw;

        // longer than V8's longest string
        let long = || String::from_utf8(vec![0; v8::String::MAX_LENGTH + 1]).unwrap();
        let mut runtime = JsRuntime::new(RuntimeOptions::default());
        let state = runtime.op_state();
        state.borrow_mut().put(ErrorClasses::default());
        let state = state.borrow();
        deno_core::scope!(scope, &mut runtime);
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "Error").unwrap();
        let error = global.get(scope, key.into()).unwrap();
        v8::tc_scope!(let tc, scope);
        state.borrow::<ErrorClasses>().hand_over(tc, error);
        let failures = [
            (
                "an FsError's cause",
                Failure::Fs {
                    kind: "io_error",
                    path: "/p".to_owned(),
                    cause: Some(long()),
                },
            ),
            ("a TypeError's message", Failure::Type(long())),
            ("an Error's message", Failure::Plain(long())),
        ];
        for (what, failure) in failures {
            throw(
                &state,
                tc,
                Some(DeclaredFamily::for_test(&FS_ERROR)),
                failure,
            );
            let thrown = tc.exception().map(|thrown| thrown.to_rust_string_lossy(tc));
            assert_eq!(
                thrown.as_deref(),
                Some("RangeError: Invalid string length"),
                "{what}"
            );
            tc.reset();
        }
    }

    /// An op's failure: leaving the handle, or the handle's own directory
    /// reached through a symlink, is `outside_root` naming the path as
    /// requested; any other error is its kind, naming the absolute path.
    #[test]
    fn an_op_s_failure_is_named_as_the_node_module_names_it() {
        let os = |code| std::io::Error::from_raw_os_error(code);
        let failure = |error| shown(op_failure(error, "x/../y", "/h", "/h/y"));
        let outside = ("outside_root".to_owned(), "x/../y".to_owned());
        assert_eq!(failure(sys::OpError::Target(os(libc::EXDEV))), outside);
        assert_eq!(failure(sys::OpError::Handle(os(libc::EXDEV))), outside);
        assert_eq!(failure(sys::OpError::Handle(os(libc::ELOOP))), outside);
        assert_eq!(
            failure(sys::OpError::Target(os(libc::ELOOP))),
            ("io_error".to_owned(), "/h/y".to_owned())
        );
        assert_eq!(
            failure(sys::OpError::Handle(os(libc::ENOTDIR))),
            ("not_a_directory".to_owned(), "/h/y".to_owned())
        );
        for (code, kind) in [
            (libc::ENOENT, "not_found"),
            (libc::EEXIST, "already_exists"),
            (libc::EACCES, "permission_denied"),
            (libc::EPERM, "permission_denied"),
            (libc::ENOTDIR, "not_a_directory"),
            (libc::EISDIR, "is_a_directory"),
            (libc::ENOTEMPTY, "not_empty"),
            (libc::EAGAIN, "io_error"),
            (libc::ENAMETOOLONG, "io_error"),
            (libc::EXDEV, "io_error"),
        ] {
            assert_eq!(kind_of(&os(code)), kind, "{code}");
        }
        let unsupported = std::io::Error::new(std::io::ErrorKind::Unsupported, "no openat2");
        assert_eq!(kind_of(&unsupported), "io_error");
    }
}
