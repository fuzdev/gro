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
//! constructor converts the kind and the target to strings for its
//! message, `kind: target`, and reads its options' `cause`, as
//! `super(``${kind}: ${target}``, options)` does; it takes no message.
//! V8 reads `new.target`'s `prototype` for the receiver it makes for any
//! native constructor, before the callback runs, so a proxy given as
//! `new.target` has that read once more, and first.
//! The guard reads only data properties, and takes a proxy, or an accessor
//! on the way, for no match, where the Node guard runs the proxy's traps
//! and the getter.
//!
//! Where it comes from: the realm's `Error.prototype` is the prototype of
//! an error V8 makes, which no program can replace, and the realm's `Error`,
//! which the class extends, is handed over from `primordials`, taken before
//! any program ran, by the implementation of a row whose namespace or root
//! the host makes (the error row's, and a row whose ops throw a family's
//! errors): the host has no native way to it that a program can't have
//! replaced, and taking it at every start would cost every start. The first
//! handed over is held for the run ([`ErrorClasses::hold_error`]), once it's
//! checked to be a function whose own `prototype` is the realm's
//! `Error.prototype`, so an op that throws a family's error finds it,
//! whichever row's import came first, and an op is reached only through a
//! row whose import handed it over. Nothing of the class is in the snapshot,
//! and a start that needs none makes none. A class is put in
//! [`ErrorClasses`] only once it's whole, so an implementation ended early
//! while the class was being made finds none, and makes it again, and one
//! that ran once finds it, as an instance per origin will.
//!
//! A family is one of [`FAMILIES`], which a class's native functions find
//! their own in by name: making the class of a family missing from it
//! throws, so it never makes a class whose constructor would return a plain
//! object.

use std::cell::RefCell;

use deno_core::v8;

use super::{Thrown, name, own_data, throw_error, throw_range_error, throw_type_error};

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

/// What an op that throws a family's error throws when the run holds no
/// realm `Error` to make the class from.
const NO_REALM_ERROR: &str = "the host holds no realm Error to make an error class from: \
                              no row whose ops throw one handed it over";

/// What a family's class's native function throws when its family isn't
/// one of [`FAMILIES`], naming it.
fn missing_family(name: &str) -> String {
    format!("the host has no error family {name}: a family's class is one of the host's table")
}

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

/// The error classes made in a run, and the realm's `Error` they extend,
/// put in its `OpState` when it starts, empty: the `Error` joins when it's
/// first handed over, and a class once it's whole.
#[derive(Default)]
pub struct ErrorClasses {
    /// The realm's `Error`, once a row's implementation handed it over.
    error: RefCell<Option<v8::Global<v8::Function>>>,
    /// The classes made.
    made: RefCell<Vec<Made>>,
}

impl ErrorClasses {
    /// How many families' classes the run has made.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.made.borrow().len()
    }

    /// The realm's `Error`: the one the run holds, or `error`, a value a
    /// row's implementation handed over from `primordials`, held now if
    /// it's a function, not a proxy, whose own data `prototype` is the
    /// realm's `Error.prototype`, which a script handing over the wrong
    /// intrinsic fails. Later values are ignored: the first held is the
    /// run's. `None` with a `TypeError` thrown when the run holds none and
    /// `error` isn't that, and with what V8 threw when a read failed.
    pub fn hold_error<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        error: v8::Local<'s, v8::Value>,
    ) -> Option<v8::Local<'s, v8::Function>> {
        if let Some(held) = &*self.error.borrow() {
            return Some(v8::Local::new(scope, held));
        }
        let function = v8::Local::<v8::Function>::try_from(error)
            .ok()
            .filter(|_| !error.is_proxy());
        let is_error = match function {
            Some(function) => {
                let intrinsic = error_prototype(scope)?;
                let prototype = own_data(scope, function.into(), name(scope, "prototype")?).ok()?;
                prototype.is_some_and(|prototype| prototype.strict_equals(intrinsic))
            }
            None => false,
        };
        let Some(function) = function.filter(|_| is_error) else {
            throw_type_error(scope, "an error class extends the realm's Error");
            return None;
        };
        *self.error.borrow_mut() = Some(v8::Global::new(scope, function));
        Some(function)
    }

    /// The class and guard of `family`, if the run made them.
    fn find<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: &ErrorFamily,
    ) -> Option<(v8::Local<'s, v8::Function>, v8::Local<'s, v8::Function>)> {
        let made = self.made.borrow();
        let made = made.iter().find(|made| made.family == family)?;
        Some((
            v8::Local::new(scope, &made.class),
            v8::Local::new(scope, &made.guard),
        ))
    }

    /// The class and guard of `family`: the run's, made now if it has
    /// none, extending `error`, the realm's `Error`. `None` when making
    /// them threw, the exception pending.
    fn get_or_make<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: &'static ErrorFamily,
        error: v8::Local<'s, v8::Function>,
    ) -> Option<(v8::Local<'s, v8::Function>, v8::Local<'s, v8::Function>)> {
        if let Some(made) = self.find(scope, family) {
            return Some(made);
        }
        let (class, guard) = make(scope, family, error)?;
        self.made.borrow_mut().push(Made {
            family,
            class: v8::Global::new(scope, class),
            guard: v8::Global::new(scope, guard),
        });
        Some((class, guard))
    }

    /// Throws a new error of `family` through `scope`, of `kind`, about
    /// `target`: what `new Class(kind, target)` makes, its message `kind:
    /// target`, made by the family's class, which is made now if the run
    /// has none, extending the realm's `Error` the run holds. Runs no code
    /// of a caller's: both are strings, and the class's own `prototype` is
    /// a data property no program can change. With no `Error` held, which
    /// no op reached through a row that hands it over meets, it throws an
    /// `Error` saying so instead.
    pub fn throw<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: &'static ErrorFamily,
        kind: &str,
        target: v8::Local<'s, v8::String>,
    ) {
        let Some(error) = self
            .error
            .borrow()
            .as_ref()
            .map(|error| v8::Local::new(scope, error))
        else {
            throw_error(scope, NO_REALM_ERROR);
            return;
        };
        let Some((class, _)) = self.get_or_make(scope, family, error) else {
            return;
        };
        let Some(kind) = v8::String::new(scope, kind) else {
            return;
        };
        if let Some(instance) = class.new_instance(scope, &[kind.into(), target.into()]) {
            scope.throw_exception(instance.into());
        }
    }

    /// The namespace of `family`'s error row: a new null-prototype record
    /// of its class, its guard, and the `default` every builtin has,
    /// `undefined`, the class and guard made now if the run has none,
    /// extending the realm's `Error`, which the row's implementation hands
    /// over as `error` ([`Self::hold_error`]). `None` when making them
    /// threw, the exception pending.
    pub fn namespace<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: &'static ErrorFamily,
        error: v8::Local<'s, v8::Value>,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let error = self.hold_error(scope, error)?;
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

/// The realm's `Error.prototype`, the prototype of an error V8 makes,
/// never a property a program can have replaced.
fn error_prototype<'s>(scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
    let empty = v8::String::empty(scope);
    let made = v8::Local::<v8::Object>::try_from(v8::Exception::error(scope, empty)).ok()?;
    made.get_prototype(scope)
}

/// Makes `family`'s class, extending `error`, the realm's `Error`, and its
/// guard. `None` when it threw, the exception pending: an `Error` when
/// `family` isn't one of [`FAMILIES`], whose class's functions couldn't find
/// it.
fn make<'s>(
    scope: &v8::PinScope<'s, '_>,
    family: &'static ErrorFamily,
    error: v8::Local<'s, v8::Function>,
) -> Option<(v8::Local<'s, v8::Function>, v8::Local<'s, v8::Function>)> {
    if !FAMILIES.contains(&family) {
        throw_error(scope, &missing_family(family.name));
        return None;
    }
    let data: v8::Local<'s, v8::Value> = v8::String::new(scope, family.name)?.into();
    let template = v8::FunctionTemplate::builder(construct)
        .data(data)
        .length(3)
        .build(scope);
    template.set_class_name(v8::String::new(scope, family.name)?);
    let class = template.get_function(scope)?;
    let prototype = class.get(scope, name(scope, "prototype")?.into())?;
    let prototype = v8::Local::<v8::Object>::try_from(prototype).ok()?;
    prototype.set_prototype(scope, error_prototype(scope)?)?;
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

/// A class's constructor, `new Class(kind, target, options?)`, as the Node
/// class constructs: the message `kind: target`, each converted to a
/// string; the prototype `new.target`'s, else the realm's
/// `Error.prototype`; `cause` from the options, when they have one; then
/// its own `name`, `kind`, and target, as the class's fields define them.
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
        throw_error(
            scope,
            &missing_family(&args.data().to_rust_string_lossy(scope)),
        );
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
    let (kind, target, options) = (args.get(0), args.get(1), args.get(2));
    let kind_text = kind.to_string(scope)?;
    let target_text = target.to_string(scope)?;
    let separator = v8::String::new(scope, ": ")?;
    let head = concat(scope, kind_text, separator)?;
    let message = concat(scope, head, target_text)?;
    let prototype = new_target.get(scope, name(scope, "prototype")?.into())?;
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
        throw_error(
            scope,
            &missing_family(&args.data().to_rust_string_lossy(scope)),
        );
        return;
    };
    if let Ok(found) = is_of_family(scope, family, args.get(0), args.get(1)) {
        rv.set_bool(found);
    }
}

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
