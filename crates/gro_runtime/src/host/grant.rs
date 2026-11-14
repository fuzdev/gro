//! The grant check every host op makes first: [`require`], the one place a
//! [`Granted`] token is made.

use deno_core::OpState;
use deno_core::error::ModuleLoaderError as JsErrorBox;

use crate::resolver::Capability;
use crate::{ArgsGrant, Authority, Grants};

/// A host module's grant, as [`Grants`] holds it.
pub trait Grant: Clone + 'static {
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

/// Proof that the run holds the grant `G`, carrying its own copy of it, so
/// an op holds no borrow of the `OpState` while it works. Its field is
/// private to this module, so only [`require`] makes one.
#[derive(Debug)]
pub struct Granted<G: Grant> {
    /// The grant, scoped data an op reads once a grant carries some.
    #[expect(dead_code, reason = "no grant carries data yet")]
    grant: G,
}

/// A host op called without the grant it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotGranted(pub Capability);

impl NotGranted {
    /// The error the op named `op` throws: an `Error` whose message starts
    /// `not_granted` and names the grant.
    pub fn into_js(self, op: &str) -> JsErrorBox {
        JsErrorBox::generic(format!(
            "not_granted: {op} needs the grant '{}', which this runtime doesn't hold",
            self.0
        ))
    }
}

/// The grant `G` the runtime's [`Authority`] holds, or [`NotGranted`] when it
/// holds none: no authority in `state` at all (eval's runtime), or one that
/// doesn't grant `G`.
pub fn require<G: Grant>(state: &OpState) -> Result<Granted<G>, NotGranted> {
    let grant = match state.try_borrow::<Authority>() {
        Some(Authority::Confined(grants)) => G::of(grants),
        None => None,
    };
    grant
        .cloned()
        .map(|grant| Granted { grant })
        .ok_or(NotGranted(G::CAPABILITY))
}
