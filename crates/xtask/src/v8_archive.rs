//! The pinned V8 archive: the prebuilt static library the `v8` crate links,
//! fetched once and verified by its SHA-256.
//!
//! The pin protects the download's integrity, the supply chain. Local writes
//! to the target directory are out of scope: anyone who can write there can
//! alter any build output (rlibs, binaries) already.
//!
//! `.cargo/config.toml` points every cargo invocation at the file through
//! `RUSTY_V8_ARCHIVE`, so the `v8` build script copies it instead of
//! downloading one. That copy is unchecked, and the build script reruns only
//! when the variable's value changes, not when the file does. So:
//!
//! - [`ensure`] verifies the archive before the xtask's cargo runs. A file at
//!   the path that doesn't match is deleted, the `v8` crate's build outputs
//!   are cleaned (`cargo clean -p v8`, every profile) so the next build
//!   recopies and relinks, and the archive is fetched again.
//! - [`verify_linked`] is a stale-copy check: it hashes the library copy the
//!   `v8` build script last wrote (`<target>/<profile>/gn_out/obj/librusty_v8.a`),
//!   which other build units of `v8` share, so it can't tell which copy a given
//!   binary linked.
//!
//! What's left: a plain cargo build against a bad file at the path, with the
//! file then fixed by hand before any xtask runs, isn't detectable here; a
//! newer `v8` that verifies `RUSTY_V8_ARCHIVE_SHA256` itself closes that. The
//! path names the version, so a version change is a new value and recopies.
//! A fresh checkout runs `cargo xtask fetch-v8` (or any xtask that builds)
//! before plain cargo.
//!
//! [`SHA256`] is of the `.gz` release asset, the convention later `v8`
//! releases verify through `RUSTY_V8_ARCHIVE_SHA256`; [`INFLATED_SHA256`] is
//! of the library inflated from it, as the build script writes it. Upstream
//! publishes no checksum, so both were computed on first download and
//! cross-checked: the inflated archive matched, byte for byte, the one cargo
//! had already linked from the same URL. A `v8` version in `Cargo.lock` other
//! than the pin's fails before anything builds against a stale archive.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `v8` crate version the archive belongs to.
pub const VERSION: &str = "150.4.0";

/// The release asset: `deno_core` enables `simdutf`, and every profile links the
/// release build.
pub const FILE_NAME: &str = "librusty_v8_simdutf_release_x86_64-unknown-linux-gnu.a.gz";

/// The SHA-256 of the `.gz` asset.
pub const SHA256: &str = "f48762ca10d1f1fc605a441c5ae430ec8ce1e9e80f14d78fbc42cb878c30b476";

/// The SHA-256 of the static library inflated from it, as the `v8` build
/// script writes it to `<target>/<profile>/gn_out/obj/librusty_v8.a`.
pub const INFLATED_SHA256: &str =
    "8c65c63246328268277bbc87e2a32128dd07f54c6e5438ac68e5b44e0b855a75";

/// Where the archive lives, relative to the workspace root, as
/// `.cargo/config.toml` names it.
pub fn relative_path() -> String {
    format!("target/rusty_v8/v{VERSION}/{FILE_NAME}")
}

fn url() -> String {
    format!("https://github.com/denoland/rusty_v8/releases/download/v{VERSION}/{FILE_NAME}")
}

/// Why the archive isn't ready.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// Only Linux x86-64 has a pin.
    #[error("no V8 archive is pinned for this platform (only x86_64 Linux)")]
    UnsupportedPlatform,
    /// `Cargo.lock` doesn't lock exactly the pinned `v8`.
    #[error(
        "Cargo.lock locks {locked}, but the pinned archive is for v8 {VERSION}: \
         update the pin in crates/xtask/src/v8_archive.rs and .cargo/config.toml"
    )]
    VersionMismatch {
        /// What it locks, described: `no v8`, `v8 X`, or several versions.
        locked: String,
    },
    /// The library copy the `v8` build script last wrote isn't the one
    /// inflated from the pinned archive.
    #[error(
        "{path} has SHA-256 {actual}, not the pinned {INFLATED_SHA256}: the v8 build script \
         copied another archive, and cargo won't recopy it on its own. Fix: \
         `cargo clean -p v8{clean_flag}`, then rerun the xtask"
    )]
    LinkedMismatch {
        /// The library copy.
        path: PathBuf,
        /// Its hash.
        actual: String,
        /// The flag `cargo clean` needs for this profile.
        clean_flag: &'static str,
    },
    /// cargo didn't hand the xtask the pinned path.
    #[error(
        "RUSTY_V8_ARCHIVE is {actual:?}, not the pinned {expected}: \
         .cargo/config.toml and crates/xtask/src/v8_archive.rs disagree"
    )]
    PathMismatch {
        /// What the environment says.
        actual: Option<String>,
        /// The pinned absolute path.
        expected: PathBuf,
    },
    /// A download's hash didn't match the pin; the file was deleted.
    #[error("the downloaded V8 archive's SHA-256 is {actual}, not the pinned {SHA256}; deleted it")]
    HashMismatch {
        /// The hash of what was downloaded.
        actual: String,
    },
    /// A command failed.
    #[error("{0}")]
    Command(String),
    /// A filesystem operation failed.
    #[error("{path}: {source}")]
    Io {
        /// The path involved.
        path: PathBuf,
        /// The failure.
        source: io::Error,
    },
}

/// Makes sure the pinned archive is in place and verified, downloading it if
/// it's missing or doesn't match.
///
/// # Errors
///
/// Returns an [`ArchiveError`] if the platform has no pin, the lockfile or
/// cargo config disagrees with the pin, or the archive can't be fetched and
/// verified.
pub fn ensure(root: &Path) -> Result<PathBuf, ArchiveError> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err(ArchiveError::UnsupportedPlatform);
    }
    let lockfile = root.join("Cargo.lock");
    let lock = fs::read_to_string(&lockfile).map_err(|source| ArchiveError::Io {
        path: lockfile,
        source,
    })?;
    let locked = locked_versions(&lock, "v8");
    if locked != [VERSION] {
        return Err(ArchiveError::VersionMismatch {
            locked: match locked.as_slice() {
                [] => "no v8".to_owned(),
                [version] => format!("v8 {version}"),
                versions => format!("several v8 versions ({})", versions.join(", ")),
            },
        });
    }
    let path = root.join(relative_path());
    // cargo sets it from `.cargo/config.toml` for every process it runs, this one included
    let actual = std::env::var("RUSTY_V8_ARCHIVE").ok();
    if actual.as_deref().map(Path::new) != Some(path.as_path()) {
        return Err(ArchiveError::PathMismatch {
            actual,
            expected: path,
        });
    }

    if path.is_file() {
        let hash = sha256(&path)?;
        if hash == SHA256 {
            return Ok(path);
        }
        println!(
            "[xtask] {} has SHA-256 {hash}, not the pinned {SHA256}; deleting it, cleaning \
             what cargo built from it, and fetching it again",
            path.display()
        );
        fs::remove_file(&path).map_err(|source| ArchiveError::Io {
            path: path.clone(),
            source,
        })?;
        clean_v8(root)?;
    }
    download(&path)?;
    Ok(path)
}

/// Removes the `v8` crate's build outputs for every profile the workspace
/// builds, so the next build reruns its build script (which recopies the
/// archive) and relinks everything downstream of it. Run when the file at the
/// path was wrong, since a build may already have copied it.
fn clean_v8(root: &Path) -> Result<(), ArchiveError> {
    // the built-in `dev` and `release`, plus every custom `[profile.*]` in `Cargo.toml`
    for profile in ["dev", "release", "profiling"] {
        crate::run(
            root,
            &[],
            "cargo",
            &["clean", "--locked", "-p", "v8", "--profile", profile],
        )
        .map_err(|_| {
            ArchiveError::Command(format!(
                "cargo clean -p v8 --profile {profile} failed; see the output above"
            ))
        })?;
    }
    Ok(())
}

/// Checks the library copy the `v8` build script last wrote for `profile`
/// under `target_dir` against [`INFLATED_SHA256`], after a build of that
/// profile and before anything runs V8 code. A stale-copy check: the copy is
/// shared by every build unit of `v8`, so it doesn't prove which one a binary
/// linked.
///
/// # Errors
///
/// Returns [`ArchiveError::LinkedMismatch`] with the remedy when it differs,
/// and an I/O error when it's missing.
pub fn verify_linked(target_dir: &Path, profile: &str) -> Result<(), ArchiveError> {
    let path = target_dir.join(profile).join("gn_out/obj/librusty_v8.a");
    if !path.is_file() {
        return Err(ArchiveError::Io {
            path,
            source: io::Error::new(
                io::ErrorKind::NotFound,
                "the v8 build script wrote no library here",
            ),
        });
    }
    let actual = sha256(&path)?;
    if actual == INFLATED_SHA256 {
        return Ok(());
    }
    Err(ArchiveError::LinkedMismatch {
        path,
        actual,
        clean_flag: if profile == "release" {
            " --release"
        } else {
            ""
        },
    })
}

/// Downloads the archive to a temporary name beside `path`, verifies it, and
/// renames it into place, deleting it on a mismatch. Leftovers of an earlier
/// download that died before cleaning up are removed first.
fn download(path: &Path) -> Result<(), ArchiveError> {
    let io_error = |path: &Path| {
        let path = path.to_path_buf();
        move |source| ArchiveError::Io { path, source }
    };
    let dir = path.parent().unwrap_or(path);
    fs::create_dir_all(dir).map_err(io_error(dir))?;
    remove_partials(dir)?;
    let partial = path.with_extension(format!("partial-{}", std::process::id()));
    let url = url();
    println!("[xtask] downloading {url}");
    let status = Command::new("curl")
        .args([
            "-fL",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--retry",
            "3",
            "-sS",
            "-o",
        ])
        .arg(&partial)
        .arg(&url)
        .status()
        .map_err(|error| ArchiveError::Command(format!("failed to run curl: {error}")))?;
    if !status.success() {
        let _ = fs::remove_file(&partial);
        return Err(ArchiveError::Command(format!(
            "curl {url} failed ({status})"
        )));
    }
    let hash = sha256(&partial)?;
    if hash != SHA256 {
        fs::remove_file(&partial).map_err(io_error(&partial))?;
        return Err(ArchiveError::HashMismatch { actual: hash });
    }
    fs::rename(&partial, path).map_err(io_error(path))?;
    println!("[xtask] verified and saved {}", path.display());
    Ok(())
}

/// The start of a temporary download's file name: `download` names it after
/// the archive with its `.gz` replaced by `.partial-<pid>`.
fn partial_prefix() -> String {
    format!("{}.partial-", FILE_NAME.trim_end_matches(".gz"))
}

/// Removes the temporary files of earlier downloads from `dir`.
fn remove_partials(dir: &Path) -> Result<(), ArchiveError> {
    let io_error = |source| ArchiveError::Io {
        path: dir.to_path_buf(),
        source,
    };
    let prefix = partial_prefix();
    for entry in fs::read_dir(dir).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            fs::remove_file(entry.path()).map_err(io_error)?;
        }
    }
    Ok(())
}

/// The file's SHA-256 as lowercase hex, by `sha256sum`.
fn sha256(path: &Path) -> Result<String, ArchiveError> {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .map_err(|error| ArchiveError::Command(format!("failed to run sha256sum: {error}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    match stdout.split_whitespace().next() {
        Some(hash) if output.status.success() && hash.len() == 64 => Ok(hash.to_owned()),
        _ => Err(ArchiveError::Command(format!(
            "sha256sum {} failed ({}): {}",
            path.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// Every version `Cargo.lock` locks the package `name` at.
fn locked_versions<'a>(lock: &'a str, name: &str) -> Vec<&'a str> {
    let name_line = format!("name = \"{name}\"");
    lock.split("[[package]]")
        .filter(|package| package.lines().any(|line| line == name_line))
        .filter_map(|package| {
            package
                .lines()
                .find_map(|line| line.strip_prefix("version = \""))
                .and_then(|version| version.strip_suffix('"'))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_versions_reads_every_package_of_the_name() {
        let lock = "[[package]]\nname = \"v8\"\nversion = \"150.4.0\"\n\n\
                    [[package]]\nname = \"deno_v8\"\nversion = \"0.4.0\"\n";
        assert_eq!(locked_versions(lock, "v8"), ["150.4.0"]);
        assert!(locked_versions(lock, "nope").is_empty());
        let two = format!("{lock}\n[[package]]\nname = \"v8\"\nversion = \"152.2.0\"\n");
        assert_eq!(locked_versions(&two, "v8"), ["150.4.0", "152.2.0"]);
    }

    #[test]
    fn the_pin_matches_the_lockfile_and_the_cargo_config() {
        let root = crate::root();
        let lock = fs::read_to_string(root.join("Cargo.lock")).unwrap();
        assert_eq!(locked_versions(&lock, "v8"), [VERSION]);
        let config = fs::read_to_string(root.join(".cargo/config.toml")).unwrap();
        assert!(
            config.contains(&format!("value = \"{}\"", relative_path())),
            "{config}"
        );
    }

    #[test]
    fn a_linked_library_off_the_pin_fails_with_the_remedy() {
        let dir = std::env::temp_dir().join(format!("xtask_verify_linked_{}", std::process::id()));
        let lib = dir.join("release/gn_out/obj");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("librusty_v8.a"), b"tampered").unwrap();
        let error = verify_linked(&dir, "release").unwrap_err();
        assert!(
            matches!(error, ArchiveError::LinkedMismatch { .. }),
            "{error}"
        );
        assert!(
            error.to_string().contains("cargo clean -p v8 --release"),
            "{error}"
        );
        // a profile the build script never wrote is an error too, not a pass
        let error = verify_linked(&dir, "debug").unwrap_err();
        assert!(matches!(error, ArchiveError::Io { .. }), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_partial_download_is_named_for_cleanup() {
        // `download` names its temporary file this way, and `remove_partials` matches it
        let partial = Path::new(FILE_NAME).with_extension("partial-1");
        let prefix = partial_prefix();
        assert!(
            partial.to_string_lossy().starts_with(&prefix),
            "{partial:?}"
        );
    }
}
