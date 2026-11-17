//! The members that make a handle: `fs.project`, `fs.open_dir`, and a
//! handle's `dir()` and `readonly()`; `fs.real_path`, which reaches an
//! entry as `fs.open_dir` reaches a directory ([`reach`]); and a member
//! not built on fgro yet.

use deno_core::{OpState, v8};

use super::walk::reach;
use super::{
    Failure, MODULE, Op, Scope, beneath_of, finish, join, make_dir, path_text, read_handle,
    run_roots, run_state, sys,
};
use crate::host::error_class::DeclaredFamily;
use crate::host::handle::{FS_DIR, FS_READ_DIR, HandleKind};
use crate::host::{native, string};

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
        let op = Op::new(state, scope, kind, receiver)?;
        let end = op.path(scope, path)?;
        let (fd, dir, rel) = op.call(&end)?;
        let root = op.root();
        let real = sys::open_dir(fd, root.path.clone(), dir, rel)
            .map_err(|error| op.failure(&end, error))?;
        let beneath = beneath_of(&root.path, real).map_err(|why| {
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
        let op = Op::new(state, scope, &FS_DIR, receiver)?;
        let prototype = native::prototype(state, scope, &FS_READ_DIR).ok_or(Failure::Thrown)?;
        make_dir(
            scope,
            &FS_READ_DIR,
            prototype,
            op.handle.root,
            &op.root().path,
            &op.handle.beneath,
            op.handle.scope.read_only(),
        )
        .map(Into::into)
        .ok_or(Failure::Thrown)
    })();
    finish(state, scope, None, result)
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
