//! `gro_sys::child` from inside a child process, which starts with
//! `SIGCHLD` ignored, as a parent that ignores it hands it on through
//! `exec`: it sets the default back, then spawns programs and holds each by
//! its pidfd, reporting how each ended, read or waited for without reaping
//! and then reaped, its exit seen on a tokio runtime's reactor, and a signal sent
//! through the pidfd before and after the reap. Changing the process's
//! signal disposition never happens in this one, nor under libtest's
//! harness.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]

mod support;

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use gro_sys::child::{Ended, Pidfd, default_child_signal};

/// The case's name.
const CASE: &str = "a_child_is_held_waited_for_and_reaped_by_its_pidfd";

/// What the child reports.
const EXPECTED: &str = "\
exited 0, read twice, a zombie: true
reaped: Exited(0), gone: true
exited 3: Exited(3)
waited for, unreaped: Exited(6), a zombie: true, then reaped: Exited(6)
running: None
killed: Killed { signal: 9, dumped: false }
signaled after the reap: no such process
";

fn main() {
    support::dispatch(parent, |_| child());
}

/// Runs the child with `SIGCHLD` ignored and checks what it reports.
fn parent() {
    let mut command = Command::new("env");
    command
        .arg("--ignore-signal=CHLD")
        .arg(std::env::current_exe().unwrap())
        .arg("child");
    support::run_case(CASE, command, EXPECTED);
}

/// Spawns `program` with `args`, its streams null, returning its pid and
/// its pidfd.
fn spawn(program: &str, args: &[&str]) -> (u32, Arc<Pidfd>) {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    // the child is never waited for by std: its pidfd reaps it
    drop(child);
    (pid, Arc::new(Pidfd::open(pid).unwrap()))
}

/// Whether `pid` is a zombie, exited and not reaped.
fn is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
    })
}

/// Waits on a current-thread tokio runtime's reactor until `pidfd`'s
/// child has exited.
fn await_exit(pidfd: &Arc<Pidfd>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let watch = pidfd.watch().unwrap();
        std::future::poll_fn(|cx| watch.poll_exited(cx))
            .await
            .unwrap();
        // ready again once it was, its readiness kept
        std::future::poll_fn(|cx| watch.poll_exited(cx))
            .await
            .unwrap();
    });
}

/// Sets `SIGCHLD`'s default back, then holds each program by its pidfd.
fn child() {
    default_child_signal().unwrap();
    let (pid, pidfd) = spawn("true", &[]);
    await_exit(&pidfd);
    let first = pidfd.ended().unwrap();
    let second = pidfd.ended().unwrap();
    assert_eq!(first, second);
    assert_eq!(first, Some(Ended::Exited(0)));
    println!("exited 0, read twice, a zombie: {}", is_zombie(pid));
    let reaped = pidfd.reap().unwrap();
    println!(
        "reaped: {reaped:?}, gone: {}",
        !Path::new(&format!("/proc/{pid}")).exists()
    );

    let (_, pidfd) = spawn("sh", &["-c", "exit 3"]);
    await_exit(&pidfd);
    println!("exited 3: {:?}", pidfd.reap().unwrap());

    let (pid, pidfd) = spawn("sh", &["-c", "sleep 0.1; exit 6"]);
    let waited = pidfd.wait_ended().unwrap();
    println!(
        "waited for, unreaped: {waited:?}, a zombie: {}, then reaped: {:?}",
        is_zombie(pid),
        pidfd.reap().unwrap()
    );

    let (_, pidfd) = spawn("sleep", &["60"]);
    println!("running: {:?}", pidfd.ended().unwrap());
    pidfd.send_signal(libc::SIGKILL).unwrap();
    println!("killed: {:?}", pidfd.reap().unwrap());
    let after = pidfd.send_signal(libc::SIGTERM).unwrap_err();
    assert_eq!(after.raw_os_error(), Some(libc::ESRCH), "{after}");
    println!("signaled after the reap: no such process");
}
