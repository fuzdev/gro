//! The host ops: the native half of the builtins, the only ops that reach
//! past pure computation on a host module's behalf.
//!
//! Every host op is declared through `host_ops!`, which makes its first
//! step [`require`] (in `grant.rs`, the one place a [`Granted`] token is
//! made) and lists it in `HOST_OPS`, so a host op can't skip the run's
//! grants. The runtime's tests hold the rest: every op gro's extension
//! registers, whatever its name, is either one of these or on the
//! bootstrap's ungated list, and each of these refuses when the run grants
//! nothing.
//!
//! The loader refuses an ungranted builtin at link time already, so a
//! refusal here is the second check, for a path to an op past the loader.

mod grant;

use deno_core::error::ModuleLoaderError as JsErrorBox;
use deno_core::{OpDecl, OpState, op2};

use grant::{Granted, require};

use crate::ProcessGrant;

/// The script's arguments, put in the runtime's `OpState` with the run's
/// [`Authority`](crate::Authority) when `fgro run`'s runtime starts.
#[derive(Debug, Clone, Default)]
pub struct ScriptArgs(pub Vec<String>);

/// Declares host ops, each an `#[op2]` whose first step is [`require`] of
/// the grant it names, refusing before its body runs (the
/// [`NotGranted`](grant::NotGranted) message names the op and the grant),
/// and lists them in `HOST_OPS` (for the tests) and [`host_ops`].
///
/// A body is the op's result, given the op state and the [`Granted`]
/// token. A `#[serde]` after the arrow is the op's return attribute, and
/// the return type is written as identifiers (`Vec<String>`), which
/// `#[op2]` reads token by token. Slow ops only: whether a fast call's
/// refusal throws on every path is unmeasured. Arguments join the pattern
/// with the first op that takes one.
macro_rules! host_ops {
    ($(
        $(#[doc = $doc:literal])*
        fn $name:ident($state:ident, $granted:ident: $grant:ty)
            -> $(#[$ret:ident])? $out:ident $(<$($out_arg:ident),+>)? $body:block
    )*) => {
        $(
            $(#[doc = $doc])*
            #[op2]
            $(#[$ret])?
            fn $name($state: &OpState) -> Result<$out $(<$($out_arg),+>)?, JsErrorBox> {
                let $granted: Granted<$grant> =
                    require::<$grant>($state).map_err(|refused| refused.into_js(stringify!($name)))?;
                Ok($body)
            }
        )*

        /// Every host op's name, in declaration order.
        #[cfg(test)]
        pub const HOST_OPS: &[&str] = &[$(stringify!($name)),*];

        /// Every host op, for the bootstrap's extension.
        pub fn host_ops() -> Vec<OpDecl> {
            vec![$($name()),*]
        }
    };
}

host_ops! {
    /// The script's arguments, a new array of strings each call.
    fn op_gro_process_args(state, _granted: ProcessGrant) -> #[serde] Vec<String> {
        state.borrow::<ScriptArgs>().0.clone()
    }
}
