//! The build's identity: the version and the commit it was built from, each
//! validated at compile time, the runtime's version, which carries both, and
//! the `--version` line.

use gro_runtime::{BINARY_NAME, phases, prices};

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

/// The version: the `version` of gro's `package.json`, which
/// `cargo xtask build` stamps through `GRO_VERSION` for its stage-two build,
/// as it stamps the commit, read at compile time, never at runtime. Only
/// that build's `snapshot` feature reads it, and it requires it, so a
/// snapshot build can't name another version; a malformed value fails the
/// build ([`version`]). A plain `cargo build` names Cargo's, `0.0.0`, which
/// never moves: the crates aren't versioned apart from the package. Cargo
/// records the variable as an input, so a changed value rebuilds the crate.
#[cfg(feature = "snapshot")]
pub const VERSION: &str = version(env!(
    "GRO_VERSION",
    "a snapshot build names package.json's version, which `cargo xtask build` sets as GRO_VERSION"
));

#[cfg(not(feature = "snapshot"))]
pub const VERSION: &str = version(env!("CARGO_PKG_VERSION"));

/// What [`RUNTIME_VERSION`] puts between the runtime's name and its version.
const VERSION_SEPARATOR: u8 = b'/';

/// What [`RUNTIME_VERSION`] puts between the version and [`COMMIT`], as
/// semver's build metadata is written.
const COMMIT_SEPARATOR: u8 = b'+';

/// The runtime's name and version, what a script reads as `runtime_version`
/// from the `runtime_version` host module: [`BINARY_NAME`], `/`, and
/// [`VERSION`], as `fgro/<version>`, followed in a build with a
/// [`COMMIT`] by `+` and the whole of it, the full SHA and its `-dirty`
/// suffix as `--version` prints them, so builds of different
/// commits never name themselves alike, which a cache keyed by the string
/// relies on. A `-dirty` build's names the commit its tree differed from,
/// not its sources. Opaque, compared for equality only, and a constant like
/// [`COMMIT`]: nothing is read at runtime.
pub const RUNTIME_VERSION: &str = {
    const LEN: usize = runtime_version_len(BINARY_NAME, VERSION, COMMIT);
    const BYTES: [u8; LEN] = runtime_version_bytes(BINARY_NAME, VERSION, COMMIT);
    match std::str::from_utf8(&BYTES) {
        Ok(runtime_version) => runtime_version,
        Err(_) => panic!("the runtime's version is strings joined by an ASCII byte"),
    }
};

/// What `--version` prints for a build without [`COMMIT`].
pub const COMMIT_UNKNOWN: &str = "unknown";

/// The suffix [`COMMIT`] carries when the working tree differed from the commit.
const DIRTY_SUFFIX: &str = "-dirty";

/// Validates a `GRO_VERSION` value: a semver version, `MAJOR.MINOR.PATCH`,
/// each a number without a leading zero, optionally followed by `-` and a
/// prerelease, dot-separated identifiers of ASCII letters, digits, and `-`,
/// none empty and none of digits alone with a leading zero. Never build
/// metadata: [`RUNTIME_VERSION`] puts the commit after a `+`. A malformed
/// value panics, which, evaluated for [`VERSION`], fails the build rather
/// than printing it.
const fn version(value: &'static str) -> &'static str {
    let bytes = value.as_bytes();
    let mut i = 0;
    let mut part = 0;
    while part < 3 {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        assert!(
            i > start && (i - start == 1 || bytes[start] != b'0'),
            "GRO_VERSION must be MAJOR.MINOR.PATCH, each a number without a leading zero"
        );
        part += 1;
        if part < 3 {
            assert!(
                i < bytes.len() && bytes[i] == b'.',
                "GRO_VERSION must be MAJOR.MINOR.PATCH, each a number without a leading zero"
            );
            i += 1;
        }
    }
    if i < bytes.len() {
        assert!(
            bytes[i] == b'-',
            "GRO_VERSION may go on only with a prerelease after `-`, never build metadata"
        );
        i += 1;
        let mut start = i;
        let mut numeric = true;
        while i <= bytes.len() {
            if i == bytes.len() || bytes[i] == b'.' {
                assert!(
                    i > start,
                    "GRO_VERSION's prerelease identifiers can't be empty"
                );
                assert!(
                    !numeric || i - start == 1 || bytes[start] != b'0',
                    "GRO_VERSION's numeric prerelease identifiers have no leading zero"
                );
                start = i + 1;
                numeric = true;
            } else {
                numeric = numeric && bytes[i].is_ascii_digit();
                assert!(
                    bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-',
                    "GRO_VERSION's prerelease holds only ASCII letters, digits, `-`, and `.`"
                );
            }
            i += 1;
        }
    }
    value
}

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
const fn runtime_version_len(name: &str, version: &str, commit: Option<&str>) -> usize {
    let name_and_version = name.len() + 1 + version.len();
    match commit {
        Some(commit) => name_and_version + 1 + commit.len(),
        None => name_and_version,
    }
}

/// The bytes of [`RUNTIME_VERSION`] for a build of `name` at `version`:
/// the two joined by [`VERSION_SEPARATOR`], followed with a `commit` by
/// [`COMMIT_SEPARATOR`] and the commit. `N` is [`runtime_version_len`]'s,
/// and any other fails the build, as [`commit`]'s refusals do.
const fn runtime_version_bytes<const N: usize>(
    name: &str,
    version: &str,
    commit: Option<&str>,
) -> [u8; N] {
    assert!(
        N == runtime_version_len(name, version, commit),
        "the runtime's version has the length `runtime_version_len` gives"
    );
    let mut bytes = [0; N];
    let at = copy_into(&mut bytes, 0, name.as_bytes());
    bytes[at] = VERSION_SEPARATOR;
    let at = copy_into(&mut bytes, at + 1, version.as_bytes());
    if let Some(commit) = commit {
        bytes[at] = COMMIT_SEPARATOR;
        copy_into(&mut bytes, at + 1, commit.as_bytes());
    }
    bytes
}

/// Copies `from` into `bytes` at `at`, returning the index after it.
const fn copy_into(bytes: &mut [u8], at: usize, from: &[u8]) -> usize {
    let mut i = 0;
    while i < from.len() {
        bytes[at + i] = from[i];
        i += 1;
    }
    at + from.len()
}

/// The `--version` line, [`BINARY_NAME`] and the rest:
/// `fgro <version> (commit <sha>[-dirty], V8 <version>, snapshot)`, the
/// version [`VERSION`], with `commit unknown` for a build without [`COMMIT`]
/// and `no snapshot` for a build without one, and `, phases` before the parenthesis closes in a
/// build that records phases (`gro_runtime::phases::ENABLED`), and
/// `, prices` in one that reads the prices switches
/// (`gro_runtime::prices::ENABLED`).
pub fn version_line() -> String {
    format!(
        "{BINARY_NAME} {VERSION} (commit {}, V8 {}, {}{}{})",
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
    fn version_accepts_semver_with_a_prerelease() {
        for value in [
            "0.0.0",
            "0.239.0",
            "10.20.30",
            "1.0.0-next.1",
            "1.0.0-rc-1.x.0",
            "1.0.0-0",
            "1.0.0-0a.01a",
        ] {
            assert_eq!(version(value), value);
        }
    }

    #[test]
    fn version_refuses_anything_else() {
        for value in [
            "",
            "1",
            "1.0",
            "1.0.0.0",
            "01.0.0",
            "1.00.0",
            "v1.0.0",
            "1.0.0+build",
            "1.0.0-",
            "1.0.0-a..b",
            "1.0.0-a.",
            "1.0.0-a_b",
            "1.0.0-01",
            "1.0.0-a.00",
            "1.0.0 ",
            "1.0.x",
        ] {
            let result = std::panic::catch_unwind(|| version(value));
            assert!(result.is_err(), "{value}");
        }
    }

    /// A plain build, which `cargo test` runs, names Cargo's version, never
    /// moved from `0.0.0`.
    #[test]
    #[cfg(not(feature = "snapshot"))]
    fn a_plain_build_names_cargo_s_version() {
        assert_eq!(VERSION, "0.0.0");
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }

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
            assert_eq!(runtime_version_len("test", "1.2.3", commit), N);
            let bytes = runtime_version_bytes::<N>("test", "1.2.3", commit);
            String::from_utf8(bytes.to_vec()).unwrap()
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
        for refused in [
            std::panic::catch_unwind(|| runtime_version_bytes::<9>("test", "1.2.3", None).len()),
            std::panic::catch_unwind(|| runtime_version_bytes::<11>("test", "1.2.3", None).len()),
            std::panic::catch_unwind(|| {
                runtime_version_bytes::<10>("test", "1.2.3", Some(sha)).len()
            }),
            std::panic::catch_unwind(|| {
                runtime_version_bytes::<52>("test", "1.2.3", Some(sha)).len()
            }),
        ] {
            assert!(refused.is_err());
        }
    }

    /// The build's own: the binary's name, `/`, the version, and the commit
    /// when the build has one. Only a `snapshot` build reads `GRO_COMMIT`,
    /// so the plain build `cargo test` runs has no commit, whatever the shell
    /// sets.
    #[test]
    fn runtime_version_names_the_build() {
        let version = VERSION;
        let expected = option_env!("GRO_COMMIT")
            .filter(|value| cfg!(feature = "snapshot") && !value.is_empty())
            .map_or_else(
                || format!("{BINARY_NAME}/{version}"),
                |commit| format!("{BINARY_NAME}/{version}+{commit}"),
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
            line.starts_with(&format!("{BINARY_NAME} {VERSION} (commit {expected}, V8 ")),
            "{line}"
        );
    }
}
