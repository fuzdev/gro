//! The ops that write through an `FsDir`: `write`, `mkdir`, `remove`, and
//! `rename`, and the ones not built on fgro yet. Each checks its paths as
//! the Node module checks them, then its other arguments, read as data
//! ([`read_option`]), then that the handle may do it: an op that reads one
//! of its paths (a copy's `from`) is refused `not_granted` naming that
//! path when the handle doesn't read, before the write is judged, as the
//! op would read it first; and one the handle may not write, naming the
//! first path it writes; each recorded as it's thrown with what would let
//! it through ([`Op::check`]).
//!
//! `write` takes a string, written as UTF-8 with each lone surrogate a
//! U+FFFD, as Node writes one, or a `Uint8Array`, a `TypeError` otherwise;
//! the bytes are copied out of V8's heap before the call, which owns them,
//! so a write's bytes are in memory twice while it runs. It opens its
//! target beneath the handle without waiting, a final symlink followed
//! beneath it, and refuses anything but a regular file by the opened
//! descriptor before a byte is written ([`sys::write`]): a directory is
//! `is_a_directory`, and a FIFO or a device `io_error`, where the Node
//! module's write would wait on a FIFO or write to a device. The file is
//! emptied as it opens, before the write, so a write that fails can leave
//! it empty or short, as Node's `writeFile` can.
//!
//! The handle's own directory can't be removed or renamed, either end of a
//! rename, an `io_error` as the Node module's; and nor can any root's
//! directory, or an entry that holds one ([`Held`]): `remove` and `rename`
//! refuse it `io_error`, before anything is removed or moved, whatever the
//! run grants, so a refused one needs no grant. A rename that would move
//! an entry where the deepest root holding it grants what the source's
//! doesn't, a write-only root's file into a root the run reads, is refused
//! too. A recursive `remove` empties a directory entry by entry, each
//! beneath its own directory's descriptor, no symlink followed
//! ([`gro_sys::beneath::Last::remove_tree`]), and names no entry inside it
//! in its error.

use deno_core::{OpState, v8};

use super::roots::{Held, Refusal};
use super::{
    Access, End, Failure, MODULE, Op, finish, is_missing, read_flag, read_option, rule_failure, sys,
};
use crate::host::error_class::DeclaredFamily;
use crate::host::handle::FS_DIR;
use crate::host::string;

/// The permission bits a file `write` makes gets when its options name
/// none, less the process's umask, as Node's `writeFile` makes one.
const FILE_MODE: u32 = 0o666;

impl End {
    /// Whether it's the handle's own directory.
    const fn is_own(&self) -> bool {
        self.lexical.segments.is_empty()
    }

    /// `io_error` about it, the handle's own directory, which an op can't
    /// do what `verb` says to, as the Node module refuses it.
    fn own_refused(&self, verb: &str) -> Failure {
        Failure::Fs {
            kind: "io_error",
            path: self.target.clone(),
            cause: Some(format!(
                "the handle's own directory can't be {verb}: {}",
                self.target
            )),
        }
    }

    /// `io_error` about it, which the rule for a root's directory refused
    /// as `why` says, `to` the rename's destination.
    fn rule_refused(&self, why: Refusal, to: &Self) -> Failure {
        rule_failure(why, &self.target, Some(&to.target))
    }
}

impl Op<'_> {
    /// The roots beneath the handle's directory, which a rename or a remove
    /// checks its ends against, or none when none is.
    fn held(&self) -> Option<Held> {
        Held::of(&self.roots, &self.handle_path)
    }
}

/// `write(path, data, options?)` called on `receiver`: writes `data`, a
/// string or a `Uint8Array`, to the file at `path`, replacing what it held,
/// or made with `options.mode` masked to `0o777`, `0o666` by default;
/// with `create_new`, only made, `'created'`, or `'exists'` when an entry
/// was already there.
pub fn write<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let op = Op::new(state, scope, &FS_DIR, receiver)?;
        let end = op.path(scope, path)?;
        let bytes = data_bytes(scope, data)?;
        let create_new = read_flag(scope, options, "create_new")?;
        let mode = write_mode(scope, options)?;
        op.check(Access::Write, &[&end], false)?;
        let (fd, dir, rel) = op.call(&end)?;
        let written = sys::write(fd, dir, rel, bytes, create_new, mode);
        let answer = match written {
            Ok(()) if create_new => "created",
            Ok(()) => return Ok(v8::undefined(scope).into()),
            Err(sys::OpError::Target(error))
                if create_new && error.raw_os_error() == Some(libc::EEXIST) =>
            {
                "exists"
            }
            Err(error) => return Err(op.failure(&end, error)),
        };
        string(scope, answer).map(Into::into).ok_or(Failure::Thrown)
    })();
    finish(state, scope, Some(errors), result)
}

/// `data` as the bytes `write` writes: a string as UTF-8, each lone
/// surrogate a U+FFFD, or a `Uint8Array`'s bytes, copied, so the write owns
/// them; a `TypeError` for anything else, read without running any of its
/// code.
fn data_bytes(
    scope: &v8::PinScope<'_, '_>,
    data: v8::Local<'_, v8::Value>,
) -> Result<Vec<u8>, Failure> {
    if let Ok(text) = v8::Local::<v8::String>::try_from(data) {
        return Ok(text.to_rust_string_lossy(scope).into_bytes());
    }
    if let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(data) {
        let mut bytes = vec![0; view.byte_length()];
        let copied = view.copy_contents(&mut bytes);
        bytes.truncate(copied);
        return Ok(bytes);
    }
    Err(Failure::Type(
        "fs write data is a string or a Uint8Array".to_owned(),
    ))
}

/// The permission bits `options.mode` gives a file `write` makes, as the
/// Node module's `to_write_mode` checks them: [`FILE_MODE`] when it's
/// missing or `undefined`, else an integer from `0` to `0o7777`, masked to
/// `0o777`, so no write makes a setuid, setgid, or sticky file; anything
/// else a `TypeError`, a `Number` object among them, never converted, so no
/// code of a caller's runs.
fn write_mode(
    scope: &v8::PinScope<'_, '_>,
    options: v8::Local<'_, v8::Value>,
) -> Result<u32, Failure> {
    let Some(mode) = read_option(scope, options, "mode")?.filter(|mode| !mode.is_undefined())
    else {
        return Ok(FILE_MODE);
    };
    v8::Local::<v8::Number>::try_from(mode)
        .ok()
        .map(|number| number.value())
        .filter(|mode| mode.fract() == 0.0 && (0.0..=f64::from(0o7777)).contains(mode))
        .map(|mode| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "an integer from 0 to 0o7777, checked"
            )]
            let mode = mode as u32;
            mode & 0o777
        })
        .ok_or_else(|| Failure::Type("fs write modes are integers from 0 to 0o7777".to_owned()))
}

/// `mkdir(path, options?)` called on `receiver`: makes the directory at
/// `path`, and with `recursive` each directory missing on the way. An
/// entry already there is judged as Node judges it ([`sys::mkdir`]): a
/// final symlink is followed beneath the handle, and a failure to resolve
/// it, but for nothing there, is the call's (`io_error` for a loop,
/// `not_a_directory` through a file); a directory passes with `recursive`
/// or `exists_ok`; a dangling symlink with `recursive` is `not_found`, a
/// file named with a trailing `/` with `exists_ok` `not_a_directory`;
/// anything else is `already_exists`. The handle's own directory is one
/// that's there.
pub fn mkdir<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let op = Op::new(state, scope, &FS_DIR, receiver)?;
        let end = op.path(scope, path)?;
        let recursive = read_flag(scope, options, "recursive")?;
        let exists_ok = read_flag(scope, options, "exists_ok")?;
        op.check(Access::Write, &[&end], false)?;
        let (fd, dir, rel) = op.call(&end)?;
        sys::mkdir(fd, dir, rel, recursive, exists_ok).map_err(|error| op.failure(&end, error))?;
        Ok(v8::undefined(scope).into())
    })();
    finish(state, scope, Some(errors), result)
}

/// `remove(path, options?)` called on `receiver`: removes the entry at
/// `path`, a final symlink, never its target, and a directory only when
/// it's empty, or with `recursive`, with everything in it; with
/// `missing_ok`, nothing there is no failure. The handle's own directory,
/// a root's, and one holding a root's are refused `io_error`.
pub fn remove<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let op = Op::new(state, scope, &FS_DIR, receiver)?;
        let end = op.path(scope, path)?;
        let recursive = read_flag(scope, options, "recursive")?;
        let missing_ok = read_flag(scope, options, "missing_ok")?;
        if end.is_own() {
            return Err(end.own_refused("removed"));
        }
        op.check(Access::Write, &[&end], true)?;
        let (fd, dir, rel) = op.call(&end)?;
        let removed = sys::remove(
            fd,
            op.root().path.clone(),
            dir,
            op.handle_path.clone(),
            rel,
            recursive,
            op.held(),
        );
        match removed {
            Ok(()) => {}
            Err(error) if missing_ok && is_missing(&error) => {}
            Err(error) => return Err(op.failure(&end, error)),
        }
        Ok(v8::undefined(scope).into())
    })();
    finish(state, scope, Some(errors), result)
}

/// `rename(from, to)` called on `receiver`: moves the entry at `from` to
/// `to`, replacing what's there as the kernel does, a symlink moved, never
/// its target. The handle's own directory at either end, a root's
/// directory or one holding a root's at either end, and a move into a
/// root that grants what the source's doesn't are refused `io_error`; a
/// failure is named as the Node module names it ([`sys::rename`]), and
/// the kernel's refusal to move between two directories (`EXDEV`) is
/// `io_error`.
pub fn rename<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    from: v8::Local<'s, v8::Value>,
    to: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let op = Op::new(state, scope, &FS_DIR, receiver)?;
        let source = op.path(scope, from)?;
        let destination = op.path(scope, to)?;
        if source.is_own() {
            return Err(source.own_refused("renamed"));
        }
        if destination.is_own() {
            return Err(destination.own_refused("replaced"));
        }
        op.check(Access::Write, &[&source, &destination], true)?;
        let (fd, dir, source_rel) = op.call(&source)?;
        let renamed = sys::rename(
            fd,
            op.root().path.clone(),
            dir,
            op.handle_path.clone(),
            source_rel,
            destination.lexical.rel(),
            op.held(),
        );
        let end = |to: bool| if to { &destination } else { &source };
        renamed.map_err(|error| match error {
            sys::RenameError::Handle(error) => op.failure(&source, sys::OpError::Handle(error)),
            sys::RenameError::At { to, error } => op.failure(end(to), sys::OpError::Target(error)),
            sys::RenameError::Refused { to, why } => end(to).rule_refused(why, &destination),
        })?;
        Ok(v8::undefined(scope).into())
    })();
    finish(state, scope, Some(errors), result)
}

/// The write op `name`, not built on fgro yet, called on `receiver`, an
/// `FsDir`, with `read`, the path it reads, when it reads one (a copy's
/// `from`), then `written`, the path it writes, each checked in order as
/// the op would check it: refused as [`Op::check`] refuses it, the read
/// before the write, else a plain `Error` saying the op isn't built on
/// fgro yet.
pub fn unbuilt<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    name: &str,
    receiver: v8::Local<'s, v8::Value>,
    read: Option<v8::Local<'s, v8::Value>>,
    written: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let result = (|| {
        let op = Op::new(state, scope, &FS_DIR, receiver)?;
        let read = read.map(|path| op.path(scope, path)).transpose()?;
        let written = op.path(scope, written)?;
        if let Some(read) = &read {
            op.check(Access::Read, &[read], false)?;
        }
        op.check(Access::Write, &[&written], false)?;
        Err::<v8::Local<'s, v8::Value>, _>(Failure::Plain(format!(
            "{name} of {MODULE} isn't built on fgro yet"
        )))
    })();
    finish(state, scope, Some(errors), result)
}
