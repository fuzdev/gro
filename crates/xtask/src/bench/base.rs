//! `cargo xtask bench --counters --base <rev>`'s other side: the release
//! snapshot `fgro` built at a git revision, in a clone of its own.
//!
//! The clone sits in `<base-dir>/src` (by default
//! `$XDG_CACHE_HOME/gro/bench_base`, else `~/.cache/gro/bench_base`), made
//! once with `git clone --shared --no-checkout` from the checkout, so it reads
//! the checkout's objects and never writes to it. Each build checks out the
//! revision there (detached, forced), links the checkout's V8 archives into
//! the clone's `target/rusty_v8/` so nothing downloads, and runs that
//! revision's own `cargo xtask build --release` with the clone's own target
//! directory, which stays between builds, so a later revision rebuilds only
//! what changed. The binary is copied out to `<base-dir>/bin/<sha>/fgro`,
//! where the next build can't replace it mid-run. The clone is outside the
//! checkout because cargo reads the `.cargo/config.toml` of every directory
//! above the one it runs in: inside the checkout, the checkout's config would
//! fill in whatever the revision's lacks.
//!
//! The revision's build starts from a cleared environment holding only
//! [`BASE_BUILD_ENV`], so it builds with the toolchain its own
//! `rust-toolchain.toml` pins: `cargo xtask` runs with rustup's
//! `RUSTUP_TOOLCHAIN` set to the checkout's pin, and cargo's `CARGO`,
//! `CARGO_*`, `RUSTY_V8_ARCHIVE`, and `LD_LIBRARY_PATH` naming the checkout
//! and its toolchain. After the build, the revision's `rustc --version`, run
//! the same way in the clone, must match its pin when the pin is a version
//! number ([`check_pin`]).
//!
//! An A/B holds an exclusive `flock` on `<base-dir>/lock` ([`lock`]) from the
//! build to the end of its run, so two A/Bs on one base dir can't check out
//! under each other or replace a binary mid-run.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use crate::cmd::run;

/// The variables the base build keeps from this process's environment; every
/// other is cleared.
///
/// - `PATH`: finds `cargo` and `rustc` (rustup's proxies, which then read the
///   revision's pin), `git`, and the linker
/// - `HOME`: rustup's and cargo's default homes, and git's config
/// - `RUSTUP_HOME`, `CARGO_HOME`: the homes, when set elsewhere, so the
///   installed toolchains and the registry cache are found
/// - `CARGO_BUILD_JOBS`: the caller's cap on the build's load
/// - `TMPDIR`: where rustc and the linker write temporary files, when moved
/// - `RUSTFLAGS`: the caller's flags, which the head's build takes too, so the
///   A/B builds both sides alike
pub const BASE_BUILD_ENV: [&str; 7] = [
    "PATH",
    "HOME",
    "RUSTUP_HOME",
    "CARGO_HOME",
    "CARGO_BUILD_JOBS",
    "TMPDIR",
    "RUSTFLAGS",
];

/// The caller's variables that configure how cargo compiles, which the
/// head's build takes and the base's cleared environment would drop, so an
/// A/B with any set would compare two configurations: `RUSTC`,
/// `CARGO_ENCODED_RUSTFLAGS`, `CARGO_INCREMENTAL`, and every `CARGO_BUILD_*`
/// (but `CARGO_BUILD_JOBS`, which changes no output), `CARGO_PROFILE_*`, and
/// `CARGO_TARGET_*` (but `CARGO_TARGET_DIR`, which only places it). The A/B
/// refuses them rather than mirror them: cargo's set of variables grows, and
/// a refusal can't miss one silently the way a list passed through can, only
/// loudly. `RUSTFLAGS`, the common one, passes through ([`BASE_BUILD_ENV`]).
pub fn codegen_overrides(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut found: Vec<String> = names
        .into_iter()
        .filter(|name| {
            matches!(
                name.as_str(),
                "RUSTC" | "CARGO_ENCODED_RUSTFLAGS" | "CARGO_INCREMENTAL"
            ) || (name.starts_with("CARGO_BUILD_") && name != "CARGO_BUILD_JOBS")
                || name.starts_with("CARGO_PROFILE_")
                || (name.starts_with("CARGO_TARGET_") && name != "CARGO_TARGET_DIR")
        })
        .collect();
    found.sort();
    found
}

/// A base build: the copy of the binary and the `rustc --version` line of
/// the toolchain that built it.
#[derive(Debug)]
pub struct BaseBuild {
    pub binary: PathBuf,
    pub rustc: String,
}

/// A command for `program` in `dir` with the cleared environment of
/// [`BASE_BUILD_ENV`].
fn clean_command(program: &str, dir: &Path) -> Command {
    let mut command = Command::new(program);
    command.env_clear().current_dir(dir);
    for key in BASE_BUILD_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

/// The first line `rustc --version` prints from `command`, a `rustc`
/// command set up as the caller wants it.
pub fn rustc_version(mut command: Command) -> Result<String, ExitCode> {
    let output = command
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            eprintln!("[xtask] failed to run rustc --version: {error}");
            ExitCode::FAILURE
        })?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || line.is_empty() {
        eprintln!(
            "[xtask] rustc --version failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Err(ExitCode::FAILURE);
    }
    Ok(line)
}

/// The `channel` a `rust-toolchain.toml`'s text pins, when it names one.
fn pinned_channel(toml: &str) -> Option<String> {
    toml.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "channel").then(|| value.trim().trim_matches('"').to_owned())
    })
}

/// Whether `rustc`, a `rustc --version` line, is the toolchain `channel`
/// pins, or `None` when the channel isn't a version number (`stable`, a
/// dated nightly) and so can't be checked from the line.
fn check_pin(rustc: &str, channel: &str) -> Option<bool> {
    let parts: Vec<&str> = channel.split('.').collect();
    let numeric = (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    if !numeric {
        return None;
    }
    let version = rustc.strip_prefix("rustc ")?.split(' ').next()?;
    Some(if parts.len() == 3 {
        version == channel
    } else {
        version.strip_prefix(channel)?.starts_with('.')
    })
}

/// The lock an A/B holds on its base dir: an exclusive `flock` on
/// `<dir>/lock`, which the kernel releases when the file closes, on drop or
/// on any exit, so no lock outlives its process.
#[derive(Debug)]
pub struct BaseLock {
    _file: fs::File,
}

/// Takes `<dir>/lock` without waiting, writing this process's id into it for
/// another's refusal to name. A lock another process holds is refused,
/// naming the id it wrote.
pub fn lock(dir: &Path) -> Result<BaseLock, ExitCode> {
    let failed = |error: &io::Error| {
        eprintln!(
            "[xtask] failed to lock the base dir {}: {error}",
            dir.display()
        );
        ExitCode::FAILURE
    };
    fs::create_dir_all(dir).map_err(|error| failed(&error))?;
    let path = dir.join("lock");
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| failed(&error))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => {
            let holder = fs::read_to_string(&path).unwrap_or_default();
            let holder = holder.trim();
            eprintln!(
                "[xtask] the base dir {} is in use by process {} (it holds {}); wait for it, \
                 or pass another --base-dir",
                dir.display(),
                if holder.is_empty() { "unknown" } else { holder },
                path.display()
            );
            return Err(ExitCode::from(crate::EXIT_INPUT));
        }
        Err(fs::TryLockError::Error(error)) => return Err(failed(&error)),
    }
    file.set_len(0)
        .and_then(|()| writeln!(file, "{}", std::process::id()))
        .map_err(|error| failed(&error))?;
    Ok(BaseLock { _file: file })
}

/// The base build's directory: `--base-dir`, or the cache's.
pub fn base_dir(given: Option<PathBuf>) -> Result<PathBuf, ExitCode> {
    if let Some(dir) = given {
        return std::path::absolute(&dir).map_err(|error| {
            eprintln!("[xtask] invalid --base-dir {}: {error}", dir.display());
            ExitCode::from(crate::EXIT_INPUT)
        });
    }
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .ok_or_else(|| {
            eprintln!("[xtask] neither XDG_CACHE_HOME nor HOME is set; pass --base-dir");
            ExitCode::from(crate::EXIT_INPUT)
        })?;
    Ok(cache.join("gro").join("bench_base"))
}

/// The commit `rev` names in the checkout at `root`.
pub fn resolve(root: &Path, rev: &str) -> Result<String, ExitCode> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--verify", "--quiet", "--end-of-options"])
        .arg(format!("{rev}^{{commit}}"))
        .output()
        .map_err(|error| {
            eprintln!("[xtask] failed to run git: {error}");
            ExitCode::FAILURE
        })?;
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || sha.is_empty() {
        eprintln!("[xtask] --base {rev} names no commit in {}", root.display());
        return Err(ExitCode::from(crate::EXIT_INPUT));
    }
    Ok(sha)
}

/// Builds the release snapshot `fgro` at `sha` in `dir`'s clone, the caller
/// holding `dir`'s [`lock`], and returns the copy of it under
/// `dir/bin/<sha>/fgro` with the toolchain that built it, refusing one off
/// the revision's pin.
pub fn build(root: &Path, dir: &Path, sha: &str, _lock: &BaseLock) -> Result<BaseBuild, ExitCode> {
    let failed = |what: &str, error: &dyn std::fmt::Display| {
        eprintln!("[xtask] the base build failed to {what}: {error}");
        ExitCode::FAILURE
    };
    let src = dir.join("src");
    let root_arg = root.display().to_string();
    let src_arg = src.display().to_string();
    if !src.join(".git").exists() {
        fs::create_dir_all(dir).map_err(|error| failed("create its directory", &error))?;
        run(
            dir,
            &[],
            "git",
            &[
                "clone",
                "--quiet",
                "--shared",
                "--no-checkout",
                &root_arg,
                &src_arg,
            ],
        )?;
    }
    // the shared clone reads the checkout's objects, so any commit of it checks out
    run(
        &src,
        &[],
        "git",
        &["checkout", "--quiet", "--force", "--detach", sha],
    )?;
    link_v8_archives(root, &src).map_err(|error| failed("link the V8 archives", &error))?;
    // from a cleared environment, so the revision's own toolchain pin and
    // cargo config decide the build
    println!(
        "\n[xtask] cargo xtask build --release (in {src_arg}, at {sha}, environment cleared but {})",
        BASE_BUILD_ENV.join(", ")
    );
    let status = clean_command("cargo", &src)
        .args(["xtask", "build", "--release"])
        .env("CARGO_TARGET_DIR", src.join("target"))
        .status()
        .map_err(|error| failed("run cargo", &error))?;
    if !status.success() {
        eprintln!("[xtask] the base build at {sha} failed ({status})");
        return Err(ExitCode::FAILURE);
    }
    let built = src.join("target/gro_snapshot/release/fgro");
    let copy = dir.join("bin").join(sha).join("fgro");
    let parent = copy.parent().unwrap_or(dir);
    fs::create_dir_all(parent).map_err(|error| failed("create the binary's directory", &error))?;
    // copied, not linked: the next build replaces the file it built
    let _ = fs::remove_file(&copy);
    fs::copy(&built, &copy).map_err(|error| failed("copy the binary", &error))?;
    println!("\n[xtask] the base fgro at {sha}: {}", copy.display());
    let rustc = rustc_version(clean_command("rustc", &src))?;
    let pin = fs::read_to_string(src.join("rust-toolchain.toml"))
        .ok()
        .and_then(|toml| pinned_channel(&toml));
    match pin
        .as_deref()
        .map(|channel| (channel, check_pin(&rustc, channel)))
    {
        Some((channel, Some(false))) => {
            eprintln!(
                "[xtask] the base at {sha} pins the toolchain {channel}, but its build ran {rustc}"
            );
            return Err(ExitCode::FAILURE);
        }
        Some((channel, Some(true))) => {
            println!("[xtask] the base built with {rustc}, its pin {channel}");
        }
        Some((channel, None)) => {
            println!(
                "[xtask] the base built with {rustc}; its pin {channel} names no version to check"
            );
        }
        None => println!("[xtask] the base built with {rustc}; it pins no toolchain"),
    }
    Ok(BaseBuild {
        binary: copy,
        rustc,
    })
}

/// Links each V8 archive under the checkout's `target/rusty_v8/` into the
/// clone's, where its `.cargo/config.toml` looks, when the clone lacks it, so
/// its build reuses the archive (the revision's own `fetch-v8` verifies it,
/// and downloads only a version the checkout doesn't have).
fn link_v8_archives(root: &Path, src: &Path) -> io::Result<()> {
    let archives = root.join("target/rusty_v8");
    let Ok(versions) = fs::read_dir(&archives) else {
        return Ok(());
    };
    for version in versions {
        let version = version?;
        if !version.file_type()?.is_dir() {
            continue;
        }
        for file in fs::read_dir(version.path())? {
            let file = file?;
            let to = src
                .join("target/rusty_v8")
                .join(version.file_name())
                .join(file.file_name());
            if to.exists() {
                continue;
            }
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            if fs::hard_link(file.path(), &to).is_err() {
                fs::copy(file.path(), &to)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_pinned_channel() {
        let toml = "[toolchain]\n# a comment\nchannel = \"1.99.1\"\ncomponents = [\"clippy\"]\n";
        assert_eq!(pinned_channel(toml).as_deref(), Some("1.99.1"));
        assert_eq!(pinned_channel("[toolchain]\n"), None);
    }

    #[test]
    fn checks_rustc_against_a_version_pin() {
        let rustc = "rustc 1.99.1 (abcdef012 2026-10-01)";
        assert_eq!(check_pin(rustc, "1.99.1"), Some(true));
        assert_eq!(check_pin(rustc, "1.99"), Some(true));
        assert_eq!(check_pin(rustc, "1.98.0"), Some(false));
        assert_eq!(check_pin(rustc, "1.9"), Some(false));
        assert_eq!(check_pin(rustc, "1.99.10"), Some(false));
        assert_eq!(check_pin(rustc, "stable"), None);
        assert_eq!(check_pin(rustc, "nightly-2026-10-01"), None);
    }

    #[test]
    fn codegen_overrides_are_found_and_the_harmless_ones_left() {
        let names = [
            "PATH",
            "RUSTFLAGS",
            "CARGO_BUILD_JOBS",
            "CARGO_TARGET_DIR",
            "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
            "CARGO_BUILD_RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_INCREMENTAL",
            "RUSTC",
            "CARGO_HOME",
        ]
        .map(str::to_owned);
        assert_eq!(
            codegen_overrides(names),
            [
                "CARGO_BUILD_RUSTFLAGS",
                "CARGO_ENCODED_RUSTFLAGS",
                "CARGO_INCREMENTAL",
                "CARGO_PROFILE_RELEASE_LTO",
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
                "RUSTC",
            ]
        );
    }

    #[test]
    fn a_held_lock_is_refused_until_dropped() {
        let dir = std::env::temp_dir().join(format!("xtask_base_lock_{}", std::process::id()));
        let held = lock(&dir).unwrap();
        // a second open file can't take it while the first holds it
        let refused = lock(&dir).is_err();
        let text = fs::read_to_string(dir.join("lock")).unwrap();
        drop(held);
        let again = lock(&dir).is_ok();
        fs::remove_dir_all(&dir).unwrap();
        assert!(refused);
        assert_eq!(text.trim(), std::process::id().to_string());
        assert!(again);
    }
}
