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
//! The loader refuses an ungranted builtin at link time already, and every
//! builtin in an eval, so a refusal here is the second check, for a path to
//! an op past the loader.

pub mod env;
pub mod error_class;
pub mod exit;
mod grant;
mod handle;

use deno_core::error::ModuleLoaderError as JsErrorBox;
use deno_core::{OpDecl, OpState, op2, v8};

use error_class::{ENV_ERROR, ErrorClasses};
use exit::{ExitState, NOT_A_CODE, to_exit_code};
#[cfg(test)]
use grant::Grant;
use grant::{Granted, require, require_authority};

use crate::loader::RefusalLog;
#[cfg(test)]
use crate::resolver::Capability;
use crate::{ArgsGrant, EnvGrant, EnvVars, ExitGrant};

/// What an exit op throws in a runtime started without the exit host
/// module's state, which only a runtime made outside a run's start is: a
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
    /// The builtin module it serves, by its specifier in the builtin table.
    pub module: &'static str,
    /// The grant it requires, or none for a grant-free module's op.
    pub grant: Option<Capability>,
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
/// be written for a module that needs one.
///
/// An op that takes arguments names them after a `;`, the scope first:
/// `fn name(state, granted: Grant; scope, value) for "specifier"`. Each
/// argument is the JS value as it was passed, a `v8::Local<'s, v8::Value>`,
/// never converted on the way in: `#[op2]`'s typed arguments convert, and
/// a conversion can run the caller's code or truncate, so the body reads
/// the value natively and throws through the scope. The check still comes
/// first, before the body sees an argument.
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
/// takes the scope has no such signature, so it's slow as it's written.
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
        [$scope:ident $(, $arg:ident)*], [], $body:block
    ) => {
        host_ops!(
            @op [$($attr)*] $name, $state, [$($granted)*], [$scope $(, $arg)*], [()], $body
        );
    };
    (
        @op [$($attr:tt)*] $name:ident, $state:ident, [$($granted:tt)*],
        [$scope:ident $(, $arg:ident)*], [$($out:tt)+], $body:block
    ) => {
        $($attr)*
        fn $name<'s>(
            $state: &OpState,
            $scope: &v8::PinScope<'s, '_>
            $(, $arg: v8::Local<'s, v8::Value>)*
        ) -> Result<$($out)+, JsErrorBox> {
            host_ops!(@require $name, $state $($granted)*);
            Ok($body)
        }
    };
    ($(
        $(#[doc = $doc:literal])*
        $(@$slow:ident)? fn $name:ident(
            $state:ident $(, $granted:ident: $grant:ty)? $(; $scope:ident $(, $arg:ident)*)?
        )
            for $module:literal
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
                [$($scope $(, $arg)*)?],
                [$($($out)::+ $(<$($out_lt,)? $($($out_arg)::+),+>)?)?],
                $body
            );
        )*

        /// Every host op, in declaration order.
        #[cfg(test)]
        pub const HOST_OPS: &[HostOp] = &[$(HostOp {
            name: stringify!($name),
            module: $module,
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
    /// The script's arguments, a new array of strings each call.
    fn op_gro_args(state, _granted: ArgsGrant) for "@fuzdev/gro/args.ts"
        -> #[serde] Vec<String>
    {
        state.borrow::<HostData>().args.clone()
    }

    /// Ends the run now, by the embedder's step, with `code`, or, given
    /// `undefined`, with the code `set_exit_code` set, else 0. Any other
    /// value throws a `TypeError` and ends nothing.
    fn op_gro_exit(state, _granted: ExitGrant; scope, code) for "@fuzdev/gro/exit.ts" {
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
    }

    /// Sets the code the run exits with when it ends on its own, or
    /// throws a `TypeError` for a value that is no code.
    fn op_gro_set_exit_code(state, _granted: ExitGrant; scope, code)
        for "@fuzdev/gro/exit.ts"
    {
        match (to_exit_code(code), state.try_borrow::<ExitState>()) {
            (None, _) => throw_type_error(scope, NOT_A_CODE),
            (Some(code), Some(exit)) => exit.set_code(code),
            (Some(_), None) => throw_error(scope, NO_EXIT_STATE),
        }
    }

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

    /// The working directory the run was started in, or, when its path
    /// isn't UTF-8, which a JS string can't hold, an `Error` thrown saying
    /// so.
    fn op_gro_cwd(state; scope) for "@fuzdev/gro/cwd.ts" -> v8::Local<'s, v8::Value> {
        let cwd = state
            .borrow::<HostData>()
            .cwd
            .as_deref()
            .and_then(|cwd| v8::String::new(scope, cwd));
        cwd.map_or_else(
            || {
                throw_error(scope, CWD_NOT_UTF8);
                v8::undefined(scope).into()
            },
            Into::into,
        )
    }

    /// The process's id, asked of the OS when called.
    @nofast fn op_gro_pid(state) for "@fuzdev/gro/pid.ts" -> u32 {
        std::process::id()
    }

    /// The runtime's name and version.
    fn op_gro_runtime_version(state) for "@fuzdev/gro/runtime_version.ts" -> #[string] String {
        state.borrow::<HostData>().runtime_version.clone()
    }
}
