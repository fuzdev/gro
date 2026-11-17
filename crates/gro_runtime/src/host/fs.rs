//! The native half of the fs host module: the run's roots, the
//! directories its handles name beneath them, and every check of a call.
//!
//! Its children's docs own their parts: the run's roots, their scopes and
//! nesting, and the directory each grant names ([`roots`]); the walk of an
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
//! the write; nothing widens it, and no op checks the run's roots. An op a
//! handle's scope lacks is `not_granted`, a write op on a handle that may
//! write a plain `Error` until the ops that write are built. Its
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

use super::error_class::{DeclaredFamily, ErrorClasses};
use super::handle::{self, HandleKind};
use super::{HostData, NO_ERROR_CLASSES, name, string, throw_error, throw_type_error};
use crate::loader::RefusalLog;
use crate::resolver::Capability;
use crate::{Authority, NeededGrant, RefusedBy};
use roots::Root;
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

/// The kind of the error a reach outside the run's roots throws.
const NOT_GRANTED: &str = "not_granted";

/// The keys of a handle's state.
const ROOT: &str = "root";
const BENEATH: &str = "beneath";
const READ: &str = "read";
const WRITE: &str = "write";

/// The run's fs state, put in its `OpState` when it starts, empty: its
/// roots join when the fs namespace is first made.
#[derive(Debug, Default)]
pub struct FsState {
    /// The run's roots, each opened once, by its place in the table, made
    /// whole when the fs namespace is first made. It's only appended to, so
    /// a handle's place stays its root's, and a root made later (a
    /// temporary directory's) joins at the end.
    roots: RefCell<Option<Vec<Root>>>,
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
        kind: &'static str,
        path: String,
        cause: Option<String>,
    },
    /// `not_granted` about `path`, recorded as it's thrown with the grant
    /// that would have allowed it, if one would.
    NotGranted {
        /// The path refused.
        path: String,
        /// The grant that would have allowed it.
        needed: Option<NeededGrant>,
    },
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

    /// `not_granted` about `path`, which no root the call may use reaches:
    /// none holds it by its text, or it leaves each that does through a
    /// symlink, so a grant to read lets the call through.
    fn unreached(path: &str) -> Self {
        Self::NotGranted {
            path: path.to_owned(),
            needed: Some(NeededGrant::Read),
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
        Failure::NotGranted { path, needed } => {
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
                needed,
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
fn run_roots(state: &OpState) -> Result<Ref<'_, [Root]>, Failure> {
    let fs = run_state(state)?;
    let data = state
        .try_borrow::<HostData>()
        .ok_or_else(|| Failure::Plain(NO_FS_STATE.to_owned()))?;
    Ok(fs.roots(data, state.try_borrow::<Authority>()))
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

/// The read on `requested` beneath `handle`, of the run's `roots`: its
/// path resolved by its text ([`to_lexical`]), `outside_root` for a climb
/// above the handle; then `not_granted` when the handle doesn't read,
/// before anything is looked up, recorded with what would let it through
/// ([`scope_needed`]); and `io_error` when the handle's root isn't open.
fn beneath(roots: &[Root], handle: Handle, requested: String) -> Result<Beneath, Failure> {
    let (root, handle_path) = handle_root(roots, &handle)?;
    let lexical = to_lexical(&requested, &handle_path)?;
    let target = lexical.path_beneath(&handle_path);
    if !handle.scope.read {
        let needed = scope_needed(
            roots,
            std::slice::from_ref(&target),
            Scope::READ,
            NeededGrant::Read,
        );
        return Err(Failure::NotGranted {
            path: target,
            needed: Some(needed),
        });
    }
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

#[cfg(test)]
mod tests {
    use super::{Failure, beneath_of, kind_of, op_failure, sys, to_lexical};

    /// What `failure` is, as data: its kind of error, and an `FsError`'s
    /// kind and path, a `TypeError`'s message.
    pub(super) fn shown(failure: Failure) -> (String, String) {
        match failure {
            Failure::Type(message) => ("TypeError".to_owned(), message),
            Failure::Fs { kind, path, .. } => (kind.to_owned(), path),
            Failure::NotGranted { path, .. } => ("not_granted".to_owned(), path),
            Failure::Plain(message) => ("Error".to_owned(), message),
            Failure::Thrown => ("thrown".to_owned(), String::new()),
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
