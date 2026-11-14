//! The build's identity: the commit it was built from, validated at compile
//! time, and the `--version` line.

use gro_runtime::phases;
use gro_runtime::prices;

use crate::snapshot::STARTUP_SNAPSHOT;

/// The commit this binary was built from, read from `GRO_COMMIT` at compile
/// time, never at runtime: a full SHA, with `-dirty` appended when the working
/// tree differed from it. `cargo xtask build` sets it for its stage-two build,
/// and only that build's `snapshot` feature reads it, so a plain `cargo build`
/// has none whatever the shell sets; unset or empty, there is none either.
/// Cargo records the variable as an input, so a changed value rebuilds the
/// crate.
pub const COMMIT: Option<&str> = if cfg!(feature = "snapshot") {
    commit(option_env!("GRO_COMMIT"))
} else {
    None
};

/// The runtime's name and version, what a script reads as `runtime_version`
/// from the `runtime_version` host module: opaque, compared for equality only.
pub const RUNTIME_VERSION: &str = concat!("fgro/", env!("CARGO_PKG_VERSION"));

/// What `--version` prints for a build without [`COMMIT`].
pub const COMMIT_UNKNOWN: &str = "unknown";

/// The suffix [`COMMIT`] carries when the working tree differed from the commit.
const DIRTY_SUFFIX: &str = "-dirty";

/// Validates a `GRO_COMMIT` value: `None` when it's unset or empty, else a
/// full SHA (40 or 64 lowercase hex digits), optionally followed by
/// [`DIRTY_SUFFIX`]. A malformed value panics, which, evaluated for
/// [`COMMIT`], fails the build rather than printing it.
const fn commit(value: Option<&'static str>) -> Option<&'static str> {
    let Some(value) = value else {
        return None;
    };
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let suffix = DIRTY_SUFFIX.as_bytes();
    let mut sha_len = bytes.len();
    if sha_len > suffix.len() {
        let start = sha_len - suffix.len();
        let mut i = 0;
        while i < suffix.len() && bytes[start + i] == suffix[i] {
            i += 1;
        }
        if i == suffix.len() {
            sha_len = start;
        }
    }
    assert!(
        sha_len == 40 || sha_len == 64,
        "GRO_COMMIT must be a full commit SHA, optionally followed by `-dirty`"
    );
    let mut i = 0;
    while i < sha_len {
        assert!(
            matches!(bytes[i], b'0'..=b'9' | b'a'..=b'f'),
            "GRO_COMMIT's SHA must be lowercase hex"
        );
        i += 1;
    }
    Some(value)
}

/// The `--version` line:
/// `fgro <version> (commit <sha>[-dirty], V8 <version>, snapshot)`, with
/// `commit unknown` for a build without [`COMMIT`] and `no snapshot` for a
/// build without one, and `, phases` before the parenthesis closes in a
/// build that records phases (`gro_runtime::phases::ENABLED`), and
/// `, prices` in one that reads the prices switches
/// (`gro_runtime::prices::ENABLED`).
pub fn version_line() -> String {
    format!(
        "fgro {} (commit {}, V8 {}, {}{}{})",
        env!("CARGO_PKG_VERSION"),
        COMMIT.unwrap_or(COMMIT_UNKNOWN),
        gro_runtime::v8_version(),
        if STARTUP_SNAPSHOT.is_some() {
            "snapshot"
        } else {
            "no snapshot"
        },
        if phases::ENABLED { ", phases" } else { "" },
        if prices::ENABLED { ", prices" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_accepts_a_full_sha_and_the_dirty_suffix() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(commit(None), None);
        assert_eq!(commit(Some("")), None);
        assert_eq!(commit(Some(sha)), Some(sha));
        let dirty = "0123456789abcdef0123456789abcdef01234567-dirty";
        assert_eq!(commit(Some(dirty)), Some(dirty));
        let sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(commit(Some(sha256)), Some(sha256));
    }

    #[test]
    fn commit_refuses_anything_else() {
        for value in [
            "unknown",
            "0123456",
            "0123456789abcdef0123456789abcdef01234567-",
            "0123456789abcdef0123456789abcdef01234567-clean",
            "0123456789ABCDEF0123456789ABCDEF01234567",
            "0123456789abcdef0123456789abcdef0123456g",
            "-dirty",
        ] {
            let result = std::panic::catch_unwind(|| commit(Some(value)));
            assert!(result.is_err(), "{value}");
        }
    }

    /// Only a `snapshot` build reads `GRO_COMMIT`, so the plain build
    /// `cargo test` runs has no commit, whatever the shell sets.
    #[test]
    fn version_line_names_the_commit() {
        let line = version_line();
        let expected = option_env!("GRO_COMMIT")
            .filter(|value| cfg!(feature = "snapshot") && !value.is_empty())
            .unwrap_or(COMMIT_UNKNOWN);
        assert!(
            line.starts_with(&format!(
                "fgro {} (commit {expected}, V8 ",
                env!("CARGO_PKG_VERSION")
            )),
            "{line}"
        );
    }
}
