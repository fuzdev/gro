//! The host's error classes: one per family of the host modules' errors,
//! made natively, once a run, the first time anything needs it, and held
//! by the host, so the family's error row exports it and any op can throw
//! it.
//!
//! A class is what the Node module's class is to a program: a frozen
//! function whose instances are real errors (`Error.isError`, a stack,
//! `structuredClone` and JSON as an `Error`'s), each with its own `name`,
//! `kind`, and target, and `cause` when the options give one, whose
//! prototype is frozen and inherits from the realm's `Error.prototype`,
//! and which a class of the program's can extend. Its guard recognizes an
//! instance of it, or of a class of the same name, by its prototype chain,
//! as the Node module's guard does.
//!
//! Neither runs code of a caller's but where the Node class does: the
//! constructor converts the kind, the target, and the message to strings
//! and reads its options' `cause`, as `super(message ?? ``${kind}:
//! ${target}``, options)` does. V8 reads `new.target`'s `prototype` for the
//! receiver it makes for any native constructor, before the callback runs,
//! so a proxy given as `new.target` has that read once more, and first.
//! The guard reads only data properties, and takes a proxy, or an accessor
//! on the way, for no match, where the Node guard runs the proxy's traps
//! and the getter.
//!
//! Where it comes from: the realm's `Error.prototype` is the prototype of
//! an error V8 makes, which no program can replace, and the realm's `Error`,
//! which the class extends, is handed over by the row's implementation from
//! `primordials`, taken before any program ran. Nothing of the class is in
//! the snapshot, and a start that needs none makes none. A class is put in
//! [`ErrorClasses`] only once it's whole, so an implementation cut short
//! while the class was being made finds none, and makes it again, and one
//! that ran once finds it, as an instance per origin will.

use std::cell::RefCell;

use deno_core::v8;

use super::{throw_range_error, throw_type_error};

/// A family of host-module errors, as its class and its guard are named.
#[derive(Debug, PartialEq, Eq)]
pub struct ErrorFamily {
    /// The class's name, `EnvError`.
    pub name: &'static str,
    /// The guard's name, `is_env_error`.
    pub guard: &'static str,
    /// The own property naming what the error is about: `target`, the
    /// variable's name for an `EnvError`.
    pub target: &'static str,
}

/// The env host module's errors: `EnvError`, `is_env_error`.
pub const ENV_ERROR: ErrorFamily = ErrorFamily {
    name: "EnvError",
    guard: "is_env_error",
    target: "target",
};

/// Every family the host makes a class of, which a class's native
/// functions find their own by name in, one for each error row of the
/// builtin table, as a test holds it.
const FAMILIES: [&ErrorFamily; 1] = [&ENV_ERROR];

/// What a class's functions are called with as their data: the family's
/// name, by which they find it in [`FAMILIES`].
fn family_of(
    data: v8::Local<'_, v8::Value>,
    scope: &v8::PinScope<'_, '_>,
) -> Option<&'static ErrorFamily> {
    let name = v8::Local::<v8::String>::try_from(data).ok()?;
    let name = name.to_rust_string_lossy(scope);
    FAMILIES.into_iter().find(|family| family.name == name)
}

/// A family's class and guard, made.
struct Made {
    /// The family.
    family: &'static ErrorFamily,
    /// The class.
    class: v8::Global<v8::Function>,
    /// The guard.
    guard: v8::Global<v8::Function>,
}

/// The error classes made in a run, put in its `OpState` when it starts,
/// empty: a class joins once it's whole.
#[derive(Default)]
pub struct ErrorClasses(RefCell<Vec<Made>>);

impl ErrorClasses {
    /// How many families' classes the run has made.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.borrow().len()
    }

    /// The class and guard of `family`, if the run made them.
    fn find<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: &ErrorFamily,
    ) -> Option<(v8::Local<'s, v8::Function>, v8::Local<'s, v8::Function>)> {
        let made = self.0.borrow();
        let made = made.iter().find(|made| made.family == family)?;
        Some((
            v8::Local::new(scope, &made.class),
            v8::Local::new(scope, &made.guard),
        ))
    }

    /// The class and guard of `family`: the run's, made now if it has
    /// none, `error` the realm's `Error`, which the class extends. `None`
    /// when making them threw, the exception pending.
    fn get_or_make<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: &'static ErrorFamily,
        error: v8::Local<'s, v8::Value>,
    ) -> Option<(v8::Local<'s, v8::Function>, v8::Local<'s, v8::Function>)> {
        if let Some(made) = self.find(scope, family) {
            return Some(made);
        }
        let (class, guard) = make(scope, family, error)?;
        self.0.borrow_mut().push(Made {
            family,
            class: v8::Global::new(scope, class),
            guard: v8::Global::new(scope, guard),
        });
        Some((class, guard))
    }

    /// The namespace of `family`'s error row: a new null-prototype record
    /// of its class, its guard, and the `default` every builtin has,
    /// `undefined`, the class and guard made now if the run has none.
    /// `None` when making them threw, the exception pending.
    pub fn namespace<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: &'static ErrorFamily,
        error: v8::Local<'s, v8::Value>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let (class, guard) = self.get_or_make(scope, family, error)?;
        let names = [
            name(scope, family.name)?,
            name(scope, family.guard)?,
            name(scope, "default")?,
        ];
        let values = [class.into(), guard.into(), v8::undefined(scope).into()];
        let null = v8::null(scope).into();
        Some(v8::Object::with_prototype_and_properties(
            scope, null, &names, &values,
        ))
    }
}

/// `text` as a property name.
fn name<'s>(scope: &v8::PinScope<'s, '_>, text: &str) -> Option<v8::Local<'s, v8::Name>> {
    v8::String::new(scope, text).map(Into::into)
}

/// Makes `family`'s class, extending `error`, which must be a function,
/// and its guard. `None` when it threw, the exception pending.
fn make<'s>(
    scope: &v8::PinScope<'s, '_>,
    family: &'static ErrorFamily,
    error: v8::Local<'s, v8::Value>,
) -> Option<(v8::Local<'s, v8::Function>, v8::Local<'s, v8::Function>)> {
    let Ok(error) = v8::Local::<v8::Function>::try_from(error) else {
        throw_type_error(scope, "an error class extends the realm's Error");
        return None;
    };
    let data: v8::Local<'s, v8::Value> = v8::String::new(scope, family.name)?.into();
    let template = v8::FunctionTemplate::builder(construct)
        .data(data)
        .length(4)
        .build(scope);
    template.set_class_name(v8::String::new(scope, family.name)?);
    let class = template.get_function(scope)?;
    let prototype = class.get(scope, name(scope, "prototype")?.into())?;
    let prototype = v8::Local::<v8::Object>::try_from(prototype).ok()?;
    // the intrinsic, as every error V8 makes has it, never a property a
    // program can have replaced
    let empty = v8::String::empty(scope);
    let made = v8::Local::<v8::Object>::try_from(v8::Exception::error(scope, empty)).ok()?;
    let error_prototype = made.get_prototype(scope)?;
    prototype.set_prototype(scope, error_prototype)?;
    class.set_prototype(scope, error.into())?;
    prototype.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    class.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    let guard = v8::Function::builder(guard)
        .data(data)
        .length(2)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)?;
    guard.set_name(v8::String::new(scope, family.guard)?);
    Some((class, guard))
}

/// A class's constructor, `new Class(kind, target, message?, options?)`,
/// as the Node class constructs: the default message `kind: target`, each
/// converted to a string, when no message is given; the prototype
/// `new.target`'s, else the realm's `Error.prototype`; the message given,
/// converted; `cause` from the options, when they have one; then its own
/// `name`, `kind`, and target, as the class's fields define them.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the signature V8 calls a function's callback with"
)]
fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(family) = family_of(args.data(), scope) else {
        return;
    };
    let Ok(new_target) = v8::Local::<v8::Object>::try_from(args.new_target()) else {
        throw_type_error(
            scope,
            &format!(
                "Class constructor {} cannot be invoked without 'new'",
                family.name
            ),
        );
        return;
    };
    let Some(error) = construct_error(scope, family, new_target, &args) else {
        return;
    };
    rv.set(error.into());
}

/// The error [`construct`] makes, `None` when a step threw.
fn construct_error<'s>(
    scope: &v8::PinScope<'s, '_>,
    family: &ErrorFamily,
    new_target: v8::Local<'s, v8::Object>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let (kind, target, message, options) = (args.get(0), args.get(1), args.get(2), args.get(3));
    let default = if message.is_null_or_undefined() {
        let kind_text = kind.to_string(scope)?;
        let target_text = target.to_string(scope)?;
        let separator = v8::String::new(scope, ": ")?;
        let head = concat(scope, kind_text, separator)?;
        Some(concat(scope, head, target_text)?)
    } else {
        None
    };
    let prototype = new_target.get(scope, name(scope, "prototype")?.into())?;
    let message = match default {
        Some(message) => message,
        None => message.to_string(scope)?,
    };
    let error = v8::Local::<v8::Object>::try_from(v8::Exception::error(scope, message)).ok()?;
    if prototype.is_object() {
        error.set_prototype(scope, prototype)?;
    }
    if let Ok(options) = v8::Local::<v8::Object>::try_from(options) {
        let cause = name(scope, "cause")?;
        if options.has(scope, cause.into())? {
            let value = options.get(scope, cause.into())?;
            error.define_own_property(scope, cause, value, v8::PropertyAttribute::DONT_ENUM)?;
        }
    }
    let class_name = v8::String::new(scope, family.name)?.into();
    for (key, value) in [
        ("name", class_name),
        ("kind", kind),
        (family.target, target),
    ] {
        error.create_data_property(scope, name(scope, key)?, value)?;
    }
    Some(error)
}

/// `left` and `right` joined, or `None` with a `RangeError` thrown, as a
/// template literal throws, when the two are longer than V8's longest
/// string, for which V8 throws nothing.
fn concat<'s>(
    scope: &v8::PinScope<'s, '_>,
    left: v8::Local<'s, v8::String>,
    right: v8::Local<'s, v8::String>,
) -> Option<v8::Local<'s, v8::String>> {
    let joined = v8::String::concat(scope, left, right);
    if joined.is_none() {
        throw_range_error(scope, "Invalid string length");
    }
    joined
}

/// A class's guard, `is_error(error, kind?)`: whether `error` is an
/// instance of a class of the family's name, by its prototype chain, and,
/// given a `kind`, of that kind.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the signature V8 calls a function's callback with"
)]
fn guard<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(family) = family_of(args.data(), scope) else {
        return;
    };
    if let Ok(found) = is_of_family(scope, family, args.get(0), args.get(1)) {
        rv.set_bool(found);
    }
}

/// A read that threw, its exception pending.
struct Thrown;

/// Whether `value` is an error of `family`, of `kind` unless it's
/// `undefined`, as [`guard`] answers, reading only data properties.
fn is_of_family(
    scope: &v8::PinScope<'_, '_>,
    family: &ErrorFamily,
    value: v8::Local<'_, v8::Value>,
    kind: v8::Local<'_, v8::Value>,
) -> Result<bool, Thrown> {
    if value.is_proxy() || value.is_function() {
        return Ok(false);
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(false);
    };
    let constructor_key = name(scope, "constructor").ok_or(Thrown)?;
    let name_key = name(scope, "name").ok_or(Thrown)?;
    let mut next = object.get_prototype(scope);
    while let Some(prototype) = next {
        let Ok(prototype) = v8::Local::<v8::Object>::try_from(prototype) else {
            break;
        };
        if prototype.is_proxy() {
            return Ok(false);
        }
        if let Some(constructor) = own_data(scope, prototype, constructor_key)?
            && let Ok(constructor) = v8::Local::<v8::Object>::try_from(constructor)
            && let Some(class_name) = data_property(scope, constructor, name_key)?
            && class_name.is_string()
            && class_name.to_rust_string_lossy(scope) == family.name
        {
            if kind.is_undefined() {
                return Ok(true);
            }
            let kind_key = name(scope, "kind").ok_or(Thrown)?;
            let own_kind = data_property(scope, object, kind_key)?;
            return Ok(own_kind.is_some_and(|own_kind| own_kind.strict_equals(kind)));
        }
        next = prototype.get_prototype(scope);
    }
    Ok(false)
}

/// The value of `object`'s own data property `key`, `None` when it has
/// none, or an accessor, whose getter isn't run, or is a proxy.
fn own_data<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
) -> Result<Option<v8::Local<'s, v8::Value>>, Thrown> {
    if object.is_proxy() {
        return Ok(None);
    }
    let descriptor = object
        .get_own_property_descriptor(scope, key)
        .ok_or(Thrown)?;
    // a descriptor V8 made is an ordinary object with its fields as its own
    // data properties, so reading the one present reads nothing else
    let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) else {
        return Ok(None);
    };
    let value_key = name(scope, "value").ok_or(Thrown)?;
    if !descriptor
        .has_own_property(scope, value_key)
        .ok_or(Thrown)?
    {
        return Ok(None);
    }
    descriptor
        .get(scope, value_key.into())
        .map(Some)
        .ok_or(Thrown)
}

/// The value of `object`'s data property `key`, its own or the nearest
/// prototype's, as a read of it finds it: `None` when there's none, when
/// the one found is an accessor, or when a proxy is on the way, none of
/// which is run.
fn data_property<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
) -> Result<Option<v8::Local<'s, v8::Value>>, Thrown> {
    let mut next = Some(object);
    while let Some(object) = next {
        if object.is_proxy() {
            return Ok(None);
        }
        if object.has_own_property(scope, key).ok_or(Thrown)? {
            return own_data(scope, object, key);
        }
        next = object
            .get_prototype(scope)
            .and_then(|prototype| v8::Local::<v8::Object>::try_from(prototype).ok());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::FAMILIES;
    use crate::resolver::BUILTINS;

    /// Every error row's class and guard are a family of [`FAMILIES`], in
    /// the table's order, and every family is a row's: the class of a
    /// family missing from it would construct a plain object, and its
    /// guard return `undefined`.
    #[test]
    fn the_families_are_the_error_rows() {
        let rows: Vec<&[&str]> = BUILTINS
            .iter()
            .filter(|row| row.specifier.ends_with("_error.ts"))
            .map(|row| row.exports)
            .collect();
        let families: Vec<[&str; 2]> = FAMILIES
            .iter()
            .map(|family| [family.name, family.guard])
            .collect();
        assert!(!rows.is_empty(), "the table has an error row");
        assert_eq!(families, rows);
    }
}
