//! The native half of the fs host module: the run's roots, the
//! directories its handles name beneath them, and every check of a call.
//!
//! The roots are the directories the run reads or writes, each opened once,
//! as a path only (`O_PATH | O_DIRECTORY`), by its real path with no
//! symlink on it, the first time a run makes the fs namespace
//! ([`FsState`]): the project's root, the entry's, first, read and never
//! written but by a grant; then each directory the run's fs grant reads or
//! writes ([`FsGrant`]), one root per real path, its scopes joined; or,
//! unconfined, one more at `/`, read and written. A root's scope is what it
//! reads and writes, its own grant's joined with every root's whose real
//! path holds it, so a directory written inside the project is read too,
//! and write never implies read. A root matches a path by its real path and
//! by each path it was granted as, so a directory granted by a path through
//! a symlink is held by that path too, while every lookup resolves beneath
//! its descriptor. The table is only ever appended to, so a handle's place
//! in it stays its root's.
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
//! `fs.open_dir` and `fs.real_path` take an absolute path and walk it as
//! Node's `realpath` does, a `..` after the symlink before it ([`walk`]):
//! the names before a `..` open as a directory beneath the longest root
//! that holds them by their text, the `..` takes a name off that
//! directory's real path, and what's left walks on, so a lookup always
//! starts at a root, and a `..` above one is text. A step that leaves the
//! longest root by a symlink is opened beneath the next longest that holds
//! its names, if one does, so a root granted inside another takes nothing
//! from what the outer one reached. `fs.real_path` takes only the roots the
//! run reads, `fs.open_dir` every root. The entry's real path is always
//! beneath its root, checked by its text before it's looked up, beneath
//! the root: one that left the root while it was opened is `io_error`, its
//! cause naming no path outside. A path no root holds is `not_granted`,
//! refused whether or not anything is there, and so is one through a
//! symlink that leaves every root holding it by its target's text: an
//! absolute symlink, even to a path inside the root, or a relative one
//! whose target climbs above the root, even back into it, both of which
//! Node follows; but for a root at `/`, an unconfined run's or a flag's
//! naming `/`, which resolves an absolute path as the kernel does.
//! Such a refusal is recorded in the run's record of refusals as it's
//! thrown, with what would have allowed it ([`NeededGrant`]), which fails
//! the run whatever the program does with the error: a grant to read; and
//! for an op a handle's scope lacks, a handle `fs.open_dir` makes on a root
//! that grants it, since a grant never widens the handle that refused it,
//! or, with no such root, a grant of what the scope lacked. A need names
//! its kind, never a directory: a path a program chose, shown back, can
//! name another directory than the one the refusal needed, through a
//! symlink, and the refusal itself names the path.
//!
//! A whole-file read opens its target beneath the handle without waiting
//! for it, and `gro_sys` refuses anything but a regular file by the opened
//! descriptor before a byte is read: a directory is `is_a_directory`, as
//! the Node module's, and a FIFO, a socket, or a device `io_error`, where
//! the Node module's read would wait on a FIFO or read a device without
//! end. A regular file is read to its end, never trusting the size it
//! reports to end it, and one past `2 ** 31 - 1` bytes, or that reports
//! more, is `io_error`, a size past it refused before a byte is read. Its
//! bytes are read straight into a buffer V8 allocated of the size it
//! reported ([`read_bytes`]), so V8's collector counts them as it counts a
//! buffer JS allocates; only a file whose size was wrong is copied.
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
//! with no change to what it's given.

use std::cell::{Cell, Ref, RefCell};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use deno_core::{OpState, v8};
use gro_v8::PendingArrayBuffer;

use super::error_class::{DeclaredFamily, ErrorClasses};
use super::handle::{self, FS_DIR, FS_LISTING, FS_READ_DIR, HandleKind};
use super::{HostData, NO_ERROR_CLASSES, name, native, string, throw_error, throw_type_error};
use crate::loader::RefusalLog;
use crate::resolver::Capability;
use crate::{Authority, FsGrant, NeededGrant, RefusedBy};

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

/// A root of the run: a directory whose tree it reads or writes.
#[derive(Debug)]
struct Root {
    /// Its absolute real path, which the paths of its handles extend; lossy
    /// only when its path isn't UTF-8, when it isn't open, and empty for a
    /// runtime given no project, which no path matches.
    path: String,
    /// The other absolute paths it was granted as, each by its text with no
    /// `.`, `..`, or empty name, which match a path as its real path does.
    granted: Vec<String>,
    /// What it reads and writes: its grants', joined with each root's that
    /// holds its real path.
    scope: Scope,
    /// Whether it's `/`, which resolves an absolute symlink and a `..` as
    /// the kernel resolves a path (`gro_sys`'s `Links::InRoot`).
    whole: bool,
    /// Its descriptor, a path only, shared with any op under way, or why it
    /// isn't open, which every op through it reports.
    fd: Result<Arc<OwnedFd>, String>,
}

impl Root {
    /// Each absolute path it matches as, its real path first, as its names.
    fn spellings(&self) -> impl Iterator<Item = Vec<&str>> {
        std::iter::once(&self.path)
            .chain(&self.granted)
            .filter(|path| !path.is_empty())
            .map(|path| path.split('/').filter(|name| !name.is_empty()).collect())
    }
}

impl FsState {
    /// The run's roots, opened now if they weren't, from the run's data
    /// and authority ([`root_table`]).
    fn roots(&self, data: &HostData, authority: Option<&Authority>) -> Ref<'_, [Root]> {
        if self.roots.borrow().is_none() {
            let table = root_table(data.project.as_deref(), authority);
            *self.roots.borrow_mut() = Some(table);
        }
        Ref::map(self.roots.borrow(), |roots| roots.as_deref().unwrap_or(&[]))
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

/// The run's roots: the project's, `project`, a real path, read; then the
/// directories `authority`'s fs grant reads and writes ([`FsGrant`]), or,
/// unconfined, `/`, read and written. Each granted path's real path is
/// read now, and the paths of one real path are one root, the first's
/// place, its scopes joined and each path it was granted as kept; then
/// each root's scope joins every root's whose real path holds its own; then
/// each is opened by its real path ([`open_root`]).
fn root_table(project: Option<&Path>, authority: Option<&Authority>) -> Vec<Root> {
    let mut wanted: Vec<(Option<PathBuf>, Scope, Option<&Path>)> =
        vec![(project.map(Path::to_path_buf), Scope::READ, None)];
    match authority {
        Some(Authority::Unconfined) => wanted.push((Some(PathBuf::from("/")), Scope::ALL, None)),
        Some(Authority::Confined(grants)) => {
            if let Some(FsGrant { read, write }) = &grants.fs {
                for (dirs, scope) in [(read, Scope::READ), (write, Scope::WRITE)] {
                    for dir in dirs {
                        // read as real once, as the run opens it
                        wanted.push((dir.canonicalize().ok(), scope, Some(dir.as_path())));
                    }
                }
            }
        }
        None => {}
    }
    let mut table: Vec<(Option<PathBuf>, Scope, Vec<&Path>)> = Vec::new();
    for (real, scope, granted) in wanted {
        let merged = real.as_ref().and_then(|real| {
            table
                .iter_mut()
                .find(|(other, ..)| other.as_ref() == Some(real))
        });
        if let Some((_, joined, spellings)) = merged {
            *joined = joined.join(scope);
            spellings.extend(granted);
        } else {
            table.push((real, scope, granted.into_iter().collect()));
        }
    }
    let scopes: Vec<Scope> = table
        .iter()
        .map(|(real, scope, _)| {
            table
                .iter()
                .filter(|(outer, ..)| match (outer, real) {
                    (Some(outer), Some(real)) => real.starts_with(outer),
                    _ => false,
                })
                .fold(*scope, |joined, (_, outer, _)| joined.join(*outer))
        })
        .collect();
    table
        .into_iter()
        .zip(scopes)
        .enumerate()
        .map(|(index, ((real, _, granted), scope))| {
            open_root(index == 0, real.as_deref(), &granted, scope)
        })
        .collect()
}

/// The root at `real`, its real path, granted as `granted` and doing what
/// `scope` says, opened, or held as why it isn't: none given (no project,
/// or a granted path gone before its real path was read), or a path that
/// isn't UTF-8. `project` says it's the project's, which its reasons name.
fn open_root(project: bool, real: Option<&Path>, granted: &[&Path], scope: Scope) -> Root {
    let what = if project {
        "the project's directory".to_owned()
    } else {
        format!("the directory granted as {}", granted_text(granted))
    };
    let names: Vec<String> = granted
        .iter()
        .filter_map(|path| spelled(path))
        .filter(|path| real.and_then(Path::to_str) != Some(path))
        .fold(Vec::new(), |mut names, path| {
            if !names.contains(&path) {
                names.push(path);
            }
            names
        });
    let Some(dir) = real else {
        let fd = Err(if project {
            NO_PROJECT.to_owned()
        } else {
            format!("{what} is gone, or no longer a directory")
        });
        return Root {
            path: String::new(),
            granted: names,
            scope,
            whole: false,
            fd,
        };
    };
    let path = dir.to_string_lossy().into_owned();
    // the real path, unless it's the path the root was granted as
    let what = if !project && granted_text(granted) == path {
        what
    } else {
        format!("{what}, {path},")
    };
    let fd = if dir.to_str().is_none() {
        Err(format!("{what} isn't UTF-8, so no string names it"))
    } else {
        sys::open_root(dir)
            .map(Arc::new)
            .map_err(|error| format!("{what} can't be opened: {error}"))
    };
    Root {
        whole: path == "/",
        path,
        granted: names,
        scope,
        fd,
    }
}

/// The paths a root was granted as, for a reason to name it by.
fn granted_text(granted: &[&Path]) -> String {
    granted
        .first()
        .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
}

/// `path`, an absolute path as granted, as a root's spelling: its names
/// joined, none `.` or empty, or none when it holds a `..`, which the
/// kernel resolves after the symlink before it, so its text names no
/// directory, or isn't UTF-8.
fn spelled(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    let mut names = Vec::new();
    for name in text.split('/') {
        match name {
            "" | "." => {}
            ".." => return None,
            name => names.push(name),
        }
    }
    Some(format!("/{}", names.join("/")))
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
    // a runtime made outside a run's start has no root, which every op
    // through the handle reports
    let roots = run_roots(state).ok();
    let first = roots.as_ref().and_then(|roots| roots.first());
    let path = first.map_or_else(String::new, |root| root.path.clone());
    let access = first.map_or_else(Scope::default, |root| root.scope);
    make_dir(scope, &FS_DIR, prototype, 0, &path, "", access).map(Into::into)
}

/// `fs.open_dir(path)`: an `FsDir` on the directory at `path`, an absolute
/// path inside a root, of the root's scope, `not_granted` outside every
/// root, also when nothing is there, or when it leads out of its root by a
/// `..` or a symlink; `errors`, `FsError`'s family as the member declares
/// it.
pub fn open_dir<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        run_state(state)?.ready()?;
        let path = path_text(scope, path)?;
        let roots = run_roots(state)?;
        let (index, root, beneath) = reach(
            &roots,
            &path,
            "fs.open_dir",
            sys::Opened::Directory,
            Scope::ALL,
        )?;
        let prototype = native::prototype(state, scope, &FS_DIR).ok_or(Failure::Thrown)?;
        make_dir(
            scope, &FS_DIR, prototype, index, &root.path, &beneath, root.scope,
        )
        .map(Into::into)
        .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `fs.real_path(path)`: the real path of the entry at `path`, an absolute
/// path inside a root the run reads, every symlink followed beneath it,
/// always a path inside its root; refused as `open_dir` refuses one, a
/// root only written holding none, and `io_error` when the entry left its
/// root while it was opened.
pub fn real_path<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        run_state(state)?.ready()?;
        let path = path_text(scope, path)?;
        let roots = run_roots(state)?;
        let (_, root, beneath) = reach(
            &roots,
            &path,
            "fs.real_path",
            sys::Opened::Entry,
            Scope::READ,
        )?;
        string(scope, &join(&root.path, &beneath))
            .map(Into::into)
            .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `dir(path)` called on `receiver`, a handle of `kind`, the method's: a
/// handle of `kind` on the subdirectory at `path` beneath it, which must
/// exist, of the receiver's scope, writing only when `kind` writes too. A
/// handle that may neither read nor write makes one all the same, which
/// may do nothing either.
pub fn dir<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    kind: &'static HandleKind,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        run_state(state)?.ready()?;
        let handle = read_handle(scope, kind, receiver)?;
        let requested = path_text(scope, path)?;
        let roots = run_roots(state)?;
        let (root, handle_path) = handle_root(&roots, &handle)?;
        let lexical = to_lexical(&requested, &handle_path)?;
        let target = lexical.path_beneath(&handle_path);
        let fd = root_fd(root, &target)?;
        let real = sys::open_dir(fd, root.path.clone(), handle.beneath.clone(), lexical.rel())
            .map_err(|error| op_failure(error, &requested, &handle_path, &target))?;
        let beneath = beneath_of(&root.path, real).map_err(|why| {
            Failure::outside(
                &requested,
                format!("{requested} beneath {handle_path}: {why}"),
            )
        })?;
        let prototype = native::prototype(state, scope, kind).ok_or(Failure::Thrown)?;
        let access = if *kind == FS_DIR {
            handle.scope
        } else {
            handle.scope.read_only()
        };
        make_dir(
            scope,
            kind,
            prototype,
            handle.root,
            &root.path,
            &beneath,
            access,
        )
        .map(Into::into)
        .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `readonly()` called on `receiver`, an `FsDir`: an `FsReadDir` on the same
/// directory, which no op writes through, reading as the receiver reads.
pub fn readonly<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        run_state(state)?.ready()?;
        let handle = read_handle(scope, &FS_DIR, receiver)?;
        let roots = run_roots(state)?;
        let (root, _) = handle_root(&roots, &handle)?;
        let prototype = native::prototype(state, scope, &FS_READ_DIR).ok_or(Failure::Thrown)?;
        make_dir(
            scope,
            &FS_READ_DIR,
            prototype,
            handle.root,
            &root.path,
            &handle.beneath,
            handle.scope.read_only(),
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
    run_state(state)?.ready()?;
    let handle = read_handle(scope, &FS_READ_DIR, receiver)?;
    let requested = path_text(scope, path)?;
    let follow = match options {
        Some(options) => follow_option(scope, options)?,
        None => true,
    };
    let op = beneath(&run_roots(state)?, handle, requested)?;
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
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    contents: Contents,
    if_exists: bool,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        run_state(state)?.ready()?;
        let handle = read_handle(scope, &FS_READ_DIR, receiver)?;
        let requested = path_text(scope, path)?;
        let op = beneath(&run_roots(state)?, handle, requested)?;
        let (fd, dir, rel) = op.call();
        let reading = match sys::open_read(fd, dir, rel) {
            Ok(reading) => reading,
            Err(error) if if_exists && is_missing(&error) => return Ok(v8::null(scope).into()),
            Err(error) => return Err(op.failure(error)),
        };
        let read_failure = |error| op.failure(sys::OpError::Target(error));
        match contents {
            Contents::Bytes => {
                let bytes = read_bytes(scope, reading).map_err(read_failure)?;
                to_bytes(scope, bytes)
            }
            Contents::Text => {
                let bytes = reading.to_end().map_err(read_failure)?;
                v8::String::new_from_utf8(scope, &bytes, v8::NewStringType::Normal)
                    .map(Into::into)
                    .ok_or_else(|| Failure::Fs {
                        kind: "io_error",
                        path: op.target.clone(),
                        cause: Some(format!(
                            "{} holds {} bytes, more text than a string holds",
                            op.target,
                            bytes.len()
                        )),
                    })
            }
        }
    })();
    finish(state, scope, Some(errors), result)
}

/// The file `reading` opened, read to its end into a buffer V8 allocated
/// ([`PendingArrayBuffer`]), so its collector paces itself by every read's
/// bytes as by a buffer JS allocates, and frees what JS drops before more
/// piles up.
///
/// The buffer is allocated of the size the file reported and read
/// straight into, so a file whose size is right is in memory once. One
/// that shrank since it reported its size is copied into a buffer of what
/// it held; one that grew, or reported none, as a `/proc` file reports 0,
/// has its rest read into a `Vec` and both parts copied into a buffer of
/// their length.
fn read_bytes(
    isolate: &mut v8::Isolate,
    mut reading: sys::Reading,
) -> std::io::Result<PendingArrayBuffer> {
    let mut sized = PendingArrayBuffer::new(isolate, reading.size());
    let filled = reading.fill(sized.bytes_mut())?;
    if filled < sized.len() {
        // the file ended short of its size, so its end is read
        return Ok(copied(isolate, &sized.bytes()[..filled], &[]));
    }
    let rest = reading.rest()?;
    if rest.is_empty() {
        return Ok(sized);
    }
    Ok(copied(isolate, sized.bytes(), &rest))
}

/// `bytes` as a `Uint8Array` of the buffer of its own they become.
fn to_bytes<'s>(
    scope: &v8::PinScope<'s, '_>,
    bytes: PendingArrayBuffer,
) -> Result<v8::Local<'s, v8::Value>, Failure> {
    let length = bytes.len();
    let buffer = bytes.into_array_buffer(scope);
    v8::Uint8Array::new(scope, buffer, 0, length)
        .map(Into::into)
        .ok_or_else(|| Failure::Plain(format!("V8 made no Uint8Array of {length} bytes")))
}

/// `head` then `tail` in a buffer of their length, which V8 allocated.
fn copied(isolate: &mut v8::Isolate, head: &[u8], tail: &[u8]) -> PendingArrayBuffer {
    let mut whole = PendingArrayBuffer::new(isolate, head.len() + tail.len());
    let (start, end) = whole.bytes_mut().split_at_mut(head.len());
    start.copy_from_slice(head);
    end.copy_from_slice(tail);
    whole
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
        run_state(state)?.ready()?;
        let handle = read_handle(scope, &FS_READ_DIR, receiver)?;
        let requested = path_text(scope, path)?;
        let op = beneath(&run_roots(state)?, handle, requested)?;
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
    run_state(state)?.ready()?;
    let handle = read_handle(scope, &FS_READ_DIR, dir)?;
    let requested = if path.is_undefined() {
        String::new()
    } else {
        path_text(scope, path)?
    };
    let op = beneath(&run_roots(state)?, handle, requested)?;
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

/// The write op `name` called on `receiver`, an `FsDir`, with `paths`, each
/// checked in order as the op would check it, the op writing those from
/// `written` on (a copy's `to`, every path of the others): refused
/// `not_granted` when the handle may not write, naming the first path it
/// writes, recorded as it's thrown with what would let the op write them
/// all ([`scope_needed`]); else a plain `Error` saying the op isn't built
/// on fgro yet.
pub fn write_op<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    name: &str,
    receiver: v8::Local<'s, v8::Value>,
    paths: &[v8::Local<'s, v8::Value>],
    written: usize,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        run_state(state)?.ready()?;
        let handle = read_handle(scope, &FS_DIR, receiver)?;
        let roots = run_roots(state)?;
        let (_, handle_path) = handle_root(&roots, &handle)?;
        let mut targets = Vec::with_capacity(paths.len());
        for path in paths {
            let requested = path_text(scope, *path)?;
            targets.push(to_lexical(&requested, &handle_path)?.path_beneath(&handle_path));
        }
        if handle.scope.write {
            return Err(Failure::Plain(format!(
                "{name} of {MODULE} isn't built on fgro yet"
            )));
        }
        let written = targets.get(written..).unwrap_or_default();
        let needed = scope_needed(&roots, written, Scope::WRITE, NeededGrant::Write);
        let first = written.first().cloned().unwrap_or(handle_path);
        Err::<v8::Local<'s, v8::Value>, _>(Failure::NotGranted {
            path: first,
            needed: Some(needed),
        })
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
        run_state(state)?.ready()?;
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

/// What lets through an op on `targets`, absolute paths, that a handle
/// refused for want of `need`: a handle `fs.open_dir` makes on a root
/// doing `need` that holds every target by its text, when one does, since
/// a grant wouldn't widen the handle that refused it; else `grant`, the
/// grant of `need`.
fn scope_needed(
    roots: &[Root],
    targets: &[String],
    need: Scope,
    grant: NeededGrant,
) -> NeededGrant {
    // each target's roots doing `need`
    let mut held = targets.iter().map(|target| {
        let names: Vec<String> = target
            .split('/')
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        find_roots(roots, &names, false, need)
            .into_iter()
            .map(|(index, ..)| index)
            .collect::<Vec<_>>()
    });
    let first = held.next().unwrap_or_default();
    let rest: Vec<Vec<usize>> = held.collect();
    if first
        .iter()
        .any(|index| rest.iter().all(|other| other.contains(index)))
    {
        NeededGrant::OpenDir
    } else {
        grant
    }
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

/// The entry at `path`, an absolute path inside a root whose scope meets
/// `need`, opened as `opened`, as [`walk`] reaches it, each step opened
/// beneath its root by `openat2`: its root's place, the root, and its real
/// path beneath the root.
fn reach<'r>(
    roots: &'r [Root],
    path: &str,
    what: &str,
    opened: sys::Opened,
    need: Scope,
) -> Result<(usize, &'r Root, String), Failure> {
    walk(roots, path, what, opened, need, |root, rel, step| {
        let fd = root_fd(root, path)?;
        let found =
            sys::open_in_root(fd, root.path.clone(), rel, step, root.whole).map_err(|error| {
                if sys::is_escape(&error) {
                    Failure::unreached(path)
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
        beneath_of(&root.path, found).map_err(|why| Failure::Fs {
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
/// Only the roots whose scope meets `need` hold a path. A step is opened
/// beneath the longest root that holds its names, and, when that open is
/// `not_granted` (it left the root by a symlink), beneath the next longest,
/// and so on, so a root granted inside another refuses nothing the other
/// reaches.
///
/// The root's place, the root, and the entry's real path beneath it; a
/// `TypeError` naming `what` for a path that isn't absolute, and
/// `not_granted` for one whose names no root holds at any step.
fn walk<'r>(
    roots: &'r [Root],
    path: &str,
    what: &str,
    opened: sys::Opened,
    need: Scope,
    mut open: impl FnMut(&'r Root, String, sys::Opened) -> Result<String, Failure>,
) -> Result<(usize, &'r Root, String), Failure> {
    if !path.starts_with('/') {
        return Err(Failure::Type(format!(
            "{what} takes an absolute path: {path}"
        )));
    }
    let mut step = |names: &[String], dir: bool, step: sys::Opened| {
        for (index, root, rel) in find_roots(roots, names, dir, need) {
            match open(root, rel, step) {
                Ok(beneath) => return Ok((index, root, beneath)),
                // it left this root by a symlink, so the next may hold it
                Err(Failure::NotGranted { .. }) => {}
                Err(failure) => return Err(failure),
            }
        }
        Err(Failure::unreached(path))
    };
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
            let (_, root, beneath) = step(&names, false, sys::Opened::Directory)?;
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
    step(&names, dir, opened)
}

/// The roots of `roots` whose scope meets `need` that hold the path of
/// `names`, none of them `.` or `..`, by their text, by their real path or
/// a path they were granted as, the longest first, each with its place and
/// the path beneath it for the kernel to open: `.` for the root's own
/// directory, and a trailing `/` when `dir` asks for a directory, but for
/// the root's own, as a handle's isn't asked for.
fn find_roots<'r>(
    roots: &'r [Root],
    names: &[String],
    dir: bool,
    need: Scope,
) -> Vec<(usize, &'r Root, String)> {
    let mut found: Vec<(usize, &Root, usize)> = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        if !root.scope.meets(need) {
            continue;
        }
        let depth = root
            .spellings()
            .filter(|held| {
                names.len() >= held.len()
                    && names
                        .iter()
                        .zip(held)
                        .all(|(name, root_name)| name == root_name)
            })
            .map(|held| held.len())
            .max();
        if let Some(depth) = depth {
            found.push((index, root, depth));
        }
    }
    // stable, so of two the same length the earlier root comes first
    found.sort_by(|(.., a), (.., b)| b.cmp(a));
    found
        .into_iter()
        .map(|(index, root, depth)| {
            let beneath = names[depth..].join("/");
            let rel = if beneath.is_empty() {
                ".".to_owned()
            } else if dir {
                format!("{beneath}/")
            } else {
                beneath
            };
            (index, root, rel)
        })
        .collect()
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
/// and any can move to another thread. A read's open does too, and the
/// [`Reading`](sys::Reading) it returns can move; the read into the buffer
/// V8 allocated, which isn't `Send`, is made on the isolate's thread.
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

        pub use gro_sys::beneath::Reading;
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

        /// The regular file at `rel` beneath the handle's directory, at
        /// `beneath` the root, opened to read to its end; anything else,
        /// and a file that reports more than the most a read takes, is
        /// refused before a byte is read.
        pub fn open_read(
            root: Arc<OwnedFd>,
            beneath: String,
            rel: String,
        ) -> Result<Reading, OpError> {
            let held = handle_dir(&root, &beneath)?;
            let dir = held
                .as_ref()
                .map_or_else(|| root.as_fd(), |held| held.as_fd());
            beneath::open_read(dir, Path::new(&rel)).map_err(OpError::Target)
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

        /// The real path beneath the root, whose real path is
        /// `root_path`, of the directory at `rel` beneath the handle's
        /// directory, at `beneath` the root; none when the kernel's path
        /// isn't beneath the root's ([`beneath::path_beneath`]).
        pub fn open_dir(
            root: Arc<OwnedFd>,
            root_path: String,
            beneath: String,
            rel: String,
        ) -> Result<Option<OsString>, OpError> {
            let held = handle_dir(&root, &beneath)?;
            let dir = held
                .as_ref()
                .map_or_else(|| root.as_fd(), |held| held.as_fd());
            let opened = beneath::open(dir, Path::new(&rel), OpenAs::Directory, Links::Beneath)
                .map_err(OpError::Target)?;
            beneath::path_beneath(root.as_fd(), Path::new(&root_path), opened.as_fd())
                .map_err(OpError::Target)
        }

        /// The real path beneath `root`, whose real path is `root_path`,
        /// of what `rel` names beneath it, opened as `opened` says: as the
        /// kernel resolves a path, from `root`, when `whole` says the root
        /// is `/`, else beneath it; none when the kernel's path isn't
        /// beneath the root's ([`beneath::path_beneath`]).
        pub fn open_in_root(
            root: Arc<OwnedFd>,
            root_path: String,
            rel: String,
            opened: Opened,
            whole: bool,
        ) -> io::Result<Option<OsString>> {
            let how = match opened {
                Opened::Directory => OpenAs::Directory,
                Opened::Entry => OpenAs::Path,
            };
            let links = if whole { Links::InRoot } else { Links::Beneath };
            let fd = beneath::open(root.as_fd(), Path::new(&rel), how, links)?;
            beneath::path_beneath(root.as_fd(), Path::new(&root_path), fd.as_fd())
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
            _root_path: String,
            _beneath: String,
            _rel: String,
        ) -> Result<Option<OsString>, OpError> {
            Err(OpError::Target(unsupported()))
        }

        /// A file opened to read, which nothing makes off Linux.
        pub enum Reading {}

        impl Reading {
            pub const fn size(&self) -> usize {
                match *self {}
            }

            pub fn fill(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                match *self {}
            }

            pub fn rest(&mut self) -> io::Result<Vec<u8>> {
                match *self {}
            }

            pub fn to_end(self) -> io::Result<Vec<u8>> {
                match self {}
            }
        }

        pub fn open_read(
            _root: Arc<OwnedFd>,
            _beneath: String,
            _rel: String,
        ) -> Result<Reading, OpError> {
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
            _root_path: String,
            _rel: String,
            _opened: Opened,
            _whole: bool,
        ) -> io::Result<Option<OsString>> {
            Err(unsupported())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Failure, Root, Scope, beneath_of, find_roots, kind_of, op_failure, root_table,
        scope_needed, spelled, sys, to_lexical, walk,
    };
    use crate::NeededGrant;

    /// What `failure` is, as data: its kind of error, and an `FsError`'s
    /// kind and path, a `TypeError`'s message.
    fn shown(failure: Failure) -> (String, String) {
        match failure {
            Failure::Type(message) => ("TypeError".to_owned(), message),
            Failure::Fs { kind, path, .. } => (kind.to_owned(), path),
            Failure::NotGranted { path, .. } => ("not_granted".to_owned(), path),
            Failure::Plain(message) => ("Error".to_owned(), message),
            Failure::Thrown => ("thrown".to_owned(), String::new()),
        }
    }

    /// A root at `path`, never opened, reading and writing.
    fn root(path: &str) -> Root {
        scoped(path, &[], Scope::ALL)
    }

    /// A root at `path`, never opened, granted as `granted` too, doing what
    /// `scope` says.
    fn scoped(path: &str, granted: &[&str], scope: Scope) -> Root {
        Root {
            path: path.to_owned(),
            granted: granted.iter().map(|&path| path.to_owned()).collect(),
            scope,
            whole: path == "/",
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
            let result = walk(
                &roots,
                path,
                "fs.open_dir",
                opened,
                Scope::ALL,
                |root, rel, step| {
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
                            "out" => return Err(Failure::unreached(path)),
                            name => real.push(name),
                        }
                    }
                    let real = format!("/{}", real.join("/"));
                    let beneath = real
                        .strip_prefix(root.path.as_str())
                        .expect("the fake stays beneath its root");
                    Ok(beneath.trim_start_matches('/').to_owned())
                },
            );
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

    /// A refused walk names the path as asked and needs a grant to read,
    /// whatever step it was refused at: a directory step's, the names
    /// before a `..` included, an entry's, or one that leaves the roots
    /// holding it through a symlink; a root that holds the names but
    /// doesn't do what's needed changes nothing.
    #[test]
    fn a_refused_walk_needs_a_grant_to_read() {
        let roots = [root("/p"), scoped("/w", &[], Scope::WRITE)];
        let needed = |path: &str, opened: sys::Opened, need: Scope| match walk(
            &roots,
            path,
            "fs.real_path",
            opened,
            need,
            |_, rel, _| match rel.as_str() {
                "out" => Err(Failure::unreached(path)),
                "." => Ok(String::new()),
                _ => Ok(rel),
            },
        ) {
            Err(Failure::NotGranted {
                path: refused,
                needed,
            }) => {
                assert_eq!(refused, path, "the refusal names the path as asked");
                needed
            }
            _ => panic!("{path} wasn't refused"),
        };
        let (directory, entry) = (sys::Opened::Directory, sys::Opened::Entry);
        for (path, opened, need) in [
            ("/q/a", directory, Scope::ALL),
            ("/q/a/", directory, Scope::ALL),
            ("/q/a", entry, Scope::READ),
            ("/q/../p/a", entry, Scope::READ),
            ("/usr/../etc", directory, Scope::ALL),
            ("/p/../etc", directory, Scope::ALL),
            ("/", entry, Scope::READ),
            ("/w/a/f.txt", entry, Scope::READ),
            ("/p/out", directory, Scope::ALL),
            ("/p/out", entry, Scope::READ),
        ] {
            assert_eq!(
                needed(path, opened, need),
                Some(NeededGrant::Read),
                "{path}"
            );
        }
    }

    /// An op a handle's scope lacks needs a handle `fs.open_dir` makes on
    /// a root doing it that holds every one of the op's targets by its
    /// text, by its real path or a path it was granted as; else a grant of
    /// what it lacks.
    #[test]
    fn a_scope_refusal_needs_a_handle_on_a_root_that_grants_it() {
        let roots = [
            scoped("/p", &[], Scope::READ),
            scoped("/p/out", &[], Scope::ALL),
            scoped("/p/out/deeper", &[], Scope::ALL),
            scoped("/data/w", &["/w"], Scope::WRITE),
            scoped("/data/w/sub", &[], Scope::ALL),
        ];
        let needed = |targets: &[&str], need: Scope| {
            let grant = if need.write {
                NeededGrant::Write
            } else {
                NeededGrant::Read
            };
            let targets: Vec<String> = targets.iter().map(|&target| target.to_owned()).collect();
            scope_needed(&roots, &targets, need, grant)
        };
        let (open_dir, read, write) = (NeededGrant::OpenDir, NeededGrant::Read, NeededGrant::Write);
        for (targets, need, expected) in [
            (&["/p/out/x"][..], Scope::WRITE, &open_dir),
            (&["/p/out"], Scope::WRITE, &open_dir),
            (&["/p/out/deeper/x"], Scope::WRITE, &open_dir),
            (&["/p/outer/x"], Scope::WRITE, &write),
            (&["/p/x"], Scope::WRITE, &write),
            (&["/w/x"], Scope::WRITE, &open_dir),
            (&["/data/w/sub/x"], Scope::READ, &open_dir),
            (&["/data/w/x"], Scope::READ, &read),
            (&["/p/x"], Scope::READ, &open_dir),
            // every target held by one root doing the need
            (&["/p/out/a", "/p/out/b"], Scope::WRITE, &open_dir),
            (&["/p/out/deeper/a", "/p/out/b"], Scope::WRITE, &open_dir),
            (&["/p/out/a", "/w/b"], Scope::WRITE, &write),
            (&["/p/src/a", "/p/out/b"], Scope::WRITE, &write),
            (&["/p/out/b", "/p/src/a"], Scope::WRITE, &write),
            (&[], Scope::WRITE, &write),
        ] {
            assert_eq!(&needed(targets, need), expected, "{targets:?}");
        }
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

    /// A root holds a path by its real path or a path it was granted as,
    /// the longest first, each with the rest beneath it, and only when its
    /// scope meets the need: a root only written holds nothing for a read.
    #[test]
    fn roots_hold_a_path_by_any_spelling_the_longest_first() {
        let roots = [
            scoped("/p", &[], Scope::READ),
            scoped("/data/home/u/c", &["/home/u/c"], Scope::READ),
            scoped("/p/out", &[], Scope::WRITE),
            scoped("/p/out/both", &["/link"], Scope::ALL),
            scoped("", &["/gone"], Scope::READ),
        ];
        let held = |path: &str, need: Scope| {
            let names: Vec<String> = path
                .split('/')
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            find_roots(&roots, &names, false, need)
                .into_iter()
                .map(|(index, _, rel)| (index, rel))
                .collect::<Vec<_>>()
        };
        let owned = |found: &[(usize, &str)]| {
            found
                .iter()
                .map(|(index, rel)| (*index, (*rel).to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(held("/p/a", Scope::ALL), owned(&[(0, "a")]));
        assert_eq!(held("/p", Scope::READ), owned(&[(0, ".")]));
        assert_eq!(
            held("/p/out/x", Scope::ALL),
            owned(&[(2, "x"), (0, "out/x")])
        );
        assert_eq!(held("/p/out/x", Scope::READ), owned(&[(0, "out/x")]));
        assert_eq!(held("/p/out/x", Scope::WRITE), owned(&[(2, "x")]));
        assert_eq!(
            held("/p/out/both/y", Scope::READ),
            owned(&[(3, "y"), (0, "out/both/y")])
        );
        assert_eq!(held("/link/y", Scope::WRITE), owned(&[(3, "y")]));
        assert_eq!(held("/home/u/c/d", Scope::READ), owned(&[(1, "d")]));
        assert_eq!(held("/data/home/u/c", Scope::READ), owned(&[(1, ".")]));
        assert_eq!(held("/home/u", Scope::READ), owned(&[]));
        assert_eq!(held("/home/u/cc", Scope::READ), owned(&[]));
        assert_eq!(held("/gone/x", Scope::READ), owned(&[(4, "x")]));
        assert_eq!(held("/", Scope::ALL), owned(&[]));
    }

    /// A step that leaves the longest root holding it by a symlink is opened
    /// beneath the next longest, so a root inside another refuses nothing
    /// the outer one reaches, and the entry is the outer root's; one that
    /// leaves every root is `not_granted`, and any other failure ends the
    /// walk at the first root.
    #[test]
    fn a_step_out_of_an_inner_root_falls_back_to_the_outer() {
        let roots = [root("/p"), root("/p/in")];
        let walked = |path: &str| {
            let mut opens = Vec::new();
            let result = walk(
                &roots,
                path,
                "fs.open_dir",
                sys::Opened::Entry,
                Scope::ALL,
                |root, rel, _| {
                    opens.push(format!("{}:{rel}", root.path));
                    match (root.path.as_str(), rel.as_str()) {
                        // `up`, a symlink to `../x`, leaves `/p/in` into
                        // `/p`, and `out` leaves both
                        ("/p/in", "up") | (_, "in/out" | "out") => Err(Failure::unreached(path)),
                        ("/p", "in/up") => Ok("x".to_owned()),
                        (_, "missing") => Err(Failure::Fs {
                            kind: "not_found",
                            path: path.to_owned(),
                            cause: None,
                        }),
                        (_, rel) => Ok(rel.to_owned()),
                    }
                },
            );
            (
                result
                    .map(|(index, _, beneath)| (index, beneath))
                    .map_err(shown),
                opens,
            )
        };
        assert_eq!(
            walked("/p/in/up"),
            (
                Ok((0, "x".to_owned())),
                vec!["/p/in:up".to_owned(), "/p:in/up".to_owned()]
            )
        );
        assert_eq!(
            walked("/p/in/out"),
            (
                Err(("not_granted".to_owned(), "/p/in/out".to_owned())),
                vec!["/p/in:out".to_owned(), "/p:in/out".to_owned()]
            )
        );
        assert_eq!(
            walked("/p/in/missing"),
            (
                Err(("not_found".to_owned(), "/p/in/missing".to_owned())),
                vec!["/p/in:missing".to_owned()]
            )
        );
    }

    /// A granted path spells its root by its names, `.` and empty ones
    /// dropped; one holding a `..`, which the kernel resolves after the
    /// symlink before it, spells none.
    #[test]
    fn a_granted_path_spells_its_root_by_its_text() {
        use std::path::Path;
        for (granted, spelling) in [
            ("/a/b", Some("/a/b")),
            ("/a//./b/", Some("/a/b")),
            ("/", Some("/")),
            ("/a/../b", None),
            ("/a/..", None),
        ] {
            assert_eq!(
                spelled(Path::new(granted)).as_deref(),
                spelling,
                "{granted}"
            );
        }
    }

    /// The table: the project's root first, read; each granted directory a
    /// root of its real path, the paths of one real path one root with its
    /// scopes joined and each spelling kept; each root reading and writing
    /// what every root holding it does, so write never implies read but a
    /// directory written inside a read one is read; a granted path gone
    /// before the table is made a root held as why; and unconfined, `/`,
    /// read and written, holding every other.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_root_table_joins_each_real_path_s_grants() {
        use std::path::PathBuf;

        use crate::{Authority, FsGrant, Grants};

        let dir = crate::test_support::TestDir::on_disk("root_table");
        let at = |name: &str| dir.path().join(name);
        for name in ["p", "p/out", "w", "r"] {
            std::fs::create_dir_all(at(name)).unwrap();
        }
        std::os::unix::fs::symlink("w", at("to_w")).unwrap();
        let path = |name: &str| at(name).display().to_string();
        let grants = |read: &[PathBuf], write: &[PathBuf]| {
            Authority::Confined(Grants {
                fs: Some(FsGrant {
                    read: read.iter().cloned().collect(),
                    write: write.iter().cloned().collect(),
                }),
                ..Grants::default()
            })
        };
        let table = root_table(
            Some(&at("p")),
            Some(&grants(
                &[at("r"), at("p/./out"), at("missing")],
                &[at("p/out"), at("to_w"), at("w"), at("r/../w"), at("r")],
            )),
        );
        let shown: Vec<(String, Vec<String>, Scope, bool)> = table
            .iter()
            .map(|root| {
                (
                    root.path.clone(),
                    root.granted.clone(),
                    root.scope,
                    root.fd.is_ok(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                (path("p"), vec![], Scope::READ, true),
                (String::new(), vec![path("missing")], Scope::READ, false),
                (path("p/out"), vec![], Scope::ALL, true),
                (path("r"), vec![], Scope::ALL, true),
                (path("w"), vec![path("to_w")], Scope::WRITE, true),
            ]
        );
        assert!(
            table[1]
                .fd
                .as_ref()
                .is_err_and(|why| why.contains("is gone")),
            "{:?}",
            table[1].fd
        );
        let unconfined = root_table(Some(&at("p")), Some(&Authority::Unconfined));
        let shown: Vec<(&str, Scope, bool)> = unconfined
            .iter()
            .map(|root| (root.path.as_str(), root.scope, root.whole))
            .collect();
        assert_eq!(
            shown,
            [
                (path("p").as_str(), Scope::ALL, false),
                ("/", Scope::ALL, true)
            ]
        );
    }

    /// A root that can't be opened is held as why, naming the path it was
    /// granted as and its real path, once when they're one path.
    #[test]
    fn a_root_that_cant_open_names_its_path_once() {
        use super::open_root;

        let dir = crate::test_support::TestDir::on_disk("root_unopened");
        let file = dir.path().join("f");
        std::fs::write(&file, "").unwrap();
        let link = dir.path().join("to_f");
        std::os::unix::fs::symlink("f", &link).unwrap();
        let why = |project: bool, granted: &std::path::Path| {
            open_root(project, Some(&file), &[granted], Scope::READ)
                .fd
                .unwrap_err()
        };
        let (f, l) = (file.display(), link.display());
        for (why, expected) in [
            (
                why(false, &file),
                format!("the directory granted as {f} can't be opened: "),
            ),
            (
                why(false, &link),
                format!("the directory granted as {l}, {f}, can't be opened: "),
            ),
            (
                why(true, &file),
                format!("the project's directory, {f}, can't be opened: "),
            ),
        ] {
            assert!(why.starts_with(&expected), "{why}");
        }
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

    /// A read's bytes come from the isolate's own `ArrayBuffer` allocator,
    /// a success after a failed allocation clearing what the recording
    /// allocator holds, and end where the file ends when read: one of the
    /// size it reported, one grown or shrunk since its open, an empty one,
    /// and a `/proc` file, which reports 0; each a `Uint8Array` of its
    /// whole buffer.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_read_s_bytes_are_v8_s_and_end_where_the_file_ends() {
        use std::io::Write as _;
        use std::path::Path;
        use std::sync::Arc;

        use deno_core::{JsRuntime, RuntimeOptions, v8};

        use super::{read_bytes, to_bytes};

        let dir = crate::test_support::TestDir::on_disk("read_bytes");
        let file = |name: &str| dir.path().join(name);
        for name in ["sized", "grows", "shrinks"] {
            std::fs::write(file(name), "12345").unwrap();
        }
        std::fs::write(file("empty"), "").unwrap();
        let open = |root: &Path, name: &str| {
            let root = std::fs::File::open(root).unwrap();
            let opened = sys::open_read(Arc::new(root.into()), String::new(), name.to_owned());
            opened.unwrap_or_else(|_| panic!("{name} didn't open"))
        };
        let (create_params, record) = gro_v8::record_array_buffers(v8::Isolate::create_params());
        // the collections after a failed allocation post delayed tasks
        let tokio = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _entered = tokio.enter();
        let mut runtime = JsRuntime::new(RuntimeOptions {
            create_params: Some(create_params),
            ..Default::default()
        });
        // an allocation too large for any process, which the record holds
        // until the next one succeeds
        let fail = |scope: &mut v8::PinScope<'_, '_>| {
            let source = "try { new ArrayBuffer(2 ** 52); } catch {}";
            let source = v8::String::new(scope, source).unwrap();
            let script = v8::Script::compile(scope, source, None).unwrap();
            script.run(scope).unwrap();
            assert!(record.latest_failed());
        };
        let reads = [
            ("sized", b"12345".as_slice(), None),
            ("grows", b"123456789", Some(true)),
            ("shrinks", b"12", Some(false)),
            ("empty", b"", None),
        ];
        let mut opened = Vec::new();
        for (name, expected, change) in reads {
            let reading = open(dir.path(), name);
            match change {
                Some(true) => std::fs::OpenOptions::new()
                    .append(true)
                    .open(file(name))
                    .unwrap()
                    .write_all(b"6789")
                    .unwrap(),
                Some(false) => std::fs::OpenOptions::new()
                    .write(true)
                    .open(file(name))
                    .unwrap()
                    .set_len(2)
                    .unwrap(),
                None => {}
            }
            opened.push((name, expected, reading));
        }
        let status = open(Path::new("/proc/self"), "status");
        assert_eq!(status.size(), 0);
        deno_core::scope!(scope, &mut runtime);
        for (name, expected, reading) in opened {
            fail(scope);
            let bytes = read_bytes(scope, reading).unwrap_or_else(|_| panic!("{name} failed"));
            if !expected.is_empty() {
                assert!(!record.latest_failed(), "{name}'s buffer isn't V8's");
            }
            assert_eq!(bytes.bytes(), expected, "{name}");
            let length = bytes.len();
            let Ok(array) = to_bytes(scope, bytes) else {
                panic!("{name} made no Uint8Array");
            };
            let array = v8::Local::<v8::Uint8Array>::try_from(array).unwrap();
            let buffer = array.buffer(scope).unwrap();
            assert_eq!(
                (array.byte_length(), buffer.byte_length()),
                (length, length),
                "{name}"
            );
        }
        fail(scope);
        let status = read_bytes(scope, status).unwrap_or_else(|_| panic!("status failed"));
        assert!(
            status.bytes().starts_with(b"Name:"),
            "{}",
            String::from_utf8_lossy(status.bytes())
        );
        assert!(!record.latest_failed(), "status's buffer isn't V8's");
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
