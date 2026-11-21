//! The host's error classes: one per family of the host modules' errors,
//! made natively, once a run, the first time anything needs it, and held
//! by the host, so the family's error row exports it and any native member
//! can throw it.
//!
//! A class is what the Node module's class is to a program: a frozen
//! function whose instances are real errors (`Error.isError`, a stack,
//! `structuredClone` and JSON as an `Error`'s), each with its own `name`,
//! `kind`, and target, and `cause` when the options give one, whose
//! prototype is frozen and inherits from the realm's `Error.prototype`,
//! and which a class of the program's can extend. Its guard, a native
//! member of the error row (`native.rs`), recognizes an instance of it, or
//! of a class of the same name, by its prototype chain, as the Node
//! module's guard does ([`is_of_family`]). Each instance the constructor
//! makes, a subclass's included, holds the family's brand, a private
//! symbol no program can name or forge, as the Node class's private field
//! is, which the guard of the family's own errors (`is_own_fs_error`)
//! reads ([`is_own`]).
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
//! The constructor checks no grant, where every native member checks its
//! row's: it reads none of the run's state and reaches nothing a grant
//! guards, as the Node class doesn't, and a runtime with no authority,
//! eval's, never makes a class, since its start puts no [`ErrorClasses`]
//! and the namespace op refuses it before reading them.
//!
//! Where it comes from: the realm's `Error.prototype` is the prototype of
//! an error V8 makes, which no program can replace, and the realm's `Error`,
//! which the class extends, is handed over from `primordials`, taken before
//! any program ran, by the backing script of each row with a member that
//! declares a family (`native_members!`'s `throws` for a member that throws
//! the family's error, `class` for the error row's class), and of no other
//! row ([`hands_over_error`](super::native::hands_over_error)). The host has
//! no native way to it that a program can't have replaced, and taking it at
//! every start would cost every start; and reading it costs a row's first
//! import, so a row whose members declare no family hands over nothing.
//! The first handed over is held for the run ([`ErrorClasses::hand_over`]),
//! and checked to be a function whose own `prototype` is the realm's
//! `Error.prototype` the first time a class is made of it, so a run that
//! makes none never pays for the check. Throwing a family's error and
//! making its class each take a [`DeclaredFamily`], which only
//! `native_members!` makes, from a member's declaration, so a member throws
//! no family it didn't declare, and finds the `Error` held, whichever row
//! was imported first: it's reached only through its row's namespace, or a
//! handle of a prototype made with it, and its row's import handed the
//! `Error` over. Nothing of the class is in the snapshot, and a start that
//! needs none makes none. A class is put in [`ErrorClasses`] only once it's
//! whole, so a first import ended early while the class was being made
//! finds none, and makes it again, and one that ran once finds it, as an
//! instance per origin will.
//!
//! A family is one of [`FAMILIES`], which a class's native functions find
//! their own in by name: making the class of a family missing from it
//! throws, so it never makes a class whose constructor would return a plain
//! object. A family is declared once as its class, by its error row's value
//! (`value EnvError(state; scope) for "@fuzdev/gro/env_error.ts" class
//! ENV_ERROR(family)`), and as thrown by each member that throws it (`fn
//! get(..) for "@fuzdev/gro/env.ts" on ENV_HANDLE(handle) throws
//! ENV_ERROR(errors)`), a member of another row's among them, as `fs`'s
//! and `watch`'s members will each declare `FsError`'s.

use std::cell::RefCell;

use deno_core::{OpState, v8};

use super::{
    NO_ERROR_CLASSES, Thrown, name, own_data, throw_error, throw_range_error, throw_type_error,
};

/// A family of host-module errors, as its class is named.
#[derive(Debug, PartialEq, Eq)]
pub struct ErrorFamily {
    /// The class's name, `EnvError`.
    pub name: &'static str,
    /// The own property naming what the error is about: `target`, the
    /// variable's name for an `EnvError`, `path` for an `FsError`.
    pub target: &'static str,
    /// The name of the private symbol each instance holds, the family's
    /// brand: its error row's specifier, `#`, and the class's name.
    pub brand: &'static str,
}

/// The env host module's errors: `EnvError`, which `env`'s handles throw.
pub const ENV_ERROR: ErrorFamily = ErrorFamily {
    name: "EnvError",
    target: "target",
    brand: "@fuzdev/gro/env_error.ts#EnvError",
};

/// The fs host module's errors: `FsError`, which `fs`'s functions and
/// handles throw.
pub const FS_ERROR: ErrorFamily = ErrorFamily {
    name: "FsError",
    target: "path",
    brand: "@fuzdev/gro/fs_error.ts#FsError",
};

/// The spawn host module's errors: `SpawnError`, which `spawn`'s handles
/// throw.
pub const SPAWN_ERROR: ErrorFamily = ErrorFamily {
    name: "SpawnError",
    target: "program",
    brand: "@fuzdev/gro/spawn_error.ts#SpawnError",
};

/// The net host module's errors: `NetError`, which the module throws.
pub const NET_ERROR: ErrorFamily = ErrorFamily {
    name: "NetError",
    target: "target",
    brand: "@fuzdev/gro/net_error.ts#NetError",
};

/// The stdio host modules' errors: `StdioError`, which their handles
/// throw.
pub const STDIO_ERROR: ErrorFamily = ErrorFamily {
    name: "StdioError",
    target: "target",
    brand: "@fuzdev/gro/stdio_error.ts#StdioError",
};

/// A family a native member declares, `throws FAMILY(name)` or, for an
/// error row's class, `class FAMILY(name)` in `native_members!`, handed to
/// the member's body under that name: what throwing the family's error
/// ([`ErrorClasses::throw`]) and making its class ([`ErrorClasses::class`])
/// take. The macro alone makes one, from the declaration, which the native
/// members' table lists, so a member throws no family it didn't declare,
/// and every row with a member that throws one hands the realm's `Error`
/// over ([`hands_over_error`](super::native::hands_over_error)). Nothing at
/// compile time keeps the maker to the macro: a test reads the source for
/// any other call of it.
#[derive(Debug, Clone, Copy)]
pub struct DeclaredFamily(&'static ErrorFamily);

impl DeclaredFamily {
    /// The family a member's declaration names, made by `native_members!`
    /// alone.
    pub const fn of_member_declaration(family: &'static ErrorFamily) -> Self {
        Self(family)
    }

    /// `family`, for a test that throws or makes a class outside a member.
    #[cfg(test)]
    pub const fn for_test(family: &'static ErrorFamily) -> Self {
        Self(family)
    }

    /// The family declared.
    pub const fn family(self) -> &'static ErrorFamily {
        self.0
    }
}

/// Every family the host makes a class of, which a class's native
/// functions find their own by name in, one for each error row of the
/// builtin table, as a test holds it.
const FAMILIES: [&ErrorFamily; 5] = [
    &ENV_ERROR,
    &FS_ERROR,
    &SPAWN_ERROR,
    &STDIO_ERROR,
    &NET_ERROR,
];

/// What a member that needs a family's class throws when the run holds
/// no realm `Error` to make the class from.
const NO_REALM_ERROR: &str = "the host holds no realm Error to make an error class from: \
                              no row's import handed it over";

/// What making a class throws when the value handed over as the realm's
/// `Error` isn't it.
const NOT_REALM_ERROR: &str = "an error class extends the realm's Error";

/// What a family's class's native function throws when its family isn't
/// one of [`FAMILIES`], naming it.
fn missing_family(name: &str) -> String {
    format!("the host has no error family {name}: a family's class is one of the host's table")
}

/// What a class's constructor is called with as its data: the family's
/// name, by which it finds it in [`FAMILIES`].
fn family_of(
    data: v8::Local<'_, v8::Value>,
    scope: &v8::PinScope<'_, '_>,
) -> Option<&'static ErrorFamily> {
    let name = v8::Local::<v8::String>::try_from(data).ok()?;
    let name = name.to_rust_string_lossy(scope);
    FAMILIES.into_iter().find(|family| family.name == name)
}

/// A family's class, made.
struct Made {
    /// The family.
    family: &'static ErrorFamily,
    /// The class.
    class: v8::Global<v8::Function>,
}

/// The realm's `Error` as the run holds it.
enum RealmError {
    /// Handed over by a row's backing script, not yet checked.
    Handed(v8::Global<v8::Value>),
    /// Checked, when a class was first made of it.
    Checked(v8::Global<v8::Function>),
}

/// The error classes made in a run, and the realm's `Error` they extend,
/// put in its `OpState` when it starts, empty: the `Error` joins when it's
/// first handed over, and a class once it's whole.
#[derive(Default)]
pub struct ErrorClasses {
    /// The realm's `Error`, once a row's backing script handed it over.
    error: RefCell<Option<RealmError>>,
    /// The classes made.
    made: RefCell<Vec<Made>>,
}

impl ErrorClasses {
    /// How many families' classes the run has made.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.made.borrow().len()
    }

    /// Whether the run holds a realm's `Error`, checked or not.
    #[cfg(test)]
    pub fn holds_error(&self) -> bool {
        self.error.borrow().is_some()
    }

    /// Holds `error`, a value a row's backing script handed over from
    /// `primordials` as the realm's `Error`, unchecked, when the run holds
    /// none: later values are ignored while one is held. It's checked when
    /// a class is first made of it ([`Self::class`]), so a run that makes
    /// none never pays for the check.
    pub fn hand_over(&self, scope: &v8::PinScope<'_, '_>, error: v8::Local<'_, v8::Value>) {
        let mut held = self.error.borrow_mut();
        if held.is_none() {
            *held = Some(RealmError::Handed(v8::Global::new(scope, error)));
        }
    }

    /// The realm's `Error` the run holds, checked now if it wasn't: a
    /// function, not a proxy, whose own data `prototype` is the realm's
    /// `Error.prototype`, which a script handing over the wrong intrinsic
    /// fails. A value that fails is dropped, so the next import of a row
    /// that hands one over hands over another. `None` with an `Error`
    /// thrown when the run holds none, a `TypeError` when what it holds
    /// isn't that, and what V8 threw when a read failed.
    fn realm_error<'s>(&self, scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Function>> {
        let handed = match &*self.error.borrow() {
            None => None,
            Some(RealmError::Checked(error)) => return Some(v8::Local::new(scope, error)),
            Some(RealmError::Handed(error)) => Some(v8::Local::new(scope, error)),
        };
        let Some(error) = handed else {
            throw_error(scope, NO_REALM_ERROR);
            return None;
        };
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
            *self.error.borrow_mut() = None;
            throw_type_error(scope, NOT_REALM_ERROR);
            return None;
        };
        *self.error.borrow_mut() = Some(RealmError::Checked(v8::Global::new(scope, function)));
        Some(function)
    }

    /// The class of `family`: the run's, made now if it has none,
    /// extending the realm's `Error` the run holds, checked now if it
    /// wasn't ([`Self::realm_error`]). `None` when making it threw, the
    /// exception pending, an `Error` saying so among them when the run
    /// holds no `Error`, which no member that declares the family meets.
    pub fn class<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: DeclaredFamily,
    ) -> Option<v8::Local<'s, v8::Function>> {
        let family = family.family();
        if let Some(made) = self.made.borrow().iter().find(|made| made.family == family) {
            return Some(v8::Local::new(scope, &made.class));
        }
        let error = self.realm_error(scope)?;
        let class = make(scope, family, error)?;
        self.made.borrow_mut().push(Made {
            family,
            class: v8::Global::new(scope, class),
        });
        Some(class)
    }

    /// Throws a new error of `family`, the one its member declared,
    /// through `scope`, of `kind`, about `target`, with `cause` when one is
    /// given: what `new Class(kind, target, {cause})` makes, its message
    /// `kind: target`, made by the family's class, which is made now if the
    /// run has none, extending the realm's `Error` the run holds. Runs no
    /// code of a caller's: both are strings, the options a null-prototype
    /// record made here, and the class's own `prototype` a data property no
    /// program can change. With no `Error` held, it throws an `Error`
    /// saying so instead ([`Self::class`]).
    pub fn throw<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
        family: DeclaredFamily,
        kind: &str,
        target: v8::Local<'s, v8::String>,
        cause: Option<v8::Local<'s, v8::Value>>,
    ) {
        let Some(class) = self.class(scope, family) else {
            return;
        };
        let Some(kind) = v8::String::new(scope, kind) else {
            return;
        };
        let mut args = vec![kind.into(), target.into()];
        if let Some(cause) = cause {
            let Some(key) = name(scope, "cause") else {
                return;
            };
            let null = v8::null(scope).into();
            let options = v8::Object::with_prototype_and_properties(scope, null, &[key], &[cause]);
            args.push(options.into());
        }
        if let Some(instance) = class.new_instance(scope, &args) {
            scope.throw_exception(instance.into());
        }
    }
}

/// What an error row exports as its class (`EnvError`, `FsError`): the
/// run's class of `family` ([`ErrorClasses::class`]). `None` when making it
/// threw, the exception pending, an `Error` saying so among them when the
/// runtime was started without the run's error classes.
pub(super) fn exported_class<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    family: DeclaredFamily,
) -> Option<v8::Local<'s, v8::Value>> {
    let Some(classes) = state.try_borrow::<ErrorClasses>() else {
        throw_error(scope, NO_ERROR_CLASSES);
        return None;
    };
    classes.class(scope, family).map(Into::into)
}

/// The realm's `Error.prototype`, the prototype of an error V8 makes,
/// never a property a program can have replaced.
fn error_prototype<'s>(scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
    let empty = v8::String::empty(scope);
    let made = v8::Local::<v8::Object>::try_from(v8::Exception::error(scope, empty)).ok()?;
    made.get_prototype(scope)
}

/// Makes `family`'s class, extending `error`, the realm's `Error`. `None`
/// when it threw, the exception pending: an `Error` when `family` isn't one
/// of [`FAMILIES`], whose class's constructor couldn't find it.
fn make<'s>(
    scope: &v8::PinScope<'s, '_>,
    family: &'static ErrorFamily,
    error: v8::Local<'s, v8::Function>,
) -> Option<v8::Local<'s, v8::Function>> {
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
    Some(class)
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
    let held = v8::Boolean::new(scope, true).into();
    error.set_private(scope, brand(scope, family)?, held)?;
    Some(error)
}

/// The brand of `family`'s instances: the same private symbol every time,
/// which V8 keeps for the isolate.
fn brand<'s>(
    scope: &v8::PinScope<'s, '_>,
    family: &ErrorFamily,
) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, family.brand)?;
    Some(v8::Private::for_api(scope, Some(name)))
}

/// Whether `value` is an error the host made of `family`'s class, or of a
/// class extending it, by the brand its constructor gave it, as the Node
/// module's private field tells its own: reads nothing a caller can
/// answer, so a proxy, a look-alike, or another copy's error is none.
pub(super) fn is_own(
    scope: &v8::PinScope<'_, '_>,
    family: &ErrorFamily,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    if value.is_proxy() {
        return false;
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    brand(scope, family).is_some_and(|brand| object.has_private(scope, brand) == Some(true))
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

/// What a family's guard (`is_env_error(error, kind?)`, `is_fs_error`)
/// returns: whether `value` is an error of `family`, of `kind` unless it's
/// `undefined` ([`is_of_family`]). `None` when a read threw, the exception
/// pending.
pub(super) fn guard<'s>(
    scope: &v8::PinScope<'s, '_>,
    family: &ErrorFamily,
    value: v8::Local<'_, v8::Value>,
    kind: v8::Local<'_, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let found = is_of_family(scope, family, value, kind).ok()?;
    Some(v8::Boolean::new(scope, found).into())
}

/// Whether `value` is an error of `family`, of `kind` unless it's
/// `undefined`, as its guard answers ([`guard`]), by its prototype chain,
/// reading only data properties: a proxy, or an accessor on the way, is no
/// match, and none is run.
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
    use crate::host::NATIVE_MEMBERS;
    use crate::host::native::{Member, ValueKind};
    use crate::resolver::BUILTINS;

    /// Every error row's class is a family of [`FAMILIES`], in the table's
    /// order, its first export, and every family is a row's: the class of a
    /// family missing from it would construct a plain object.
    #[test]
    fn the_families_are_the_error_rows() {
        let rows: Vec<&str> = BUILTINS
            .iter()
            .filter(|row| row.specifier.ends_with("_error.ts"))
            .map(|row| row.exports[0])
            .collect();
        let families: Vec<&str> = FAMILIES.iter().map(|family| family.name).collect();
        assert!(!rows.is_empty(), "the table has an error row");
        assert_eq!(families, rows);
    }

    /// Each family's brand follows its rule: the module of the member
    /// declared its class (`class FAMILY`), `#`, and the class's name.
    #[test]
    fn each_family_s_brand_is_its_class_s_module_and_name() {
        for family in FAMILIES {
            let module = NATIVE_MEMBERS
                .iter()
                .find(|member| {
                    matches!(
                        member.member,
                        Member::Value {
                            kind: ValueKind::Class,
                            ..
                        }
                    ) && member.family.is_some_and(|declared| declared == family)
                })
                .map(|member| member.module);
            assert_eq!(
                Some(family.brand.to_owned()),
                module.map(|module| format!("{module}#{}", family.name)),
                "{}",
                family.name
            );
        }
    }

    /// Each family is declared by its error row's class, the export named
    /// for it, a value declared a class, and by no other member as its
    /// class; and every
    /// family a member declares, thrown or a class, is one of
    /// [`FAMILIES`], whose class the host can make.
    #[test]
    fn each_family_is_its_error_row_s_class_and_the_host_s() {
        for family in FAMILIES {
            let classes: Vec<(&str, &str)> = NATIVE_MEMBERS
                .iter()
                .filter(|member| {
                    matches!(
                        member.member,
                        Member::Value {
                            kind: ValueKind::Class,
                            ..
                        }
                    ) && member.family.is_some_and(|declared| declared == family)
                })
                .map(|member| (member.module, member.name))
                .collect();
            let row = BUILTINS
                .iter()
                .find(|row| row.exports.first() == Some(&family.name))
                .map(|row| row.specifier);
            assert_eq!(classes, [(row.unwrap(), family.name)], "{}", family.name);
        }
        for member in NATIVE_MEMBERS {
            if let Some(declared) = member.family {
                assert!(
                    FAMILIES.contains(&declared),
                    "{} of {}: {}",
                    member.name,
                    member.module,
                    declared.name
                );
            }
        }
    }
}
