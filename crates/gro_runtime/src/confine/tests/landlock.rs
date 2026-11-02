//! Tests for the Landlock floor.

use crate::confine::landlock::{LANDLOCK_ABI, landlock_floor};
use crate::confine::{ConfineError, REQUIRED_LANDLOCK_ABI};

/// A kernel with no Landlock, with it off, or below ABI 6 is refused,
/// naming what is missing; ABI 6 and later pass.
#[cfg(target_os = "linux")]
#[test]
fn the_landlock_floor_is_abi_6() {
    use landlock::{ABI, LandlockStatus};
    let available = |abi| LandlockStatus::Available {
        effective_abi: abi,
        kernel_abi: None,
    };
    for abi in [ABI::V1, ABI::V4, ABI::V5] {
        let error = landlock_floor(available(abi)).unwrap_err();
        assert!(
            matches!(error, ConfineError::LandlockAbi { abi: found } if found == abi as u32),
            "{error:?}"
        );
        assert!(error.is_host_unsupported());
        assert!(
            error.to_string().starts_with(&format!(
                "the kernel's Landlock is ABI {}, below the ABI 6",
                abi as u32
            )),
            "{error}"
        );
    }
    for (status, says) in [
        (LandlockStatus::NotImplemented, "the kernel has no Landlock"),
        (
            LandlockStatus::NotEnabled,
            "the kernel has Landlock but it isn't enabled",
        ),
    ] {
        let error = landlock_floor(status).unwrap_err();
        assert!(error.is_host_unsupported());
        assert!(error.to_string().starts_with(says), "{error}");
    }
    for abi in [ABI::V6, ABI::V7, ABI::V9] {
        assert!(landlock_floor(available(abi)).is_ok());
    }
    // the number the messages name is the floor's
    assert_eq!(LANDLOCK_ABI as u32, REQUIRED_LANDLOCK_ABI);
}
