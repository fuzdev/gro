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
//! the first's compile.
//!
//! What it can't see: a build on another machine, with another toolchain,
//! or with another cargo home (whose registry paths the binary names), which
//! the claim excludes; a dirty tree or one outside git, which embeds another
//! commit by design and which the check refuses rather than builds; the
//! debug build, since it holds the release one; and what both builds read
//! alike on one machine, inside the checkout or out: a directory's listing
//! order on one filesystem, the day's date, a cache they share.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use gro_bench::lock::{DirLock, LockError, LockIoError, try_lock_dir};
use gro_bench::record::GitState;

use crate::bench::base::{
    BASE_BUILD_ENV, clean_command, codegen_overrides, link_v8_archives, rustc_version,
};
use crate::cmd::{StepResult, run};
use crate::error::XtaskError;
use crate::snapshot::{self, Measurement, SNAPSHOT_FILES};

/// Checks that the release `fgro` builds byte for byte from a second clone
/// of the checkout's commit, cloned and built under `dir`.
pub fn check_reproducible(root: &Path, dir: Option<PathBuf>) -> StepResult {
    let overrides =
        codegen_overrides(std::env::vars_os().filter_map(|(name, _)| name.into_string().ok()));
    if !overrides.is_empty() {
        return Err(XtaskError::reproducible_input(format!(
            "the clone builds from a cleared environment, so the checkout's build would take {} \
             and the clone's wouldn't; unset them for the check",
            overrides.join(", ")
        )));
    }
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
    let dir = work_dir(root, dir)?;
    let _lock = lock(&dir)?;

    let target_dir = crate::check::fetch_v8(root)?;
    let first = snapshot::build(root, &target_dir, true, Measurement::None)?;
    if first.commit != state.sha {
        return Err(XtaskError::Failed(format!(
            "the checkout moved during its build: it embeds `{}`, not `{}`",
            first.commit, state.sha
        )));
    }

    let src = dir.join("src");
    clone(root, &src, &state.sha)?;
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
    fs::remove_dir_all(&src)
        .map_err(|error| XtaskError::Failed(format!("failed to remove {}: {error}", src.display())))
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

/// The directory the clone goes in: `given`, or the cache's; refused when it
/// and the checkout hold one another, since the clone's cargo would read the
/// checkout's config, or the two paths would be one.
fn work_dir(root: &Path, given: Option<PathBuf>) -> Result<PathBuf, XtaskError> {
    let dir = if let Some(dir) = given {
        std::path::absolute(&dir).map_err(|error| {
            XtaskError::reproducible_input(format!("invalid --dir {}: {error}", dir.display()))
        })?
    } else {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .ok_or_else(|| {
                XtaskError::reproducible_input("neither XDG_CACHE_HOME nor HOME is set; pass --dir")
            })?;
        cache.join("gro").join("reproducible")
    };
    let real = real_path(&dir);
    if real.starts_with(root) || root.starts_with(&real) {
        return Err(XtaskError::reproducible_input(format!(
            "the clone's directory {} and the checkout {} must not hold one another",
            dir.display(),
            root.display()
        )));
    }
    Ok(dir)
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

/// Makes `src` a fresh clone holding the one commit `sha` of the checkout at
/// `root`, checked out detached, with the checkout's V8 archives linked in.
fn clone(root: &Path, src: &Path, sha: &str) -> StepResult {
    match fs::remove_dir_all(src) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(XtaskError::Failed(format!(
                "failed to remove the earlier clone {}: {error}",
                src.display()
            )));
        }
        _ => {}
    }
    fs::create_dir_all(src).map_err(|error| {
        XtaskError::Failed(format!("failed to create {}: {error}", src.display()))
    })?;
    let root_arg = root.display().to_string();
    run(src, &[], "git", &["init", "--quiet"])?;
    // a fetch from a path honors --depth, and git serves a commit by its id
    run(
        src,
        &[],
        "git",
        &["fetch", "--quiet", "--depth", "1", &root_arg, sha],
    )?;
    run(
        src,
        &[],
        "git",
        &["checkout", "--quiet", "--detach", "FETCH_HEAD"],
    )?;
    link_v8_archives(root, src).map_err(|error| {
        XtaskError::Failed(format!(
            "failed to link the V8 archives into {}: {error}",
            src.display()
        ))
    })
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
        let inside = work_dir(&root, Some(root.join("target/reproducible"))).unwrap_err();
        assert_eq!(inside.exit_code(), crate::EXIT_INPUT);
        assert!(
            inside.to_string().starts_with("check-reproducible: "),
            "{inside}"
        );
        let above = root.parent().unwrap().to_owned();
        assert!(work_dir(&root, Some(above)).is_err());
        let apart = std::env::temp_dir().join("gro_reproducible_apart");
        assert_eq!(work_dir(&root, Some(apart.clone())).unwrap(), apart);
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
        let through = work_dir(&root, Some(dir.join("link/target/not_yet")));
        let beside = work_dir(&root, Some(dir.join("not_yet/either")));
        fs::remove_dir_all(&dir).unwrap();
        let refused = through.unwrap_err();
        assert!(
            refused.to_string().contains("must not hold one another"),
            "{refused}"
        );
        assert_eq!(beside.unwrap(), dir.join("not_yet/either"));
    }
}
