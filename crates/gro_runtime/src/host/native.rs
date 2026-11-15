//! Native members: a builtin's exports, and the methods of its handles, as
//! functions and values the host makes, natively, the first time a run
//! imports the builtin, so neither a start nor a call pays for a script's
//! shell or an op of its own.
//!
//! Every native member is declared through `native_members!`, which lists
//! it in [`NATIVE_MEMBERS`](super::NATIVE_MEMBERS), the one table the host
//! makes members from, with its builtin, its name, where it goes (the
//! namespace, or the prototype of a kind of handle), what it is, and the
//! grant it requires. A member is a function, of a `length`, which is how
//! many arguments it declares, or a value, made once with the namespace:
//! the data a row exports, a handle the host makes of a prototype it made
//! with the namespace (`env`'s root), or a class the host makes
//! (`error_class.rs`). The callback the macro writes for a function checks
//! the grant first, as a host op does (`grant.rs`):
//! [`require`](super::grant::require) of the grant it names, or, for a
//! grant-free module's member,
//! [`require_authority`](super::grant::require_authority), throwing
//! `not_granted` through the scope before the body runs. Then it
//! hands the body its receiver, for a method, and each argument as the JS
//! value passed, never converted, so the body reads each natively and runs
//! no code of a caller's, as a host op's does; and throws what the body
//! throws, the host's error classes and the realm's `TypeError` and
//! `RangeError` among them, through the scope. A member's body is the
//! member's result, `None` when it has none or when it threw. A value's
//! maker makes the same check first, behind the namespace op's of the row.
//!
//! A row's script reaches its namespace through one op, `op_gro_namespace`,
//! which serves every row with native members. The op takes the row from
//! the script that calls it, the backing script `deno_core` compiled under
//! the row's specifier from the table, never from an argument, so no row's
//! script can ask for another's ([`calling_row`]). It checks the row's
//! grant ([`require_grant`]), takes the scope of the importer's grant
//! (`undefined` while no row has one) and the realm's `Error`, which the
//! run holds for the error classes, and returns the namespace the run
//! holds, or makes it and holds it ([`NativeNamespaces`]). Making
//! one makes each kind of handle's prototype, a null-prototype record of its
//! methods, and then the namespace, a null-prototype record of its exports
//! and the `default` every builtin has, `undefined`; each function is
//! frozen, named, of its `length`, and not a constructor, and each record is
//! frozen. Nothing is held until the namespace is whole, so a first import
//! ended early holds nothing and the next makes it all again, and one that
//! ran once finds the same members, as an instance per origin will. A
//! factory makes its handles of the prototype the run holds
//! ([`prototype`]), and a value of the prototype being made with it
//! ([`Prototypes`]).
//!
//! Nothing at compile time makes a native function come from the table:
//! the tests are the check. One fails on a native function or callback
//! made anywhere in the code of the crates linked into the binary
//! (`gro_runtime`, `gro_cli`, `gro_v8`, `gro_sys`) but here and the error
//! classes (`error_class.rs`), read from the source by the type every native
//! callback names and by each API that makes one; one on a member that
//! runs without its grant or without an authority, or whose body ran before
//! its refusal; one on a member whose grant isn't its row's; one on a
//! namespace whose exports or a prototype whose methods aren't the table's,
//! or a function or record that isn't frozen; and the table is pinned in
//! `tests/native_members.txt`.
//!
//! What the form doesn't have yet, each for a row to come (`fs`), and how
//! each would be declared: a kind of handle whose prototype extends
//! another's (`FsDir` over `FsReadDir`), a `parent` on [`HandleKind`] whose
//! prototype [`make`] makes first and gives the child's record as its
//! prototype, a method's receiver check accepting the kinds below it; a
//! member keyed by a well-known symbol (`Symbol.asyncDispose`), a key
//! written in place of the name, `fn [async_dispose](..)`, the record
//! keyed by the symbol V8 gives; and a `length` short of the arguments a
//! member reads (a defaulted parameter, `temp_dir(prefix = 'gro-')`),
//! written `fn temp_dir(state; scope, prefix?)`, the arguments after the
//! first marked `?` left out of the `length`.//!
//! And what `fs` needs that the next slices settle, none designed here: a
//! root record whose functions are its own properties beside a value
//! (`fs.open_dir` beside `fs.project`), a place for a record's members
//! beside the namespace and a kind's prototype; an async form, whose
//! callback makes a promise and rejects it with what the check or the body
//! threw, the refusal included; own data on a handle (an `FsDir`'s `path`),
//! which [`make`](super::handle::make) doesn't take; a brand on an error
//! instance, which the class's constructor sets, for a guard of the
//! family's own instances (`is_own_fs_error`); a listing whose work waits
//! for its first `next()`, as an async generator's does; the table of
//! native state behind a handle (`blake3.rs`'s hashers, with the weak
//! reference that drops an entry) lifted into `handle.rs` for every kind;
//! and types for handles whose methods return each other, more than the one
//! `Handle` type argument the namespace op's declaration carries.

use std::cell::RefCell;

use deno_core::{JsRuntime, OpState, v8};

use super::error_class::ErrorClasses;
use super::grant::require_grant;
use super::handle::HandleKind;
use super::{NO_ERROR_CLASSES, name, throw_error, throw_type_error};
use crate::bootstrap::IMPLEMENTATIONS;
#[cfg(test)]
use crate::resolver::Capability;
use crate::resolver::{BUILTINS, Builtin};

/// What a member throws when the runtime's state is borrowed mutably as it
/// runs, which no host code does while JS runs.
pub const STATE_BUSY: &str = "the host's state is in use, so a native member can't run";

/// What the namespace op throws to a caller that is no row's backing
/// script.
const NOT_A_ROW: &str = "a namespace is asked for by a builtin's own script";

/// What the namespace op throws for a scope that isn't `undefined`: no row
/// with native members has one yet.
const NOT_A_SCOPE: &str = "a row with native members takes no scope";

/// What the namespace op throws in a runtime started without the holder
/// of the namespaces, which only a runtime made outside a run's start is.
const NO_NAMESPACES: &str = "this runtime was started without the native namespaces' holder";

/// What the namespace op throws for a row with no native members.
fn no_members(specifier: &str) -> String {
    format!("{specifier} has no native members, so the host makes no namespace of it")
}

/// Where a native member goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The builtin's namespace, as one of its exports.
    Export,
    /// The prototype of a kind of handle the builtin makes, as one of its
    /// methods, called on a handle as its receiver.
    Method(&'static HandleKind),
}

/// The prototypes a row's namespace is being made with, each kind's, for a
/// value made with it to make a handle of: what [`prototype`] gives once the
/// namespace is held.
pub struct Prototypes<'s>(Vec<(&'static HandleKind, v8::Local<'s, v8::Object>)>);

impl Prototypes<'_> {
    /// None, for a test that makes a value with no namespace.
    #[cfg(test)]
    pub const fn none() -> Self {
        Self(Vec::new())
    }
}

/// What makes a value member: the callback `native_members!` wrote, given
/// the runtime's state, the scope, and the prototypes the row's namespace
/// is being made with, whose first step is the check. `None` with an
/// exception pending when a step failed.
pub type Maker = for<'s> fn(
    &OpState,
    &v8::PinScope<'s, '_>,
    &Prototypes<'s>,
) -> Option<v8::Local<'s, v8::Value>>;

/// What the host makes of a native member.
#[derive(Debug, Clone, Copy)]
pub enum Member {
    /// A function of its callback, of a `length`.
    Function {
        /// The function's `length`: how many arguments it declares.
        length: usize,
        /// Its callback, as V8 calls it: the one `native_members!` wrote,
        /// whose first step is the check.
        callback: fn() -> v8::FunctionCallback,
    },
    /// A value, made once with the namespace: the data a row exports (the
    /// script's arguments, the process's id), a handle the host makes (a
    /// root), or a class the host makes (`error_class.rs`).
    Value(Maker),
}

/// A native member, as [`NATIVE_MEMBERS`](super::NATIVE_MEMBERS) lists it.
#[derive(Debug, Clone, Copy)]
pub struct NativeMember {
    /// The builtin it's a member of, by its specifier in the builtin table.
    pub module: &'static str,
    /// Its name, the export's or the method's, and a function's `name`.
    pub name: &'static str,
    /// Where it goes.
    pub place: Place,
    /// What the host makes of it.
    pub member: Member,
    /// The grant it requires, or none for a grant-free module's member: the
    /// one its callback or its maker checks, listed for the tests.
    #[cfg(test)]
    pub grant: Option<Capability>,
}

/// The arguments a member declares, as the values passed: `undefined` past
/// the last one passed, as V8 gives it, reading nothing.
pub fn arguments<'s, const N: usize>(
    args: &v8::FunctionCallbackArguments<'s>,
) -> [v8::Local<'s, v8::Value>; N] {
    std::array::from_fn(|index| args.get(i32::try_from(index).unwrap_or(i32::MAX)))
}

/// What a member's callback borrows the runtime's `OpState` from: the
/// state of the runtime whose isolate calls it, as an op's is.
pub fn op_state(isolate: &v8::Isolate) -> std::rc::Rc<RefCell<OpState>> {
    JsRuntime::op_state_from(isolate)
}

/// Declares native members, each a function the host makes of the
/// callback written here, or a value it makes with the namespace, whose
/// first step is the check of what it names, refusing before its body
/// runs, and lists them in `NATIVE_MEMBERS`, the table the host makes them
/// from.
///
/// A member written `fn name(state, granted: Grant; scope, args...) for
/// "specifier"` requires that grant ([`require`](super::grant::require)),
/// and its body is given the [`Granted`](super::grant::Granted) token; one
/// written `fn name(state; scope, args...) for "specifier"` is a grant-free
/// module's, and requires only an authority. After `for "specifier"`, a
/// method names the kind of handle whose prototype it goes on and binds its
/// receiver, `on KIND(receiver)`; an export names nothing more. Each
/// argument is the JS value passed, as `v8::Local<'s, v8::Value>`, and the
/// member's `length` is how many it declares. The scope is
/// `&mut v8::PinScope<'s, '_>`. The body's value is the member's result,
/// an `Option<v8::Local<'s, v8::Value>>`, `None` leaving it `undefined`,
/// which is what a member with no result and one that threw return.
///
/// A member written `value name(state[, granted: Grant]; scope[,
/// prototypes]) for "specifier"` is an export the host makes once, with the
/// namespace, after the same check: its body is the value, an
/// `Option<v8::Local<'s, v8::Value>>`, `None` only when it threw. The scope
/// is `&v8::PinScope<'s, '_>`, and `prototypes`, when it's named, the
/// [`Prototypes`] the namespace is being made with, for a value that is a
/// handle of one of them.
macro_rules! native_members {
    (
        @require $what:expr, $kind:literal, $state:ident, $scope:ident,
        $granted:ident: $grant:ty
    ) => {
        let $granted: $crate::host::grant::Granted<$grant> =
            match $crate::host::grant::require::<$grant>($state) {
                Ok(granted) => granted,
                Err(refused) => {
                    $crate::host::throw_error($scope, &refused.message(&$what, $kind));
                    return None;
                }
            };
    };
    (@require $what:expr, $kind:literal, $state:ident, $scope:ident) => {
        if let Err(refused) = $crate::host::grant::require_authority($state) {
            $crate::host::throw_error($scope, &refused.message(&$what, $kind));
            return None;
        }
    };
    (@grant $grant:ty) => {
        Some(<$grant as $crate::host::grant::Grant>::CAPABILITY)
    };
    (@grant) => {
        None
    };
    (@place) => {
        $crate::host::native::Place::Export
    };
    (@place $kind:ident) => {
        $crate::host::native::Place::Method(&$kind)
    };
    (@receiver $args:ident) => {};
    (@receiver $args:ident $receiver:ident) => {
        let $receiver = deno_core::v8::Local::<deno_core::v8::Value>::from($args.this());
    };
    (@prototypes) => {
        _
    };
    (@prototypes $prototypes:ident) => {
        $prototypes
    };
    (
        @member fn [$($doc:literal)*] $name:ident, $module:literal, $state:ident,
        [$($granted:ident: $grant:ty)?], $scope:ident, [$($arg:ident)*],
        [$($kind:ident($receiver:ident))?], $body:block
    ) => {
        $crate::host::native::Member::Function {
            length: <[&str]>::len(&[$(stringify!($arg)),*]),
            callback: || {
                $(#[doc = $doc])*
                fn callback<'s>(
                    $scope: &mut deno_core::v8::PinScope<'s, '_>,
                    args: deno_core::v8::FunctionCallbackArguments<'s>,
                    mut result: deno_core::v8::ReturnValue<'s, deno_core::v8::Value>,
                ) {
                    let state = $crate::host::native::op_state($scope);
                    let Ok(state) = state.try_borrow() else {
                        $crate::host::throw_error($scope, $crate::host::native::STATE_BUSY);
                        return;
                    };
                    let value = (|| -> Option<deno_core::v8::Local<'s, deno_core::v8::Value>> {
                        let $state: &deno_core::OpState = &state;
                        native_members!(
                            @require
                            format!("{} of {}", stringify!($name), $module),
                            "host function",
                            $state,
                            $scope
                            $(, $granted: $grant)?
                        );
                        native_members!(@receiver args $($receiver)?);
                        let [$($arg),*] = $crate::host::native::arguments(&args);
                        $body
                    })();
                    if let Some(value) = value {
                        result.set(value);
                    }
                }
                deno_core::v8::MapFnTo::map_fn_to(callback)
            },
        }
    };
    (
        @member value [$($doc:literal)*] $name:ident, $module:literal, $state:ident,
        [$($granted:ident: $grant:ty)?], $scope:ident, [$($prototypes:ident)?], [], $body:block
    ) => {
        $crate::host::native::Member::Value({
            $(#[doc = $doc])*
            fn make<'s>(
                $state: &deno_core::OpState,
                $scope: &deno_core::v8::PinScope<'s, '_>,
                native_members!(@prototypes $($prototypes)?): &$crate::host::native::Prototypes<'s>,
            ) -> Option<deno_core::v8::Local<'s, deno_core::v8::Value>> {
                native_members!(
                    @require
                    format!("{} of {}", stringify!($name), $module),
                    "host value",
                    $state,
                    $scope
                    $(, $granted: $grant)?
                );
                $body
            }
            make
        })
    };
    ($(
        $(#[doc = $doc:literal])*
        $what:ident $name:ident(
            $state:ident $(, $granted:ident: $grant:ty)?;
            $scope:ident $(, $arg:ident)*
        )
            for $module:literal
            $(on $kind:ident($receiver:ident))?
            $body:block
    )*) => {
        /// Every native member, the table the host makes them from, in
        /// declaration order.
        pub const NATIVE_MEMBERS: &[$crate::host::native::NativeMember] = &[$(
            $crate::host::native::NativeMember {
                module: $module,
                name: stringify!($name),
                place: native_members!(@place $($kind)?),
                member: native_members!(
                    @member $what [$($doc)*] $name, $module, $state,
                    [$($granted: $grant)?], $scope, [$($arg)*],
                    [$($kind($receiver))?], $body
                ),
                #[cfg(test)]
                grant: native_members!(@grant $($grant)?),
            }
        ),*];
    };
}
pub(crate) use native_members;

/// The namespaces the run's native rows have made, put in its `OpState`
/// when it starts, empty: a row's joins once it's whole.
#[derive(Default)]
pub struct NativeNamespaces {
    /// The rows made, each held whole.
    made: RefCell<Vec<Made>>,
}

/// A row's namespace and its handles' prototypes, made.
struct Made {
    /// The row's specifier.
    module: &'static str,
    /// Its namespace.
    namespace: v8::Global<v8::Object>,
    /// The prototype of each kind of handle it makes.
    prototypes: Vec<(&'static HandleKind, v8::Global<v8::Object>)>,
}

impl NativeNamespaces {
    /// How many rows' namespaces the run has made.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.made.borrow().len()
    }
}

/// The namespace of the row whose backing script asks: the one the run
/// holds, or one made now, then held, once the row's grant is checked and
/// what the script hands over is: `importer_scope`, the scope of its
/// importer's grant, which no row with native members has yet, so it's
/// `undefined`; and `error`, the realm's `Error`, which the run holds
/// ([`ErrorClasses::hold_error`]) for the error classes a row throws.
/// `None` with an exception pending when a step failed: a `TypeError` for
/// a caller that is no row's backing script, `not_granted` for a row whose
/// grant the run doesn't hold, an `Error` for a row with no native
/// members, and a `TypeError` for a scope or an `Error` that isn't one.
pub fn namespace<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    importer_scope: v8::Local<'s, v8::Value>,
    error: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Object>> {
    let Some(builtin) = calling_row(scope) else {
        throw_type_error(scope, NOT_A_ROW);
        return None;
    };
    if let Err(refused) = require_grant(state, builtin.grant) {
        throw_error(scope, &refused.message("op_gro_namespace", "host op"));
        return None;
    }
    let module = builtin.specifier;
    if !super::NATIVE_MEMBERS
        .iter()
        .any(|member| member.module == module)
    {
        throw_error(scope, &no_members(module));
        return None;
    }
    // TODO a row whose grant has a scope (`env`, `fs`) keys its namespace by
    // it, the host holding the functions once and a record per scope
    if !importer_scope.is_undefined() {
        throw_type_error(scope, NOT_A_SCOPE);
        return None;
    }
    let Some(classes) = state.try_borrow::<ErrorClasses>() else {
        throw_error(scope, NO_ERROR_CLASSES);
        return None;
    };
    classes.hold_error(scope, error)?;
    let Some(namespaces) = state.try_borrow::<NativeNamespaces>() else {
        throw_error(scope, NO_NAMESPACES);
        return None;
    };
    let held = namespaces
        .made
        .borrow()
        .iter()
        .find(|made| made.module == module)
        .map(|made| v8::Local::new(scope, &made.namespace));
    if held.is_some() {
        return held;
    }
    let made = make(state, scope, module, super::NATIVE_MEMBERS)?;
    let namespace = v8::Local::new(scope, &made.namespace);
    namespaces.made.borrow_mut().push(made);
    Some(namespace)
}

/// The row whose backing script called the op: the one whose script the
/// frame on top of the stack runs, by the name `deno_core` compiled it
/// under, the row's backing specifier from the table
/// ([`IMPLEMENTATIONS`]), never a name the script gives, so no row's script
/// can ask for another row's namespace. `None` for any other caller.
fn calling_row(scope: &v8::PinScope<'_, '_>) -> Option<&'static Builtin> {
    let trace = v8::StackTrace::current_stack_trace(scope, 1)?;
    let frame = trace.get_frame(scope, 0)?;
    let script = frame.get_script_name(scope)?.to_rust_string_lossy(scope);
    BUILTINS
        .iter()
        .zip(IMPLEMENTATIONS)
        .find(|(_, implementation)| implementation.script == script)
        .map(|(builtin, _)| builtin)
}

/// The prototype the run holds for handles of `kind`, made with its row's
/// namespace, for a factory to make a handle of; `None` when the row's
/// namespace wasn't made, which only a factory reached some other way than
/// through the namespace meets.
pub fn prototype<'s>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    kind: &HandleKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let namespaces = state.try_borrow::<NativeNamespaces>()?;
    let made = namespaces.made.borrow();
    made.iter()
        .flat_map(|made| &made.prototypes)
        .find(|(held, _)| *held == kind)
        .map(|(_, prototype)| v8::Local::new(scope, prototype))
}

/// `module`'s namespace and its handles' prototypes, made of `members`'
/// rows for it: each kind's prototype first, in the order the members name
/// the kinds, then the namespace, its values made with those prototypes.
/// `None` with an exception pending when a step failed.
fn make(
    state: &OpState,
    scope: &v8::PinScope<'_, '_>,
    module: &'static str,
    members: &[NativeMember],
) -> Option<Made> {
    let members: Vec<&NativeMember> = members
        .iter()
        .filter(|member| member.module == module)
        .collect();
    let mut kinds: Vec<&'static HandleKind> = Vec::new();
    for member in &members {
        if let Place::Method(kind) = member.place
            && !kinds.contains(&kind)
        {
            kinds.push(kind);
        }
    }
    let mut prototypes = Prototypes(Vec::with_capacity(kinds.len()));
    for kind in kinds {
        let methods = members
            .iter()
            .filter(|member| member.place == Place::Method(kind));
        let prototype = record(state, scope, methods, &prototypes, None)?;
        prototypes.0.push((kind, prototype));
    }
    let exports = members
        .iter()
        .filter(|member| member.place == Place::Export);
    let namespace = record(state, scope, exports, &prototypes, Some("default"))?;
    Some(Made {
        module,
        namespace: v8::Global::new(scope, namespace),
        prototypes: prototypes
            .0
            .into_iter()
            .map(|(kind, prototype)| (kind, v8::Global::new(scope, prototype)))
            .collect(),
    })
}

/// A frozen null-prototype record of each of `members`, under its name, a
/// function or the value its maker makes with `prototypes`, and
/// `undefined` under `undefined_at` when given.
fn record<'s, 'm>(
    state: &OpState,
    scope: &v8::PinScope<'s, '_>,
    members: impl Iterator<Item = &'m &'m NativeMember>,
    prototypes: &Prototypes<'s>,
    undefined_at: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    let mut names: Vec<v8::Local<'s, v8::Name>> = Vec::new();
    let mut values: Vec<v8::Local<'s, v8::Value>> = Vec::new();
    for member in members {
        names.push(name(scope, member.name)?);
        values.push(match member.member {
            Member::Function { length, callback } => {
                function(scope, member.name, length, callback)?.into()
            }
            Member::Value(make) => make(state, scope, prototypes)?,
        });
    }
    if let Some(key) = undefined_at {
        names.push(name(scope, key)?);
        values.push(v8::undefined(scope).into());
    }
    let null = v8::null(scope).into();
    let record = v8::Object::with_prototype_and_properties(scope, null, &names, &values);
    record.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    Some(record)
}

/// The function of a member named `name`: `callback`, named, of `length`,
/// not a constructor, so it has no `prototype`, and frozen. `None` with an
/// exception pending when V8 failed.
fn function<'s>(
    scope: &v8::PinScope<'s, '_>,
    name: &str,
    length: usize,
    callback: fn() -> v8::FunctionCallback,
) -> Option<v8::Local<'s, v8::Function>> {
    let function = v8::Function::builder_raw(callback())
        .length(i32::try_from(length).unwrap_or(i32::MAX))
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)?;
    function.set_name(v8::String::new(scope, name)?);
    function.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
    Some(function)
}

/// A function and a value of a row that needs a grant, for the tests of
/// the form's check and of a namespace made whole or not at all.
#[cfg(test)]
pub mod probe {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use deno_core::{OpState, v8};

    use crate::ArgsGrant;
    use crate::host::throw_error;

    /// How many times the probe's body ran, past its check, so a test
    /// sees a body that ran before a refusal.
    pub static RAN: AtomicUsize = AtomicUsize::new(0);

    /// How many times the probe value's maker ran, past its check.
    pub static MADE: AtomicUsize = AtomicUsize::new(0);

    /// Whether the probe value's maker fails, past its check.
    pub static FAIL: AtomicBool = AtomicBool::new(false);

    /// The probe value, as the host makes a namespace's value.
    pub fn value<'s>(
        state: &OpState,
        scope: &v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Value>> {
        let super::Member::Value(make) = NATIVE_MEMBERS[1].member else {
            return None;
        };
        make(state, scope, &super::Prototypes::none())
    }

    /// The probe table's namespace, made as the namespace op makes a
    /// row's, whether it was made: not when a value's maker failed.
    pub fn namespace_made(state: &OpState, scope: &v8::PinScope<'_, '_>) -> bool {
        super::make(state, scope, NATIVE_MEMBERS[0].module, NATIVE_MEMBERS).is_some()
    }

    /// The probe's function, as the host makes a member's.
    pub fn function<'s>(scope: &v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Function>> {
        let super::Member::Function { length, callback } = NATIVE_MEMBERS[0].member else {
            return None;
        };
        super::function(scope, NATIVE_MEMBERS[0].name, length, callback)
    }

    native_members! {
        /// Returns `"ran"`, once the check passed.
        fn probe(state, _granted: ArgsGrant; scope, value) for "@fuzdev/gro/args.ts" {
            let _ = value;
            RAN.fetch_add(1, Ordering::Relaxed);
            v8::String::new(scope, "ran").map(Into::into)
        }

        /// `"made"`, once the check passed, or an `Error` thrown when
        /// [`FAIL`] is set.
        value probe_value(state, _granted: ArgsGrant; scope) for "@fuzdev/gro/args.ts" {
            MADE.fetch_add(1, Ordering::Relaxed);
            if FAIL.load(Ordering::Relaxed) {
                throw_error(scope, "the probe value failed");
                return None;
            }
            v8::String::new(scope, "made").map(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Member, NativeMember, Place};
    use crate::host::NATIVE_MEMBERS;
    use crate::resolver::{BUILTINS, find_builtin};

    /// The line `tests/native_members.txt` pins `member` as.
    fn pinned_line(member: &NativeMember) -> String {
        let place = match member.place {
            Place::Export => "export".to_owned(),
            Place::Method(kind) => format!("method of {}", kind.brand),
        };
        let made = match member.member {
            Member::Function { length, .. } => format!("length {length}"),
            Member::Value(_) => "value".to_owned(),
        };
        let grant = member
            .grant
            .map_or_else(|| "grant-free".to_owned(), |grant| format!("grant {grant}"));
        format!("{} {} {place} {made} {grant}", member.module, member.name)
    }

    /// Every native member, pinned in `tests/native_members.txt` in the
    /// table's order, as `tests/op_names.txt` pins the ops: its builtin,
    /// name, place, a function's `length` or `value`, and grant, so a member added, removed, or
    /// changed fails here and gets a look. To accept a change deliberately,
    /// write the list the assertion prints over the file, one member a line.
    #[test]
    fn the_native_members_are_pinned() {
        let mut listed: String = NATIVE_MEMBERS
            .iter()
            .map(|member| pinned_line(member) + "\n")
            .collect();
        if listed.is_empty() {
            listed.push('\n');
        }
        assert_eq!(
            listed,
            include_str!("../../tests/native_members.txt"),
            "the native members differ from tests/native_members.txt; update it only if the \
             change is deliberate (see this test's doc)"
        );
    }

    /// Each native member is its row's: a row of the table, requiring
    /// exactly the row's grant, so a member of a module that needs a grant
    /// can't be declared grant-free, or one of a grant-free module's need
    /// one; a row's exports are its row's named exports, each once; each
    /// method goes on a kind of handle of its own row's, once a kind.
    #[test]
    fn the_native_members_are_their_rows() {
        for member in NATIVE_MEMBERS {
            let row = find_builtin(member.module)
                .unwrap_or_else(|| panic!("{}: {} is no row", member.name, member.module));
            assert_eq!(
                member.grant, row.grant,
                "{} of {}",
                member.name, member.module
            );
            if let Place::Method(kind) = member.place {
                assert!(
                    kind.brand.starts_with(&format!("{}#", member.module)),
                    "{} of {}: {}",
                    member.name,
                    member.module,
                    kind.brand
                );
            }
            let twins = NATIVE_MEMBERS.iter().filter(|other| {
                other.module == member.module
                    && other.place == member.place
                    && other.name == member.name
            });
            assert_eq!(twins.count(), 1, "{} of {}", member.name, member.module);
        }
        for row in BUILTINS {
            let mut exports: Vec<&str> = NATIVE_MEMBERS
                .iter()
                .filter(|member| member.module == row.specifier && member.place == Place::Export)
                .map(|member| member.name)
                .collect();
            if exports.is_empty() {
                continue;
            }
            let mut named = row.exports.to_vec();
            exports.sort_unstable();
            named.sort_unstable();
            assert_eq!(exports, named, "{}", row.specifier);
        }
        assert!(
            NATIVE_MEMBERS
                .iter()
                .any(|member| matches!(member.place, Place::Method(_))),
            "a method is among them, so the checks of one run"
        );
    }

    /// What makes a function, or a callback JS can reach, natively, in V8's
    /// API, as source names it, however the path is spelled: every native
    /// callback takes its arguments as `*CallbackArguments`, so an alias, a
    /// macro, or a builder still names one where its callback is written;
    /// and the form's own parts, which make a function of any callback.
    const MAKERS: [&str; 17] = [
        "CallbackArguments",
        "FunctionBuilder",
        "Template",
        "::builder",
        "builder_raw",
        "new_raw",
        "Function::new",
        "Function as ",
        "set_accessor",
        "set_native_data_property",
        "set_lazy_data_property",
        "as_function_handler",
        "property_handler",
        "map_fn_to",
        "MapFnTo",
        "NativeMember {",
        "native::function",
    ];

    /// Each line of `source` that names a maker of [`MAKERS`].
    fn makers_in(source: &str) -> Vec<&str> {
        source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| MAKERS.iter().any(|maker| line.contains(maker)))
            .collect()
    }

    /// Every `.rs` file under `dir`, recursively.
    fn rust_files(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }

    /// No code of the crates linked into the binary (this one, `gro_cli`,
    /// `gro_v8`, `gro_sys`) makes a native function, or a native callback JS
    /// can reach, but this module, from the table, and the error classes
    /// (`error_class.rs`): a function a row's namespace or a handle held
    /// that was made elsewhere would skip the form's check. A test's own
    /// code, in a `tests` module's file, may make one. Read from the
    /// source, as no build can tell, and every spelling the reader must
    /// catch is held below.
    #[test]
    fn no_native_function_is_made_outside_the_form() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let src = manifest.join("src");
        let mut files = Vec::new();
        // this crate's code, and the crates linked into the binary with it
        for dir in [
            src.clone(),
            manifest.join("../gro_cli/src"),
            manifest.join("../gro_v8/src"),
            manifest.join("../gro_sys/src"),
        ] {
            rust_files(&dir, &mut files);
        }
        let allowed = [src.join("host/native.rs"), src.join("host/error_class.rs")];
        let is_tests = |path: &Path| {
            path.components().any(|part| part.as_os_str() == "tests")
                || path.file_name().is_some_and(|name| name == "tests.rs")
        };
        let mut found = Vec::new();
        for file in files {
            if allowed.contains(&file) || is_tests(&file) {
                continue;
            }
            let source = std::fs::read_to_string(&file).unwrap();
            for line in makers_in(&source) {
                found.push(format!("{}: {}", file.display(), line.trim()));
            }
        }
        assert_eq!(found, [""; 0]);
        // the reader finds each maker, and not one in a comment
        let made = "let f = v8::Function::builder(callback).build(scope);\n// v8::Function::new\n";
        assert_eq!(makers_in(made).len(), 1);
        for spelled in [
            "v8::FunctionBuilder::<v8::Function>::new(callback)",
            "use deno_core::v8::Function as Made; Made::builder(callback)",
            "<v8::Function>::builder(callback)",
            "v8::ObjectTemplate::new(scope).set_call_as_function_handler(callback)",
            "object.set_accessor(scope, key, getter)",
            "template.set_native_data_property(key, getter)",
            "object.set_lazy_data_property(scope, key, getter)",
            "native::function(scope, &member)",
            "fn callback(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments) {}",
        ] {
            assert_eq!(makers_in(spelled).len(), 1, "{spelled}");
        }
        for maker in MAKERS {
            assert_eq!(makers_in(&format!("x::{maker}y")).len(), 1, "{maker}");
        }
    }
}
