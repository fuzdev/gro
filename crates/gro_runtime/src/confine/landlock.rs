//! The box's Landlock rules: the filesystem, network, and scope walls, and
//! the kernel floor they need.

use super::{ConfineError, Pin};

/// Installs the Landlock rules, reading beneath each of `pins`, the roots'
/// handles opened before, after `no_new_privs`, which the kernel requires
/// first and Landlock sets.
pub(super) fn landlock(pins: &[Pin]) -> Result<(), ConfineError> {
    use std::os::fd::AsFd as _;

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
    let restricted = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(LANDLOCK_ABI))
        .and_then(|ruleset| ruleset.handle_access(AccessNet::from_all(LANDLOCK_ABI)))
        .and_then(|ruleset| ruleset.scope(Scope::from_all(LANDLOCK_ABI)))
        .and_then(Ruleset::create)
        .and_then(|ruleset| {
            ruleset.add_rules(pins.iter().map(|pin| {
                Ok::<_, RulesetError>(PathBeneath::new(
                    pin.as_fd(),
                    AccessFs::ReadFile | AccessFs::ReadDir,
                ))
            }))
        })
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
