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

mod grant;

use deno_core::error::ModuleLoaderError as JsErrorBox;
use deno_core::{OpDecl, OpState, op2};

#[cfg(test)]
use grant::Grant;
use grant::{Granted, require, require_authority};

use crate::ArgsGrant;
#[cfg(test)]
use crate::resolver::Capability;

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
/// A body is the op's result, given the op state. A `#[serde]` or
/// `#[string]` after the arrow is the op's return attribute, and the return
/// type is written as identifiers (`Vec<String>`), which `#[op2]` reads
/// token by token. Slow ops only: whether a fast call's refusal throws on
/// every path is unmeasured, so an op whose signature a fast call could
/// serve is written `@nofast fn`, which `#[op2]` otherwise refuses to
/// compile. Arguments join the pattern with the first op that takes one.
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
    ($(
        $(#[doc = $doc:literal])*
        $(@$slow:ident)? fn $name:ident($state:ident $(, $granted:ident: $grant:ty)?)
            for $module:literal
            -> $(#[$ret:ident])? $out:ident $(<$($out_arg:ident),+>)? $body:block
    )*) => {
        $(
            $(#[doc = $doc])*
            #[op2$(($slow))?]
            $(#[$ret])?
            fn $name($state: &OpState) -> Result<$out $(<$($out_arg),+>)?, JsErrorBox> {
                host_ops!(@require $name, $state $(, $granted: $grant)?);
                Ok($body)
            }
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

host_ops! {
    /// The script's arguments, a new array of strings each call.
    fn op_gro_args(state, _granted: ArgsGrant) for "@fuzdev/gro/args.ts"
        -> #[serde] Vec<String>
    {
        state.borrow::<HostData>().args.clone()
    }

    /// The working directory the run was started in, or `null` when its
    /// path isn't UTF-8.
    fn op_gro_cwd(state) for "@fuzdev/gro/cwd.ts" -> #[string] Option<String> {
        state.borrow::<HostData>().cwd.clone()
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
