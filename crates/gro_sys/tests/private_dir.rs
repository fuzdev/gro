//! `make_private_dir` from inside a child process started under a umask of
//! its own: each case runs this binary again, the child, which makes its
//! private directory in a scratch directory, or is refused one where an
//! entry is planted at its name, and reports what it then finds, the name
//! read from `/proc/self/stat` here, not by the crate. A umask is the whole
//! process's, so none is set in this one, nor under libtest's harness.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]
#![expect(clippy::panic, reason = "a test fails by panicking")]

mod support;

use std::fs::{self, File};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::os::unix::fs::{MetadataExt as _, symlink};
use std::path::PathBuf;
use std::process::Command;

use gro_sys::private_dir::{self, PrivateDirError};

fn main() {
    support::dispatch(parent, child);
}

/// Runs each case in a child under its umask and checks what it reports.
fn parent() {
    let exe = std::env::current_exe().unwrap();
    for (case, umask, expected) in [
        (
            "makes",
            "022",
            "name: gro-<pid>-<start>\nkind: directory\nmode: 700\nowner: the process's\nlocked: true\nagain: \
             exists\n",
        ),
        // a umask taking the owner's own bits off, set back
        (
            "makes",
            "777",
            "name: gro-<pid>-<start>\nkind: directory\nmode: 700\nowner: the process's\nlocked: true\nagain: exists\n",
        ),
        (
            "symlink",
            "022",
            "refused: exists\nthe link: kept\nits target: empty\n",
        ),
        (
            "dangling",
            "022",
            "refused: exists\nthe link: kept\nits target: missing\n",
        ),
        ("file", "022", "refused: exists\nthe file: kept\n"),
        (
            "directory",
            "022",
            "refused: exists\nthe directory: empty\n",
        ),
        (
            "sweep",
            "022",
            "removed: gro-4194305-1, gro-<pid>-<start + 1>\nfailed: none\nleft: gro-4194306-1, \
             gro-4194307-1, gro-4194308-1, gro-4194310-1, gro-42x, gro-<pid>-<start>\nthe inner link's target: kept\nthe \
             planted link's target: kept\n",
        ),
    ] {
        // made here, under this process's umask, and removed here after
        let scratch = std::env::temp_dir().join(format!(
            "gro_sys_private_dir_{}_{case}_{umask}",
            std::process::id()
        ));
        fs::create_dir_all(scratch.join("parent")).unwrap();
        fs::create_dir_all(scratch.join("outside")).unwrap();
        let mut command = Command::new("sh");
        command
            .args(["-c", r#"umask "$1"; exec "$2" child "$3""#, "sh", umask])
            .arg(&exe)
            .arg(case)
            .env("SCRATCH", &scratch);
        support::run_case(&format!("{case} (umask {umask})"), command, expected);
        fs::remove_dir_all(&scratch).unwrap();
    }
}

/// Runs `case` in this process, the child, in the scratch directory the
/// parent made.
fn child(case: &str) {
    let scratch = PathBuf::from(std::env::var_os("SCRATCH").unwrap());
    let parent: OwnedFd = File::open(scratch.join("parent")).unwrap().into();
    let name = expected_name();
    let at = scratch.join("parent").join(&name);
    match case {
        "makes" => {
            let made = private_dir::make_private_dir(parent.as_fd()).unwrap();
            assert_eq!(made.name(), name);
            println!("name: gro-<pid>-<start>");
            // the descriptor is the entry at the name
            let held = File::from(made.dir().try_clone_to_owned().unwrap())
                .metadata()
                .unwrap();
            let entry = fs::symlink_metadata(&at).unwrap();
            assert_eq!((held.dev(), held.ino()), (entry.dev(), entry.ino()));
            println!(
                "kind: {}",
                if entry.is_dir() { "directory" } else { "other" }
            );
            println!("mode: {:o}", entry.mode() & 0o7777);
            println!(
                "owner: {}",
                if entry.uid() == effective_uid() {
                    "the process's"
                } else {
                    "another"
                }
            );
            // its shared lock held: an exclusive one, a sweep's, waits
            let other = File::open(&at).unwrap();
            println!(
                "locked: {}",
                matches!(other.try_lock(), Err(fs::TryLockError::WouldBlock))
            );
            print_refused("again", private_dir::make_private_dir(parent.as_fd()));
        }
        "symlink" | "dangling" => {
            let target = scratch.join("outside").join("target");
            if case == "symlink" {
                fs::create_dir(&target).unwrap();
            }
            symlink(&target, &at).unwrap();
            print_refused("refused", private_dir::make_private_dir(parent.as_fd()));
            println!(
                "the link: {}",
                if fs::read_link(&at).unwrap() == target {
                    "kept"
                } else {
                    "changed"
                }
            );
            println!(
                "its target: {}",
                match fs::read_dir(&target).map(|mut entries| entries.next().is_none()) {
                    Ok(true) => "empty",
                    Ok(false) => "written",
                    Err(_) => "missing",
                }
            );
        }
        "file" => {
            fs::write(&at, "planted").unwrap();
            print_refused("refused", private_dir::make_private_dir(parent.as_fd()));
            println!(
                "the file: {}",
                if fs::read_to_string(&at).unwrap() == "planted" {
                    "kept"
                } else {
                    "changed"
                }
            );
        }
        "directory" => {
            fs::create_dir(&at).unwrap();
            print_refused("refused", private_dir::make_private_dir(parent.as_fd()));
            println!(
                "the directory: {}",
                if fs::read_dir(&at).unwrap().next().is_none() {
                    "empty"
                } else {
                    "written"
                }
            );
        }
        "sweep" => sweep(&scratch, parent.as_fd(), &name),
        _ => unreachable!("no case {case}"),
    }
}

/// Sweeps a parent holding this process's own directory, kept; one named
/// for a process id no kernel gives (above 2 ** 22) with a symlink inside
/// to a file outside, and one named for this process's id with another
/// start time, a reused id, both removed, the file kept; and a name of
/// another shape, a symlink at a gone process's name to a directory
/// outside and one to a directory beside it, a file at one, and a
/// directory at one that a live process holds locked, all left.
fn sweep(scratch: &std::path::Path, parent: BorrowedFd<'_>, name: &str) {
    let at = |entry: &str| scratch.join("parent").join(entry);
    let (prefix, start) = name.rsplit_once('-').unwrap();
    let reused = format!("{prefix}-{}", start.parse::<u64>().unwrap() + 1);
    for dir in [name, "gro-4194305-1", reused.as_str(), "gro-42x"] {
        fs::create_dir(at(dir)).unwrap();
    }
    let target = scratch.join("outside").join("kept.txt");
    fs::write(&target, "kept").unwrap();
    symlink(&target, at("gro-4194305-1").join("link")).unwrap();
    fs::create_dir(scratch.join("outside").join("dir")).unwrap();
    fs::write(scratch.join("outside").join("dir").join("kept.txt"), "kept").unwrap();
    symlink(scratch.join("outside").join("dir"), at("gro-4194306-1")).unwrap();
    // a link inside the parent, which a lookup beneath it would follow
    symlink("gro-42x", at("gro-4194308-1")).unwrap();
    fs::write(at("gro-4194307-1"), "kept").unwrap();
    // a live run that `/proc` doesn't show (hidden by `hidepid`, or in
    // another pid namespace), named for a gone id, holding its lock
    fs::create_dir(at("gro-4194310-1")).unwrap();
    let held = File::open(at("gro-4194310-1")).unwrap();
    held.lock_shared().unwrap();
    let swept = private_dir::sweep_private_dirs(parent).unwrap();
    drop(held);
    let shown = |entry: &str| {
        entry
            .replace(name, "gro-<pid>-<start>")
            .replace(&reused, "gro-<pid>-<start + 1>")
    };
    let mut removed: Vec<String> = swept.removed.iter().map(|entry| shown(entry)).collect();
    removed.sort();
    println!("removed: {}", removed.join(", "));
    println!(
        "failed: {}",
        if swept.failed.is_empty() {
            "none".to_owned()
        } else {
            format!("{:?}", swept.failed)
        }
    );
    let mut left: Vec<String> = fs::read_dir(scratch.join("parent"))
        .unwrap()
        .map(|entry| shown(&entry.unwrap().file_name().to_string_lossy()))
        .collect();
    left.sort();
    println!("left: {}", left.join(", "));
    let kept = |path: PathBuf| {
        if fs::read_to_string(path).is_ok_and(|text| text == "kept") {
            "kept"
        } else {
            "changed"
        }
    };
    println!("the inner link's target: {}", kept(target));
    println!(
        "the planted link's target: {}",
        kept(scratch.join("outside").join("dir").join("kept.txt"))
    );
}

/// Prints `label` and that the make was refused for an entry at the name;
/// anything else fails.
fn print_refused(label: &str, result: Result<private_dir::PrivateDir, PrivateDirError>) {
    match result {
        Err(PrivateDirError::Exists { .. }) => println!("{label}: exists"),
        other => panic!("{label}: not refused as existing: {other:?}"),
    }
}

/// `gro-<pid>-<start>`, the start the 22nd field of `/proc/self/stat`.
fn expected_name() -> String {
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    let after_name = &stat[stat.rfind(')').unwrap() + 1..];
    let start = after_name.split_whitespace().nth(19).unwrap();
    format!("gro-{}-{start}", std::process::id())
}

/// The process's effective user id, from `/proc/self/status`.
fn effective_uid() -> u32 {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let ids = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .unwrap();
    ids.split_whitespace().nth(1).unwrap().parse().unwrap()
}
