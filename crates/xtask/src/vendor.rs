//! The vendored crates under `vendor/`: each `vendor/<name>/` is a published
//! crate plus gro's patch, `vendor/<name>.patch`, and `vendor/README.md`
//! indexes them, recording each `.crate`'s SHA-256.
//!
//! - [`check`], a `check-rust` step, proves each tree is its crate plus its
//!   patch: it fetches `https://static.crates.io/crates/<name>/<name>-<version>.crate`
//!   once into `target/vendor/<sha256>/`, verifying the README's SHA-256
//!   ([`crate::v8_archive::download`]), extracts it, applies the patch
//!   (`patch -p1 --fuzz=0`), and compares the result with the vendored tree,
//!   contents and executable bits, naming each file that differs. All of it
//!   runs under a lock on `target/vendor/`, as does [`write_patch`].
//! - [`write_patch`], `cargo xtask vendor-patch <name>`, regenerates
//!   `vendor/<name>.patch` from the tree: the diff of the vendored directory
//!   against the extracted crate, with `--- a/<path>` and `+++ b/<path>`
//!   headers relative to the crate root, which the unit tests parse.
//! - the unit tests check the README and the trees against the patches: every
//!   file a patch touches is listed in the README, every file it modifies
//!   (rather than adds) carries a `gro patch:` marker, and the markers in each
//!   vendored tree are the ones its patch adds. The marker is matched in any
//!   file, whatever its comment syntax (`// gro patch:` in Rust).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gro_bench::lock::{DirLock, LockIoError, lock_dir_blocking};

use crate::cmd::StepResult;
use crate::error::XtaskError;

/// A vendored crate, as `vendor/` and `vendor/README.md` describe it.
struct Vendored {
    name: String,
    version: String,
    /// The published `.crate`'s SHA-256, as the README records it.
    sha256: String,
}

impl Vendored {
    fn tree(&self, root: &Path) -> PathBuf {
        root.join("vendor").join(&self.name)
    }

    fn patch(&self, root: &Path) -> PathBuf {
        root.join("vendor").join(format!("{}.patch", self.name))
    }

    fn crate_file(&self) -> String {
        format!("{}-{}.crate", self.name, self.version)
    }
}

fn read(path: &Path) -> Result<String, XtaskError> {
    fs::read_to_string(path)
        .map_err(|error| XtaskError::Failed(format!("failed to read {}: {error}", path.display())))
}

/// Every crate with a `vendor/<name>.patch`, sorted by name.
fn vendored(root: &Path) -> Result<Vec<Vendored>, XtaskError> {
    let vendor = root.join("vendor");
    let readme = read(&vendor.join("README.md"))?;
    let entries = fs::read_dir(&vendor).map_err(|error| {
        XtaskError::Failed(format!("failed to read {}: {error}", vendor.display()))
    })?;
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "patch").then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let manifest = read(&vendor.join(&name).join("Cargo.toml"))?;
            let version = package_version(&manifest).ok_or_else(|| {
                XtaskError::vendor_input(format!("vendor/{name}/Cargo.toml names no version"))
            })?;
            let sha256 = recorded_sha256(&readme, &name).ok_or_else(|| {
                XtaskError::vendor_input(format!(
                    "vendor/README.md records no SHA-256 for {name} (a backticked hash under `## {name}`)"
                ))
            })?;
            Ok(Vendored {
                version: version.to_owned(),
                sha256: sha256.to_owned(),
                name,
            })
        })
        .collect()
}

/// The crate's version, from the `version` line in its `[package]` table.
fn package_version(manifest: &str) -> Option<&str> {
    manifest
        .split("[package]")
        .nth(1)?
        .lines()
        .take_while(|line| !line.starts_with('['))
        .find_map(|line| line.strip_prefix("version = \""))?
        .strip_suffix('"')
}

/// Locks `target/vendor/`, so one process at a time fetches, extracts,
/// patches, and compares there, and none deletes another's work tree.
fn lock(root: &Path) -> Result<DirLock, XtaskError> {
    lock_dir_blocking(&root.join("target/vendor")).map_err(|LockIoError { path, source }| {
        XtaskError::Failed(format!("failed to lock {}: {source}", path.display()))
    })
}

/// The first backticked 64-digit hex string in the README's `## <name>`
/// section.
fn recorded_sha256<'a>(readme: &'a str, name: &str) -> Option<&'a str> {
    let heading = format!("\n## {name}\n");
    let start = readme.find(&heading)? + heading.len();
    let section = &readme[start..];
    let section = &section[..section.find("\n## ").unwrap_or(section.len())];
    section.split('`').skip(1).step_by(2).find(|token| {
        token.len() == 64
            && token
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// The published `.crate`, fetched once and verified: cached under its
/// SHA-256, so a changed pin is a new path. The caller holds [`lock`].
fn fetch(root: &Path, vendored: &Vendored) -> Result<PathBuf, XtaskError> {
    let dir = root.join("target/vendor");
    let path = dir.join(&vendored.sha256).join(vendored.crate_file());
    if path.is_file() {
        if crate::v8_archive::sha256(&path)? == vendored.sha256 {
            return Ok(path);
        }
        let _ = fs::remove_file(&path);
    }
    let url = format!(
        "https://static.crates.io/crates/{}/{}",
        vendored.name,
        vendored.crate_file()
    );
    crate::v8_archive::download(&url, &path, &vendored.sha256)?;
    Ok(path)
}

/// Runs `command`, failing with `display` and its status unless it succeeds.
fn run_quiet(command: &mut Command, display: &str) -> StepResult {
    let status = command.status().map_err(|source| XtaskError::Spawn {
        display: display.to_owned(),
        source,
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(XtaskError::StepFailed {
            display: display.to_owned(),
            status,
        })
    }
}

/// Each file's executable bit under `dir`, by its path relative to `dir`.
fn executable_bits(dir: &Path) -> Result<BTreeMap<PathBuf, bool>, XtaskError> {
    let mut bits = BTreeMap::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(current) = pending.pop() {
        let entries = fs::read_dir(&current).map_err(|error| {
            XtaskError::Failed(format!("failed to read {}: {error}", current.display()))
        })?;
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    XtaskError::Failed(format!("failed to read {}: {error}", current.display()))
                })?
                .path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                XtaskError::Failed(format!("failed to read {}: {error}", path.display()))
            })?;
            if metadata.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(dir).unwrap_or(&path).to_owned();
                bits.insert(relative, metadata.permissions().mode() & 0o111 != 0);
            }
        }
    }
    Ok(bits)
}

/// Extracts the verified `.crate` into a fresh `target/vendor/work/<name>/`
/// and returns the crate's directory there.
fn extract(root: &Path, vendored: &Vendored) -> Result<PathBuf, XtaskError> {
    let published = fetch(root, vendored)?;
    let work = root.join("target/vendor/work").join(&vendored.name);
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|error| {
        XtaskError::Failed(format!("failed to create {}: {error}", work.display()))
    })?;
    run_quiet(
        Command::new("tar")
            .arg("-xzf")
            .arg(&published)
            .arg("-C")
            .arg(&work),
        &format!("tar -xzf {}", published.display()),
    )?;
    Ok(work.join(format!("{}-{}", vendored.name, vendored.version)))
}

/// Checks that each vendored tree is its published crate plus its patch,
/// contents and executable bits.
pub fn check(root: &Path) -> StepResult {
    let _lock = lock(root)?;
    for vendored in vendored(root)? {
        let extracted = extract(root, &vendored)?;
        let patch = vendored.patch(root);
        run_quiet(
            Command::new("patch")
                .args(["-p1", "-s", "--fuzz=0", "--no-backup-if-mismatch", "-i"])
                .arg(&patch)
                .current_dir(&extracted),
            &format!(
                "applying vendor/{}.patch to {}",
                vendored.name,
                vendored.crate_file()
            ),
        )?;
        let tree = vendored.tree(root);
        let diff = Command::new("diff")
            .arg("-rq")
            .arg(&extracted)
            .arg(&tree)
            .stderr(Stdio::inherit())
            .output()
            .map_err(|error| XtaskError::Failed(format!("failed to run diff: {error}")))?;
        let mut differences = String::from_utf8_lossy(&diff.stdout).trim_end().to_owned();
        if diff.status.success() {
            let (expected, actual) = (executable_bits(&extracted)?, executable_bits(&tree)?);
            for (file, executable) in &expected {
                if actual.get(file).is_some_and(|bit| bit != executable) {
                    let _ = write!(
                        differences,
                        "\n{} is {}executable here, {}executable in the patched crate",
                        file.display(),
                        if *executable { "not " } else { "" },
                        if *executable { "" } else { "not " },
                    );
                }
            }
        }
        if !differences.is_empty() {
            return Err(XtaskError::vendor_input(format!(
                "vendor/{name}/ isn't {file} plus vendor/{name}.patch (left as {extracted}); \
                 regenerate the patch with `cargo xtask vendor-patch {name}` if the tree is right:\n{}",
                differences.trim_start(),
                name = vendored.name,
                file = vendored.crate_file(),
                extracted = extracted.display(),
            )));
        }
        let _ = fs::remove_dir_all(extracted.parent().unwrap_or(&extracted));
        println!(
            "[xtask] vendor/{}/ is {} plus vendor/{}.patch",
            vendored.name,
            vendored.crate_file(),
            vendored.name
        );
    }
    Ok(())
}

/// Writes `vendor/<name>.patch`: the vendored tree's diff against its
/// published crate, by `git diff` in a scratch repository whose index holds
/// the crate and whose working tree holds the vendored copy. Git runs with no
/// global or system config, so neither changes the format.
pub fn write_patch(root: &Path, name: &str) -> StepResult {
    let _lock = lock(root)?;
    let vendored = vendored(root)?
        .into_iter()
        .find(|vendored| vendored.name == name)
        .ok_or_else(|| {
            XtaskError::vendor_input(format!(
                "no vendored crate {name}: vendor/{name}.patch must exist"
            ))
        })?;
    let extracted = extract(root, &vendored)?;
    let git_dir = extracted.join(".git");
    // git names the scratch repository explicitly and inherits nothing that
    // could point it at another, such as gro's own index
    let git = |args: &[&str]| {
        let mut command = Command::new("git");
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CEILING_DIRECTORIES",
            "GIT_CONFIG",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
            "GIT_EXTERNAL_DIFF",
            "GIT_DIFF_OPTS",
        ] {
            command.env_remove(variable);
        }
        command
            .args(args)
            .current_dir(&extracted)
            .env("GIT_DIR", &git_dir)
            .env("GIT_WORK_TREE", &extracted)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    };
    run_quiet(&mut git(&["init", "-q"]), "git init")?;
    run_quiet(&mut git(&["add", "-A", "-f"]), "git add")?;
    // the working tree becomes the vendored copy, `.git` aside
    for entry in fs::read_dir(&extracted).map_err(|error| {
        XtaskError::Failed(format!("failed to read {}: {error}", extracted.display()))
    })? {
        let path = entry
            .map_err(|error| {
                XtaskError::Failed(format!("failed to read {}: {error}", extracted.display()))
            })?
            .path();
        if path.file_name().is_some_and(|file| file == ".git") {
            continue;
        }
        let removed = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed.map_err(|error| {
            XtaskError::Failed(format!("failed to remove {}: {error}", path.display()))
        })?;
    }
    run_quiet(
        Command::new("cp")
            .arg("-a")
            .arg(vendored.tree(root).join("."))
            .arg(&extracted),
        "cp -a",
    )?;
    run_quiet(&mut git(&["add", "-N", "-f", "."]), "git add -N")?;
    let diff = git(&[
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-renames",
        "--diff-algorithm=myers",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ])
    .stderr(Stdio::inherit())
    .output()
    .map_err(|error| XtaskError::Failed(format!("failed to run git diff: {error}")))?;
    if !diff.status.success() {
        return Err(XtaskError::Failed(format!(
            "git diff failed ({})",
            diff.status
        )));
    }
    let patch = vendored.patch(root);
    fs::write(&patch, &diff.stdout).map_err(|error| {
        XtaskError::Failed(format!("failed to write {}: {error}", patch.display()))
    })?;
    let _ = fs::remove_dir_all(extracted.parent().unwrap_or(&extracted));
    println!(
        "[xtask] wrote vendor/{name}.patch, vendor/{name}/ against {}",
        vendored.crate_file()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The comment text every patched site carries.
    const MARKER: &str = "gro patch:";

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Each file a patch touches, from its `diff --git a/<path> b/<path>`
    /// lines, with whether the patch adds it whole.
    fn touched_files(patch: &str) -> Vec<(&str, bool)> {
        let mut files: Vec<(&str, bool)> = Vec::new();
        for line in patch.lines() {
            if let Some(header) = line.strip_prefix("diff --git a/") {
                let path = header.split(" b/").next().unwrap();
                files.push((path, false));
            } else if line.starts_with("new file mode")
                && let Some(last) = files.last_mut()
            {
                last.1 = true;
            }
        }
        files
    }

    /// How many lines of the files under `dir` hold the marker.
    fn count_markers(dir: &Path) -> usize {
        let mut count = 0;
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                count += count_markers(&path);
            } else {
                let bytes = fs::read(&path).unwrap();
                count += String::from_utf8_lossy(&bytes)
                    .lines()
                    .filter(|line| line.contains(MARKER))
                    .count();
            }
        }
        count
    }

    #[test]
    fn every_patch_is_indexed_and_marked() {
        let root = root();
        let readme = fs::read_to_string(root.join("vendor/README.md")).unwrap();
        let crates = vendored(&root).unwrap();
        assert!(!crates.is_empty(), "no vendor/*.patch");
        for vendored in crates {
            let name = &vendored.name;
            let patch = fs::read_to_string(vendored.patch(&root)).unwrap();
            let files = touched_files(&patch);
            assert!(!files.is_empty(), "vendor/{name}.patch touches no file");
            for (file, added) in files {
                assert!(
                    readme.contains(&format!("`{file}`")),
                    "vendor/README.md doesn't list `{file}`, which vendor/{name}.patch touches"
                );
                if !added {
                    let text = fs::read_to_string(vendored.tree(&root).join(file)).unwrap();
                    assert!(
                        text.contains(MARKER),
                        "vendor/{name}/{file} is patched but has no `{MARKER}` comment"
                    );
                }
            }
            let in_patch = patch
                .lines()
                .filter(|line| {
                    line.starts_with('+') && !line.starts_with("+++") && line.contains(MARKER)
                })
                .count();
            assert_eq!(
                count_markers(&vendored.tree(&root)),
                in_patch,
                "the `{MARKER}` comments in vendor/{name}/ differ from those vendor/{name}.patch adds"
            );
        }
    }

    #[test]
    fn the_readme_records_each_crates_hash_under_its_heading() {
        let readme = "intro `aaaa`\n\n## one\n\n- sha `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`\n\n## two\n\nnone\n";
        assert_eq!(
            recorded_sha256(readme, "one"),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert_eq!(recorded_sha256(readme, "two"), None);
        assert_eq!(recorded_sha256(readme, "three"), None);
    }

    #[test]
    fn a_manifest_names_its_package_version() {
        let manifest =
            "[workspace]\nversion = \"9\"\n[package]\nname = \"x\"\nversion = \"1.2.3\"\n";
        assert_eq!(package_version(manifest), Some("1.2.3"));
        assert_eq!(package_version("[lib]\n"), None);
        // a later table's version isn't the package's
        assert_eq!(
            package_version("[package]\nname = \"x\"\n[dependencies.y]\nversion = \"2\"\n"),
            None
        );
    }
}
