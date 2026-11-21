//! The boxes' Landlock ruleset as installed, with its filesystem rules
//! (`rules.rs`), its network and scope walls, and the kernel floor they
//! need.
//!
//! Every box handles every filesystem right of [`LANDLOCK_ABI`], so it
//! grants only what its rules grant, and both scopes: no signal to a
//! process outside its domain, and no connection to an abstract unix
//! socket outside it. Its TCP wall is its caller's ([`TcpWall`]).

use super::ConfineError;
use super::rules::BoxRules;

/// Whether a box's Landlock refuses binding and connecting TCP sockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TcpWall {
    /// Refused, every port: eval's box, and the run's in the host's network.
    Refused,
    /// Left to the run's own network namespace, which holds only its own
    /// loopback, so a test binding `127.0.0.1` binds and connects there.
    Unhandled,
}

/// Installs the Landlock rules `rules`, each granting its rights beneath its
/// handle, opened before, with the TCP wall `tcp`, after `no_new_privs`,
/// which the kernel requires first and Landlock sets.
pub(super) fn landlock(rules: &BoxRules, tcp: TcpWall) -> Result<(), ConfineError> {
    use landlock::{
        Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, RestrictSelf, Ruleset,
        RulesetAttr, RulesetCreatedAttr, RulesetError, RulesetStatus, Scope,
    };
    // the kernel's status, read with nothing installed: no flags, and
    // `no_new_privs` left to the ruleset
    let status = RestrictSelf::default()
        .no_new_privs(false)
        .apply()
        .map_err(ConfineError::Landlock)?;
    landlock_floor(status.landlock)?;
    let path_rules = rules
        .rules()
        .iter()
        .map(|rule| Ok::<_, RulesetError>(PathBeneath::new(rule.handle(), rule.access())));
    // the crate refuses an empty set of rights, so an unhandled wall is no
    // call
    let restricted = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(LANDLOCK_ABI))
        .and_then(|ruleset| match tcp {
            TcpWall::Refused => ruleset.handle_access(AccessNet::from_all(LANDLOCK_ABI)),
            TcpWall::Unhandled => Ok(ruleset),
        })
        .and_then(|ruleset| ruleset.scope(Scope::from_all(LANDLOCK_ABI)))
        .and_then(Ruleset::create)
        .and_then(|ruleset| ruleset.add_rules(path_rules))
        .and_then(landlock::RulesetCreated::restrict_self)
        .map_err(ConfineError::Landlock)?;
    if restricted.ruleset != RulesetStatus::FullyEnforced || !restricted.no_new_privs {
        return Err(ConfineError::LandlockPartial);
    }
    Ok(())
}

/// The Landlock ABI the box's rules are written for, and the least the
/// kernel's may be.
pub(super) const LANDLOCK_ABI: landlock::ABI = landlock::ABI::V6;

/// Refuses a kernel whose Landlock, as `status` reports it, can't hold the
/// box: none, switched off, or below [`LANDLOCK_ABI`].
pub(super) fn landlock_floor(status: landlock::LandlockStatus) -> Result<(), ConfineError> {
    use landlock::LandlockStatus;
    match status {
        LandlockStatus::NotEnabled => Err(ConfineError::NoLandlock { built: true }),
        LandlockStatus::NotImplemented => Err(ConfineError::NoLandlock { built: false }),
        LandlockStatus::Available { effective_abi, .. } if effective_abi < LANDLOCK_ABI => {
            Err(ConfineError::LandlockAbi {
                abi: effective_abi as u32,
            })
        }
        LandlockStatus::Available { .. } => Ok(()),
    }
}
