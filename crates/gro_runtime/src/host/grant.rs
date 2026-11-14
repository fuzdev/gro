//! The check every host op makes first: [`require`] of the grant it names,
//! the one place a [`Granted`] token is made, or, for a grant-free
//! module's op, [`require_authority`].

use std::marker::PhantomData;

use deno_core::OpState;
use deno_core::error::ModuleLoaderError as JsErrorBox;

use crate::resolver::Capability;
use crate::{ArgsGrant, Authority, ExitGrant, Grants};

/// A host module's grant, as [`Grants`] holds it.
pub trait Grant: 'static {
    /// The capability it grants.
    const CAPABILITY: Capability;

    /// This grant in `grants`, if it's granted.
    fn of(grants: &Grants) -> Option<&Self>;
}

impl Grant for ArgsGrant {
    const CAPABILITY: Capability = Capability::Args;

    fn of(grants: &Grants) -> Option<&Self> {
        grants.args.as_ref()
    }
}

impl Grant for ExitGrant {
    const CAPABILITY: Capability = Capability::Exit;

    fn of(grants: &Grants) -> Option<&Self> {
        grants.exit.as_ref()
    }
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
    /// The error the op named `op` throws: an `Error` whose message starts
    /// `not_granted` and names the op, and the grant when it requires one.
    pub fn into_js(self, op: &str) -> JsErrorBox {
        JsErrorBox::generic(match self {
            Self::Grant(capability) => format!(
                "not_granted: {op} needs the grant '{capability}', which this runtime doesn't hold"
            ),
            Self::Authority => format!(
                "not_granted: {op} is a host op, and this runtime holds no authority, so it runs \
                 none"
            ),
        })
    }
}

/// The grant `G` the runtime's [`Authority`] holds, or
/// [`NotGranted::Grant`] when it holds none: no authority in `state` at all
/// (eval's runtime), or one that doesn't grant `G`.
pub fn require<G: Grant>(state: &OpState) -> Result<Granted<G>, NotGranted> {
    let grant = match state.try_borrow::<Authority>() {
        Some(Authority::Confined(grants)) => G::of(grants),
        None => None,
    };
    grant
        .map(|_| Granted { grant: PhantomData })
        .ok_or(NotGranted::Grant(G::CAPABILITY))
}

/// What a grant-free module's op requires: that the runtime holds an
/// [`Authority`], whatever it grants, as every run's does and eval's
/// doesn't, or [`NotGranted::Authority`]. No grant is read: the module
/// needs none.
pub fn require_authority(state: &OpState) -> Result<(), NotGranted> {
    match state.try_borrow::<Authority>() {
        Some(Authority::Confined(_)) => Ok(()),
        None => Err(NotGranted::Authority),
    }
}
