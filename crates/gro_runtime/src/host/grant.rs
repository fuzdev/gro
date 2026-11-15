//! The check every host op and every native member makes first:
//! [`require`] of the grant it names, the one place a [`Granted`] token is
//! made, or, for a grant-free module's, [`require_authority`]. The op that
//! makes a row's namespace checks the row's grant, named at run time, with
//! [`require_grant`].

use std::marker::PhantomData;

use deno_core::OpState;
use deno_core::error::ModuleLoaderError as JsErrorBox;

use crate::resolver::Capability;
use crate::{ArgsGrant, Authority, EnvGrant, ExitGrant};

/// A host module's grant, as [`Grants`](crate::Grants) holds it.
pub trait Grant: 'static {
    /// The capability it grants.
    const CAPABILITY: Capability;
}

impl Grant for ArgsGrant {
    const CAPABILITY: Capability = Capability::Args;
}

impl Grant for ExitGrant {
    const CAPABILITY: Capability = Capability::Exit;
}

impl Grant for EnvGrant {
    const CAPABILITY: Capability = Capability::Env;
}

/// Proof that the run holds the grant `G`. Its field is private to this
/// module, so only [`require`] makes one.
#[derive(Debug)]
pub struct Granted<G: Grant> {
    /// The grant proved.
    grant: PhantomData<G>,
}

/// A host op called without what it requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotGranted {
    /// The grant it names, which the runtime's authority doesn't hold, or
    /// which a runtime with no authority can't.
    Grant(Capability),
    /// Any authority at all: a grant-free module's op, called in a runtime
    /// that holds none, eval's, which runs no host op.
    Authority,
}

impl NotGranted {
    /// What `name`, a host op or a native member, as `what` says, throws:
    /// a message that starts `not_granted` and names it, and the grant
    /// when it requires one.
    pub fn message(self, name: &str, what: &str) -> String {
        match self {
            Self::Grant(capability) => format!(
                "not_granted: {name} needs the grant '{capability}', which this runtime doesn't \
                 hold"
            ),
            Self::Authority => format!(
                "not_granted: {name} is a {what}, and this runtime holds no authority, so it runs \
                 none"
            ),
        }
    }

    /// The error the op named `op` throws: an `Error` of [`Self::message`].
    pub fn into_js(self, op: &str) -> JsErrorBox {
        JsErrorBox::generic(self.message(op, "host op"))
    }
}

/// Proof that the runtime's [`Authority`] holds the grant `G`, or
/// [`NotGranted::Grant`] when it doesn't: no authority in `state` at all
/// (eval's runtime), or a confined one that doesn't grant `G`. An
/// unconfined run holds every grant. The proof carries none of the grant's
/// data: an op that decides by a scope decides by the one it's handed (a
/// handle's), never by the run's.
pub fn require<G: Grant>(state: &OpState) -> Result<Granted<G>, NotGranted> {
    require_capability(state, G::CAPABILITY).map(|()| Granted { grant: PhantomData })
}

/// That the runtime's [`Authority`] holds `capability`, or
/// [`NotGranted::Grant`]: the one reading of the grants, which [`require`]
/// and [`require_grant`] share.
fn require_capability(state: &OpState, capability: Capability) -> Result<(), NotGranted> {
    let granted = match state.try_borrow::<Authority>() {
        Some(Authority::Confined(grants)) => grants.holds(capability),
        Some(Authority::Unconfined) => true,
        None => false,
    };
    if granted {
        Ok(())
    } else {
        Err(NotGranted::Grant(capability))
    }
}

/// What a row whose grant is `grant` requires, named at run time: that
/// grant, as [`require`] checks it, or, for a grant-free row, an authority
/// ([`require_authority`]). For the op that serves every row with native
/// members, which learns the row from its argument; it makes no token.
pub fn require_grant(state: &OpState, grant: Option<Capability>) -> Result<(), NotGranted> {
    grant.map_or_else(
        || require_authority(state),
        |capability| require_capability(state, capability),
    )
}

/// What a grant-free module's op requires: that the runtime holds an
/// [`Authority`], whatever it grants, as every run's does and eval's
/// doesn't, or [`NotGranted::Authority`]. No grant is read: the module
/// needs none.
pub fn require_authority(state: &OpState) -> Result<(), NotGranted> {
    match state.try_borrow::<Authority>() {
        Some(Authority::Confined(_) | Authority::Unconfined) => Ok(()),
        None => Err(NotGranted::Authority),
    }
}
