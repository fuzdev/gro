//! Materializing a corpora collection: a fresh copy of one collection of the
//! pinned tree, written from git's objects, for workloads that run inside the
//! collection's own root.
//!
//! A workload naming a collection runs an entry script at the copy's root, so
//! its relative imports stay inside that root, the way a project imports its
//! own files. A runtime that confines relative imports to the importer's root
//! refuses an entry outside the collection that imports into it, and the
//! corpora checkout holds verbatim upstream bytes, so the entry can't live
//! there.
//!
//! The copy's files are the blobs of the collection's tree, read with
//! `git cat-file --batch` and written as they are, so the bytes are the
//! pinned tree's by construction: no checkout runs, so no attribute, filter,
//! or line-ending setting can rewrite them (`git archive` of the subtree
//! misses the corpora's root `.gitattributes`, and a checkout reads
//! attributes from the work tree and the user's own git config). Regular and
//! executable files are supported; a symlink or submodule in the collection,
//! or a name that isn't UTF-8, is refused.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::BenchError;
use crate::record::{git, git_output};

/// One file of a collection's tree.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeFile {
    /// Its path in the collection.
    path: PathBuf,
    /// Its blob's object id.
    oid: String,
    /// Whether its mode is executable (`100755`).
    executable: bool,
}

/// Writes the collection `name` of the corpora tree `tree`, read from the
/// repository of the checkout at `corpora`, into `dest`, after removing
/// anything already there, and returns the paths of the files written.
///
/// # Errors
///
/// Fails with [`BenchError::CollectionNotFound`] when the tree has no
/// directory `name`, [`BenchError::CollectionEntry`] when the collection
/// holds anything but regular and executable files, and otherwise when `git`
/// or a write fails.
pub fn materialize(
    corpora: &Path,
    tree: &str,
    name: &str,
    dest: &Path,
) -> Result<Vec<PathBuf>, BenchError> {
    let not_found = || BenchError::CollectionNotFound {
        corpora: corpora.to_owned(),
        tree: tree.to_owned(),
        name: name.to_owned(),
    };
    // `<tree>:<path>` names the entry; its type says whether it's a directory
    let collection = git(
        corpora,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{tree}:{name}"),
        ],
    )
    .map_err(|error| match error {
        BenchError::CommandFailed { .. } => not_found(),
        other => other,
    })?;
    let collection = collection.trim();
    if git(corpora, &["cat-file", "-t", collection])?.trim() != "tree" {
        return Err(not_found());
    }
    let listing = git_output(corpora, &["ls-tree", "-r", "-z", collection])?;
    let files = parse_tree(&listing).map_err(|(path, mode)| BenchError::CollectionEntry {
        name: name.to_owned(),
        path,
        mode,
    })?;
    remove_existing(dest)?;
    let write_error = |path: &Path| {
        let path = path.to_owned();
        move |source| BenchError::Write { path, source }
    };
    fs::create_dir_all(dest).map_err(write_error(dest))?;
    let blobs = read_blobs(corpora, &files)?;
    let mut written = Vec::with_capacity(files.len());
    for (file, bytes) in files.iter().zip(blobs) {
        let path = dest.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(write_error(parent))?;
        }
        fs::write(&path, bytes).map_err(write_error(&path))?;
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                .map_err(write_error(&path))?;
        }
        written.push(path);
    }
    Ok(written)
}

/// Removes `path`, a directory or anything else, when it exists, so nothing
/// stale survives into a fresh copy.
fn remove_existing(path: &Path) -> Result<(), BenchError> {
    let removed = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    removed.map_err(|source| BenchError::Write {
        path: path.to_owned(),
        source,
    })
}

/// Parses `git ls-tree -r -z` output into the tree's files, or returns the
/// path and mode of the first entry that isn't a regular or executable file,
/// or whose path isn't UTF-8 or isn't plain relative segments.
fn parse_tree(listing: &[u8]) -> Result<Vec<TreeFile>, (PathBuf, String)> {
    let mut files = Vec::new();
    for entry in listing
        .split(|&byte| byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        // `<mode> SP <type> SP <object> TAB <path>`
        let mut parts = entry.splitn(2, |&byte| byte == b'\t');
        let (meta, path) = (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        );
        let meta = String::from_utf8_lossy(meta);
        let mut fields = meta.split(' ');
        let (mode, kind, oid) = (
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
        );
        // decoded strictly, so a name is never written as another
        let Ok(path) = std::str::from_utf8(path) else {
            return Err((
                PathBuf::from(String::from_utf8_lossy(path).into_owned()),
                mode.to_owned(),
            ));
        };
        let path = PathBuf::from(path);
        let plain = path.components().next().is_some()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_)));
        let executable = match (mode, kind) {
            ("100644", "blob") => false,
            ("100755", "blob") => true,
            _ => return Err((path, mode.to_owned())),
        };
        if !plain {
            return Err((path, mode.to_owned()));
        }
        files.push(TreeFile {
            path,
            oid: oid.to_owned(),
            executable,
        });
    }
    Ok(files)
}

/// Reads each file's blob with one `git cat-file --batch`, in order.
fn read_blobs(corpora: &Path, files: &[TreeFile]) -> Result<Vec<Vec<u8>>, BenchError> {
    let command = format!("git -C {} cat-file --batch", corpora.display());
    let failed = |source| BenchError::Spawn {
        command: command.clone(),
        source,
    };
    let mut child = Command::new("git")
        .arg("-C")
        .arg(corpora)
        .args(["cat-file", "--batch"])
        // a local `git replace` would change the bytes behind a matching tree id
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(failed)?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(failed(io::Error::other("git's pipes weren't opened")));
    };
    let mut requests = String::new();
    for file in files {
        requests.push_str(&file.oid);
        requests.push('\n');
    }
    // written from a thread, since git's output can fill its pipe before the input is all read
    let read = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(requests.as_bytes()));
        let mut reader = BufReader::new(stdout);
        let blobs = files
            .iter()
            .map(|file| read_blob(&mut reader, &file.oid))
            .collect::<io::Result<Vec<_>>>();
        // closed before the join, so a git blocked writing to it exits and
        // stops blocking the writer, when the reading stopped early
        drop(reader);
        let written = writer
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        blobs.and_then(|blobs| written.map(|()| blobs))
    });
    let status = child.wait().map_err(failed)?;
    match read {
        // a response git wrote that isn't the blob asked for, such as `<oid> missing`
        Err(error) if error.kind() == io::ErrorKind::InvalidData => Err(failed(error)),
        // a short read means git stopped early, which its exit status says better
        _ if !status.success() => Err(BenchError::CommandFailed { command, status }),
        read => read.map_err(failed),
    }
}

/// Reads one `git cat-file --batch` response, `<oid> SP <type> SP <size> LF
/// <contents> LF`, for the blob `oid`.
fn read_blob(reader: &mut impl BufRead, oid: &str) -> io::Result<Vec<u8>> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    let mut header = String::new();
    // no response at all means git stopped, which its exit status says better
    if reader.read_line(&mut header)? == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let mut fields = header.trim_end().split(' ');
    let size = match (fields.next(), fields.next(), fields.next()) {
        (Some(found), Some("blob"), Some(size)) if found == oid => size
            .parse::<usize>()
            .map_err(|_| invalid(format!("a bad blob size in `{}`", header.trim_end())))?,
        _ => {
            return Err(invalid(format!(
                "expected the blob {oid}, got `{}`",
                header.trim_end()
            )));
        }
    };
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    let mut end = [0; 1];
    reader.read_exact(&mut end)?;
    if end != *b"\n" {
        return Err(invalid(format!(
            "the blob {oid} isn't followed by a newline"
        )));
    }
    Ok(bytes)
}

/// Every path under `dir` (files, directories, and links, which aren't
/// followed), so a run can tell whether anything was added or removed.
///
/// # Errors
///
/// Fails when a directory can't be read.
pub fn list_paths(dir: &Path) -> Result<BTreeSet<PathBuf>, BenchError> {
    let mut paths = BTreeSet::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(current) = pending.pop() {
        let read_error = |source| BenchError::Read {
            path: current.clone(),
            source,
        };
        for entry in fs::read_dir(&current).map_err(read_error)? {
            let entry = entry.map_err(read_error)?;
            let path = entry.path();
            if entry.file_type().map_err(read_error)?.is_dir() {
                pending.push(path.clone());
            }
            paths.insert(path);
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `git` in `root` with a throwaway identity and no conversions.
    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A corpora checkout at `<temp>/<name>/corpora` holding `files` under
    /// `collections/`, committed, and its `collections/` tree id.
    fn corpora(name: &str, files: &[(&str, &[u8])]) -> (PathBuf, PathBuf, String) {
        let base = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
        let root = base.join("corpora");
        fs::create_dir_all(&root).unwrap();
        run_git(&root, &["init", "--quiet"]);
        for (path, bytes) in files {
            let path = root.join("collections").join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "--quiet", "-m", "init"]);
        let tree = git(&root, &["rev-parse", "HEAD:collections"]).unwrap();
        (base, root, tree.trim().to_owned())
    }

    #[test]
    fn materializes_the_tree_bytes_and_wipes_a_stale_copy() {
        // bytes a conversion would rewrite: CRLF, a lone CR, and invalid UTF-8
        let raw: &[u8] = b"a\r\nb\rc\xff\n";
        let (base, root, tree) = corpora(
            "gro_bench_materialize",
            &[
                ("c/package.json", b"{\"type\": \"module\"}\n"),
                ("c/src/lib/a.ts", raw),
                ("c/src/lib/deep/b.ts", b""),
                ("other/x.ts", b"export {};\n"),
            ],
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = root.join("collections/c/run.sh");
            fs::write(&script, "true\n").unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "--quiet", "-m", "exec"]);
        let tree_with_exec = git(&root, &["rev-parse", "HEAD:collections"]).unwrap();
        let tree_with_exec = tree_with_exec.trim();
        let dest = base.join("copy/c");
        // a stale copy, with a file the tree doesn't have
        fs::create_dir_all(dest.join("stale")).unwrap();
        fs::write(dest.join("stale/old.ts"), "").unwrap();
        fs::write(dest.join("src"), "a file where the tree has a directory").unwrap();
        let written = materialize(&root, tree_with_exec, "c", &dest).unwrap();
        let listed = list_paths(&dest).unwrap();
        let read = |path: &str| fs::read(dest.join(path)).unwrap();
        let (a, deep, package) = (
            read("src/lib/a.ts"),
            read("src/lib/deep/b.ts"),
            read("package.json"),
        );
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(dest.join("run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        // the earlier tree, without the script, replaces this copy wholesale
        materialize(&root, &tree, "c", &dest).unwrap();
        let replaced = list_paths(&dest).unwrap();
        let missing = materialize(&root, &tree, "nope", &base.join("copy/nope"));
        let not_a_dir = materialize(&root, &tree, "c/package.json", &base.join("copy/x"));
        fs::remove_dir_all(&base).unwrap();

        assert_eq!(a, raw);
        assert!(deep.is_empty());
        assert_eq!(package, b"{\"type\": \"module\"}\n");
        #[cfg(unix)]
        assert_eq!(mode, 0o755);
        let relative = |paths: &BTreeSet<PathBuf>| -> Vec<String> {
            paths
                .iter()
                .map(|path| path.strip_prefix(&dest).unwrap().display().to_string())
                .collect()
        };
        assert_eq!(
            relative(&listed),
            [
                "package.json",
                "run.sh",
                "src",
                "src/lib",
                "src/lib/a.ts",
                "src/lib/deep",
                "src/lib/deep/b.ts",
            ]
        );
        assert_eq!(written.len(), 4);
        assert!(written.iter().all(|path| listed.contains(path)));
        assert_eq!(
            relative(&replaced),
            [
                "package.json",
                "src",
                "src/lib",
                "src/lib/a.ts",
                "src/lib/deep",
                "src/lib/deep/b.ts",
            ]
        );
        assert!(matches!(
            missing,
            Err(BenchError::CollectionNotFound { ref name, .. }) if name == "nope"
        ));
        assert!(matches!(
            not_a_dir,
            Err(BenchError::CollectionNotFound { .. })
        ));
    }

    #[test]
    fn parse_tree_refuses_links_submodules_and_non_utf8_names() {
        let oid = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
        assert_eq!(
            parse_tree(format!("100644 blob {oid}\ta b/c.ts\0100755 blob {oid}\tx\0").as_bytes()),
            Ok(vec![
                TreeFile {
                    path: PathBuf::from("a b/c.ts"),
                    oid: oid.to_owned(),
                    executable: false,
                },
                TreeFile {
                    path: PathBuf::from("x"),
                    oid: oid.to_owned(),
                    executable: true,
                },
            ])
        );
        for (entry, mode) in [
            (format!("120000 blob {oid}\tlink"), "120000"),
            (format!("160000 commit {oid}\tsub"), "160000"),
            (format!("100644 blob {oid}\t../up"), "100644"),
            (format!("100644 blob {oid}\t/abs"), "100644"),
        ] {
            let (_, found) = parse_tree(format!("{entry}\0").as_bytes()).unwrap_err();
            assert_eq!(found, mode, "{entry}");
        }
        // a name that isn't UTF-8 is refused, not written as another
        let mut entry = format!("100644 blob {oid}\ta").into_bytes();
        entry.extend(b"\xff.ts\0");
        let (path, mode) = parse_tree(&entry).unwrap_err();
        assert_eq!(
            (path, mode.as_str()),
            (PathBuf::from("a\u{fffd}.ts"), "100644")
        );
    }

    #[cfg(unix)]
    #[test]
    fn materialize_refuses_a_name_that_isnt_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let (base, root, _) = corpora("gro_bench_non_utf8", &[("c/a.ts", b"")]);
        let name = std::ffi::OsStr::from_bytes(b"b\xff.ts");
        fs::write(root.join("collections/c").join(name), "").unwrap();
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "--quiet", "-m", "non-utf8"]);
        let tree = git(&root, &["rev-parse", "HEAD:collections"]).unwrap();
        let result = materialize(&root, tree.trim(), "c", &base.join("copy/c"));
        fs::remove_dir_all(&base).unwrap();
        assert!(
            matches!(result, Err(BenchError::CollectionEntry { ref path, .. }) if path == Path::new("b\u{fffd}.ts")),
            "{result:?}"
        );
    }

    #[test]
    fn read_blobs_stops_git_after_a_missing_blob() {
        // the requests and git's responses each overflow a pipe's buffer, so
        // git blocks unless the reader's end closes once the reading stops
        let blob = vec![b'x'; 1024];
        let (base, root, tree) = corpora("gro_bench_read_blobs", &[("c/x.ts", &blob)]);
        let listing = git_output(&root, &["ls-tree", "-r", "-z", &tree]).unwrap();
        let file = parse_tree(&listing).unwrap().remove(0);
        let missing = TreeFile {
            oid: "0".repeat(40),
            ..file.clone()
        };
        let files: Vec<TreeFile> = std::iter::once(missing)
            .chain(std::iter::repeat_n(file, 10_000))
            .collect();
        let (send, receive) = std::sync::mpsc::channel();
        // a thread, so a hang fails the test instead of stalling it
        std::thread::spawn(move || send.send(read_blobs(&root, &files)).unwrap());
        let result = receive.recv_timeout(std::time::Duration::from_secs(60));
        fs::remove_dir_all(&base).unwrap();
        let read = result.expect("read_blobs hung after the missing blob");
        assert!(
            matches!(read, Err(BenchError::Spawn { ref source, .. }) if source.kind() == io::ErrorKind::InvalidData),
            "{read:?}"
        );
    }

    #[test]
    fn read_blob_checks_the_response() {
        let oid = "abc";
        let mut ok = &b"abc blob 3\nx\ny\n"[..];
        assert_eq!(read_blob(&mut ok, oid).unwrap(), b"x\ny");
        let mut empty = &b""[..];
        assert_eq!(
            read_blob(&mut empty, oid).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        for bad in [
            &b"abc missing\n"[..],
            b"abd blob 1\nx\n",
            b"abc tree 1\nx\n",
            b"abc blob 1\nxy",
            b"abc blob 2\nx",
        ] {
            let mut reader = bad;
            assert!(read_blob(&mut reader, oid).is_err(), "{bad:?}");
        }
    }
}
