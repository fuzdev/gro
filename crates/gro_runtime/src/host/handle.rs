//! Handles: the objects a host module's methods act through, made by the
//! host and branded by it, so a method recognizes one it's called on and
//! refuses anything else.
//!
//! A handle is an ordinary object the host makes of a prototype, the frozen
//! null-prototype record of native methods the host made with the row's
//! namespace (`native.rs`), with the handle's state under a private symbol
//! of its kind, the brand, and then frozen. A private symbol made through
//! V8's API is a key no JS can name, enumerate, or read: `Reflect.ownKeys`
//! doesn't list it, a proxy's traps never see it, and no program can make
//! one. So a look-alike, an object of the same prototype, a copy of a
//! handle's own properties, and a proxy of a handle all lack the brand, and
//! a method refuses each: it checks its receiver is an object, not a proxy,
//! that holds its kind's brand, and reads the state from there, natively,
//! running no code of a caller's, as a member handed another row's handle
//! would check it. A handle's prototype is fixed (it's frozen), so a
//! child handle takes the prototype its parent has.
//!
//! The state is a value of the row's choosing, which no JS ever holds:
//! `env`'s is a record of the names a handle may read and their values,
//! and `blake3`'s hasher's is a number naming its native state, a Rust
//! value in a table of the run's `OpState`, which a weak reference to the
//! handle drops when the handle is collected (`blake3.rs`).
//! A handle of another row's kind is no handle of this one's, and a row
//! that must recognize another's handle (`spawn` taking an `fs` directory)
//! reads it with that row's kind.

use deno_core::v8;

/// A kind of handle: the name its brand, a private symbol, is made for.
/// Each row's own, so no row's member takes another's handle for one of its
/// own.
#[derive(Debug, PartialEq, Eq)]
pub struct HandleKind {
    /// The private symbol's name, in V8's one namespace of API private
    /// symbols, qualified so no other embedder code's collides.
    pub brand: &'static str,
}

/// The env host module's handles.
pub const ENV_HANDLE: HandleKind = HandleKind {
    brand: "@fuzdev/gro/env.ts#handle",
};

/// The blake3 host module's hashers.
pub const BLAKE3_HASHER: HandleKind = HandleKind {
    brand: "@fuzdev/gro/blake3.ts#hasher",
};

/// The brand of `kind`: the same private symbol every time, which V8 keeps
/// for the isolate.
fn brand<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, kind.brand)?;
    Some(v8::Private::for_api(scope, Some(name)))
}

/// Makes a handle of `kind`: a new object whose prototype is `prototype`,
/// holding `state` under the kind's brand, then frozen. `None` when V8
/// failed to make it, an exception pending.
pub fn make<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
    prototype: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let brand = brand(scope, kind)?;
    let handle = v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[]);
    handle.set_private(scope, brand, state)?;
    handle.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    Some(handle)
}

/// A handle of `kind`, as a member is handed it: the object `value` is and
/// the state its brand holds, or `None` when `value` is anything else (not
/// an object, a proxy, or an object without the brand). Reads nothing a
/// caller can answer.
pub fn read<'s>(
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
    value: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Value>)> {
    if value.is_proxy() {
        return None;
    }
    let handle = v8::Local::<v8::Object>::try_from(value).ok()?;
    let brand = brand(scope, kind)?;
    if handle.has_private(scope, brand) != Some(true) {
        return None;
    }
    Some((handle, handle.get_private(scope, brand)?))
}

/// The prototype `handle`, a handle [`read`] returned, was made of, for a
/// child handle to take: fixed, since a handle is frozen.
pub fn prototype_of<'s>(
    scope: &v8::PinScope<'s, '_>,
    handle: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    v8::Local::<v8::Object>::try_from(handle.get_prototype(scope)?).ok()
}
