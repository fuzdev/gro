//! The build's identity: the commit it was built from, validated at compile
//! time, the runtime's version, which carries it, and the `--version` line.

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

/// The runtime's name and its package version, all of [`RUNTIME_VERSION`] in
/// a build without [`COMMIT`].
const NAME_AND_VERSION: &str = concat!("fgro/", env!("CARGO_PKG_VERSION"));

/// What [`RUNTIME_VERSION`] puts between the version and [`COMMIT`], as
/// semver's build metadata is written.
const COMMIT_SEPARATOR: u8 = b'+';

/// The runtime's name and version, what a script reads as `runtime_version`
/// from the `runtime_version` host module: `fgro/<version>`, followed in a
/// build with a [`COMMIT`] by `+` and the whole of it, the full SHA and its
/// `-dirty` suffix as `--version` prints them, so builds of different
/// commits never name themselves alike, which a cache keyed by the string
/// relies on. A `-dirty` build's names the commit its tree differed from,
/// not its sources. Opaque, compared for equality only, and a constant like
/// [`COMMIT`]: nothing is read at runtime.
pub const RUNTIME_VERSION: &str = {
    const LEN: usize = runtime_version_len(NAME_AND_VERSION, COMMIT);
    const BYTES: [u8; LEN] = runtime_version_bytes(NAME_AND_VERSION, COMMIT);
    match std::str::from_utf8(&BYTES) {
        Ok(runtime_version) => runtime_version,
        Err(_) => panic!("the runtime's version is strings joined by an ASCII byte"),
    }
};

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

/// The length of [`runtime_version_bytes`]'s string.
const fn runtime_version_len(name_and_version: &str, commit: Option<&str>) -> usize {
    match commit {
        Some(commit) => name_and_version.len() + 1 + commit.len(),
        None => name_and_version.len(),
    }
}

/// The bytes of [`RUNTIME_VERSION`] for a build named `name_and_version`:
/// those alone without a `commit`, else followed by [`COMMIT_SEPARATOR`] and
/// the commit. `N` is [`runtime_version_len`]'s, and any other fails the
/// build, as [`commit`]'s refusals do.
const fn runtime_version_bytes<const N: usize>(
    name_and_version: &str,
    commit: Option<&str>,
) -> [u8; N] {
    assert!(
        N == runtime_version_len(name_and_version, commit),
        "the runtime's version has the length `runtime_version_len` gives"
    );
    let mut bytes = [0; N];
    let name_and_version = name_and_version.as_bytes();
    let mut at = 0;
    while at < name_and_version.len() {
        bytes[at] = name_and_version[at];
        at += 1;
    }
    if let Some(commit) = commit {
        bytes[at] = COMMIT_SEPARATOR;
        at += 1;
        let commit = commit.as_bytes();
        let mut i = 0;
        while i < commit.len() {
            bytes[at + i] = commit[i];
            i += 1;
        }
    }
    bytes
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

    /// The runtime's version is the name and version alone without a
    /// commit, and with one, the commit after a `+`, whole: the full SHA of
    /// either length and its `-dirty` suffix.
    #[test]
    fn runtime_version_appends_the_whole_commit() {
        fn runtime_version<const N: usize>(commit: Option<&str>) -> String {
            assert_eq!(runtime_version_len("test/1.2.3", commit), N);
            String::from_utf8(runtime_version_bytes::<N>("test/1.2.3", commit).to_vec()).unwrap()
        }
        assert_eq!(runtime_version::<10>(None), "test/1.2.3");
        assert_eq!(
            runtime_version::<51>(Some("89abcdef0123456789abcdef0123456789abcdef")),
            "test/1.2.3+89abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            runtime_version::<57>(Some("89abcdef0123456789abcdef0123456789abcdef-dirty")),
            "test/1.2.3+89abcdef0123456789abcdef0123456789abcdef-dirty"
        );
        assert_eq!(
            runtime_version::<75>(Some(
                "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
            )),
            "test/1.2.3+fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
        );
    }

    /// A length that isn't the string's fails, which for [`RUNTIME_VERSION`]
    /// fails the build.
    #[test]
    fn runtime_version_refuses_another_length() {
        let sha = "89abcdef0123456789abcdef0123456789abcdef";
        assert!(
            std::panic::catch_unwind(|| runtime_version_bytes::<9>("test/1.2.3", None)).is_err()
        );
        assert!(
            std::panic::catch_unwind(|| runtime_version_bytes::<11>("test/1.2.3", None)).is_err()
        );
        assert!(
            std::panic::catch_unwind(|| runtime_version_bytes::<10>("test/1.2.3", Some(sha)))
                .is_err()
        );
        assert!(
            std::panic::catch_unwind(|| runtime_version_bytes::<52>("test/1.2.3", Some(sha)))
                .is_err()
        );
    }

    /// The build's own: `fgro/<version>`, and the commit when the build has
    /// one. Only a `snapshot` build reads `GRO_COMMIT`, so the plain build
    /// `cargo test` runs has no commit, whatever the shell sets.
    #[test]
    fn runtime_version_names_the_build() {
        let version = env!("CARGO_PKG_VERSION");
        let expected = option_env!("GRO_COMMIT")
            .filter(|value| cfg!(feature = "snapshot") && !value.is_empty())
            .map_or_else(
                || format!("fgro/{version}"),
                |commit| format!("fgro/{version}+{commit}"),
            );
        assert_eq!(RUNTIME_VERSION, expected);
    }

    /// As for [`runtime_version_names_the_build`], the plain build has no
    /// commit.
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
