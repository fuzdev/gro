//! `kill_guarded_groups_on_signal` from inside a child process: the child
//! installs the handler, spawns two groups, guards one and releases the
//! other's guard, and reports their ids; the parent interrupts it, as a
//! terminal's Ctrl-C would, and checks the child died by the interrupt, the
//! guarded group with it, and the released group lived. A handled signal
//! never reaches this process, nor libtest's harness, which it would end.

#![cfg_attr(
    target_os = "linux",
    expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")
)]

#[cfg(target_os = "linux")]
mod support;

fn main() {
    #[cfg(target_os = "linux")]
    linux::main();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::Write as _;
    use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::support;

    /// The case's name, which the bounded waits fail under.
    const CASE: &str = "a_guarded_group_dies_with_its_interrupted_parent";

    /// How long a killed group's process may take to be gone.
    const DEADLINE: Duration = Duration::from_secs(10);

    pub fn main() {
        support::dispatch(parent, |_| child());
    }

    /// Interrupts the child once it reports its groups, and checks which
    /// survived.
    fn parent() {
        let exe = std::env::current_exe().unwrap();
        let mut child = Command::new(&exe)
            .arg("child")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let line = support::read_line_bounded(CASE, &mut child);
        let ids: Vec<u32> = line
            .split_whitespace()
            .map(|id| id.parse().unwrap())
            .collect();
        assert_eq!(ids.len(), 2, "the child reported {line:?}");
        let (guarded, released) = (ids[0], ids[1]);
        signal("-INT", &child.id().to_string());
        let status = support::wait_bounded(CASE, &mut child);
        assert_eq!(status.signal(), Some(libc::SIGINT), "{status}");
        let gone_by = Instant::now() + DEADLINE;
        while is_running(guarded) {
            assert!(
                Instant::now() < gone_by,
                "the guarded group's leader {guarded} outlived its parent"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            is_running(released),
            "the released group's leader {released} was killed"
        );
        signal("-KILL", &format!("-{released}"));
        println!("test {CASE} ... ok");
    }

    /// Sends `signal` to `target` by the `kill` utility, since the test
    /// itself has no `unsafe`.
    fn signal(signal: &str, target: &str) {
        let status = Command::new("kill")
            .args([signal, "--", target])
            .status()
            .unwrap();
        assert!(status.success(), "kill {signal} {target}: {status}");
    }

    /// Whether `pid` is a process that hasn't exited: present in `/proc`
    /// and no zombie, which the reparented orphan is until its new parent
    /// reaps it.
    fn is_running(pid: u32) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z'))
        })
    }

    /// Spawns `sleep` as the leader of a group of its own, returning its id;
    /// the child never reaps it, its parent's job once the child is gone.
    fn spawn_group() -> u32 {
        Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap()
            .id()
    }

    /// Installs the handler, guards one group, releases the other's guard,
    /// reports both, and waits on stdin for the parent's interrupt.
    fn child() {
        gro_sys::kill_guarded_groups_on_signal().unwrap();
        let guarded = spawn_group();
        let _guard = gro_sys::guard_group(guarded).unwrap();
        let released = spawn_group();
        drop(gro_sys::guard_group(released).unwrap());
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{guarded} {released}").unwrap();
        stdout.flush().unwrap();
        drop(stdout);
        // the parent never writes, so this returns only should the
        // interrupt not end the process
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        unreachable!("the interrupt didn't end the child");
    }
}
