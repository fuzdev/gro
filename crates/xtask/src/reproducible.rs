//! `cargo xtask check-reproducible`: the release snapshot `fgro` built twice
//! from one commit, once in the checkout and once in a clone of that commit
//! at another path, compared byte for byte.
//!
//! The claim it holds is the one the checkout's docs make: a clean checkout
//! of one commit, with the same toolchain and cargo home, builds the same
//! release `fgro` from any path. So the checkout must be clean (the build
//! embeds its commit, and a dirty tree's `-dirty` alone would differ), and
//! the caller's environment may not set a variable that configures how cargo
//! compiles, which the clone's cleared environment would drop
//! ([`crate::bench::base::codegen_overrides`]; `RUSTFLAGS` passes to both).
//!
//! 1. The checkout's own `build --release`, into its target directory.
//! 2. A fresh clone at `<dir>/src` (by default
//!    `$XDG_CACHE_HOME/gro/reproducible`, else `~/.cache/gro/reproducible`),
//!    outside the checkout, since cargo reads the `.cargo/config.toml` of
//!    every directory above the one it runs in: `git init`, then a fetch of
//!    the one commit at depth 1 from the checkout, so the clone holds that
//!    commit and no other history, checked out detached. The checkout's V8
//!    archives are linked into its `target/rusty_v8/`, so nothing downloads.
//!    The directory is one the check owns ([`take`]): it removes an earlier
//!    clone only from a directory it marked as its own.
//! 3. The clone's `cargo xtask build --release`, from the cleared environment
//!    the perf rule's base build uses ([`crate::bench::base::BASE_BUILD_ENV`]),
//!    so the clone's own toolchain pin decides, into the clone's own target
//!    directory, cold.
//! 4. Both builds' `rustc --version` must agree, then the two binaries and
//!    the snapshot files each embeds are compared. A difference fails naming
//!    the region of the first differing byte (an ELF section, or a header),
//!    every section whose bytes differ (the first is often the build-id
//!    note, a hash of the rest, so the list says where the change is), the
//!    snapshot files that differ, and each binary's `--version`; the clone
//!    is kept for inspection. A pass removes the clone.
//!
//! It refuses a rustc wrapper ([`rustc_wrapper`]): one in cargo's home
//! applies to both builds, and a compiler cache could serve the second from
//! the first's compile. And it refuses a cargo config only the clone's
//! build would read, in `<dir>` or above it and not above the checkout
//! ([`refuse_clone_configs`]): its forced `[env]` could set a wrapper for
//! the clone's build alone, and its `rustflags` join the tree's, where a
//! `--remap-path-prefix` could write the checkout's path in place of the
//! clone's and hide the very difference the check looks for.
//!
//! What it can't see: a build on another machine, with another toolchain,
//! or with another cargo home (whose registry paths the binary names), which
//! the claim excludes; a dirty tree or one outside git, which embeds another
//! commit by design and which the check refuses rather than builds; the
//! debug build, since it holds the release one; a cargo config only the
//! checkout's build reads, above the checkout and not above `<dir>`, where a
//! caller keeps one over a directory of checkouts, whose flags could make
//! the two builds differ, or alike where they'd differ; and what both
//! builds read alike on one machine, inside the checkout or out: a
//! directory's listing order on one filesystem, the day's date, a cache they
//! share.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use gro_bench::lock::{DirLock, LOCK_FILE, LockError, LockIoError, try_lock_dir};
use gro_bench::record::GitState;

use crate::bench::base::{
    BASE_BUILD_ENV, clean_command, codegen_overrides, link_v8_archives, rustc_version,
};
use crate::cmd::{StepResult, run};
use crate::error::XtaskError;
use crate::snapshot::{self, Measurement, SNAPSHOT_FILES};

/// Checks that the release `fgro` builds byte for byte from a second clone
/// of the checkout's commit, cloned and built under `dir`; `env` is the
/// check's own environment.
pub fn check_reproducible(
    root: &Path,
    dir: Option<PathBuf>,
    env: &[(OsString, OsString)],
) -> StepResult {
    refuse_environment(env)?;
    if let Some(wrapper) = rustc_wrapper(root) {
        return Err(XtaskError::reproducible_input(format!(
            "{wrapper} sets a rustc wrapper, which could hand the clone's build what the \
             checkout's compiled, as a shared compiler cache does; unset it for the check"
        )));
    }
    let state = GitState::read(root, &root.join(crate::bench::HISTORY)).map_err(|error| {
        XtaskError::reproducible_input(format!(
            "the build embeds the checkout's commit, and git can't read it: {error}"
        ))
    })?;
    if state.dirty {
        return Err(XtaskError::reproducible_input(format!(
            "the checkout differs from its commit {}, so its build embeds `{}-dirty` and a clone's \
             can't match it; check a clean checkout",
            state.sha, state.sha
        )));
    }
    let dir = work_dir(root, dir, env)?;
    let (dir, _lock) = take(root, &dir)?;

    let target_dir = crate::check::fetch_v8(root)?;
    let first = snapshot::build(root, &target_dir, true, Measurement::None)?;
    if first.commit != state.sha {
        return Err(XtaskError::Failed(format!(
            "the checkout moved during its build: it embeds `{}`, not `{}`",
            first.commit, state.sha
        )));
    }

    let src = clone(root, &dir, &state.sha, env)?;
    println!(
        "\n[xtask] cargo xtask build --release (in {}, at {}, environment cleared but {})",
        src.display(),
        state.sha,
        BASE_BUILD_ENV.join(", ")
    );
    let status = clean_command("cargo", &src)
        .args(["xtask", "build", "--release"])
        .env("CARGO_TARGET_DIR", src.join("target"))
        .status()
        .map_err(|source| XtaskError::Spawn {
            display: "cargo xtask build --release".to_owned(),
            source,
        })?;
    if !status.success() {
        return Err(XtaskError::StepFailed {
            display: format!("the clone's build at {}", src.display()),
            status,
        });
    }
    let second = src.join("target/gro_snapshot/release/fgro");

    let mut rustc = Command::new("rustc");
    rustc.current_dir(root);
    let (first_rustc, second_rustc) = (
        rustc_version(rustc)?,
        rustc_version(clean_command("rustc", &src))?,
    );
    if first_rustc != second_rustc {
        return Err(XtaskError::reproducible_input(format!(
            "the two builds ran different toolchains, {first_rustc} in the checkout and \
             {second_rustc} in the clone"
        )));
    }

    compare(&first.binary, &second, &src)?;
    // kept on a failure, for inspection; a pass needs nothing of it
    remove_clone(&dir)
}

/// Refuses `env`, the check's own environment, when it sets a variable that
/// configures how cargo compiles ([`codegen_overrides`]), which the clone's
/// cleared environment would drop.
fn refuse_environment(env: &[(OsString, OsString)]) -> StepResult {
    let overrides = codegen_overrides(
        env.iter()
            .filter_map(|(name, _)| name.to_str().map(str::to_owned)),
    );
    if !overrides.is_empty() {
        return Err(XtaskError::reproducible_input(format!(
            "the clone builds from a cleared environment, so the checkout's build would take {} \
             and the clone's wouldn't; unset them for the check",
            overrides.join(", ")
        )));
    }
    Ok(())
}

/// The value of the variable `name` in `env`, when it's set.
fn var<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

/// The keys that set a rustc wrapper in a cargo config file, under `[build]`.
const WRAPPER_KEYS: [&str; 2] = ["rustc-wrapper", "rustc-workspace-wrapper"];

/// The environment variables that set a rustc wrapper.
const WRAPPER_VARS: [&str; 4] = [
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
];

/// Where a rustc wrapper is set for the checkout's build: a variable of
/// [`WRAPPER_VARS`] that isn't empty, or a cargo config file, in the
/// checkout or above it or in cargo's home, with a line setting a key of
/// [`WRAPPER_KEYS`]. A wrapper in cargo's home applies to the clone's build
/// too, and a cache it runs can serve both from one compile. A line read
/// as a key is a match, so a key under another table refuses too: loud
/// rather than missed.
fn rustc_wrapper(root: &Path) -> Option<String> {
    if let Some(var) = WRAPPER_VARS
        .into_iter()
        .find(|var| std::env::var_os(var).is_some_and(|value| !value.is_empty()))
    {
        return Some(format!("the environment's {var}"));
    }
    root.ancestors()
        .map(|dir| dir.join(".cargo"))
        .chain([snapshot::cargo_home()])
        .flat_map(|dir| [dir.join("config.toml"), dir.join("config")])
        .find(|file| fs::read_to_string(file).is_ok_and(|text| sets_a_wrapper(&text)))
        .map(|file| file.display().to_string())
}

/// Whether a line of `config`, a cargo config file's text, sets a key of
/// [`WRAPPER_KEYS`], bare (under `[build]`) or dotted (`build.`), quoted or
/// not.
fn sets_a_wrapper(config: &str) -> bool {
    config.lines().any(|line| {
        line.split_once('=').is_some_and(|(key, _)| {
            let key = key.trim().trim_start_matches("build.").trim_matches('"');
            WRAPPER_KEYS.contains(&key)
        })
    })
}

/// The directory the clone goes in: `given`, or the cache's from `env`
/// (`XDG_CACHE_HOME`, else `HOME`'s `.cache`, each taken only when
/// absolute), absolute; refused when it climbs with `..`, which the kernel
/// resolves only once a directory below it exists, so no reading of the
/// path before then can say where it ends, or when it and the checkout hold
/// one another ([`kept_apart`]), read through the symlinks of the part that
/// exists, or when a cargo config only the clone's build would read is in
/// it or above it ([`refuse_clone_configs`]).
fn work_dir(
    root: &Path,
    given: Option<PathBuf>,
    env: &[(OsString, OsString)],
) -> Result<PathBuf, XtaskError> {
    let dir = if let Some(dir) = given {
        std::path::absolute(&dir).map_err(|error| {
            XtaskError::reproducible_input(format!("invalid --dir {}: {error}", dir.display()))
        })?
    } else {
        let absolute = |name| {
            var(env, name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let cache = absolute("XDG_CACHE_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".cache")))
            .ok_or_else(|| {
                XtaskError::reproducible_input(
                    "neither XDG_CACHE_HOME nor HOME is set to an absolute path; pass --dir",
                )
            })?;
        cache.join("gro").join("reproducible")
    };
    if dir
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(XtaskError::reproducible_input(format!(
            "the clone's directory {} climbs with `..`; name it without",
            dir.display()
        )));
    }
    let real = real_path(&dir);
    kept_apart(root, &dir, &real)?;
    refuse_clone_configs(root, &real, env)?;
    Ok(dir)
}

/// Refuses `real`, the real path of `dir`, when it and the checkout at
/// `root` hold one another: the clone's cargo would read the checkout's
/// config, or the two paths would be one.
fn kept_apart(root: &Path, dir: &Path, real: &Path) -> StepResult {
    if real.starts_with(root) || root.starts_with(real) {
        return Err(XtaskError::reproducible_input(format!(
            "the clone's directory {} and the checkout {} must not hold one another",
            dir.display(),
            root.display()
        )));
    }
    Ok(())
}

/// `path` with the part that exists replaced by its real path, so a symlink
/// can't hide where a directory not yet made would be.
fn real_path(path: &Path) -> PathBuf {
    path.ancestors()
        .find_map(|existing| {
            let real = existing.canonicalize().ok()?;
            let rest = path.strip_prefix(existing).ok()?;
            Some(real.join(rest))
        })
        .unwrap_or_else(|| path.to_owned())
}

/// The file that marks a directory as this check's own, the one kind whose
/// `src` it removes.
const MARKER: &str = ".check-reproducible";

/// What [`MARKER`] holds, for whoever finds it.
const MARKER_TEXT: &str = "This directory is `cargo xtask check-reproducible`'s: each run replaces \
                           its src/ with a fresh clone.\n";

/// Takes `dir` as the check's own, returning its real path and its lock:
/// refused unless [`owned`] says it may be, before the lock is taken (whose
/// file it would otherwise write over) and again once it's held; then its
/// real path is read again, now that every part of it exists, so a symlink
/// made in a missing part since [`work_dir`] read it can't move it into the
/// checkout ([`real_dir`]); then marked ([`MARKER`]).
fn take(root: &Path, dir: &Path) -> Result<(PathBuf, DirLock), XtaskError> {
    owned(dir)?;
    let held = lock(dir)?;
    let real = real_dir(root, dir)?;
    owned(&real)?;
    let marker = real.join(MARKER);
    fs::write(&marker, MARKER_TEXT).map_err(|error| {
        XtaskError::Failed(format!("failed to write {}: {error}", marker.display()))
    })?;
    Ok((real, held))
}

/// The real path of `dir`, which exists, refused by [`kept_apart`].
fn real_dir(root: &Path, dir: &Path) -> Result<PathBuf, XtaskError> {
    let real = dir.canonicalize().map_err(|error| {
        XtaskError::Failed(format!("failed to resolve {}: {error}", dir.display()))
    })?;
    kept_apart(root, dir, &real)?;
    Ok(real)
}

/// Refuses `dir` unless the check may take it as its own: it's missing or
/// empty, it holds the [`MARKER`] an earlier run wrote, or it holds nothing
/// but a lock file as [`try_lock_dir`] leaves one (a process id, or
/// nothing), which holds nothing to lose. Anything else is
/// another's, or an earlier run's clone with no marker, and is left alone:
/// the refusal names the directory to remove when it's the check's.
fn owned(dir: &Path) -> StepResult {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotADirectory => {
            return Err(XtaskError::reproducible_input(format!(
                "the clone's directory {} isn't a directory",
                dir.display()
            )));
        }
        Err(error) => {
            return Err(XtaskError::Failed(format!(
                "failed to read {}: {error}",
                dir.display()
            )));
        }
    };
    if fs::symlink_metadata(dir.join(MARKER)).is_ok_and(|meta| meta.is_file()) {
        return Ok(());
    }
    let mut others = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            XtaskError::Failed(format!("failed to read {}: {error}", dir.display()))
        })?;
        if !(entry.file_name() == LOCK_FILE && is_left_lock(&entry.path())) {
            others.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    if others.is_empty() {
        return Ok(());
    }
    others.sort();
    Err(XtaskError::reproducible_input(format!(
        "{} holds files the check didn't make ({}) and no {MARKER}, so it isn't the check's to \
         clone into; pass another --dir, or remove the directory if an earlier \
         check-reproducible made it",
        dir.display(),
        others.join(", ")
    )))
}

/// Whether `path` is a lock file as [`try_lock_dir`] leaves it: a regular
/// file holding a process id, or nothing.
fn is_left_lock(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() <= 32)
        && fs::read_to_string(path)
            .is_ok_and(|text| text.trim().bytes().all(|byte| byte.is_ascii_digit()))
}

/// Refuses `dir`, the real path [`take`] returned, when it no longer is
/// one: its real path is read again, so a symlink swapped in for one of its
/// parents can't redirect a removal in it, and [`owned`] must still hold,
/// so a directory moved into its place is another's to keep.
fn still_taken(dir: &Path) -> StepResult {
    let real = dir.canonicalize().map_err(|error| {
        XtaskError::Failed(format!("failed to resolve {}: {error}", dir.display()))
    })?;
    if real != dir {
        return Err(XtaskError::reproducible_input(format!(
            "the clone's directory {} resolves to {} since the check took it, so nothing in it \
             is removed; see what changed a parent of it, and check again",
            dir.display(),
            real.display()
        )));
    }
    owned(dir)
}

/// Removes the clone of a pass, `<dir>/src`, once [`still_taken`] holds of
/// `dir` again, since the clone's build ran after [`clone`] read it.
fn remove_clone(dir: &Path) -> StepResult {
    still_taken(dir)?;
    let src = dir.join("src");
    fs::remove_dir_all(&src)
        .map_err(|error| XtaskError::Failed(format!("failed to remove {}: {error}", src.display())))
}

/// The files cargo reads its config from in a directory's `.cargo`.
const CARGO_CONFIGS: [&str; 2] = ["config.toml", "config"];

/// Refuses a cargo config the clone's build would read and the checkout's
/// wouldn't: one in `dir`, the real path of the clone's directory, or in a
/// parent of it that isn't a parent of the checkout at `root`, since cargo
/// reads the config of every directory above the one it runs in. Such a
/// config changes the clone's build alone: its `[env]` with `force = true`
/// can set a rustc wrapper for it, and its `rustflags` join the tree's,
/// where a `--remap-path-prefix` could write the checkout's path in place
/// of the clone's and hide the very difference the check looks for.
/// The config of cargo's home, as `env` names it ([`cargo_home`]), is left
/// alone: cargo reads it for every build, the checkout's too, and once,
/// though it's above the clone.
fn refuse_clone_configs(root: &Path, dir: &Path, env: &[(OsString, OsString)]) -> StepResult {
    let home = cargo_home(env).and_then(|home| home.canonicalize().ok());
    let found: Vec<String> = dir
        .ancestors()
        .filter(|parent| !root.starts_with(parent))
        .map(|parent| parent.join(".cargo"))
        .filter(|configs| home.is_none() || configs.canonicalize().ok() != home)
        .flat_map(|configs| CARGO_CONFIGS.map(|config| configs.join(config)))
        .filter(|config| fs::symlink_metadata(config).is_ok())
        .map(|config| config.display().to_string())
        .collect();
    if found.is_empty() {
        return Ok(());
    }
    Err(XtaskError::reproducible_input(format!(
        "the clone's build would read a cargo config the checkout's wouldn't ({}), which could \
         change the clone's build alone (a flag, or a wrapper its [env] forces); pass a --dir \
         with no such config in it or above it, or remove the config",
        found.join(" and ")
    )))
}

/// Cargo's home as `env` names it: a non-empty `CARGO_HOME`, else `HOME`'s
/// `.cargo`, each taken only when absolute (cargo reads a relative
/// `CARGO_HOME` from the directory it runs in, which the two builds don't
/// share).
fn cargo_home(env: &[(OsString, OsString)]) -> Option<PathBuf> {
    let absolute = |value: &OsString| Some(PathBuf::from(value)).filter(|path| path.is_absolute());
    var(env, "CARGO_HOME")
        .filter(|value| !value.is_empty())
        .map_or_else(
            || {
                var(env, "HOME")
                    .and_then(absolute)
                    .map(|home| home.join(".cargo"))
            },
            absolute,
        )
}

/// Takes `<dir>/lock` without waiting, so two checks can't clone into one
/// directory under each other.
fn lock(dir: &Path) -> Result<DirLock, XtaskError> {
    try_lock_dir(dir).map_err(|error| match error {
        LockError::Held { path, holder } => XtaskError::reproducible_input(format!(
            "{} is in use by process {} (it holds {}); wait for it, or pass another --dir",
            dir.display(),
            holder.as_deref().unwrap_or("unknown"),
            path.display()
        )),
        LockError::Io(LockIoError { source, .. }) => {
            XtaskError::Failed(format!("failed to lock {}: {source}", dir.display()))
        }
    })
}

/// Makes `<dir>/src` a fresh clone holding the one commit `sha` of the
/// checkout at `root`, checked out detached, with the checkout's V8
/// archives linked in, replacing an earlier one, and returns its path.
/// `dir` is the real path [`take`] returned, and the checkout's build ran
/// since, so it's read again first ([`still_taken`]), and the cargo configs
/// in it and above it with it ([`refuse_clone_configs`], `env` the check's
/// own environment). What remains is the moment between that read and the
/// removal's opening `src`, and only for someone who can write a parent of
/// `dir`.
fn clone(
    root: &Path,
    dir: &Path,
    sha: &str,
    env: &[(OsString, OsString)],
) -> Result<PathBuf, XtaskError> {
    still_taken(dir)?;
    refuse_clone_configs(root, dir, env)?;
    let src = dir.join("src");
    match fs::remove_dir_all(&src) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(XtaskError::Failed(format!(
                "failed to remove the earlier clone {}: {error}",
                src.display()
            )));
        }
        _ => {}
    }
    fs::create_dir_all(&src).map_err(|error| {
        XtaskError::Failed(format!("failed to create {}: {error}", src.display()))
    })?;
    let root_arg = root.display().to_string();
    run(&src, &[], "git", &["init", "--quiet"])?;
    // a fetch from a path honors --depth, and git serves a commit by its id
    run(
        &src,
        &[],
        "git",
        &["fetch", "--quiet", "--depth", "1", &root_arg, sha],
    )?;
    run(
        &src,
        &[],
        "git",
        &["checkout", "--quiet", "--detach", "FETCH_HEAD"],
    )?;
    link_v8_archives(root, &src).map_err(|error| {
        XtaskError::Failed(format!(
            "failed to link the V8 archives into {}: {error}",
            src.display()
        ))
    })?;
    Ok(src)
}

/// Compares the binary `first` with `second`, and the snapshot files beside
/// each, failing with what [`describe`] finds when they differ, which names
/// `kept`, the clone left for inspection.
fn compare(first: &Path, second: &Path, kept: &Path) -> StepResult {
    let read = |path: &Path| {
        fs::read(path).map_err(|error| {
            XtaskError::Failed(format!("failed to read {}: {error}", path.display()))
        })
    };
    let sha = |path: &Path| crate::v8_archive::sha256(path).map_err(XtaskError::from);
    let (first_sha, second_sha) = (sha(first)?, sha(second)?);
    println!(
        "\n[xtask] {first_sha}  {}\n[xtask] {second_sha}  {}",
        first.display(),
        second.display()
    );
    let (a, b) = (read(first)?, read(second)?);
    let mut differing_files = Vec::new();
    for file in SNAPSHOT_FILES {
        if read(&first.with_file_name(file))? != read(&second.with_file_name(file))? {
            differing_files.push(file);
        }
    }
    if a == b && differing_files.is_empty() {
        println!("[xtask] the two builds of fgro are identical, byte for byte");
        return Ok(());
    }
    let version = |path: &Path| {
        Command::new(path).arg("--version").output().map_or_else(
            |error| format!("(doesn't run: {error})"),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )
    };
    let snapshot = if differing_files.is_empty() {
        "the snapshot files are identical".to_owned()
    } else {
        format!(
            "the snapshot files that differ: {}",
            differing_files.join(", ")
        )
    };
    Err(XtaskError::Failed(format!(
        "fgro isn't reproducible: {} ({} bytes, SHA-256 {first_sha}) and {} ({} bytes, SHA-256 \
         {second_sha}) {}; {snapshot}\n  checkout: {}\n  clone:    {}\nThe clone and its build \
         are kept in {}",
        first.display(),
        a.len(),
        second.display(),
        b.len(),
        describe(&a, &b),
        version(first),
        version(second),
        kept.display()
    )))
}

/// Where `a` and `b`, two builds of one ELF binary, differ: the region of
/// the first differing byte in `a`'s layout, and every section by name whose
/// bytes differ or that only one has, in `a`'s file order. Two equal inputs
/// say so.
fn describe(a: &[u8], b: &[u8]) -> String {
    let Some(offset) = first_difference(a, b) else {
        return "are identical".to_owned();
    };
    let (Some(sections_a), Some(sections_b)) = (sections(a), sections(b)) else {
        return format!(
            "differ first at byte {offset}; one isn't a 64-bit little-endian ELF file, so no \
             section is named"
        );
    };
    let mut differing: Vec<&str> = sections_a
        .iter()
        .filter(|section| {
            sections_b
                .iter()
                .find(|other| other.name == section.name)
                .is_none_or(|other| section.bytes(a) != other.bytes(b))
        })
        .map(|section| section.name.as_str())
        .collect();
    differing.extend(
        sections_b
            .iter()
            .filter(|section| sections_a.iter().all(|other| other.name != section.name))
            .map(|section| section.name.as_str()),
    );
    let differing = if differing.is_empty() {
        "no section's bytes differ".to_owned()
    } else {
        format!("the sections that differ: {}", differing.join(", "))
    };
    format!(
        "differ first at byte {offset}, in {}; {differing}",
        region_of(a, &sections_a, offset)
    )
}

/// The first offset where `a` and `b` differ, the shorter's length when one
/// is a prefix of the other, or `None` when they're equal.
fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len())))
}

/// A section of an ELF file, as its header places it in the file.
#[derive(Debug, PartialEq, Eq)]
struct Section {
    name: String,
    /// Its bytes' range in the file; empty for one that has none
    /// (`SHT_NOBITS`, as `.bss`).
    offset: usize,
    size: usize,
}

impl Section {
    /// Its bytes in `file`, empty where they'd run past its end.
    fn bytes<'a>(&self, file: &'a [u8]) -> &'a [u8] {
        file.get(self.offset..self.offset.saturating_add(self.size))
            .unwrap_or_default()
    }
}

/// `SHT_NOBITS`: a section that occupies no bytes in the file.
const SHT_NOBITS: u32 = 8;

/// The size of an ELF64 file header.
const ELF64_HEADER: usize = 64;

fn read_u16(file: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(file.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(file: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(file.get(at..at + 4)?.try_into().ok()?))
}

fn read_offset(file: &[u8], at: usize) -> Option<usize> {
    usize::try_from(u64::from_le_bytes(file.get(at..at + 8)?.try_into().ok()?)).ok()
}

/// The sections of `file`, a 64-bit little-endian ELF file, in their
/// headers' order, or `None` when it isn't one or its headers run past its
/// end.
fn sections(file: &[u8]) -> Option<Vec<Section>> {
    // the magic, ELFCLASS64, and ELFDATA2LSB
    if file.get(..6)? != b"\x7fELF\x02\x01" {
        return None;
    }
    let table = read_offset(file, 0x28)?;
    let entry = usize::from(read_u16(file, 0x3a)?);
    let count = usize::from(read_u16(file, 0x3c)?);
    let names_index = usize::from(read_u16(file, 0x3e)?);
    // the whole table in the file, each entry as large as the fields read
    if entry < 40 || table.checked_add(count.checked_mul(entry)?)? > file.len() {
        return None;
    }
    let header = |index: usize| -> Option<(u32, u32, usize, usize)> {
        let at = table.checked_add(index.checked_mul(entry)?)?;
        let (name, kind) = (read_u32(file, at)?, read_u32(file, at + 4)?);
        let (offset, size) = (read_offset(file, at + 24)?, read_offset(file, at + 32)?);
        Some((name, kind, offset, size))
    };
    let (_, _, names_offset, names_size) = header(names_index)?;
    let names = file.get(names_offset..names_offset.checked_add(names_size)?)?;
    (0..count)
        .map(|index| {
            let (name, kind, offset, size) = header(index)?;
            let name = names.get(usize::try_from(name).ok()?..)?;
            let name = &name[..name.iter().position(|byte| *byte == 0)?];
            Some(Section {
                name: String::from_utf8_lossy(name).into_owned(),
                offset,
                size: if kind == SHT_NOBITS { 0 } else { size },
            })
        })
        .collect()
}

/// What holds `offset` in `file`: a section by name, the ELF header, the
/// program or section header table, or none of them.
fn region_of(file: &[u8], sections: &[Section], offset: usize) -> String {
    if offset < ELF64_HEADER {
        return "the ELF header".to_owned();
    }
    let table = |at: usize, entry: usize, count: usize| -> Option<std::ops::Range<usize>> {
        let start = read_offset(file, at)?;
        let size = usize::from(read_u16(file, entry)?) * usize::from(read_u16(file, count)?);
        Some(start..start.checked_add(size)?)
    };
    if table(0x20, 0x36, 0x38).is_some_and(|range| range.contains(&offset)) {
        return "the program headers".to_owned();
    }
    if table(0x28, 0x3a, 0x3c).is_some_and(|range| range.contains(&offset)) {
        return "the section headers".to_owned();
    }
    sections
        .iter()
        .find(|section| (section.offset..section.offset + section.size).contains(&offset))
        .map_or_else(
            || "no section".to_owned(),
            |section| format!("section {}", section.name),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF64 file: the header, two sections' bytes, the name
    /// table, and the section headers (a null one, `.text`, `.data`, and
    /// `.shstrtab`), with `.text` and `.data` holding `text` and `data`.
    fn elf(text: &[u8], data: &[u8]) -> Vec<u8> {
        let names = b"\0.text\0.data\0.shstrtab\0";
        let mut file = vec![0u8; ELF64_HEADER];
        file[..6].copy_from_slice(b"\x7fELF\x02\x01");
        let text_offset = file.len();
        file.extend_from_slice(text);
        let data_offset = file.len();
        file.extend_from_slice(data);
        let names_offset = file.len();
        file.extend_from_slice(names);
        let table = file.len();
        let mut header = |name: u32, kind: u32, offset: usize, size: usize| {
            let mut entry = [0u8; 64];
            entry[..4].copy_from_slice(&name.to_le_bytes());
            entry[4..8].copy_from_slice(&kind.to_le_bytes());
            entry[24..32].copy_from_slice(&u64::try_from(offset).unwrap().to_le_bytes());
            entry[32..40].copy_from_slice(&u64::try_from(size).unwrap().to_le_bytes());
            file.extend_from_slice(&entry);
        };
        header(0, 0, 0, 0);
        header(1, 1, text_offset, text.len());
        header(7, 1, data_offset, data.len());
        header(13, 3, names_offset, names.len());
        file[0x28..0x30].copy_from_slice(&u64::try_from(table).unwrap().to_le_bytes());
        file[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        file[0x3c..0x3e].copy_from_slice(&4u16.to_le_bytes());
        file[0x3e..0x40].copy_from_slice(&3u16.to_le_bytes());
        file
    }

    #[test]
    fn reads_the_sections_of_an_elf_file() {
        let file = elf(b"TEXT", b"DATA!");
        let found = sections(&file).unwrap();
        let names: Vec<&str> = found.iter().map(|section| section.name.as_str()).collect();
        assert_eq!(names, ["", ".text", ".data", ".shstrtab"]);
        assert_eq!(found[1].bytes(&file), b"TEXT");
        assert_eq!(found[2].bytes(&file), b"DATA!");
        assert_eq!(sections(b"not an elf file"), None);
        // a section header table past the end
        let mut cut = file.clone();
        cut.truncate(file.len() - 1);
        assert_eq!(sections(&cut), None);
    }

    /// This test's own executable, a real ELF file, has the sections every
    /// linked binary does.
    #[test]
    fn reads_the_sections_of_this_executable() {
        let file = fs::read(std::env::current_exe().unwrap()).unwrap();
        let found = sections(&file).unwrap();
        for name in [".text", ".rodata", ".data"] {
            assert!(
                found.iter().any(|section| section.name == name),
                "{name} in {found:?}"
            );
        }
    }

    #[test]
    fn names_the_first_differing_section_and_every_one_that_differs() {
        let a = elf(b"TEXT", b"DATA!");
        assert_eq!(describe(&a, &a), "are identical");
        let data = elf(b"TEXT", b"DATA?");
        assert_eq!(
            describe(&a, &data),
            format!(
                "differ first at byte {}, in section .data; the sections that differ: .data",
                ELF64_HEADER + 8
            )
        );
        let both = elf(b"TEXt", b"DATA?");
        assert_eq!(
            describe(&a, &both),
            format!(
                "differ first at byte {}, in section .text; the sections that differ: .text, .data",
                ELF64_HEADER + 3
            )
        );
        // a longer .text moves every later byte, the section header table
        // too, whose offset the ELF header holds: sections are compared by
        // name and content, not offset
        let longer = elf(b"TEXT!", b"DATA!");
        assert_eq!(
            describe(&a, &longer),
            "differ first at byte 40, in the ELF header; the sections that differ: .text"
        );
        let mut header = a.clone();
        header[7] = 1;
        assert_eq!(
            describe(&a, &header),
            "differ first at byte 7, in the ELF header; no section's bytes differ"
        );
        assert_eq!(
            describe(b"not elf", b"not elF"),
            "differ first at byte 6; one isn't a 64-bit little-endian ELF file, so no section \
             is named"
        );
    }

    #[test]
    fn a_prefix_differs_at_its_end() {
        assert_eq!(first_difference(b"abc", b"abc"), None);
        assert_eq!(first_difference(b"abc", b"abd"), Some(2));
        assert_eq!(first_difference(b"ab", b"abc"), Some(2));
        assert_eq!(first_difference(b"abc", b""), Some(0));
    }

    #[test]
    fn the_clone_s_directory_is_kept_apart_from_the_checkout() {
        let root = crate::root();
        let inside = work_dir(&root, Some(root.join("target/reproducible")), &[]).unwrap_err();
        assert_eq!(inside.exit_code(), crate::EXIT_INPUT);
        assert!(
            inside.to_string().starts_with("check-reproducible: "),
            "{inside}"
        );
        let above = root.parent().unwrap().to_owned();
        assert!(work_dir(&root, Some(above), &[]).is_err());
        let apart = std::env::temp_dir().join("gro_reproducible_apart");
        assert_eq!(work_dir(&root, Some(apart.clone()), &[]).unwrap(), apart);
    }

    /// A `--dir` that climbs with `..` is refused, though it reads as apart:
    /// the kernel resolves a missing part's `..` only once the part is made,
    /// so `<parent>/missing/../<checkout>` reads as beside the checkout and
    /// would be made as the checkout.
    #[test]
    fn a_dir_that_climbs_is_refused() {
        let root = crate::root();
        let into = root
            .parent()
            .unwrap()
            .join("xtask_reproducible_missing/..")
            .join(root.file_name().unwrap());
        let apart = std::env::temp_dir().join("xtask_reproducible_missing/../apart");
        for dir in [into, apart] {
            let refused = work_dir(&root, Some(dir), &[]).unwrap_err();
            assert_eq!(refused.exit_code(), crate::EXIT_INPUT);
            assert!(
                refused.to_string().contains("climbs with `..`"),
                "{refused}"
            );
        }
    }

    /// The check takes a directory that's missing, marked, or holding only
    /// the lock an earlier run left, and refuses any other, writing nothing
    /// in it: not its `src`, and not a lock file of another's.
    #[test]
    fn only_a_directory_the_check_owns_is_taken() {
        let root = crate::root();
        let base =
            std::env::temp_dir().join(format!("xtask_reproducible_owned_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let foreign = base.join("foreign");
        fs::create_dir_all(foreign.join("src")).unwrap();
        fs::write(foreign.join("src/keep"), "x").unwrap();
        fs::write(foreign.join(LOCK_FILE), "keep me").unwrap();
        let refused = take(&root, &foreign).map(|_| ()).unwrap_err();
        // a lock file that isn't one a run left
        let stranger = base.join("stranger");
        fs::create_dir_all(&stranger).unwrap();
        fs::write(stranger.join(LOCK_FILE), "not a process id").unwrap();
        let stranger_refused = take(&root, &stranger).is_err();
        // a lock as a run leaves it
        let left = base.join("left");
        fs::create_dir_all(&left).unwrap();
        fs::write(left.join(LOCK_FILE), "1234\n").unwrap();
        let adopted = take(&root, &left).map(|(dir, _)| dir).unwrap();
        let left_real = left.canonicalize().unwrap();
        let marked = fs::read_to_string(left.join(MARKER)).unwrap();
        // a marked directory with an earlier run's clone kept in it
        fs::create_dir_all(left.join("src/target")).unwrap();
        let again = take(&root, &left).is_ok();
        // a directory named through a symlink is taken as its real path
        let link = base.join("link");
        std::os::unix::fs::symlink(&left, &link).unwrap();
        let through_link = take(&root, &link).map(|(dir, _)| dir).unwrap();
        let fresh = take(&root, &base.join("fresh/dir")).is_ok();
        let fresh_marked = base.join("fresh/dir").join(MARKER).is_file();
        let file = base.join("file");
        fs::write(&file, "").unwrap();
        let not_a_dir = take(&root, &file).map(|_| ()).unwrap_err();
        let kept = (
            foreign.join("src/keep").is_file(),
            fs::read_to_string(foreign.join(LOCK_FILE)).unwrap(),
            foreign.join(MARKER).exists(),
        );
        fs::remove_dir_all(&base).unwrap();
        assert_eq!(refused.exit_code(), crate::EXIT_INPUT);
        let message = refused.to_string();
        assert!(
            message.contains("(lock, src) and no .check-reproducible")
                && message
                    .contains("remove the directory if an earlier check-reproducible made it"),
            "{message}"
        );
        assert_eq!(kept, (true, "keep me".to_owned(), false));
        assert!(stranger_refused);
        assert_eq!(adopted, left_real);
        assert_eq!(marked, MARKER_TEXT);
        assert!(again);
        assert_eq!(through_link, left_real);
        assert!(fresh && fresh_marked);
        assert_eq!(not_a_dir.exit_code(), crate::EXIT_INPUT);
        assert!(
            not_a_dir.to_string().contains("isn't a directory"),
            "{not_a_dir}"
        );
    }

    /// A symlink is neither the lock an earlier run left nor the marker,
    /// whatever it points at: a `lock` linked to a file holding a process
    /// id would have the check's lock write through it, and a marker linked
    /// to a file holding the marker's text would hand it a directory it
    /// didn't make. Each is refused and its target left as it was.
    #[test]
    fn a_symlink_is_neither_a_left_lock_nor_the_marker() {
        let root = crate::root();
        let base =
            std::env::temp_dir().join(format!("xtask_reproducible_links_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let pid_file = base.join("pid");
        fs::write(&pid_file, "42\n").unwrap();
        let linked_lock = base.join("linked_lock");
        fs::create_dir_all(&linked_lock).unwrap();
        std::os::unix::fs::symlink(&pid_file, linked_lock.join(LOCK_FILE)).unwrap();
        let lock_refused = take(&root, &linked_lock).map(|_| ()).unwrap_err();
        let marker_file = base.join("marker");
        fs::write(&marker_file, MARKER_TEXT).unwrap();
        let linked_marker = base.join("linked_marker");
        fs::create_dir_all(linked_marker.join("src")).unwrap();
        fs::write(linked_marker.join("src/keep"), "x").unwrap();
        std::os::unix::fs::symlink(&marker_file, linked_marker.join(MARKER)).unwrap();
        let marker_refused = take(&root, &linked_marker).map(|_| ()).unwrap_err();
        let kept = (
            fs::read_to_string(&pid_file).unwrap(),
            linked_marker.join("src/keep").is_file(),
            linked_lock.join(MARKER).exists(),
        );
        fs::remove_dir_all(&base).unwrap();
        assert_eq!(lock_refused.exit_code(), crate::EXIT_INPUT);
        assert!(
            lock_refused.to_string().contains("(lock)"),
            "{lock_refused}"
        );
        assert_eq!(marker_refused.exit_code(), crate::EXIT_INPUT);
        assert!(
            marker_refused
                .to_string()
                .contains("(.check-reproducible, src)"),
            "{marker_refused}"
        );
        assert_eq!(kept, ("42\n".to_owned(), true, false));
    }

    /// The clone reads the taken directory again before it removes an
    /// earlier `src`: a parent swapped for a symlink since [`take`], or a
    /// directory of another's moved into its place, is refused, its `src`
    /// left as it was.
    #[test]
    fn the_clone_reads_its_directory_again_before_it_removes_anything() {
        let root = crate::root();
        let base =
            std::env::temp_dir().join(format!("xtask_reproducible_again_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        // a commit no checkout holds, so a clone that went on would fail
        let sha = "0".repeat(40);
        let (swapped, _held) = take(&root, &base.join("parent/dir")).unwrap();
        fs::create_dir_all(swapped.join("src")).unwrap();
        fs::write(swapped.join("src/keep"), "x").unwrap();
        let parent = swapped.parent().unwrap().to_owned();
        let moved = parent.with_file_name("moved");
        fs::rename(&parent, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &parent).unwrap();
        let through_swap = clone(&root, &swapped, &sha, &[]).map(|_| ()).unwrap_err();
        let (replaced, _held) = take(&root, &base.join("replaced")).unwrap();
        fs::remove_file(replaced.join(MARKER)).unwrap();
        fs::create_dir_all(replaced.join("src")).unwrap();
        fs::write(replaced.join("src/keep"), "x").unwrap();
        let unmarked = clone(&root, &replaced, &sha, &[]).map(|_| ()).unwrap_err();
        let kept = (
            moved.join("dir/src/keep").is_file(),
            replaced.join("src/keep").is_file(),
        );
        fs::remove_dir_all(&base).unwrap();
        assert_eq!(through_swap.exit_code(), crate::EXIT_INPUT);
        assert!(
            through_swap
                .to_string()
                .contains("so nothing in it is removed"),
            "{through_swap}"
        );
        assert_eq!(unmarked.exit_code(), crate::EXIT_INPUT);
        assert!(
            unmarked
                .to_string()
                .contains("(src) and no .check-reproducible"),
            "{unmarked}"
        );
        assert_eq!(kept, (true, true));
    }

    /// A pass removes its clone only from a directory still taken: a parent
    /// swapped for a symlink since [`take`], or a directory of another's
    /// moved into its place, is refused, its `src` left as it was.
    #[test]
    fn a_passed_clone_is_removed_only_from_a_directory_still_taken() {
        let root = crate::root();
        let base =
            std::env::temp_dir().join(format!("xtask_reproducible_remove_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (swapped, _held) = take(&root, &base.join("parent/dir")).unwrap();
        fs::create_dir_all(swapped.join("src")).unwrap();
        fs::write(swapped.join("src/keep"), "x").unwrap();
        let parent = swapped.parent().unwrap().to_owned();
        let moved = parent.with_file_name("moved");
        fs::rename(&parent, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &parent).unwrap();
        let through_swap = remove_clone(&swapped).unwrap_err();
        let (replaced, _held) = take(&root, &base.join("replaced")).unwrap();
        fs::remove_file(replaced.join(MARKER)).unwrap();
        fs::create_dir_all(replaced.join("src")).unwrap();
        fs::write(replaced.join("src/keep"), "x").unwrap();
        let unmarked = remove_clone(&replaced).unwrap_err();
        let (taken, _held) = take(&root, &base.join("taken")).unwrap();
        fs::create_dir_all(taken.join("src")).unwrap();
        fs::write(taken.join("src/clone"), "x").unwrap();
        let removed = remove_clone(&taken).is_ok() && !taken.join("src").exists();
        let kept = (
            moved.join("dir/src/keep").is_file(),
            replaced.join("src/keep").is_file(),
        );
        fs::remove_dir_all(&base).unwrap();
        assert!(
            through_swap
                .to_string()
                .contains("so nothing in it is removed"),
            "{through_swap}"
        );
        assert!(
            unmarked
                .to_string()
                .contains("(src) and no .check-reproducible"),
            "{unmarked}"
        );
        assert_eq!(kept, (true, true));
        assert!(removed);
    }

    /// A variable that configures how cargo compiles is refused first too,
    /// `RUSTC` among them, before the `--dir` is read.
    #[test]
    fn a_codegen_override_in_the_environment_is_refused_first() {
        let root = crate::root();
        let dir = std::env::temp_dir().join("xtask_reproducible_missing/../overridden");
        let env = [(OsString::from("RUSTC"), OsString::from("rustc"))];
        let refused = check_reproducible(&root, Some(dir), &env).unwrap_err();
        assert_eq!(refused.exit_code(), crate::EXIT_INPUT);
        let message = refused.to_string();
        assert!(
            message.contains("would take RUSTC and the clone's wouldn't"),
            "{message}"
        );
    }

    /// A cargo config only the clone's build would read, above the clone's
    /// directory or in it, is refused: by [`work_dir`], before anything is
    /// made, and by [`clone`], before anything is removed. One above the
    /// checkout too, or cargo's home's, which both builds read, isn't.
    #[test]
    fn a_cargo_config_only_the_clone_reads_is_refused() {
        let root = crate::root();
        let base =
            std::env::temp_dir().join(format!("xtask_reproducible_config_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let env = |vars: &[(&str, &Path)]| -> Vec<(OsString, OsString)> {
            vars.iter()
                .map(|(name, value)| (OsString::from(name), value.as_os_str().to_owned()))
                .collect()
        };
        let config = |dir: &Path, name: &str| {
            fs::create_dir_all(dir.join(".cargo")).unwrap();
            fs::write(dir.join(".cargo").join(name), "").unwrap();
        };
        config(&base.join("above"), "config.toml");
        let above = work_dir(&root, Some(base.join("above/dir")), &[]).unwrap_err();
        let made = base.join("above/dir").exists();
        let (inside, _held) = take(&root, &base.join("inside")).unwrap();
        fs::create_dir_all(inside.join("src")).unwrap();
        fs::write(inside.join("src/keep"), "x").unwrap();
        config(&inside, "config");
        let in_dir = clone(&root, &inside, &"0".repeat(40), &[])
            .map(|_| ())
            .unwrap_err();
        let kept = inside.join("src/keep").is_file();
        // above the checkout as well as the clone's directory
        config(&base.join("shared"), "config.toml");
        let shared =
            refuse_clone_configs(&base.join("shared/checkout"), &base.join("shared/dir"), &[]);
        // cargo's home, named by CARGO_HOME, through a symlink, or by HOME,
        // an empty CARGO_HOME as cargo reads it
        let home = base.join("home");
        config(&home, "config.toml");
        std::os::unix::fs::symlink(home.join(".cargo"), base.join("linked_home")).unwrap();
        let homes = [
            env(&[("CARGO_HOME", &home.join(".cargo"))]),
            env(&[("CARGO_HOME", &base.join("linked_home"))]),
            env(&[("HOME", &home)]),
            env(&[("CARGO_HOME", Path::new("")), ("HOME", &home)]),
        ]
        .map(|env| work_dir(&root, Some(home.join("dir")), &env).is_ok());
        let elsewhere = work_dir(
            &root,
            Some(home.join("dir")),
            &env(&[("CARGO_HOME", &base.join("other")), ("HOME", &home)]),
        )
        .unwrap_err();
        fs::remove_dir_all(&base).unwrap();
        for (refused, config) in [
            (above, "above/.cargo/config.toml"),
            (in_dir, "inside/.cargo/config"),
            (elsewhere, "home/.cargo/config.toml"),
        ] {
            assert_eq!(refused.exit_code(), crate::EXIT_INPUT);
            let message = refused.to_string();
            assert!(
                message.contains(&format!("{config})"))
                    && message.contains(
                        "the clone's build would read a cargo config the checkout's wouldn't"
                    ),
                "{message}"
            );
        }
        assert!(!made);
        assert!(kept);
        assert!(shared.is_ok());
        assert_eq!(homes, [true; 4]);
    }

    /// With no `--dir`, the directory is the cache's, from `XDG_CACHE_HOME`
    /// or `HOME`, each taken only when absolute: a relative one would put
    /// the clone wherever the check runs.
    #[test]
    fn the_default_dir_takes_only_an_absolute_cache() {
        let root = crate::root();
        let env = |vars: &[(&str, &str)]| -> Vec<(OsString, OsString)> {
            vars.iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect()
        };
        let default = |vars: &[(&str, &str)]| work_dir(&root, None, &env(vars));
        assert_eq!(
            default(&[("XDG_CACHE_HOME", "/cache"), ("HOME", "/home/someone")]).unwrap(),
            Path::new("/cache/gro/reproducible")
        );
        assert_eq!(
            default(&[("XDG_CACHE_HOME", "cache"), ("HOME", "/home/someone")]).unwrap(),
            Path::new("/home/someone/.cache/gro/reproducible")
        );
        for vars in [
            &[("XDG_CACHE_HOME", "cache"), ("HOME", "home")][..],
            &[("HOME", "")],
            &[],
        ] {
            let refused = default(vars).unwrap_err();
            assert_eq!(refused.exit_code(), crate::EXIT_INPUT);
            assert!(
                refused.to_string().contains("set to an absolute path"),
                "{refused}"
            );
        }
    }

    /// Once the directory exists, its real path is read whole, through a
    /// symlink into the checkout.
    #[test]
    fn the_taken_directory_is_read_again_through_its_symlinks() {
        let root = crate::root();
        let dir =
            std::env::temp_dir().join(format!("xtask_reproducible_real_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(root.join("crates"), dir.join("link")).unwrap();
        let through = real_dir(&root, &dir.join("link"));
        let beside = real_dir(&root, &dir);
        let dir_real = dir.canonicalize().unwrap();
        fs::remove_dir_all(&dir).unwrap();
        let refused = through.unwrap_err();
        assert!(
            refused.to_string().contains("must not hold one another"),
            "{refused}"
        );
        assert_eq!(beside.unwrap(), dir_real);
    }

    #[test]
    fn a_rustc_wrapper_in_a_config_file_is_found() {
        assert!(sets_a_wrapper("[build]\nrustc-wrapper = \"sccache\"\n"));
        assert!(sets_a_wrapper("build.rustc-workspace-wrapper = \"w\"\n"));
        assert!(sets_a_wrapper("[build]\n\"rustc-wrapper\" = \"sccache\"\n"));
        assert!(!sets_a_wrapper(
            "[build]\n# rustc-wrapper = \"sccache\"\njobs = 4\n"
        ));
        assert!(!sets_a_wrapper("[env]\nRUSTC_WRAPPER_NOTE = \"x\"\n"));
    }

    /// A `--dir` that doesn't exist yet, reached through a symlink into the
    /// checkout, is refused: the nesting is read from the real path of the
    /// part that exists.
    #[test]
    fn a_symlink_into_the_checkout_is_seen_through() {
        let root = crate::root();
        let dir = std::env::temp_dir().join(format!("xtask_reproducible_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&root, dir.join("link")).unwrap();
        let through = work_dir(&root, Some(dir.join("link/target/not_yet")), &[]);
        let beside = work_dir(&root, Some(dir.join("not_yet/either")), &[]);
        fs::remove_dir_all(&dir).unwrap();
        let refused = through.unwrap_err();
        assert!(
            refused.to_string().contains("must not hold one another"),
            "{refused}"
        );
        assert_eq!(beside.unwrap(), dir.join("not_yet/either"));
    }
}
