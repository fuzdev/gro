//! The git reads behind the run record: each checkout's state.
//!
//! They cover the gro checkout, the other checkouts a workload reads, and the
//! corpora checkout, beside the `git` calls they and the collections' copies
//! are made of.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::BenchError;
use crate::record::{CheckoutState, GitState};

impl GitState {
    /// Reads the state of the checkout at `root`. The history file, when it's
    /// inside the checkout, doesn't count as a change: every run appends to
    /// it. A `history` that climbs out with `..` can't be named to git as an
    /// exclusion, so it gets none (callers pass a normalized path).
    ///
    /// # Errors
    ///
    /// Fails when `git` can't run or fails, as outside a checkout.
    pub fn read(root: &Path, history: &Path) -> Result<Self, BenchError> {
        let sha = git(root, &["rev-parse", "HEAD"])?.trim().to_owned();
        // untracked files listed whatever `status.showUntrackedFiles` says
        let mut status_args = vec![
            "status".to_owned(),
            "--porcelain".to_owned(),
            "--untracked-files=normal".to_owned(),
        ];
        if let Some(relative) = history.strip_prefix(root).ok().filter(|relative| {
            relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        }) {
            status_args.push("--".to_owned());
            status_args.push(".".to_owned());
            status_args.push(format!(":(exclude){}", relative.display()));
        }
        let status_args: Vec<&str> = status_args.iter().map(String::as_str).collect();
        // raw bytes, since `core.quotePath=false` prints names that aren't UTF-8
        let dirty = !git_output(root, &status_args)?.trim_ascii().is_empty();
        Ok(Self { sha, dirty })
    }
}

/// The root of the git checkout holding `path` (a file or directory),
/// canonical, or `None` when it's in none or git can't say.
pub fn checkout_root(path: &Path) -> Option<PathBuf> {
    let dir = if path.is_dir() { path } else { path.parent()? };
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    PathBuf::from(root.trim_end_matches('\n'))
        .canonicalize()
        .ok()
}

impl CheckoutState {
    /// Reads the state of the checkout whose root is `root`.
    ///
    /// # Errors
    ///
    /// Fails when `git` can't run or fails, as outside a checkout, or an
    /// untracked file can't be read.
    pub fn read(root: &Path) -> Result<Self, BenchError> {
        let sha = git(root, &["rev-parse", "HEAD"])?.trim().to_owned();
        // the options a flag can pin set to git's default, so a user's diff
        // settings change neither the bytes nor what runs (no external diff,
        // textconv, or color)
        let diff = git_output(
            root,
            &[
                "-c",
                "core.quotePath=true",
                "-c",
                "diff.suppressBlankEmpty=false",
                "diff",
                "HEAD",
                "--binary",
                "--full-index",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--diff-algorithm=myers",
                "--indent-heuristic",
                "--unified=3",
                "--inter-hunk-context=0",
                "-O/dev/null",
            ],
        )?;
        // listed by `ls-files`, not `status`, which `status.showUntrackedFiles`
        // could make blind to them
        let untracked = git_output(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        if diff.is_empty() && untracked.is_empty() {
            return Ok(Self {
                sha,
                dirty: false,
                changes_blake3: None,
            });
        }
        let mut hasher = blake3::Hasher::new();
        hasher.update(&diff);
        for name in untracked
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
        {
            let name_error = || BenchError::Read {
                path: root.to_owned(),
                source: io::Error::new(
                    io::ErrorKind::InvalidData,
                    "an untracked file's name isn't UTF-8",
                ),
            };
            let path = root.join(std::str::from_utf8(name).map_err(|_| name_error())?);
            let read_error = |source| BenchError::Read {
                path: path.clone(),
                source,
            };
            let meta = fs::symlink_metadata(&path).map_err(read_error)?;
            let contents = if meta.is_symlink() {
                fs::read_link(&path)
                    .map_err(read_error)?
                    .into_os_string()
                    .into_encoded_bytes()
            } else if meta.is_file() {
                fs::read(&path).map_err(read_error)?
            } else {
                // a nested checkout, listed as a directory, which its own
                // state would name
                Vec::new()
            };
            // each part length-prefixed, so no two lists hash alike
            for part in [name, &contents[..]] {
                hasher.update(&(part.len() as u64).to_le_bytes());
                hasher.update(part);
            }
        }
        Ok(Self {
            sha,
            dirty: true,
            changes_blake3: Some(hasher.finalize().to_hex().to_string()),
        })
    }
}

/// The subtree of a corpora checkout that is the corpus, what its tree id
/// names.
const CORPORA_TREE: &str = "collections";

/// Reads the tree id of the corpora checkout at `path`, refusing local changes
/// under its `collections/`, which the tree id wouldn't name.
///
/// # Errors
///
/// Fails when `git` can't run or fails, as outside a checkout, or with
/// [`BenchError::CorporaDirty`].
pub fn read_corpora_tree(path: &Path) -> Result<String, BenchError> {
    let tree = git(path, &["rev-parse", &format!("HEAD:{CORPORA_TREE}")])?
        .trim()
        .to_owned();
    // untracked files count: a workload could import one, under any name
    // (listed whatever `status.showUntrackedFiles` says)
    let status = git_output(
        path,
        &[
            "status",
            "--porcelain",
            "--untracked-files=normal",
            "--",
            CORPORA_TREE,
        ],
    )?;
    if !status.trim_ascii().is_empty() {
        return Err(BenchError::CorporaDirty {
            path: path.to_owned(),
        });
    }
    Ok(tree)
}

/// Runs `git -C root` with `args` and returns its stdout as UTF-8, failing
/// when it can't run, exits unsuccessfully, or prints anything else, so no
/// name is ever read as another.
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String, BenchError> {
    String::from_utf8(git_output(root, args)?).map_err(|error| BenchError::Spawn {
        command: git_command(root, args),
        source: io::Error::new(io::ErrorKind::InvalidData, error),
    })
}

/// Runs `git -C root` with `args` and returns its stdout's bytes, failing
/// when it can't run or exits unsuccessfully.
pub(crate) fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, BenchError> {
    let command = git_command(root, args);
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        // a local `git replace` would change the objects behind a matching id
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        // a read never writes the index: the checkout may be another's, in use
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|source| BenchError::Spawn {
            command: command.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(BenchError::CommandFailed {
            command,
            status: output.status,
        });
    }
    Ok(output.stdout)
}

/// The command line `git` and `git_output` name in their errors.
fn git_command(root: &Path, args: &[&str]) -> String {
    format!("git -C {} {}", root.display(), args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Commits in the checkout at `root` with a throwaway identity and no
    /// signing, passing `extra` (such as `--allow-empty`) and the message.
    fn commit(root: &Path, extra: &[&str], message: &str) {
        let mut args = vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
        ];
        args.extend(extra);
        args.extend(["-m", message]);
        git(root, &args).unwrap();
    }

    /// A fresh checkout with one commit, at `<temp>/<name>/repo`.
    fn git_checkout(name: &str) -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
        let root = base.join("repo");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet"]).unwrap();
        commit(&root, &["--allow-empty"], "init");
        (base, root)
    }

    #[test]
    fn the_history_inside_the_checkout_is_not_a_change() {
        let (base, root) = git_checkout("gro_bench_git_inside");
        let history = root.join("sub/history.jsonl");
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&history, "{}\n").unwrap();
        let clean = GitState::read(&root, &history).unwrap().dirty;
        // any other new file is a change
        fs::write(root.join("other.txt"), "x").unwrap();
        let dirty = GitState::read(&root, &history).unwrap().dirty;
        fs::remove_dir_all(&base).unwrap();
        assert!(!clean);
        assert!(dirty);
    }

    #[test]
    fn a_checkout_state_names_its_changes() {
        let (base, root) = git_checkout("gro_bench_git_checkout_state");
        fs::write(root.join("tracked.txt"), "a").unwrap();
        git(&root, &["add", "."]).unwrap();
        commit(&root, &[], "add");
        let found = checkout_root(&root.join("tracked.txt"));
        let canonical = root.canonicalize().unwrap();
        let clean = CheckoutState::read(&root).unwrap();
        fs::write(root.join("tracked.txt"), "b").unwrap();
        let edited = CheckoutState::read(&root).unwrap();
        fs::write(root.join("new.txt"), "c").unwrap();
        let untracked = CheckoutState::read(&root).unwrap();
        // the same untracked file with other contents
        fs::write(root.join("new.txt"), "d").unwrap();
        let rewritten = CheckoutState::read(&root).unwrap();
        let again = CheckoutState::read(&root).unwrap();
        let outside = checkout_root(&base);
        fs::remove_dir_all(&base).unwrap();
        assert_eq!(found, Some(canonical));
        assert_eq!(outside, None);
        assert_eq!(clean.sha.len(), 40);
        assert!(!clean.dirty);
        assert_eq!(clean.changes_blake3, None);
        assert!(edited.dirty);
        let hashes = [&edited, &untracked, &rewritten].map(|state| {
            assert_eq!(state.sha, clean.sha);
            state.changes_blake3.clone().unwrap()
        });
        assert_ne!(hashes[0], hashes[1]);
        assert_ne!(hashes[1], hashes[2]);
        assert_eq!(again, rewritten);
    }

    #[test]
    fn a_checkouts_config_changes_neither_its_dirty_flag_nor_its_hash() {
        let (base, root) = git_checkout("gro_bench_git_config");
        fs::create_dir_all(root.join("collections")).unwrap();
        fs::write(root.join("collections/x.ts"), "a\n\nb\n").unwrap();
        fs::write(root.join("é.txt"), "a").unwrap();
        git(&root, &["add", "."]).unwrap();
        commit(&root, &[], "add");
        let history = root.join("history.jsonl");
        // only untracked changes first, then edits too
        fs::write(root.join("collections/new.ts"), "c").unwrap();
        let untracked_plain = CheckoutState::read(&root).unwrap();
        fs::write(root.join("collections/x.ts"), "a\n\nB\n").unwrap();
        fs::write(root.join("é.txt"), "b").unwrap();
        let edited_plain = CheckoutState::read(&root).unwrap();
        for setting in [
            "status.showUntrackedFiles=no",
            "core.quotePath=false",
            "diff.suppressBlankEmpty=true",
            "diff.noprefix=true",
            "diff.algorithm=histogram",
            "diff.context=9",
            "diff.renames=copies",
            // would run a program, and change the output, if it were honored
            "diff.external=false",
        ] {
            let (key, value) = setting.split_once('=').unwrap();
            git(&root, &["config", key, value]).unwrap();
        }
        let edited = CheckoutState::read(&root).unwrap();
        // back to only the untracked file, which `status` now doesn't list
        git(&root, &["checkout", "--quiet", "--", "."]).unwrap();
        let untracked = CheckoutState::read(&root).unwrap();
        let git_dirty = GitState::read(&root, &history).unwrap().dirty;
        let corpora = read_corpora_tree(&root);
        fs::remove_dir_all(&base).unwrap();
        assert!(untracked.dirty);
        assert_eq!(untracked, untracked_plain);
        assert_eq!(edited, edited_plain);
        assert_ne!(edited.changes_blake3, untracked.changes_blake3);
        assert!(git_dirty);
        assert!(matches!(corpora, Err(BenchError::CorporaDirty { .. })));
    }

    #[test]
    fn the_corpora_tree_is_read_and_local_changes_refused() {
        let (base, root) = git_checkout("gro_bench_git_corpora");
        fs::create_dir_all(root.join("collections/a")).unwrap();
        fs::write(root.join("collections/a/x.ts"), "export {};\n").unwrap();
        git(&root, &["add", "."]).unwrap();
        commit(&root, &[], "add");
        let tree = read_corpora_tree(&root).unwrap();
        let expected = git(&root, &["rev-parse", "HEAD:collections"]).unwrap();
        // a change outside `collections/` doesn't count
        fs::write(root.join("README.md"), "x").unwrap();
        let outside = read_corpora_tree(&root).unwrap();
        // an untracked file inside it does
        fs::write(root.join("collections/a/y.ts"), "").unwrap();
        let inside = read_corpora_tree(&root);
        fs::remove_dir_all(&base).unwrap();
        assert_eq!(tree, expected.trim());
        assert_eq!(tree.len(), 40);
        assert_eq!(outside, tree);
        assert!(matches!(inside, Err(BenchError::CorporaDirty { .. })));
    }

    #[test]
    fn a_history_outside_the_checkout_gets_no_exclusion() {
        let (base, root) = git_checkout("gro_bench_git_outside");
        // the shape `std::path::absolute` leaves for `--history ../x.jsonl`
        for history in [root.join("../x.jsonl"), base.join("x.jsonl")] {
            let state = GitState::read(&root, &history).unwrap();
            assert!(!state.dirty, "{}", history.display());
        }
        fs::write(root.join("other.txt"), "x").unwrap();
        let state = GitState::read(&root, &root.join("../x.jsonl")).unwrap();
        fs::remove_dir_all(&base).unwrap();
        assert!(state.dirty);
    }
}
