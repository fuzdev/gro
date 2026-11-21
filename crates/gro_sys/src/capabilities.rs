//! Dropping every capability the process holds, before its box goes up.
//!
//! A process that entered a user namespace of its own holds every
//! capability in it ([`crate::namespace`]), and a parent can hand one down
//! (an ambient set raised by a service manager). [`drop_capabilities`]
//! empties them all: the bounding set, which limits what an `exec` may
//! grant, dropped number by number while the process still may
//! (`CAP_SETPCAP`), then the effective, permitted, and inheritable sets by
//! one `capset` to zero, which empties the ambient set too, as the kernel
//! keeps it within both the permitted and the inheritable. Then it reads
//! the sets back from `/proc/thread-self/status` and fails unless each it
//! emptied is empty.
//!
//! It's defense in depth, not a boundary of its own: capabilities held in
//! the process's own user namespace reach only what that namespace owns,
//! and with `no_new_privs`, which the box sets, no `exec` grants any. The
//! drop takes from an exploit of the process what the namespace would let
//! it do there, such as making mounts or changing its network, and holds
//! only while the process can't enter a user namespace nested in its own,
//! which would hand every capability back inside it: refused by the
//! namespace's limit
//! ([`crate::namespace::UserNetNamespace::refuse_nested_user_namespaces`])
//! and by a seccomp filter refusing namespace calls.

use std::io;

/// `_LINUX_CAPABILITY_VERSION_3`, the version of `capset`'s two 32-bit
/// halves of each set.
const CAPABILITY_VERSION_3: u32 = 0x2008_0522;

/// The most capability numbers the kernel's sets hold: 64 bits each.
const CAPABILITY_BITS: u32 = 64;

/// `struct __user_cap_header_struct`, which the `libc` crate lacks.
#[repr(C)]
struct CapHeader {
    /// [`CAPABILITY_VERSION_3`].
    version: u32,
    /// The thread, 0 for the calling one.
    pid: libc::c_int,
}

/// `struct __user_cap_data_struct`, which the `libc` crate lacks: one
/// 32-bit half of each set.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapData {
    /// The effective set's half.
    effective: u32,
    /// The permitted set's half.
    permitted: u32,
    /// The inheritable set's half.
    inheritable: u32,
}

/// The calling thread's capability sets, each a bit per capability number,
/// as `/proc/thread-self/status` shows them, and as it prints them, in hex.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapabilitySets {
    /// `CapInh`: what an `exec` may keep.
    pub inheritable: u64,
    /// `CapPrm`: what the thread may make effective.
    pub permitted: u64,
    /// `CapEff`: what the kernel checks.
    pub effective: u64,
    /// `CapBnd`: the most an `exec` may grant.
    pub bounding: u64,
    /// `CapAmb`: what an `exec` of a program with no file capabilities
    /// keeps.
    pub ambient: u64,
}

/// What became of the bounding set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundingSet {
    /// Emptied.
    Dropped,
    /// Kept as the parent left it, since the process doesn't hold
    /// `CAP_SETPCAP`, which dropping one takes: a process outside a user
    /// namespace of its own, whose bounding set grants nothing under
    /// `no_new_privs`.
    Kept,
}

impl std::fmt::Display for CapabilitySets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CapInh {:016x}, CapPrm {:016x}, CapEff {:016x}, CapBnd {:016x}, CapAmb {:016x}",
            self.inheritable, self.permitted, self.effective, self.bounding, self.ambient
        )
    }
}

/// Why [`drop_capabilities`] failed. Some sets may be emptied.
#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    /// The thread count couldn't be read.
    #[error("failed to read the thread count")]
    ThreadsUnread(#[source] io::Error),
    /// The process has more than one thread, and capabilities are each
    /// thread's own; nothing was dropped.
    #[error("the process has {count} threads, not 1, and each holds its own capabilities")]
    Threads {
        /// The thread count read.
        count: u64,
    },
    /// Dropping `capability` from the bounding set failed.
    #[error("failed to drop capability {capability} from the bounding set")]
    Bounding {
        /// The capability's number.
        capability: u32,
        /// The call's error.
        #[source]
        source: io::Error,
    },
    /// `capset` failed.
    #[error("failed to empty the capability sets")]
    Set(#[source] io::Error),
    /// The sets couldn't be read back.
    #[error("failed to read the capability sets back")]
    Unread(#[source] io::Error),
    /// The sets read back hold a capability.
    #[error("a capability is still held: {0}")]
    Held(CapabilitySets),
}

/// Drops every capability the process holds, as the [module doc](self)
/// says, and returns what became of the bounding set.
///
/// Capabilities are each thread's own, so it refuses unless the process
/// has one thread, once a thread just joined has left the count
/// ([`crate::settled_thread_count`]): none can start but from this one,
/// which is in this call. A thread started after inherits the empty sets.
///
/// # Errors
///
/// Returns the step that failed ([`CapabilityError`]), the sets then
/// partly emptied, or [`CapabilityError::Held`] when a set read back isn't
/// empty: the effective, permitted, inheritable, or ambient, or the
/// bounding set once dropped.
pub fn drop_capabilities() -> Result<BoundingSet, CapabilityError> {
    let count = crate::settled_thread_count().map_err(CapabilityError::ThreadsUnread)?;
    if count != 1 {
        return Err(CapabilityError::Threads { count });
    }
    let bounding = drop_bounding_set()?;
    set_empty().map_err(CapabilityError::Set)?;
    let sets = capability_sets().map_err(CapabilityError::Unread)?;
    let bounding_held = bounding == BoundingSet::Dropped && sets.bounding != 0;
    if sets.inheritable | sets.permitted | sets.effective | sets.ambient != 0 || bounding_held {
        return Err(CapabilityError::Held(sets));
    }
    Ok(bounding)
}

/// Drops each capability from the bounding set by `PR_CAPBSET_DROP`,
/// numbered from 0 until the kernel knows no more (`EINVAL`), so a number
/// a newer kernel adds is dropped too; `EPERM` for the first, without
/// `CAP_SETPCAP`, keeps the set.
#[expect(unsafe_code, reason = "`prctl` is a libc call, which is `unsafe`")]
fn drop_bounding_set() -> Result<BoundingSet, CapabilityError> {
    let zero: libc::c_ulong = 0;
    for capability in 0..CAPABILITY_BITS {
        // SAFETY: `PR_CAPBSET_DROP` takes integers and touches no memory of
        // the process's
        let result = unsafe {
            libc::prctl(
                libc::PR_CAPBSET_DROP,
                libc::c_ulong::from(capability),
                zero,
                zero,
                zero,
            )
        };
        if result == 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            // past the last capability the kernel knows
            Some(libc::EINVAL) if capability > 0 => return Ok(BoundingSet::Dropped),
            Some(libc::EPERM) if capability == 0 => return Ok(BoundingSet::Kept),
            _ => {
                return Err(CapabilityError::Bounding {
                    capability,
                    source: error,
                });
            }
        }
    }
    Err(CapabilityError::Bounding {
        capability: CAPABILITY_BITS,
        source: io::Error::other("the kernel knows more capabilities than its sets hold"),
    })
}

/// Empties the calling thread's effective, permitted, and inheritable sets
/// by one `capset`, which empties its ambient set with them.
#[expect(
    unsafe_code,
    reason = "std has no `capset`, and a raw system call is `unsafe`"
)]
fn set_empty() -> io::Result<()> {
    let header = CapHeader {
        version: CAPABILITY_VERSION_3,
        pid: 0,
    };
    let data = [CapData::default(); 2];
    // SAFETY: `capset` reads the header and the two halves the pointers
    // name, live locals of the layout version 3 takes, and writes neither
    // (the header only on a version it doesn't know, which this isn't)
    let result = unsafe { libc::syscall(libc::SYS_capset, &raw const header, data.as_ptr()) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The calling thread's capability sets, from `/proc/thread-self/status`.
///
/// # Errors
///
/// Returns an error when the file can't be read or lacks a set.
pub(crate) fn capability_sets() -> io::Result<CapabilitySets> {
    let status = std::fs::read_to_string("/proc/thread-self/status")?;
    let set = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| u64::from_str_radix(value.trim(), 16).ok())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("/proc/thread-self/status has no {name}"),
                )
            })
    };
    Ok(CapabilitySets {
        inheritable: set("CapInh:")?,
        permitted: set("CapPrm:")?,
        effective: set("CapEff:")?,
        bounding: set("CapBnd:")?,
        ambient: set("CapAmb:")?,
    })
}

#[cfg(test)]
mod tests {
    use super::{CapabilityError, CapabilitySets};

    /// A set still held prints in hex, as `/proc` shows it.
    #[test]
    fn a_held_set_prints_in_hex() {
        let sets = CapabilitySets {
            bounding: 1 << 40,
            ..CapabilitySets::default()
        };
        assert_eq!(
            CapabilityError::Held(sets).to_string(),
            "a capability is still held: CapInh 0000000000000000, CapPrm 0000000000000000, \
             CapEff 0000000000000000, CapBnd 0000010000000000, CapAmb 0000000000000000"
        );
    }
}
