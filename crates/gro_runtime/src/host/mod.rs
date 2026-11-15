//! The host ops: the native half of the builtins, the only ops that reach
//! past pure computation on a host module's behalf.
//!
//! Every host op is declared through `host_ops!`, whose first step is a
//! check in `grant.rs`, and which lists the op in `HOST_OPS` with the
//! builtin module it serves, so a host op can't skip it. An op that names a
//! grant starts with [`require`], the one place a [`Granted`] token is
//! made, and refuses a run that doesn't hold the grant. An op that names
//! none is a grant-free module's: it starts with [`require_authority`],
//! which refuses only a runtime that holds no authority at all, eval's, so
//! every run's importers reach it and an eval reaches no host op. The
//! runtime's tests hold the rest: every op gro's extension registers,
//! whatever its name, is either one of these or on the bootstrap's ungated
//! list, each of these refuses a runtime with no authority, each that names
//! a grant refuses a run that grants nothing, and each requires what the
//! row of the module it serves does, so an op of a module that needs a
//! grant can't be declared grant-free.
//!
//! A row's members can be native functions instead (`native.rs`), declared
//! through `native_members!`, whose callbacks make the same check first, and
//! reached through one host op, `op_gro_namespace`, which serves every such
//! row, the row its calling script's, and checks the row's grant before it
//! makes anything.
//!
//! The loader refuses an ungranted builtin at link time already, and every
//! builtin in an eval, so a refusal here is the second check, for a path to
//! an op past the loader.

pub mod blake3;
pub mod env;
pub mod error_class;
pub mod exit;
mod grant;
pub mod handle;
pub mod native;

use deno_core::error::ModuleLoaderError as JsErrorBox;
use deno_core::{OpDecl, OpState, op2, v8};

use error_class::{ENV_ERROR, ErrorClasses};
use exit::{ExitState, NOT_A_CODE, to_exit_code};
#[cfg(test)]
use grant::Grant;
use grant::{Granted, require, require_authority};
use handle::BLAKE3_HASHER;
use native::native_members;

use crate::loader::RefusalLog;
#[cfg(test)]
use crate::resolver::Capability;
use crate::{ArgsGrant, EnvGrant, EnvVars, ExitGrant};

/// What `exit`'s functions throw in a runtime started without the exit
/// host module's state, which only a runtime made outside a run's start is: a
/// run that grants `exit` holds it.
const NO_EXIT_STATE: &str = "exit is granted, but this runtime was started without its state";

/// What `cwd` throws when the working directory's path isn't UTF-8.
const CWD_NOT_UTF8: &str = "the working directory's path isn't UTF-8, so Gro can't name it";

/// What an error row's op throws in a runtime started without the run's
/// error classes, which only a runtime made outside a run's start is.
const NO_ERROR_CLASSES: &str = "this runtime was started without the run's error classes";

/// What the host ops return of the run, put in the runtime's `OpState` with
/// the run's [`Authority`](crate::Authority) when `fgro run`'s runtime
/// starts: data the embedder gives, none of it read from the process here.
#[derive(Debug, Clone, Default)]
pub struct HostData {
    /// The script's arguments.
    pub args: Vec<String>,
    /// The working directory the run was started in, unset when its path
    /// isn't UTF-8, which a JS string can't hold.
    pub cwd: Option<String>,
    /// The runtime's name and version, as the `runtime_version` host module
    /// exports it.
    pub runtime_version: String,
    /// The environment variables the embedder read for the env host
    /// module's root handle, of which it holds the names the run grants.
    pub env: EnvVars,
}

/// A host op, as `HOST_OPS` lists it.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostOp {
    /// The op's name.
    pub name: &'static str,
    /// What it serves.
    pub serves: Serves,
    /// The grant it requires, or none for a grant-free module's op.
    pub grant: Option<Capability>,
}

/// What a host op serves.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Serves {
    /// One builtin module, by its specifier in the builtin table, whose
    /// grant the op requires.
    Row(&'static str),
    /// Every row with native members (`native.rs`), named by the op's
    /// argument, whose grant the op checks once it knows the row: the op
    /// itself is grant-free, refusing only a runtime with no authority.
    Native,
}

/// Declares host ops, each an `#[op2]` whose first step is the check of
/// what it names, refusing before its body runs, and lists them in
/// `HOST_OPS` (for the tests) and [`host_ops`].
///
/// An op written `fn name(state, granted: Grant) for "specifier"` requires
/// that grant ([`require`]; the [`NotGranted`](grant::NotGranted) message
/// names the op and the grant), and its body is given the [`Granted`]
/// token. An op written `fn name(state) for "specifier"` is a grant-free
/// module's: it requires only that the runtime holds an authority
/// ([`require_authority`]), so it runs in every run and in no eval. The
/// specifier is the builtin module the op serves, and a test holds what the
/// op requires to that module's row, so the form that names no grant can't
/// be written for a module that needs one. An op written `for native`
/// serves every row with native members, the row its argument names: it
/// names no grant, and checks the row's once it knows it
/// ([`native::namespace`]).
///
/// An op that takes arguments names them after a `;`, the scope first:
/// `fn name(state, granted: Grant; scope, value) for "specifier"`. Each
/// argument is the JS value as it was passed, a `v8::Local<'s, v8::Value>`,
/// never converted on the way in: `#[op2]`'s typed arguments convert, and
/// a conversion can run the caller's code or truncate, so the body reads
/// the value natively and throws through the scope. The check still comes
/// first, before the body sees an argument. An op that makes a handle
/// whose state is native takes the scope mutably, written `&mut scope`:
/// the weak reference that drops the state once the handle is collected
/// is made of the isolate, mutably (`blake3.rs`).
///
/// A body is the op's result, given the op state, and an op written with
/// no arrow returns nothing. A `#[serde]` or `#[string]` after the arrow is
/// the op's return attribute, and the return type is written as paths
/// (`Vec<String>`), which `#[op2]` reads token by token. An op that takes
/// the scope can return a value made in it, as `v8::Local<'s, v8::Value>`,
/// `'s` the scope's lifetime, which the macro declares. Such an op that
/// throws returns `undefined` beside the exception, which V8 throws in its
/// place.
///
/// Slow ops only: whether a fast call's refusal throws on every path is
/// unmeasured, so an op whose signature a fast call could serve is written
/// `@nofast fn`, which `#[op2]` otherwise refuses to compile. An op that
/// takes the scope has no such signature, so it's slow as it's written,
/// unless it takes it mutably and returns nothing.
macro_rules! host_ops {
    (@require $name:ident, $state:ident, $granted:ident: $grant:ty) => {
        let $granted: Granted<$grant> =
            require::<$grant>($state).map_err(|refused| refused.into_js(stringify!($name)))?;
    };
    (@require $name:ident, $state:ident) => {
        require_authority($state).map_err(|refused| refused.into_js(stringify!($name)))?;
    };
    (@grant $grant:ty) => {
        Some(<$grant as Grant>::CAPABILITY)
    };
    (@grant) => {
        None
    };
    (@serves native) => {
        Serves::Native
    };
    (@serves $module:literal) => {
        Serves::Row($module)
    };
    // no scope, and a result or none
    (@op [$($attr:tt)*] $name:ident, $state:ident, [$($granted:tt)*], [], [], $body:block) => {
        host_ops!(@op [$($attr)*] $name, $state, [$($granted)*], [], [()], $body);
    };
    (@op [$($attr:tt)*] $name:ident, $state:ident, [$($granted:tt)*], [], [$($out:tt)+], $body:block) => {
        $($attr)*
        fn $name($state: &OpState) -> Result<$($out)+, JsErrorBox> {
            host_ops!(@require $name, $state $($granted)*);
            Ok($body)
        }
    };
    // the scope and the arguments, and a result, which can be the scope's,
    // or none
    (
        @op [$($attr:tt)*] $name:ident, $state:ident, [$($granted:tt)*],
        [$($mutable:ident)? | $scope:ident $(, $arg:ident)*], [], $body:block
    ) => {
        host_ops!(
            @op [$($attr)*] $name, $state, [$($granted)*],
            [$($mutable)? | $scope $(, $arg)*], [()], $body
        );
    };
    (
        @op [$($attr:tt)*] $name:ident, $state:ident, [$($granted:tt)*],
        [$($mutable:ident)? | $scope:ident $(, $arg:ident)*], [$($out:tt)+], $body:block
    ) => {
        $($attr)*
        fn $name<'s>(
            $state: &OpState,
            $scope: &$($mutable)? v8::PinScope<'s, '_>
            $(, $arg: v8::Local<'s, v8::Value>)*
        ) -> Result<$($out)+, JsErrorBox> {
            host_ops!(@require $name, $state $($granted)*);
            Ok($body)
        }
    };
    ($(
        $(#[doc = $doc:literal])*
        $(@$slow:ident)? fn $name:ident(
            $state:ident $(, $granted:ident: $grant:ty)?
            $(; $(& $mutable:ident)? $scope:ident $(, $arg:ident)*)?
        )
            for $serves:tt
            $(
                -> $(#[$ret:ident])? $($out:ident)::+
                $(<$($out_lt:lifetime,)? $($($out_arg:ident)::+),+>)?
            )?
            $body:block
    )*) => {
        $(
            host_ops!(
                @op
                [$(#[doc = $doc])* #[op2$(($slow))?] $($(#[$ret])?)?]
                $name,
                $state,
                [$(, $granted: $grant)?],
                [$($($mutable)? | $scope $(, $arg)*)?],
                [$($($out)::+ $(<$($out_lt,)? $($($out_arg)::+),+>)?)?],
                $body
            );
        )*

        /// Every host op, in declaration order.
        #[cfg(test)]
        pub const HOST_OPS: &[HostOp] = &[$(HostOp {
            name: stringify!($name),
            serves: host_ops!(@serves $serves),
            grant: host_ops!(@grant $($grant)?),
        }),*];

        /// Every host op, for the bootstrap's extension.
        pub fn host_ops() -> Vec<OpDecl> {
            vec![$($name()),*]
        }
    };
}

/// Throws `message` as a `TypeError` through `scope`, for a value an op
/// refuses: the realm's own, made by V8 from its intrinsic, whatever
/// `globalThis.TypeError` has become.
pub fn throw_type_error(scope: &v8::PinScope<'_, '_>, message: &str) {
    if let Some(message) = v8::String::new(scope, message) {
        let error = v8::Exception::type_error(scope, message);
        scope.throw_exception(error);
    }
}

/// Throws `message` as a `RangeError` through `scope`, the realm's own, as
/// [`throw_type_error`] throws a `TypeError`.
pub fn throw_range_error(scope: &v8::PinScope<'_, '_>, message: &str) {
    if let Some(message) = v8::String::new(scope, message) {
        let error = v8::Exception::range_error(scope, message);
        scope.throw_exception(error);
    }
}

/// Throws `message` as an `Error` through `scope`, the realm's own, as
/// [`throw_type_error`] throws a `TypeError`.
pub fn throw_error(scope: &v8::PinScope<'_, '_>, message: &str) {
    if let Some(message) = v8::String::new(scope, message) {
        let error = v8::Exception::error(scope, message);
        scope.throw_exception(error);
    }
}

/// `text` as a JS string, or `None` with a `RangeError` thrown, as the
/// language throws one, when it's longer than V8's longest string, for
/// which V8 throws nothing.
fn string<'s>(scope: &v8::PinScope<'s, '_>, text: &str) -> Option<v8::Local<'s, v8::String>> {
    let string = v8::String::new(scope, text);
    if string.is_none() {
        throw_range_error(scope, "Invalid string length");
    }
    string
}

/// `text` as a property name.
fn name<'s>(scope: &v8::PinScope<'s, '_>, text: &str) -> Option<v8::Local<'s, v8::Name>> {
    v8::String::new(scope, text).map(Into::into)
}

/// A read that threw, its exception pending.
struct Thrown;

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

host_ops! {
    /// The env error row's namespace, `EnvError` and `is_env_error`: the
    /// host's, made the first time the run needs them, the class extending
    /// the realm's `Error`, which the implementation hands over from
    /// `primordials` as `error`, held unless the run holds it already.
    fn op_gro_env_error(state; scope, error) for "@fuzdev/gro/env_error.ts"
        -> v8::Local<'s, v8::Value>
    {
        let namespace = state.try_borrow::<ErrorClasses>().map_or_else(
            || {
                throw_error(scope, NO_ERROR_CLASSES);
                None
            },
            |classes| classes.namespace(scope, &ENV_ERROR, error),
        );
        namespace.map_or_else(|| v8::undefined(scope).into(), Into::into)
    }

    /// The env host module's root handle, made of `prototype`, the record
    /// of shells the implementation's outer level made, the first time an
    /// importing origin asks, and held, so a later ask, a rerun of a
    /// first import ended early among them, gets the same one; the realm's
    /// `Error`, which the implementation hands over as `error`, held for
    /// the run first ([`env::root`]).
    fn op_gro_env_root(state, _granted: EnvGrant; scope, prototype, error)
        for "@fuzdev/gro/env.ts" -> v8::Local<'s, v8::Value>
    {
        env::root(state, scope, prototype, error)
            .map_or_else(|| v8::undefined(scope).into(), Into::into)
    }

    /// `env.get(name)`, its receiver handed over as `handle` ([`env::get`]).
    fn op_gro_env_get(state, _granted: EnvGrant; scope, handle, name)
        for "@fuzdev/gro/env.ts" -> v8::Local<'s, v8::Value>
    {
        env::get(state, scope, handle, name).unwrap_or_else(|| v8::undefined(scope).into())
    }

    /// `env.get_all()`, its receiver handed over as `handle`
    /// ([`env::get_all`]).
    fn op_gro_env_get_all(state, _granted: EnvGrant; scope, handle)
        for "@fuzdev/gro/env.ts" -> v8::Local<'s, v8::Value>
    {
        env::get_all(scope, handle).unwrap_or_else(|| v8::undefined(scope).into())
    }

    /// `env.narrow(names)`, its receiver handed over as `handle`
    /// ([`env::narrow`]).
    fn op_gro_env_narrow(state, _granted: EnvGrant; scope, handle, names)
        for "@fuzdev/gro/env.ts" -> v8::Local<'s, v8::Value>
    {
        env::narrow(state, scope, handle, names).unwrap_or_else(|| v8::undefined(scope).into())
    }

    /// The namespace of the row whose backing script calls it: the host's,
    /// its members native functions made the first time the run asks and
    /// held, so every later ask, a rerun of a first import ended early
    /// among them, gets the same ones, once the row's grant is checked; the
    /// script hands over its importer's scope and the realm's `Error`, from
    /// `primordials` ([`native::namespace`]).
    fn op_gro_namespace(state; scope, importer_scope, error) for native
        -> v8::Local<'s, v8::Value>
    {
        native::namespace(state, scope, importer_scope, error)
            .map_or_else(|| v8::undefined(scope).into(), Into::into)
    }

}

native_members! {
    /// `args`, the script's arguments: a frozen array of strings, one for
    /// every importer.
    value args(state, _granted: ArgsGrant; scope) for "@fuzdev/gro/args.ts" {
        let mut elements = Vec::new();
        for arg in &state.borrow::<HostData>().args {
            elements.push(string(scope, arg)?.into());
        }
        let args = v8::Array::new_with_elements(scope, &elements);
        args.set_integrity_level(scope, v8::IntegrityLevel::Frozen)?;
        Some(args.into())
    }

    /// `cwd()`, the working directory the run was started in, or, when its
    /// path isn't UTF-8, which a JS string can't hold, an `Error` thrown
    /// saying so.
    fn cwd(state; scope) for "@fuzdev/gro/cwd.ts" {
        let cwd = state.borrow::<HostData>().cwd.as_deref();
        let Some(cwd) = cwd else {
            throw_error(scope, CWD_NOT_UTF8);
            return None;
        };
        string(scope, cwd).map(Into::into)
    }

    /// `exit(code?)`: ends the run now, by the embedder's step, with
    /// `code`, or, given `undefined`, with the code `set_exit_code` set,
    /// else 0. Any other value throws a `TypeError` and ends nothing.
    fn exit(state, _granted: ExitGrant; scope, code) for "@fuzdev/gro/exit.ts" {
        let code = if code.is_undefined() {
            Some(None)
        } else {
            to_exit_code(code).map(Some)
        };
        match (code, state.try_borrow::<ExitState>(), state.try_borrow::<RefusalLog>()) {
            (None, ..) => throw_type_error(scope, NOT_A_CODE),
            (Some(code), Some(exit), Some(refusals)) => exit.exit(code, refusals),
            (Some(_), ..) => throw_error(scope, NO_EXIT_STATE),
        }
        None
    }

    /// `set_exit_code(code)`: sets the code the run exits with when it
    /// ends on its own, or throws a `TypeError` for a value that is no
    /// code.
    fn set_exit_code(state, _granted: ExitGrant; scope, code) for "@fuzdev/gro/exit.ts" {
        match (to_exit_code(code), state.try_borrow::<ExitState>()) {
            (None, _) => throw_type_error(scope, NOT_A_CODE),
            (Some(code), Some(exit)) => exit.set_code(code),
            (Some(_), None) => throw_error(scope, NO_EXIT_STATE),
        }
        None
    }

    /// `pid`, the process's id, asked of the OS when the run first imports
    /// the module.
    value pid(state; scope) for "@fuzdev/gro/pid.ts" {
        Some(v8::Integer::new_from_unsigned(scope, std::process::id()).into())
    }

    /// `runtime_version`, the runtime's name and version.
    value runtime_version(state; scope) for "@fuzdev/gro/runtime_version.ts" {
        string(scope, &state.borrow::<HostData>().runtime_version).map(Into::into)
    }

    /// `hash(data)`, the BLAKE3 digest of `data` ([`blake3::hash`]).
    fn hash(state; scope, data) for "@fuzdev/gro/blake3.ts" {
        blake3::hash(scope, data)
    }

    /// `create_hasher()`, a new hasher ([`blake3::hasher`]).
    fn create_hasher(state; scope) for "@fuzdev/gro/blake3.ts" {
        blake3::hasher(state, scope, blake3::Mode::Plain)
    }

    /// `create_keyed_hasher(key)`, a new hasher keyed by `key`
    /// ([`blake3::hasher`]).
    fn create_keyed_hasher(state; scope, key) for "@fuzdev/gro/blake3.ts" {
        blake3::hasher(state, scope, blake3::Mode::Keyed(key))
    }

    /// `create_derive_key_hasher(context)`, a new hasher deriving a key in
    /// `context` ([`blake3::hasher`]).
    fn create_derive_key_hasher(state; scope, context) for "@fuzdev/gro/blake3.ts" {
        blake3::hasher(state, scope, blake3::Mode::DeriveKey(context))
    }

    /// `hasher.update(data)`, called on `hasher` ([`blake3::update`]).
    fn update(state; scope, data) for "@fuzdev/gro/blake3.ts" on BLAKE3_HASHER(hasher) {
        blake3::update(state, scope, hasher, data);
        None
    }

    /// `hasher.finalize()`, called on `hasher` ([`blake3::finalize`]).
    fn finalize(state; scope) for "@fuzdev/gro/blake3.ts" on BLAKE3_HASHER(hasher) {
        blake3::finalize(state, scope, hasher)
    }
}
