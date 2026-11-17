//! The native half of the fs host module: the run's roots, the
//! directories its handles name beneath them, and every check of a call.
//!
//! Its children's docs own their parts: the run's roots, their scopes and
//! nesting, the directory each grant names, and the rule that no `remove`
//! or `rename` moves a root's directory ([`roots`]); the walk of an
//! absolute path from the roots, which `fs.open_dir` and `fs.real_path`
//! take, and the grant a refusal needs ([`walk`]); the members that make a
//! handle ([`dirs`]), read beneath one ([`reads`]), list a directory
//! ([`listing`]), and write ([`writes`]); and the calls each op makes
//! ([`sys`]). This module's doc owns what they share: a handle's state and
//! scope, the lookup of a path beneath a handle, the check that the kernel
//! serves `openat2`, how an argument is read and a failure thrown, and how
//! an op's I/O runs.
//!
//! Only a root holds a descriptor. A handle, an `FsReadDir` or an `FsDir`
//! (`handle.rs`), holds plain data under its brand: its root's place in
//! the table, its directory's real path beneath the root, and its scope,
//! whether it may read and whether it may write, which is its root's when
//! `fs.open_dir` or `fs.project` makes it, and narrows: `dir()` keeps its
//! receiver's, an `FsReadDir`'s dropping the write, and `readonly()` drops
//! the write; nothing widens it, and what an op may do is never read from
//! the run's roots, which a `remove` or a `rename` reads only to refuse a
//! root's directory ([`roots::Held`]). An op a handle's scope lacks is
//! `not_granted`. Its
//! directory's absolute path is its own `path`. So no handle owns a
//! descriptor, none needs closing, and a handle follows its path as the
//! Node module's does: a directory moved away isn't followed, and one moved
//! in is what the handle names.
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
//! Every member's first step past its grant's check is the check that the
//! kernel serves `openat2` ([`FsState::ready`]), but `list`'s and its
//! listing's methods', whose one I/O, the first `next()`, checks it, so a
//! kernel below Linux 5.6 fails every op alike, as a plain `Error`, never
//! only the ones that resolve a path of more than one name. A root that
//! couldn't be opened (its path isn't UTF-8, or it's gone) is held as why,
//! and every op through it fails with `io_error` naming that in its
//! `cause`: a value the namespace holds is made without throwing.
//!
//! An argument is read as the value it is, as `env`'s are: a path is a
//! string, never converted, and an options record's fields are its data
//! ([`read_option`]), so a getter or a proxy is a `TypeError` where the
//! Node module would run it. A failure's error is an `FsError` whose `kind` and `path` are what
//! the Node module's would be, its `message` exactly `kind: path`, and its
//! `cause`, when the OS said something, an `Error` whose message is what
//! the OS said, which no case compares.
//!
//! Each op makes its calls inline and returns, for an `async` member, a
//! promise the host already settled. Its I/O ([`sys`]) takes owned
//! arguments, a root's descriptor shared by count, the paths as Rust text,
//! a write's bytes copied, and the roots a `remove` or a `rename` is held
//! to, and borrows nothing of V8's, so it can move to a blocking pool
//! with no change to what it's given. A read's fill of its buffer is the
//! exception: the buffer is V8's, so its collector counts it, and isn't
//! `Send` (`gro_v8::PendingArrayBuffer`): the open, whose `Reading` is
//! `Send`, can move, and the read into the buffer stays on the isolate's
//! thread.

pub(super) mod dirs;
pub(super) mod listing;
pub(super) mod reads;
mod roots;
mod sys;
mod walk;
pub(super) mod writes;

use std::cell::{Cell, Ref, RefCell};
use std::ffi::OsString;
use std::os::fd::OwnedFd;
use std::sync::Arc;

use deno_core::{OpState, v8};
use gro_sys::beneath::FileKind;

use super::error_class::{DeclaredFamily, ErrorClasses};
use super::handle::{self, HandleKind};
use super::{HostData, NO_ERROR_CLASSES, name, string, throw_error, throw_type_error};
use crate::loader::RefusalLog;
use crate::loader::refused::{NOT_GRANTED, NotGranted};
use crate::{Authority, FsNeed};
use roots::{Root, Table, holds_a_root};
use walk::scope_needed;

/// The fs namespace's specifier.
const MODULE: &str = "@fuzdev/gro/fs.ts";

/// What a member throws in a runtime started without the run's fs state,
/// which only a runtime made outside a run's start is.
const NO_FS_STATE: &str = "this runtime was started without the fs host module's state";

/// What a method called on anything but a handle of its kind throws.
const NOT_A_HANDLE: &str = "fs's methods are called on an fs handle of their kind, not another \
                            object";

/// What a refused reach throws in a runtime started without the run's
/// record of refusals, which only a runtime made outside a run's start is.
const NO_REFUSAL_STATE: &str = "fs refused a path, but this runtime was started without the \
                                run's record of refusals, so the refusal can't fail the run";

/// The keys of a handle's state.
const ROOT: &str = "root";
const BENEATH: &str = "beneath";
const READ: &str = "read";
const WRITE: &str = "write";

/// The run's fs state, put in its `OpState` when it starts, empty: its
/// roots join when the fs namespace is first made.
#[derive(Debug, Default)]
pub struct FsState {
    /// The run's roots, each opened once, by its place in the table, and
    /// the project's view of them ([`Table`]), made whole when the fs
    /// namespace is first made. The table is only appended to, so a
    /// handle's place stays its root's.
    roots: RefCell<Option<Table>>,
    /// Whether the kernel was found to serve `openat2`. `gro_sys` holds a
    /// failed check for the process, so this holds the other answer for the
    /// run, sparing every op after the first its probe.
    served: Cell<bool>,
}

/// What a root or a handle may do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Scope {
    /// Whether it reads.
    read: bool,
    /// Whether it writes.
    write: bool,
}

impl Scope {
    /// Read only.
    const READ: Self = Self {
        read: true,
        write: false,
    };
    /// Written only.
    const WRITE: Self = Self {
        read: false,
        write: true,
    };
    /// Read and written.
    const ALL: Self = Self {
        read: true,
        write: true,
    };

    /// What either does.
    const fn join(self, other: Self) -> Self {
        Self {
            read: self.read || other.read,
            write: self.write || other.write,
        }
    }

    /// Whether it does anything `other` does.
    const fn meets(self, other: Self) -> bool {
        (self.read && other.read) || (self.write && other.write)
    }

    /// It, written by none.
    const fn read_only(self) -> Self {
        Self {
            read: self.read,
            write: false,
        }
    }
}

/// What an op does through a handle, which the handle's scope may lack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// It reads.
    Read,
    /// It writes.
    Write,
}

impl Access {
    /// The scope that does it.
    const fn scope(self) -> Scope {
        match self {
            Self::Read => Scope::READ,
            Self::Write => Scope::WRITE,
        }
    }

    /// The grant of it.
    const fn grant(self) -> FsNeed {
        match self {
            Self::Read => FsNeed::Read,
            Self::Write => FsNeed::Write,
        }
    }
}

impl FsState {
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

/// Why a call failed, before it's thrown.
enum Failure {
    /// A `TypeError` of this message: an argument or receiver that's none.
    Type(String),
    /// An `FsError` of `kind` about `path`, with the OS's account of it as
    /// its `cause` when there is one.
    Fs {
        kind: FsErrorKind,
        path: String,
        cause: Option<String>,
    },
    /// `not_granted` about `path`, recorded as it's thrown with the kind
    /// of grant that would have allowed it, if one would.
    NotGranted {
        /// The path refused.
        path: String,
        /// The kind of grant that would have allowed it.
        need: Option<FsNeed>,
    },
    /// A plain `Error` of this message: what isn't the call's to fix.
    Plain(String),
    /// What V8 threw, already pending.
    Thrown,
}

impl Failure {
    /// An `FsError` of `kind` about `path`, its cause `error`.
    fn of(kind: FsErrorKind, path: &str, error: &std::io::Error) -> Self {
        Self::Fs {
            kind,
            path: path.to_owned(),
            cause: Some(error.to_string()),
        }
    }

    /// `not_granted` about `path`, which no root the call may use reaches:
    /// none holds it by its text, or it leaves each that does through a
    /// symlink, so a grant to read lets the call through.
    fn unreached(path: &str) -> Self {
        Self::NotGranted {
            path: path.to_owned(),
            need: Some(FsNeed::Read),
        }
    }

    /// `outside_root` about `requested`, the path as given, `why` its cause.
    fn outside(requested: &str, why: String) -> Self {
        Self::Fs {
            kind: FsErrorKind::OutsideRoot,
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
        Failure::NotGranted { path, need } => {
            let Some(log) = state.try_borrow::<RefusalLog>() else {
                return throw_error(scope, NO_REFUSAL_STATE);
            };
            // recorded as it's thrown, so the run fails whatever the
            // program does with the error
            log.record_not_granted(NotGranted::Fs(need), &path);
            (FsErrorKind::NotGranted, path, None)
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
    classes.throw(scope, errors, kind.as_str(), target, cause);
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

/// The run's fs state, or a plain `Error` in a runtime made outside a
/// run's start.
fn run_state(state: &OpState) -> Result<&FsState, Failure> {
    match (
        state.try_borrow::<FsState>(),
        state.try_borrow::<HostData>(),
    ) {
        (Some(fs), Some(_)) => Ok(fs),
        _ => Err(Failure::Plain(NO_FS_STATE.to_owned())),
    }
}

/// The run's roots, opened now if they weren't ([`FsState::roots`]), or a
/// plain `Error` in a runtime made outside a run's start.
fn run_roots(state: &OpState) -> Result<Ref<'_, Table>, Failure> {
    let fs = run_state(state)?;
    let data = state
        .try_borrow::<HostData>()
        .ok_or_else(|| Failure::Plain(NO_FS_STATE.to_owned()))?;
    Ok(fs.roots(data, state.try_borrow::<Authority>()))
}

/// An op on a handle, the run's roots borrowed while it runs: each path
/// it was given resolves beneath the handle's directory ([`Op::path`]),
/// and an op that reads or writes them checks the handle's scope for it
/// ([`Op::check`]) before anything is looked up.
struct Op<'r> {
    /// The run's roots.
    table: Ref<'r, Table>,
    /// The handle.
    handle: Handle,
    /// The handle's directory's absolute path.
    handle_path: String,
}

/// A path an op was given, resolved beneath its handle.
struct End {
    /// The path as requested.
    requested: String,
    /// It, resolved by its text beneath the handle.
    lexical: Lexical,
    /// The absolute path it names, which its errors name.
    target: String,
}

impl<'r> Op<'r> {
    /// The op called on `receiver`, a handle of `kind` or a kind extending
    /// it, once the kernel is found to serve `openat2` ([`FsState::ready`]).
    fn new(
        state: &'r OpState,
        scope: &v8::PinScope<'_, '_>,
        kind: &HandleKind,
        receiver: v8::Local<'_, v8::Value>,
    ) -> Result<Self, Failure> {
        run_state(state)?.ready()?;
        let handle = read_handle(scope, kind, receiver)?;
        let table = run_roots(state)?;
        let root = table
            .roots()
            .get(handle.root)
            .ok_or_else(|| Failure::Plain("an fs handle names no root of the run".to_owned()))?;
        let handle_path = join(root.handle_base(), &handle.beneath);
        Ok(Self {
            table,
            handle,
            handle_path,
        })
    }

    /// `path` checked as a path ([`path_text`]), then resolved beneath the
    /// handle ([`Op::resolve`]).
    fn path(
        &self,
        scope: &v8::PinScope<'_, '_>,
        path: v8::Local<'_, v8::Value>,
    ) -> Result<End, Failure> {
        self.resolve(path_text(scope, path)?)
    }

    /// `requested` resolved by its text beneath the handle
    /// ([`to_lexical`]), `outside_root` for a climb above it.
    fn resolve(&self, requested: String) -> Result<End, Failure> {
        let lexical = to_lexical(&requested, &self.handle_path)?;
        let target = lexical.path_beneath(&self.handle_path);
        Ok(End {
            requested,
            lexical,
            target,
        })
    }

    /// That the handle's scope allows `access` to `ends`, the paths the op
    /// does it to, in the order it was given them: else `not_granted`
    /// naming the first, or the handle's own directory for none, before
    /// anything is looked up, recorded as it's thrown with what would let
    /// it through ([`scope_needed`]); but with nothing that would, when
    /// `ruled`, an op the rule for a root's directory refuses, and one of
    /// `ends` is or holds a root's directory by its text, which no grant
    /// lets through.
    fn check(&self, access: Access, ends: &[&End], ruled: bool) -> Result<(), Failure> {
        if self.handle.scope.meets(access.scope()) {
            return Ok(());
        }
        let targets: Vec<&str> = ends.iter().map(|end| end.target.as_str()).collect();
        let roots = self.table.roots();
        let need = if ruled && targets.iter().any(|path| holds_a_root(roots, path)) {
            None
        } else {
            Some(scope_needed(roots, self.table.project(), &targets, access))
        };
        Err(Failure::NotGranted {
            path: ends
                .first()
                .map_or_else(|| self.handle_path.clone(), |end| end.target.clone()),
            need,
        })
    }

    /// The handle's root.
    fn root(&self) -> &Root {
        &self.table.roots()[self.handle.root]
    }

    /// What the op's call on `end` takes, owned: the root's descriptor, the
    /// handle's directory's real path beneath it, and the path beneath the
    /// handle the kernel resolves; or `io_error` about `end` when the root
    /// isn't open.
    fn call(&self, end: &End) -> Result<(Arc<OwnedFd>, String, String), Failure> {
        Ok((
            root_fd(self.root(), &end.target)?,
            self.handle.beneath.clone(),
            end.lexical.rel(),
        ))
    }

    /// What the op's failure at `end`, `error`, throws ([`op_failure`]).
    fn failure(&self, end: &End, error: sys::OpError) -> Failure {
        op_failure(error, &end.requested, &self.handle_path, &end.target)
    }
}

/// A handle, as an op reads the one it's handed.
struct Handle {
    /// Its root's place in the table.
    root: usize,
    /// Its directory's real path beneath the root, `""` for the root's own.
    beneath: String,
    /// What it may do.
    scope: Scope,
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
    let access = Scope {
        read: field(READ)?.is_true(),
        write: field(WRITE)?.is_true(),
    };
    Ok(Handle {
        root: usize::try_from(root.value()).map_err(|_| not_a_handle())?,
        beneath,
        scope: access,
    })
}

/// Makes a handle of `kind` of `prototype` on the directory `beneath` the
/// root at `index`, whose path is `root_path`, doing what `access` says.
fn make_dir<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &'static HandleKind,
    prototype: v8::Local<'s, v8::Object>,
    index: usize,
    root_path: &str,
    beneath: &str,
    access: Scope,
) -> Option<v8::Local<'s, v8::Object>> {
    let index = u32::try_from(index).ok()?;
    let keys = [
        name(scope, ROOT)?,
        name(scope, BENEATH)?,
        name(scope, READ)?,
        name(scope, WRITE)?,
    ];
    let values = [
        v8::Integer::new_from_unsigned(scope, index).into(),
        string(scope, beneath)?.into(),
        v8::Boolean::new(scope, access.read).into(),
        v8::Boolean::new(scope, access.write).into(),
    ];
    let null = v8::null(scope).into();
    let held = v8::Object::with_prototype_and_properties(scope, null, &keys, &values);
    held.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    let path = string(scope, &join(root_path, beneath))?;
    handle::make(scope, kind, prototype, held.into(), &[path.into()])
}

/// `root`'s descriptor, or `io_error` about `path` naming why it isn't
/// open.
fn root_fd(root: &Root, path: &str) -> Result<Arc<OwnedFd>, Failure> {
    root.fd.clone().map_err(|why| Failure::Fs {
        kind: FsErrorKind::IoError,
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

/// `found`, the path beneath the root at `root_path` of an entry opened
/// beneath it, as the kernel names it ([`gro_sys::beneath::path_beneath`]),
/// as text, or why it's none: the kernel's path is outside the root's,
/// which only the entry or the root moved since it opened makes, a path no
/// root holds and so the reason doesn't name; or it isn't UTF-8.
fn beneath_of(root_path: &str, found: Option<OsString>) -> Result<String, String> {
    let beneath = found.ok_or_else(|| {
        format!("it left the root {root_path} while it was opened, or the root moved")
    })?;
    beneath.into_string().map_err(|beneath| {
        format!(
            "the real path {} isn't UTF-8",
            join(root_path, &beneath.to_string_lossy())
        )
    })
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

/// The value of `options`' `key`, as the Node module reads `options?.[key]`:
/// none for no options, a primitive, or a key on none of its prototype
/// chain's objects, else the value of the first own property of that key
/// on the chain; a proxy or an accessor on the way is a `TypeError`, since
/// an op runs no code of a caller's.
fn read_option<'s>(
    scope: &v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, Failure> {
    let Ok(mut object) = v8::Local::<v8::Object>::try_from(options) else {
        return Ok(None);
    };
    let plain = || {
        Failure::Type(format!(
            "fs options are plain data, read without running code: {key}"
        ))
    };
    let name_key = name(scope, key).ok_or(Failure::Thrown)?;
    let value_key = name(scope, "value").ok_or(Failure::Thrown)?;
    loop {
        if object.is_proxy() {
            return Err(plain());
        }
        let descriptor = object
            .get_own_property_descriptor(scope, name_key)
            .ok_or(Failure::Thrown)?;
        if let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) {
            // a descriptor V8 made is an ordinary object of its fields
            if !descriptor
                .has_own_property(scope, value_key)
                .ok_or(Failure::Thrown)?
            {
                return Err(plain());
            }
            return descriptor
                .get(scope, value_key.into())
                .map(Some)
                .ok_or(Failure::Thrown);
        }
        match object
            .get_prototype(scope)
            .and_then(|prototype| v8::Local::<v8::Object>::try_from(prototype).ok())
        {
            Some(prototype) => object = prototype,
            None => return Ok(None),
        }
    }
}

/// Whether `options`' `key` is truthy, as the Node module reads a flag
/// ([`read_option`]).
fn read_flag(
    scope: &v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Value>,
    key: &str,
) -> Result<bool, Failure> {
    Ok(read_option(scope, options, key)?.is_some_and(|value| value.boolean_value(scope)))
}

/// Whether `options` say to follow a final symlink, as the Node module
/// reads `options?.follow ?? true` ([`read_option`]).
fn read_follow(
    scope: &v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Value>,
) -> Result<bool, Failure> {
    Ok(read_option(scope, options, "follow")?
        .is_none_or(|follow| follow.is_null_or_undefined() || follow.boolean_value(scope)))
}

/// Whether an op's `error` is nothing at its path, the handle's directory
/// or the target.
fn is_missing(error: &sys::OpError) -> bool {
    matches!(error, sys::OpError::Handle(error) | sys::OpError::Target(error)
        if error.kind() == std::io::ErrorKind::NotFound)
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
        sys::OpError::Refused(roots::HoldsARoot) => holds_a_root_failure(target),
    }
}

/// `io_error` about `target`, which the rule for a root's directory
/// refused to remove or move ([`roots::Held`]): it is or holds a root's
/// directory.
fn holds_a_root_failure(target: &str) -> Failure {
    Failure::Fs {
        kind: FsErrorKind::IoError,
        path: target.to_owned(),
        cause: Some(format!("{target} is or holds a root of the run")),
    }
}

/// `io_error` about `from`, which the rule for a root's directory refused
/// to rename to `to` ([`roots::Held`]): the run's grants allow more there
/// than where it is.
fn gains_scope_failure(from: &str, to: &str) -> Failure {
    Failure::Fs {
        kind: FsErrorKind::IoError,
        path: from.to_owned(),
        cause: Some(format!(
            "{from} -> {to} would move it where the run's grants allow more than where it is"
        )),
    }
}

/// Declares [`FsErrorKind`] from one list of its variants, each with the
/// name an `FsError` carries, so `FsErrorKind::ALL` can't miss one.
macro_rules! fs_error_kinds {
    ($($(#[doc = $doc:literal])* $variant:ident => $name:expr,)*) => {
        /// The `kind` of an `FsError`, the closed set `fs_error.ts`'s
        /// `FsErrorKind` declares.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum FsErrorKind {
            $($(#[doc = $doc])* $variant,)*
        }

        impl FsErrorKind {
            /// Every kind, in the order declared.
            #[cfg(test)]
            const ALL: &[Self] = &[$(Self::$variant),*];

            /// The name an `FsError` carries as its `kind`, as the Node
            /// module names it.
            const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }
        }
    };
}

fs_error_kinds! {
    /// Nothing at the path.
    NotFound => "not_found",
    /// An entry already at the path.
    AlreadyExists => "already_exists",
    /// The OS refused the access.
    PermissionDenied => "permission_denied",
    /// A directory was needed and the entry is none.
    NotADirectory => "not_a_directory",
    /// The entry is a directory where one isn't allowed.
    IsADirectory => "is_a_directory",
    /// A directory to remove or replace holds entries.
    NotEmpty => "not_empty",
    /// A path that would leave its handle.
    OutsideRoot => "outside_root",
    /// A reach outside what the run granted.
    NotGranted => NOT_GRANTED,
    /// Every failure without a kind of its own.
    IoError => "io_error",
}

/// The name of an entry's kind, as the Node module's `FileKind` names it.
const fn file_kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::File => "file",
        FileKind::Directory => "directory",
        FileKind::Symlink => "symlink",
        FileKind::Other => "other",
    }
}

/// The `FsError` kind of `error`, as the Node module maps an OS code.
fn kind_of(error: &std::io::Error) -> FsErrorKind {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => FsErrorKind::NotFound,
        ErrorKind::AlreadyExists => FsErrorKind::AlreadyExists,
        ErrorKind::PermissionDenied => FsErrorKind::PermissionDenied,
        ErrorKind::NotADirectory => FsErrorKind::NotADirectory,
        ErrorKind::IsADirectory => FsErrorKind::IsADirectory,
        ErrorKind::DirectoryNotEmpty => FsErrorKind::NotEmpty,
        _ => FsErrorKind::IoError,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::roots::{Root, Table};
    use super::{
        Access, Failure, FsErrorKind, Handle, NOT_GRANTED, Op, Scope, beneath_of, kind_of,
        op_failure, sys, to_lexical,
    };

    /// What `failure` is, as data: its kind of error, and an `FsError`'s
    /// kind and path, a `TypeError`'s message.
    pub(super) fn shown(failure: Failure) -> (String, String) {
        match failure {
            Failure::Type(message) => ("TypeError".to_owned(), message),
            Failure::Fs { kind, path, .. } => (kind.as_str().to_owned(), path),
            Failure::NotGranted { path, .. } => (NOT_GRANTED.to_owned(), path),
            Failure::Plain(message) => ("Error".to_owned(), message),
            Failure::Thrown => ("thrown".to_owned(), String::new()),
        }
    }

    /// An op through a handle on a root that didn't open is `io_error`
    /// naming the path it was given, beneath the handle, never the
    /// handle's own directory.
    #[test]
    fn an_op_on_an_unopened_root_names_its_path() {
        let roots = RefCell::new(Table::of([Root::unopened("/r", &[], Scope::ALL)]));
        let op = Op {
            table: roots.borrow(),
            handle: Handle {
                root: 0,
                beneath: "h".to_owned(),
                scope: Scope::ALL,
            },
            handle_path: "/r/h".to_owned(),
        };
        let end = op.resolve("a/b".to_owned()).map_err(shown).unwrap();
        assert_eq!(
            op.call(&end).map(|_| ()).map_err(shown),
            Err(("io_error".to_owned(), "/r/h/a/b".to_owned()))
        );
    }

    /// An op checked for a scope its handle lacks is refused even with no
    /// path to name, naming the handle's own directory.
    #[test]
    fn a_check_without_paths_still_refuses_a_scope_the_handle_lacks() {
        let roots = RefCell::new(Table::of([Root::unopened("/r", &[], Scope::READ)]));
        let op = Op {
            table: roots.borrow(),
            handle: Handle {
                root: 0,
                beneath: "h".to_owned(),
                scope: Scope::READ,
            },
            handle_path: "/r/h".to_owned(),
        };
        assert_eq!(
            op.check(Access::Write, &[], false).map_err(shown),
            Err(("not_granted".to_owned(), "/r/h".to_owned()))
        );
        assert!(op.check(Access::Read, &[], false).is_ok());
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

    /// A path the kernel named beneath a root is its text, and none is no
    /// path beneath the root, its reason naming no path outside it; one
    /// that isn't UTF-8 is named by the root's path and its escapes.
    #[test]
    fn a_real_path_is_named_beneath_its_root() {
        use std::os::unix::ffi::OsStringExt as _;
        assert_eq!(beneath_of("/p", Some("".into())), Ok(String::new()));
        assert_eq!(beneath_of("/p", Some("a/b".into())), Ok("a/b".to_owned()));
        let why = beneath_of("/p", None).unwrap_err();
        assert!(why.contains("left the root /p"), "{why}");
        let why = beneath_of("/p", Some(std::ffi::OsString::from_vec(b"\xff".to_vec())));
        assert_eq!(why, Err("the real path /p/\u{fffd} isn't UTF-8".to_owned()));
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
                    kind: FsErrorKind::IoError,
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
            assert_eq!(kind_of(&os(code)).as_str(), kind, "{code}");
        }
        let unsupported = std::io::Error::new(std::io::ErrorKind::Unsupported, "no openat2");
        assert_eq!(kind_of(&unsupported).as_str(), "io_error");
    }

    /// The kinds an `FsError` of the host can carry are the ones the Node
    /// module's `FsErrorKind` declares in `fs_error.ts`, so neither side has
    /// a kind the other lacks.
    #[test]
    fn the_error_kinds_are_the_node_module_s() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/lib/fs_error.ts");
        let source = std::fs::read_to_string(&path).unwrap();
        let declared = source
            .split_once("export type FsErrorKind =")
            .and_then(|(_, rest)| rest.split_once(';'))
            .map(|(union, _)| union)
            .unwrap();
        let node: Vec<&str> = declared
            .split('|')
            .map(|member| member.trim().trim_matches('\''))
            .filter(|member| !member.is_empty())
            .collect();
        let host: Vec<&str> = FsErrorKind::ALL.iter().map(|kind| kind.as_str()).collect();
        let unique = |kinds: &[&str]| {
            kinds
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        };
        assert_eq!(unique(&node), node.len(), "{node:?}");
        assert_eq!(unique(&host), host.len(), "{host:?}");
        let (mut node, mut host) = (node, host);
        node.sort_unstable();
        host.sort_unstable();
        assert_eq!(host, node);
    }
}
