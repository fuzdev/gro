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
use std::os::fd::{AsFd as _, OwnedFd};
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
            "name: gro-<pid>-<start>\nkind: directory\nmode: 700\nowner: the process's\nagain: \
             exists\n",
        ),
        // a umask taking the owner's own bits off, set back
        (
            "makes",
            "777",
            "name: gro-<pid>-<start>\nkind: directory\nmode: 700\nowner: the process's\nagain: exists\n",
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
        _ => unreachable!("no case {case}"),
    }
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
