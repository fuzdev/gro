//! The boxes' Landlock ruleset as installed, with its filesystem rules
//! (`rules.rs`), its network and scope walls, and the kernel floor they
//! need: built by [`ruleset`] and installed by [`restrict`] for every box,
//! the run's, eval's, and a program's nested in the run's (`program.rs`).
//!
//! Every box handles every filesystem right of [`LANDLOCK_ABI`], so it
//! grants only what its rules grant, and both scopes: no signal to a
//! process outside its domain, and no connection to an abstract unix
//! socket outside it. Its TCP wall is its caller's ([`TcpWall`]).

use std::os::fd::BorrowedFd;

use landlock::{AccessFs, BitFlags, RulesetCreated, RulesetStatus};

use super::ConfineError;
use super::rules::BoxRules;

/// Whether a box's Landlock refuses binding and connecting TCP sockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TcpWall {
    /// Refused, every port: eval's box, and the run's in the host's network.
    Refused,
    /// Left to what holds the box: for the run's, its own network
    /// namespace, which holds only its own loopback, so a test binding
    /// `127.0.0.1` binds and connects there; for a program's, the run's box
    /// it nests in, since the kernel checks an access against every layer
    /// of a domain, so a program binds and connects as the run does.
    Unhandled,
}

/// Installs the Landlock rules `rules`, each granting its rights beneath its
/// handle, opened before, with the TCP wall `tcp`, after `no_new_privs`,
/// which the kernel requires first and Landlock sets: the run's box and
/// eval's, on a kernel at the floor ([`probe_floor`]).
pub(super) fn landlock(rules: &BoxRules, tcp: TcpWall) -> Result<(), ConfineError> {
    probe_floor()?;
    let path_rules = rules
        .rules()
        .iter()
        .map(|rule| (rule.handle(), rule.access()));
    restrict(ruleset(path_rules, tcp)?)
}

/// A ruleset made and not installed, every filesystem right of
/// [`LANDLOCK_ABI`] and both scopes handled, the TCP wall `tcp`'s, granting
/// each of `rules`' Landlock rights beneath its handle: the one way every
/// box's ruleset is built, the run's, eval's, and a program's
/// (`program::ProgramBox`), each a hard requirement, so a kernel that can't
/// hold every right fails it rather than dropping one.
///
/// # Errors
///
/// Returns [`ConfineError::Landlock`] when the kernel refuses the ruleset or
/// a rule.
pub(super) fn ruleset<'fd>(
    rules: impl IntoIterator<Item = (BorrowedFd<'fd>, BitFlags<AccessFs>)>,
    tcp: TcpWall,
) -> Result<RulesetCreated, ConfineError> {
    use landlock::{
        Access, AccessNet, CompatLevel, Compatible, PathBeneath, Ruleset, RulesetAttr,
        RulesetCreatedAttr, RulesetError, Scope,
    };
    let path_rules = rules
        .into_iter()
        .map(|(handle, access)| Ok::<_, RulesetError>(PathBeneath::new(handle, access)));
    // the crate refuses an empty set of rights, so an unhandled wall is no
    // call
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(LANDLOCK_ABI))
        .and_then(|ruleset| match tcp {
            TcpWall::Refused => ruleset.handle_access(AccessNet::from_all(LANDLOCK_ABI)),
            TcpWall::Unhandled => Ok(ruleset),
        })
        .and_then(|ruleset| ruleset.scope(Scope::from_all(LANDLOCK_ABI)))
        .and_then(Ruleset::create)
        .and_then(|ruleset| ruleset.add_rules(path_rules))
        .map_err(ConfineError::Landlock)
}

/// Installs `ruleset` on the calling thread alone, nested in whatever
/// domain the thread is in, after `no_new_privs`, refusing a ruleset the
/// kernel enforced only in part: every thread and process the thread
/// starts afterward inherits it, and no other thread is changed.
///
/// # Errors
///
/// Returns [`ConfineError::Landlock`] when the install fails, or
/// [`ConfineError::LandlockPartial`] when it held only in part.
pub(super) fn restrict(ruleset: RulesetCreated) -> Result<(), ConfineError> {
    let restricted = ruleset.restrict_self().map_err(ConfineError::Landlock)?;
    if restricted.ruleset != RulesetStatus::FullyEnforced || !restricted.no_new_privs {
        return Err(ConfineError::LandlockPartial);
    }
    Ok(())
}

/// Reads the kernel's Landlock status with nothing installed (no flags, so
/// no `restrict_self` call, and `no_new_privs` left to the ruleset) and
/// refuses one below the floor ([`landlock_floor`]).
pub(super) fn probe_floor() -> Result<(), ConfineError> {
    let status = ::landlock::RestrictSelf::default()
        .no_new_privs(false)
        .apply()
        .map_err(ConfineError::Landlock)?;
    landlock_floor(status.landlock)
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
