//! The native half of the blake3 host module: BLAKE3 over the bytes a call
//! is handed, by the `blake3` crate, and the hashers, the first handles
//! whose state is native Rust.
//!
//! A member reads its arguments as the values they are: bytes are a
//! `Uint8Array`, its subclasses included, never a proxy, an array of
//! numbers, a string, or another view, whose contents it reads natively,
//! running no code of a caller's; a context is a string, encoded as UTF-8
//! with each lone surrogate replaced, as `TextEncoder` encodes it; a key is
//! 32 bytes. A view's contents are read as they are when the call runs: a
//! view of a detached buffer is a `TypeError`, as it is to the WASM package
//! on Node, one tracking a resizable buffer's length is read to the length
//! it has, and one of a `SharedArrayBuffer` is read as any other, since no
//! other thread runs code of the run's. A fixed-length view a resize left
//! out of its buffer's bounds reads as empty: V8's API reports a view's
//! length as zero then and says nothing of its bounds.
//!
//! The module's members are native (`native.rs`): `hash`, the three
//! factories, and the hasher's `update` and `finalize`, each declared in
//! `host/mod.rs` with what it does here.
//!
//! A hasher is a handle of [`BLAKE3_HASHER`]'s kind (`handle.rs`), made of
//! the prototype the host made with the module's namespace, whose state is
//! a number naming its `blake3::Hasher` in the run's [`NativeTable`], in
//! `OpState` ([`Blake3State`]), so a hasher lives as long as its handle can
//! be reached, and no longer, with no call to end it: the module has none.
//!
//! A digest is 32 bytes, a new `Uint8Array` of its own buffer each call.

use std::cell::OnceCell;
use std::rc::Rc;

use deno_core::{OpState, v8};

use super::handle::{self, BLAKE3_HASHER, NativeTable};
use super::{native, throw_error, throw_range_error, throw_type_error};

/// What a member throws for bytes that aren't a `Uint8Array`.
pub const NOT_BYTES: &str = "blake3 hashes a Uint8Array";

/// What a member throws for bytes or a key in a view of a detached buffer.
pub const DETACHED: &str = "blake3 can't read a view of a detached buffer";

/// What a keyed hasher's factory throws for a key that isn't a
/// `Uint8Array`.
pub const NOT_A_KEY: &str = "a keyed hasher's key is a Uint8Array";

/// What a keyed hasher's factory throws for a key that isn't 32 bytes.
pub const NOT_A_KEY_LENGTH: &str = "a keyed hasher's key is exactly 32 bytes";

/// What a derive-key hasher's factory throws for a context that isn't a
/// string.
pub const NOT_A_CONTEXT: &str = "a derive_key hasher's context is a string";

/// What a hasher's method called on anything but a hasher throws.
pub const NOT_A_HASHER: &str = "a hasher's methods are called on a hasher, not another object";

/// What a factory throws when the run holds no hasher prototype, which
/// only a factory reached some other way than through the namespace meets.
const NO_PROTOTYPE: &str = "this run made no blake3 namespace, so it holds no hasher prototype";

/// What a member throws in a runtime started without the module's state,
/// which only a runtime made outside a run's start is.
const NO_BLAKE3_STATE: &str = "this runtime was started without blake3's state";

/// The length of a key and of a digest, in bytes.
const LENGTH: usize = 32;

/// The blake3 host module's state, put in every run's `OpState` when it
/// starts, empty: the table joins when a run makes its first hasher.
#[derive(Default)]
pub struct Blake3State {
    /// The hashers.
    hashers: OnceCell<Rc<NativeTable<blake3::Hasher>>>,
}

impl Blake3State {
    /// How many hashers the run's table holds.
    #[cfg(test)]
    pub fn hashers_len(&self) -> usize {
        self.hashers.get().map_or(0, |hashers| hashers.len())
    }
}

/// `hash(data)`: the BLAKE3 digest of `data`, or a `TypeError` thrown when
/// it isn't a `Uint8Array`.
pub fn hash<'s>(
    scope: &v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let data = bytes(scope, data, NOT_BYTES)?;
    let mut storage = [0; v8::TYPED_ARRAY_MAX_SIZE_IN_HEAP];
    let digest = blake3::hash(data.get_contents(&mut storage));
    digest_array(scope, digest.as_bytes())
}

/// What a hasher's factory makes it of.
#[derive(Clone, Copy)]
pub enum Mode<'s> {
    /// A plain hasher.
    Plain,
    /// A keyed one, of the key the caller passed.
    Keyed(v8::Local<'s, v8::Value>),
    /// A derive-key one, of the context the caller passed.
    DeriveKey(v8::Local<'s, v8::Value>),
}

/// A new hasher in `mode`, made of the prototype the run made with the
/// namespace: a handle whose state names a new `blake3::Hasher` in the
/// run's table. The argument is checked first, then the run's state.
/// `None` with an exception pending when a step failed.
pub fn hasher<'s>(
    state: &OpState,
    scope: &mut v8::PinScope<'s, '_>,
    mode: Mode<'s>,
) -> Option<v8::Local<'s, v8::Value>> {
    let hasher = match mode {
        Mode::Plain => blake3::Hasher::new(),
        Mode::Keyed(key) => {
            let key = bytes(scope, key, NOT_A_KEY)?;
            let mut storage = [0; v8::TYPED_ARRAY_MAX_SIZE_IN_HEAP];
            let Ok(key) = <&[u8; LENGTH]>::try_from(key.get_contents(&mut storage)) else {
                throw_range_error(scope, NOT_A_KEY_LENGTH);
                return None;
            };
            blake3::Hasher::new_keyed(key)
        }
        Mode::DeriveKey(context) => {
            let Ok(context) = v8::Local::<v8::String>::try_from(context) else {
                throw_type_error(scope, NOT_A_CONTEXT);
                return None;
            };
            blake3::Hasher::new_derive_key(&context.to_rust_string_lossy(scope))
        }
    };
    let Some(blake3) = state.try_borrow::<Blake3State>() else {
        throw_error(scope, NO_BLAKE3_STATE);
        return None;
    };
    let Some(prototype) = native::prototype(state, scope, &BLAKE3_HASHER) else {
        throw_error(scope, NO_PROTOTYPE);
        return None;
    };
    let made =
        blake3
            .hashers
            .get_or_init(Rc::default)
            .make(scope, &BLAKE3_HASHER, prototype, hasher)?;
    Some(made.into())
}

/// `hasher.update(data)` called on `receiver`: `data` added to the
/// hasher's input, or a `TypeError` thrown when the receiver is no hasher
/// or `data` isn't a `Uint8Array`, the receiver checked first.
pub fn update<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
) {
    let Some(id) = read(scope, receiver) else {
        return;
    };
    let Some(data) = bytes(scope, data, NOT_BYTES) else {
        return;
    };
    let mut storage = [0; v8::TYPED_ARRAY_MAX_SIZE_IN_HEAP];
    let data = data.get_contents(&mut storage);
    if with_hasher(state, id, |hasher| {
        hasher.update(data);
    })
    .is_none()
    {
        throw_error(scope, NO_BLAKE3_STATE);
    }
}

/// `hasher.finalize()` called on `receiver`: the digest of the input so
/// far, which leaves the hasher as it was, or a `TypeError` thrown when the
/// receiver is no hasher.
pub fn finalize<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let id = read(scope, receiver)?;
    let Some(digest) = with_hasher(state, id, |hasher| hasher.finalize()) else {
        throw_error(scope, NO_BLAKE3_STATE);
        return None;
    };
    digest_array(scope, digest.as_bytes())
}

/// `step` run on the hasher `id` names in the run's table, `None` when the
/// run holds no such hasher, which only a runtime started without the
/// module's state, or a handle of another runtime's, can be.
fn with_hasher<R>(
    state: &OpState,
    id: u64,
    step: impl FnOnce(&mut blake3::Hasher) -> R,
) -> Option<R> {
    state
        .try_borrow::<Blake3State>()?
        .hashers
        .get()?
        .with(id, step)
}

/// The number the hasher `receiver` is holds, or `None` with a `TypeError`
/// thrown when it's no hasher.
fn read<'s>(scope: &v8::PinScope<'s, '_>, receiver: v8::Local<'s, v8::Value>) -> Option<u64> {
    let id = handle::read_number(scope, &BLAKE3_HASHER, receiver);
    if id.is_none() {
        throw_type_error(scope, NOT_A_HASHER);
    }
    id
}

/// `value` as bytes, or `None` with a `TypeError` of `message` thrown when
/// it isn't a `Uint8Array`: a proxy of one is none, which V8 tells by the
/// value's own type, so reading it runs none of the proxy's traps. A view
/// of a detached buffer is a `TypeError` of [`DETACHED`].
fn bytes<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    message: &str,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let Ok(bytes) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        throw_type_error(scope, message);
        return None;
    };
    // a view with no buffer yet holds its bytes on V8's heap and can't
    // have been detached; asking for its buffer would make one
    if bytes.has_buffer()
        && bytes
            .buffer(scope)
            .is_some_and(|buffer| buffer.was_detached())
    {
        throw_type_error(scope, DETACHED);
        return None;
    }
    Some(bytes)
}

/// `digest` as a new `Uint8Array` of its own buffer. `None` with an
/// exception pending when V8 failed to make it.
fn digest_array<'s>(
    scope: &v8::PinScope<'s, '_>,
    digest: &[u8; LENGTH],
) -> Option<v8::Local<'s, v8::Value>> {
    let buffer = v8::ArrayBuffer::new(scope, LENGTH);
    let store = buffer.get_backing_store();
    for (cell, byte) in store.iter().zip(digest) {
        cell.set(*byte);
    }
    v8::Uint8Array::new(scope, buffer, 0, LENGTH).map(Into::into)
}
