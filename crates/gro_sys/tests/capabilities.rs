//! `drop_capabilities` and `set_not_dumpable` from inside a child process:
//! each case runs this binary again, the child, which drops what it holds
//! or marks itself, and reports what it and a program it then starts hold,
//! each read from `/proc` here, not by the crate's reader; the parent reads
//! its output. Both change the whole process, for good, so neither happens
//! in this one, nor under libtest's harness, whose threads would keep
//! their capabilities.
//!
//! The namespace cases need unprivileged user namespaces, as
//! `tests/namespace.rs` does, and the ambient case util-linux's `unshare`
//! with `--keep-caps`.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]

mod support;

use std::process::Command;

use gro_sys::capabilities::{self, BoundingSet, CapabilityError};

fn main() {
    support::dispatch(parent, child);
}

/// Runs each case in a child and checks what it reports.
fn parent() {
    let exe = std::env::current_exe().unwrap();
    let emptied = "process: 0 0 0 0 0\nits child: 0 0 0 0 0\n";
    for (case, expected) in [
        // every set held in the namespace, the bounding set emptied too
        (
            "namespace",
            format!("before: effective held\ndropped: Dropped\n{emptied}"),
        ),
        // a parent's ambient set, emptied with the others
        (
            "ambient",
            format!("before: ambient held\ndropped: Dropped\n{emptied}"),
        ),
        // without `CAP_SETPCAP` the bounding set is kept, the others empty
        (
            "outside",
            "dropped: Kept\nprocess: 0 0 0 kept 0\nits child: 0 0 0 kept 0\n".to_owned(),
        ),
        (
            "threads",
            "refused: the process has 2 threads, not 1, and each holds its own capabilities\n\
             process: unchanged\n"
                .to_owned(),
        ),
        (
            "dumpable",
            "dumpable: true\nits child reads its environment: yes\nmarked: ok\ndumpable: \
             false\nits child reads its environment: no\n"
                .to_owned(),
        ),
    ] {
        let command = if case == "ambient" {
            // a user namespace whose capabilities the child gets as its
            // ambient set, as a parent raising them would hand them down
            let mut command = Command::new("unshare");
            command
                .args(["--user", "--map-current-user", "--keep-caps", "--"])
                .arg(&exe);
            command
        } else {
            Command::new(&exe)
        };
        let mut command = command;
        command.args(["child", case]);
        support::run_case(case, command, &expected);
    }
}

/// Runs `case` in this process, the child.
fn child(case: &str) {
    match case {
        "namespace" => {
            let _entered = support::or_chain(gro_sys::namespace::enter_user_net_namespace());
            assert_ne!(sets(&own_status())[2], 0);
            println!("before: effective held");
            report_drop();
        }
        "ambient" => {
            assert_ne!(sets(&own_status())[4], 0);
            println!("before: ambient held");
            report_drop();
        }
        "outside" => report_drop(),
        "threads" => {
            let before = own_status();
            let (ready, wait) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                ready.send(()).unwrap();
                std::thread::park();
            });
            wait.recv().unwrap();
            let error = capabilities::drop_capabilities().unwrap_err();
            assert!(matches!(error, CapabilityError::Threads { count: 2 }));
            println!("refused: {error}");
            assert_eq!(sets(&own_status()), sets(&before));
            println!("process: unchanged");
        }
        "dumpable" => {
            println!("dumpable: {}", gro_sys::is_dumpable().unwrap());
            println!(
                "its child reads its environment: {}",
                child_reads_environment()
            );
            gro_sys::set_not_dumpable().unwrap();
            println!("marked: ok");
            println!("dumpable: {}", gro_sys::is_dumpable().unwrap());
            println!(
                "its child reads its environment: {}",
                child_reads_environment()
            );
        }
        _ => unreachable!("no case {case}"),
    }
}

/// Drops the capabilities, then prints what became of the bounding set and
/// the sets the process and a program it starts then hold.
fn report_drop() {
    let bounding = support::or_chain(capabilities::drop_capabilities());
    println!("dropped: {bounding:?}");
    let kept = bounding == BoundingSet::Kept;
    println!("process: {}", shown(&own_status(), kept));
    let status = Command::new("cat")
        .arg("/proc/self/status")
        .output()
        .unwrap()
        .stdout;
    println!(
        "its child: {}",
        shown(&String::from_utf8(status).unwrap(), kept)
    );
}

/// The process's `/proc/self/status`.
fn own_status() -> String {
    std::fs::read_to_string("/proc/self/status").unwrap()
}

/// The five sets of a `/proc/<pid>/status` text, in its order: `CapInh`,
/// `CapPrm`, `CapEff`, `CapBnd`, `CapAmb`.
fn sets(status: &str) -> [u64; 5] {
    ["CapInh:", "CapPrm:", "CapEff:", "CapBnd:", "CapAmb:"].map(|name| {
        let value = status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap();
        u64::from_str_radix(value.trim(), 16).unwrap()
    })
}

/// The five sets in hex, a bounding set left as the parent had it, full,
/// shown as `kept` when `kept` is expected.
fn shown(status: &str, kept: bool) -> String {
    let sets = sets(status);
    sets.iter()
        .enumerate()
        .map(|(at, set)| {
            if at == 3 && kept && *set != 0 {
                "kept".to_owned()
            } else {
                format!("{set:x}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a program this process starts, of its user, reads this
/// process's `/proc/<pid>/environ`.
fn child_reads_environment() -> &'static str {
    let output = Command::new("cat")
        .arg(format!("/proc/{}/environ", std::process::id()))
        .output()
        .unwrap();
    if output.status.success() { "yes" } else { "no" }
}
