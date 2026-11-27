//! `list` returns a listing at once, reading nothing, not even its
//! receiver or its path, as the Node module's async generator does: its
//! first `next()` checks them, reads the whole directory, and sorts it by
//! the names' bytes, the directory's descriptor closed before that call
//! settles. So a listing, a handle whose state is the host's record under
//! its brand, holds no descriptor between calls, only its entries, which
//! a collection frees with it, however it ends.

use std::os::unix::ffi::OsStringExt as _;

use deno_core::{OpState, v8};

use super::reads::not_utf8;
use super::{Access, Failure, Op, file_kind_name, finish, sys};
use crate::host::error_class::DeclaredFamily;
use crate::host::handle::{self, FS_LISTING, FS_READ_DIR};
use crate::host::{name, native, string};

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
    let op = Op::new(state, scope, &FS_READ_DIR, dir)?;
    let end = if path.is_undefined() {
        op.resolve(String::new(), Access::Read)?
    } else {
        op.path(scope, path, Access::Read)?
    };
    op.check(Access::Read, &[&end], false)?;
    let (fd, dir, rel) = op.call(&end)?;
    let listed = sys::list(fd, dir, rel).map_err(|error| op.failure(&end, error))?;
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
        return Err(not_utf8(&bytes, &end.target, "holds an entry named"));
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
            string(scope, file_kind_name(kind))
                .ok_or(Failure::Thrown)?
                .into(),
        ];
        records.push(
            v8::Object::with_prototype_and_properties(scope, object_prototype, &keys, &values)
                .into(),
        );
    }
    Ok(v8::Array::new_with_elements(scope, &records))
}
