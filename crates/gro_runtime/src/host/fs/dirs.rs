//! The members that make a handle: `fs.project`, `fs.open_dir`,
//! `fs.temp_dir`, and a handle's `dir()` and `readonly()`; `fs.real_path`,
//! which reaches an entry as `fs.open_dir` reaches a directory
//! ([`reach`]); and a temporary directory's disposal.

use deno_core::{OpState, v8};

use super::roots::Root;
use super::walk::reach;
use super::{
    Failure, NO_FS_STATE, Op, Scope, beneath_of, finish, join, kind_of, make_dir, path_text,
    read_handle, run_roots, run_state, sys,
};
use crate::host::error_class::DeclaredFamily;
use crate::host::handle::{FS_DIR, FS_READ_DIR, FS_TEMP_DIR, HandleKind};
use crate::host::{HostData, native, string};
use crate::{Authority, FsNeed};

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
    let table = run_roots(state).ok();
    let first = table.as_ref().and_then(|table| table.roots().first());
    let path = first.map_or_else(String::new, |root| root.handle_base().to_owned());
    let access = table.map_or_else(Scope::default, |table| table.project().scope_of(0));
    make_dir(scope, &FS_DIR, prototype, 0, &path, "", access).map(Into::into)
}

/// `fs.open_dir(path)`: an `FsDir` on the directory at `path`, an absolute
/// path inside a root of the project's view, of the scope the view gives
/// the root, `not_granted` outside every root of it, also when nothing is
/// there, or when it leads out of its root by a `..` or a symlink;
/// `errors`, `FsError`'s family as the member declares it.
pub fn open_dir<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    path: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        run_state(state)?.ready()?;
        let path = path_text(scope, path)?;
        let table = run_roots(state)?;
        let view = table.project();
        let (index, root, beneath) = reach(
            table.roots(),
            view,
            &path,
            "fs.open_dir",
            sys::Opened::Directory,
            Scope::ALL,
        )?;
        let prototype = native::prototype(state, scope, &FS_DIR).ok_or(Failure::Thrown)?;
        make_dir(
            scope,
            &FS_DIR,
            prototype,
            index,
            root.handle_base(),
            &beneath,
            view.scope_of(index),
        )
        .map(Into::into)
        .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `fs.real_path(path)`: the real path of the entry at `path`, an absolute
/// path inside a root the project's view reads, every symlink followed
/// beneath it,
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
        let table = run_roots(state)?;
        let (_, root, beneath) = reach(
            table.roots(),
            table.project(),
            &path,
            "fs.real_path",
            sys::Opened::Entry,
            Scope::READ,
        )?;
        string(scope, &join(root.handle_base(), &beneath))
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
        let op = Op::new(state, scope, kind, receiver)?;
        let end = op.path(scope, path)?;
        let (fd, dir, rel) = op.call(&end)?;
        let root = op.root();
        let real = sys::open_dir(fd, root.handle_base().to_owned(), dir, rel)
            .map_err(|error| op.failure(&end, error))?;
        let beneath = beneath_of(root.handle_base(), real).map_err(|why| {
            Failure::outside(
                &end.requested,
                format!("{} beneath {}: {why}", end.requested, op.handle_path),
            )
        })?;
        let prototype = native::prototype(state, scope, kind).ok_or(Failure::Thrown)?;
        let access = if *kind == FS_DIR {
            op.handle.scope
        } else {
            op.handle.scope.read_only()
        };
        make_dir(
            scope,
            kind,
            prototype,
            op.handle.root,
            root.handle_base(),
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
        let op = Op::new(state, scope, &FS_DIR, receiver)?;
        let prototype = native::prototype(state, scope, &FS_READ_DIR).ok_or(Failure::Thrown)?;
        make_dir(
            scope,
            &FS_READ_DIR,
            prototype,
            op.handle.root,
            op.root().handle_base(),
            &op.handle.beneath,
            op.handle.scope.read_only(),
        )
        .map(Into::into)
        .ok_or(Failure::Thrown)
    })();
    finish(state, scope, None, result)
}

/// The prefix `fs.temp_dir` names its directories by when given none.
const TEMP_PREFIX: &str = "gro-";

/// What a temporary directory's ops fail with once its handle was disposed
/// of, which removed it.
fn retired_why(path: &str) -> String {
    format!("the temporary directory {path} was removed when its handle was disposed of")
}

/// `fs.temp_dir(prefix?)`: an `FsDir` that is `AsyncDisposable` on a fresh
/// directory under the OS's temporary directory, which the run grants by
/// `--allow-temp` or `-A`, named as the Node module names it, `prefix`
/// then six random letters, concatenated, never joined, so the name's the
/// only one it makes ([`sys::make_temp`]); the directory a root the
/// project's view reads and writes, appended to the run's table
/// ([`super::roots::Table::append`]). The prefix is checked as the Node
/// module checks it, `'gro-'` when it's `undefined`; then the grant, else
/// `not_granted` naming the prefix, recorded with the grant that lets it
/// through, never the temporary directory's path, which the run doesn't
/// learn; a failure to make it names the path the Node module's would,
/// the temporary directory's joined with the prefix.
pub fn temp_dir<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    prefix: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let fs = run_state(state)?;
        fs.ready()?;
        let prefix = if prefix.is_undefined() {
            TEMP_PREFIX.to_owned()
        } else {
            if !prefix.is_string() {
                return Err(Failure::Type(
                    "fs.temp_dir takes a string prefix".to_owned(),
                ));
            }
            let prefix = path_text(scope, prefix)?;
            if prefix.contains('/') {
                return Err(Failure::Type(format!(
                    "fs.temp_dir takes a name prefix: {prefix}"
                )));
            }
            prefix
        };
        let authority = state.try_borrow::<Authority>();
        let granted = match authority {
            Some(Authority::Unconfined) => true,
            Some(Authority::Confined(grants)) => grants.fs.as_ref().is_some_and(|fs| fs.temp),
            None => false,
        };
        if !granted {
            return Err(Failure::NotGranted {
                path: prefix,
                need: Some(FsNeed::Temp),
            });
        }
        let data = state
            .try_borrow::<HostData>()
            .ok_or_else(|| Failure::Plain(NO_FS_STATE.to_owned()))?;
        let temp = data
            .temp_dir
            .clone()
            .ok_or_else(|| Failure::Plain(NO_TEMP_DIR.to_owned()))?;
        let template = format!("{}/{prefix}", temp.to_string_lossy());
        let made = sys::make_temp(temp, prefix)
            .map_err(|error| Failure::of(kind_of(&error), &template, &error))?;
        let path = made.path.clone();
        let root = Root::made(made.path, made.fd, made.parent, made.name);
        let place = fs.append(data, authority, root, Scope::ALL)?;
        let prototype = native::prototype(state, scope, &FS_TEMP_DIR).ok_or(Failure::Thrown)?;
        make_dir(scope, &FS_TEMP_DIR, prototype, place, &path, "", Scope::ALL)
            .map(Into::into)
            .ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// What `fs.temp_dir` throws in a run that grants it with no temporary
/// directory named, which no embedder's run that grants it is.
const NO_TEMP_DIR: &str = "this run grants fs.temp_dir but was started naming no temporary \
                           directory";

/// `[Symbol.asyncDispose]()` called on `receiver`, a temporary directory's
/// handle: removes its directory and everything in it, beneath the root's
/// own descriptor ([`sys::remove_temp`]), then retires the root
/// ([`super::roots::Table::retire`]), so every op through a handle on it,
/// this one's and each made from it, fails `io_error` saying it was
/// removed, `fs.open_dir` reaches it through no root of its own, and the
/// rule no longer keeps its path. Called again, once it's retired, it does
/// nothing, as a forced `rm` of nothing does. A failure to remove it is the
/// kind the OS said, naming its path, and keeps the root, so a call after
/// can try again.
pub fn dispose<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let fs = run_state(state)?;
        fs.ready()?;
        let handle = read_handle(scope, &FS_TEMP_DIR, receiver)?;
        let (made_in, path, fd) = {
            let table = run_roots(state)?;
            let root = table.roots().get(handle.root).ok_or_else(|| {
                Failure::Plain("an fs handle names no root of the run".to_owned())
            })?;
            (
                root.made_in(),
                root.handle_base().to_owned(),
                root.fd.clone(),
            )
        };
        // retired already: nothing to remove
        let (Some((parent, name)), Ok(fd)) = (made_in, fd) else {
            return Ok(v8::undefined(scope).into());
        };
        sys::remove_temp(fd, parent, name)
            .map_err(|error| Failure::of(kind_of(&error), &path, &error))?;
        fs.retire(handle.root, retired_why(&path))?;
        Ok(v8::undefined(scope).into())
    })();
    finish(state, scope, Some(errors), result)
}
