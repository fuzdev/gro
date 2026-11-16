//! The native half of the env host module: its handles, what each may
//! read, and every check of a call.
//!
//! An env handle is a handle of [`ENV_HANDLE`]'s kind (`handle.rs`),
//! whose state is a record of the variables it may read: their values, a
//! frozen null-prototype record of the ones set, which `get_all` returns,
//! and its names, the set it may ask for, or none for every name, which
//! only an unconfined run's root has. Neither record is ever any other
//! object's, and no JS but `get_all`'s caller holds the first, so a read
//! of either runs no code of a caller's. Every method decides from the
//! handle it's called on, never from the run's grants: the root is the one
//! handle made from them, a value of the row's namespace made with it
//! (`native.rs`), and every other is narrowed from a handle. The methods
//! are native members, on the prototype the host makes with the namespace.
//!
//! A name is checked as the Node module checks it, in the same order and
//! with the same messages: a string, not empty, no NUL, well-formed
//! Unicode, no `=`, and, for `narrow`, no `*`. A name outside a handle's
//! set throws `EnvError` `not_granted`, whose message names the variable
//! and never a value; an unset name in it reads as `undefined`. An
//! argument is read as the value it is: a name is a string, never
//! converted, and `narrow`'s names an array whose elements are its own
//! data, so an accessor or a proxy reads as no name, where the Node module
//! would run it.

use deno_core::{OpState, v8};

use super::error_class::{DeclaredFamily, ErrorClasses};
use super::handle::{self, ENV_HANDLE};
use super::{HostData, native, own_data, string, throw_error, throw_type_error};
use crate::loader::RefusalLog;
use crate::resolver::Capability;
use crate::{Authority, EnvNameFault, Grants, NeededGrant, RefusedBy, env_name_fault};

/// What a method called on anything but an env handle throws.
const NOT_A_HANDLE: &str = "env's methods are called on an env handle, not another object";

/// What `narrow` throws for names that aren't an array.
const NOT_AN_ARRAY: &str = "env.narrow takes an array of names";

/// What making the root throws in a runtime whose authority doesn't grant
/// `env`, which the root's check refused already.
const NOT_GRANTED_ENV: &str = "env's root is made only in a run that grants env";

/// The env namespace's specifier, which holds the root as its `env`.
const MODULE: &str = "@fuzdev/gro/env.ts";

/// The root handle, made of `prototype`, the env handles' prototype the
/// host is making the namespace with, from the run's grant: the names it
/// holds, every name in an unconfined run, and the values the embedder read
/// of them. The namespace holds it, so every importing origin the run's
/// grant reaches shares it: the project's in a confined run, and every
/// origin in an unconfined one. `None` with an exception pending when a
/// step failed.
pub fn root<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let granted = match state.try_borrow::<Authority>() {
        Some(Authority::Unconfined) => None,
        Some(Authority::Confined(Grants { env: Some(env), .. })) => Some(&env.names),
        _ => {
            throw_error(scope, NOT_GRANTED_ENV);
            return None;
        }
    };
    let names = match granted {
        None => None,
        Some(granted) => {
            let names = record(scope);
            let granted_name: v8::Local<v8::Value> = v8::Boolean::new(scope, true).into();
            for name in granted {
                names.create_data_property(scope, string(scope, name)?.into(), granted_name)?;
            }
            Some(names)
        }
    };
    let vars = record(scope);
    for (name, value) in state.borrow::<HostData>().env.iter() {
        if granted.is_none_or(|granted| granted.contains(name)) {
            let value = string(scope, value)?;
            vars.create_data_property(scope, string(scope, name)?.into(), value.into())?;
        }
    }
    make(scope, prototype, vars, names)
}

/// `env.get(name)` called on `receiver`: the value of the variable `name`,
/// or `undefined` when it's unset; `errors`, `EnvError`'s family as the
/// member declares it, for a name the handle may not read. `None` with the
/// exception pending when it throws.
pub fn get<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    name: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let handle = read(scope, receiver)?;
    let name = check_name(scope, name, "an environment name", true)?;
    check_granted(state, scope, &handle, name, errors)?;
    let vars = handle.vars;
    if vars.has_own_property(scope, name.into())? {
        vars.get(scope, name.into())
    } else {
        Some(v8::undefined(scope).into())
    }
}

/// `env.get_all()` called on `receiver`: every variable the handle may read
/// that's set, the frozen null-prototype record it holds, the same one
/// each call. `None` with the exception pending when it throws.
pub fn get_all<'s>(
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    Some(read(scope, receiver)?.vars.into())
}

/// `env.narrow(names)` called on `receiver`: a new handle of the receiver's
/// prototype that may read exactly `names`, each checked in order, and each
/// one the receiver may read, or `not_granted` is thrown for the first that
/// isn't, of `errors`, as [`get`] throws it. A name given twice is one
/// name. `None` with the exception pending when it throws.
pub fn narrow<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
    names: v8::Local<'s, v8::Value>,
    errors: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let parent = read(scope, receiver)?;
    let Some(list) = v8::Local::<v8::Array>::try_from(names)
        .ok()
        .filter(|_| !names.is_proxy())
    else {
        throw_type_error(scope, NOT_AN_ARRAY);
        return None;
    };
    let vars = record(scope);
    let allowed = record(scope);
    let granted: v8::Local<v8::Value> = v8::Boolean::new(scope, true).into();
    for index in 0..list.length() {
        let key = string(scope, &index.to_string())?;
        let element = own_data(scope, list.into(), key.into()).ok()?;
        let element = element.unwrap_or_else(|| v8::undefined(scope).into());
        let name = check_name(scope, element, "a `narrow` name", false)?;
        check_granted(state, scope, &parent, name, errors)?;
        allowed.create_data_property(scope, name.into(), granted)?;
        if parent.vars.has_own_property(scope, name.into())? {
            let value = parent.vars.get(scope, name.into())?;
            vars.create_data_property(scope, name.into(), value)?;
        }
    }
    let prototype = handle::prototype_of(scope, parent.object)?;
    make(scope, prototype, vars, Some(allowed)).map(Into::into)
}

/// An env handle, as an op reads the one it's handed.
struct Handle<'s> {
    /// The handle.
    object: v8::Local<'s, v8::Object>,
    /// The variables it may read that are set, by name.
    vars: v8::Local<'s, v8::Object>,
    /// The names it may read, each `true`, or `None` for every name.
    names: Option<v8::Local<'s, v8::Object>>,
}

/// The handle `receiver` is, or `None` with a `TypeError` thrown when it's
/// no env handle.
fn read<'s>(
    scope: &v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Value>,
) -> Option<Handle<'s>> {
    let Some((handle, held)) = handle::read(scope, &ENV_HANDLE, receiver) else {
        throw_type_error(scope, NOT_A_HANDLE);
        return None;
    };
    // the host's own record, which no JS holds
    let held = v8::Local::<v8::Object>::try_from(held).ok()?;
    let vars = held.get(scope, string(scope, VARS)?.into())?;
    let names = held.get(scope, string(scope, NAMES)?.into())?;
    Some(Handle {
        object: handle,
        vars: v8::Local::<v8::Object>::try_from(vars).ok()?,
        names: v8::Local::<v8::Object>::try_from(names).ok(),
    })
}

/// The key of a handle's state that holds its values.
const VARS: &str = "vars";

/// The key of a handle's state that holds its names, `null` for every
/// name.
const NAMES: &str = "names";

/// Makes a handle of `prototype` that may read the names of `names`, every
/// name when `None`, whose set values are `vars`; both records frozen.
fn make<'s>(
    scope: &v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    vars: v8::Local<'s, v8::Object>,
    names: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    vars.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    let names: v8::Local<v8::Value> = match names {
        Some(names) => {
            names.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
            names.into()
        }
        None => v8::null(scope).into(),
    };
    let keys = [string(scope, VARS)?.into(), string(scope, NAMES)?.into()];
    let null = v8::null(scope).into();
    let held = v8::Object::with_prototype_and_properties(scope, null, &keys, &[vars.into(), names]);
    held.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    handle::make(scope, &ENV_HANDLE, prototype, held.into(), &[])
}

/// Throws `not_granted` of `errors`, `EnvError`'s family, for `name` when
/// `handle`'s set lacks it, and records the refusal in the run's
/// [`RefusalLog`], which fails the run, or, in a runtime with no record,
/// throws an `Error` saying so; every name is granted when the handle has no
/// set. `None` with the exception pending when it threw.
///
/// A refusal by the root, the handle made from the run's grant, is
/// recorded with the grant that would have allowed it, the name for
/// `env`: in a confined run the root holds the names `--allow-env=` gave
/// the project, so granting the name lets the call through, whichever
/// module made it, since a module holding the root was handed it by the
/// project, the one origin the grant reaches. A handle narrowed from
/// another refuses for the program's own narrowing, which no grant widens,
/// so its refusal needs none; nor does any in an unconfined run, whose
/// root holds every name.
fn check_granted<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    handle: &Handle<'s>,
    name: v8::Local<'s, v8::String>,
    errors: DeclaredFamily,
) -> Option<()> {
    let Some(names) = handle.names else {
        return Some(());
    };
    if names.has_own_property(scope, name.into())? {
        return Some(());
    }
    // a run's start puts both before anything runs, so only a runtime made
    // outside one lacks them, and a refusal there fails, never passes
    // unrecorded
    let (Some(classes), Some(log)) = (
        state.try_borrow::<ErrorClasses>(),
        state.try_borrow::<RefusalLog>(),
    ) else {
        throw_error(scope, NO_REFUSAL_STATE);
        return None;
    };
    // read before the throw, as no call into V8 is made with an exception
    // pending
    let root = is_root(state, scope, handle.object);
    let text = name.to_rust_string_lossy(scope);
    classes.throw(scope, errors, NOT_GRANTED, name, None);
    // recorded as it's thrown, so the run fails whatever the program does
    // with the error; the name, never a value
    let message = format!("{NOT_GRANTED}: {text}");
    let needed = root.then_some(NeededGrant::Env(text));
    log.record(
        RefusedBy::Grant {
            grant: Capability::Env,
            error: errors.family().name,
        },
        &message,
        needed,
    );
    None
}

/// Whether `object` is the run's root handle, the one made from its grant,
/// which the namespace the run holds holds as its `env`.
fn is_root(
    state: &OpState,
    scope: &v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> bool {
    native::is_held_export(state, scope, MODULE, "env", object.into())
}

/// What a refused name throws in a runtime started without the run's error
/// classes or its record of refusals, which only a runtime made outside a
/// run's start is: a refusal that couldn't fail the run is an error itself.
const NO_REFUSAL_STATE: &str = "env refused a name, but this runtime was started without the \
                                run's record of refusals, so the refusal can't fail the run";

/// The kind of the error a name outside a handle's set throws.
const NOT_GRANTED: &str = "not_granted";

/// `value` as a variable's name, checked as the Node module checks one,
/// `what` naming it in a refusal's message, a `*` refused unless `glob`.
/// `None` with a `TypeError` thrown when it's no name: one that holds a
/// `=` isn't echoed, since what follows it can be a value.
fn check_name<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    what: &str,
    glob: bool,
) -> Option<v8::Local<'s, v8::String>> {
    let Ok(name) = v8::Local::<v8::String>::try_from(value) else {
        throw_type_error(scope, &format!("{what} must be a string"));
        return None;
    };
    let mut units = vec![0; name.length()];
    name.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    let fault = if units.is_empty() {
        Some("is empty")
    } else if units.contains(&0) {
        Some("holds a NUL byte")
    } else {
        None
    };
    let text = match (fault, String::from_utf16(&units)) {
        (Some(fault), _) => {
            throw_type_error(scope, &format!("{what} {fault}"));
            return None;
        }
        (None, Err(_)) => {
            throw_type_error(scope, &format!("{what} isn't well-formed Unicode"));
            return None;
        }
        (None, Ok(text)) => text,
    };
    let message = match env_name_fault(&text, glob) {
        None => return Some(name),
        Some(EnvNameFault::Empty) => format!("{what} is empty"),
        Some(EnvNameFault::Nul) => format!("{what} holds a NUL byte"),
        Some(EnvNameFault::Equals) => format!("{what} holds a `=`"),
        Some(EnvNameFault::Glob) => format!("a `narrow` name is exact, never a glob: {text}"),
    };
    throw_type_error(scope, &message);
    None
}

/// A new, empty null-prototype record.
fn record<'s>(scope: &v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let null = v8::null(scope).into();
    v8::Object::with_prototype_and_properties(scope, null, &[], &[])
}
