//! `close_inherited_fds` and `cloexec_inherited_fds` from inside a child
//! process: each case runs this binary again, the child, which reports what
//! it then holds, and what a program it starts holds; the parent reads its
//! output. Closing descriptors by number is only sound in a process where
//! nothing owns them, so it never happens in this one, nor under libtest's
//! harness, whose threads and handles would be closed under it.
//!
//! `bash` starts the child with a descriptor above stderr it inherits, as a
//! shell that leaks one would.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]

mod support;

use std::os::fd::IntoRawFd as _;
use std::path::Path;
use std::process::Command;

/// The descriptor the shell leaves open for the child.
const INHERITED: i32 = 7;

fn main() {
    support::dispatch(parent, child);
}

/// Runs each case in a child and checks what it reports.
fn parent() {
    let exe = std::env::current_exe().unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    for (case, expected) in [
        (
            "closes",
            "before: inherited, own\nclosed: ok\nafter: none, none\nabove stderr: 1\n",
        ),
        (
            "cloexec",
            "before: inherited, own\nmarked: ok\nafter: inherited, own\nits child holds: none\n",
        ),
        (
            "threads",
            "before: inherited, own\nrefused: the process has 2 threads, not 1, so a \
             descriptor may belong to another\nafter: inherited, own\n",
        ),
    ] {
        let mut command = Command::new("bash");
        command
            .args([
                "-c",
                &format!(r#"exec {INHERITED}<"$1"; exec "$2" child "$3""#),
                "bash",
            ])
            .arg(&manifest)
            .arg(&exe)
            .arg(case);
        support::run_case(case, command, expected);
    }
}

/// Whether the descriptor `fd` is open in this process.
fn is_open(fd: i32) -> bool {
    Path::new(&format!("/proc/self/fd/{fd}")).exists()
}

/// What this process holds of the inherited descriptor and `own`.
fn holds(own: i32) -> String {
    format!(
        "{}, {}",
        if is_open(INHERITED) {
            "inherited"
        } else {
            "none"
        },
        if is_open(own) { "own" } else { "none" }
    )
}

/// Runs `case` in this process, the child.
fn child(case: &str) {
    // a descriptor of its own, its ownership given up, so closing it by
    // number breaks nothing
    let own = std::fs::File::open(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .into_raw_fd();
    println!("before: {}", holds(own));
    match case {
        "closes" => {
            gro_sys::close_inherited_fds().unwrap();
            println!("closed: ok");
            println!("after: {}", holds(own));
            // the listing's own handle, the only one left
            let above = std::fs::read_dir("/proc/self/fd")
                .unwrap()
                .filter(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_str()
                        .and_then(|name| name.parse::<i32>().ok())
                        .is_some_and(|fd| fd > 2)
                })
                .count();
            println!("above stderr: {above}");
        }
        "cloexec" => {
            gro_sys::cloexec_inherited_fds().unwrap();
            println!("marked: ok");
            println!("after: {}", holds(own));
            // what a program started now holds above stderr, its own
            // shell's descriptors aside, which it opens past the ones tried
            let output = Command::new("sh")
                .args([
                    "-c",
                    &format!(
                        "for fd in 3 4 5 6 {INHERITED} {own}; do \
                         [ -e /proc/self/fd/$fd ] && printf '%s ' $fd; done; true"
                    ),
                ])
                .output()
                .unwrap();
            let held = String::from_utf8(output.stdout).unwrap();
            let held = held.trim();
            println!(
                "its child holds: {}",
                if held.is_empty() { "none" } else { held }
            );
        }
        "threads" => {
            let (stop, wait) = std::sync::mpsc::channel::<()>();
            let other = std::thread::spawn(move || wait.recv());
            let refused = gro_sys::close_inherited_fds().unwrap_err();
            println!("refused: {refused}");
            stop.send(()).unwrap();
            other.join().unwrap().unwrap();
            println!("after: {}", holds(own));
        }
        _ => unreachable!("no case {case}"),
    }
}
