//! `guard_children_on_signal` from inside a child process: the child
//! installs the handler, spawns two groups, guards one and releases the
//! other's guard, spawns a third child in its own group and names it for a
//! `SIGTERM`, and reports their ids; the parent interrupts it, as a
//! terminal's Ctrl-C would, or sends it a `SIGTERM`, as a supervisor would,
//! and checks the child died by that signal, the guarded group with it, the
//! released group lived, and the named child was ended by the `SIGTERM`
//! alone. A handled signal never reaches this process, nor libtest's
//! harness, which it would end.
//! A third case signals the child while a thread of its holds a guard
//! reserved and not armed: the handler waits for the arming, during which
//! no guard is reserved, `Unreserved::Ending` refusing it, and the group the
//! thread then spawns and arms is killed with the rest. With the `testing`
//! feature, a fourth has that thread signal itself (`signal_thread`), so the
//! handler runs on it, mid-launch, and hands the signal on to the main
//! thread, which waits for the arming as before.
//! The handler leaves a signal the process inherits ignored as it is, so the
//! child starts under `env --default-signal=INT --default-signal=TERM`, each
//! at its default however the test was launched.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]

mod support;

use std::io::Write as _;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The cases, each by the signal the parent sends, which the bounded waits
/// fail under.
const CASES: [(&str, &str, libc::c_int); 2] = [
    (
        "a_guarded_group_dies_with_its_interrupted_parent",
        "INT",
        libc::SIGINT,
    ),
    (
        "a_named_child_gets_its_parent_s_sigterm",
        "TERM",
        libc::SIGTERM,
    ),
];

/// How long a killed group's process may take to be gone.
const DEADLINE: Duration = Duration::from_secs(10);

/// The case of a launch in flight as the signal arrives.
const IN_FLIGHT: &str = "a_launch_in_flight_is_waited_for_and_no_other_is_made";

/// The case of the signal handled on the thread of the launch in flight.
#[cfg(feature = "testing")]
const HANDED_OFF: &str = "a_signal_on_the_launching_thread_is_handed_to_the_main_thread";

fn main() {
    support::dispatch(
        || {
            parent();
            parent_in_flight(IN_FLIGHT, "in_flight");
            #[cfg(feature = "testing")]
            parent_in_flight(HANDED_OFF, "handed_off");
        },
        |case| match case {
            "in_flight" => child_in_flight(false),
            #[cfg(feature = "testing")]
            "handed_off" => child_in_flight(true),
            _ => child(),
        },
    );
}

/// Runs the child of `case`, `arg` for it, which reports once a thread of
/// its holds a guard reserved, then sends it a `SIGTERM`, unless the
/// thread signals itself, and checks what the thread saw while the
/// handler waited, and that the group it armed then died with the child.
fn parent_in_flight(case: &str, arg: &str) {
    use std::io::Read as _;

    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new("env")
        .args(["--default-signal=INT", "--default-signal=TERM"])
        .arg(&exe)
        .args(["child", arg])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let line = support::read_line_bounded(case, &mut child);
    assert_eq!(line, "reserved\n", "{case}");
    if arg == "in_flight" {
        signal("-TERM", &child.id().to_string());
    }
    let status = support::wait_bounded(case, &mut child);
    assert_eq!(status.signal(), Some(libc::SIGTERM), "{case}: {status}");
    let mut reported = String::new();
    stderr.read_to_string(&mut reported).unwrap();
    let reported = reported.trim_end();
    let (refused, armed) = reported.split_once(' ').unwrap_or((reported, ""));
    assert_eq!(refused, "Ending", "{case}: {reported:?}");
    await_gone(
        case,
        armed.parse().unwrap(),
        "the group armed while the handler waited",
    );
    println!("test {case} ... ok");
}

/// The calling thread's id, from `/proc/thread-self`, which names it as
/// `<pid>/task/<tid>`.
#[cfg(feature = "testing")]
fn thread_id() -> libc::pid_t {
    let link = std::fs::read_link("/proc/thread-self").unwrap();
    link.file_name().unwrap().to_str().unwrap().parse().unwrap()
}

/// Signals the child once it reports its children, by each case's signal,
/// and checks which survived.
fn parent() {
    for (case, name, number) in CASES {
        let exe = std::env::current_exe().unwrap();
        let mut child = Command::new("env")
            .args(["--default-signal=INT", "--default-signal=TERM"])
            .arg(&exe)
            .arg("child")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let line = support::read_line_bounded(case, &mut child);
        let ids: Vec<u32> = line
            .split_whitespace()
            .map(|id| id.parse().unwrap())
            .collect();
        assert_eq!(ids.len(), 3, "{case}: the child reported {line:?}");
        let (guarded, released, named) = (ids[0], ids[1], ids[2]);
        signal(&format!("-{name}"), &child.id().to_string());
        let status = support::wait_bounded(case, &mut child);
        assert_eq!(status.signal(), Some(number), "{case}: {status}");
        await_gone(case, guarded, "the guarded group's leader");
        if number == libc::SIGTERM {
            await_gone(case, named, "the named child");
        } else {
            assert!(
                is_running(named),
                "{case}: the named child {named} was signaled"
            );
            signal("-KILL", &named.to_string());
        }
        assert!(
            is_running(released),
            "{case}: the released group's leader {released} was killed"
        );
        signal("-KILL", &format!("-{released}"));
        println!("test {case} ... ok");
    }
}

/// Waits, bounded, for `pid`, `what`, to be gone.
fn await_gone(case: &str, pid: u32, what: &str) {
    let gone_by = Instant::now() + DEADLINE;
    while is_running(pid) {
        assert!(
            Instant::now() < gone_by,
            "{case}: {what} {pid} outlived its parent"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
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

/// Spawns `sleep`, as the leader of a group of its own when `group` says,
/// returning its id; the child never reaps it, its parent's job once the
/// child is gone.
fn spawn_sleep(group: bool) -> u32 {
    let mut command = Command::new("sleep");
    command.arg("60").stdin(Stdio::null()).stdout(Stdio::null());
    if group {
        command.process_group(0);
    }
    command.spawn().unwrap().id()
}

/// Installs the handler, then, on a thread, reserves a guard and reports
/// it, then, when `self_signaled`, sends itself a `SIGTERM`, whose handler,
/// on a thread holding a reservation, hands it on to the main thread; then
/// reserves and frees another guard until one is refused, which happens
/// once a `SIGTERM`, the parent's or its own, has begun the handler, spawns
/// a group and arms the guard held with it, and reports the refusal and
/// the group on stderr. The main thread waits on stdin, where the handler
/// waits for the arming.
fn child_in_flight(self_signaled: bool) {
    gro_sys::guard_children_on_signal().unwrap();
    std::thread::spawn(move || {
        let held = gro_sys::GroupGuard::reserve().unwrap();
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "reserved").unwrap();
        stdout.flush().unwrap();
        drop(stdout);
        if self_signaled {
            #[cfg(feature = "testing")]
            gro_sys::signal_thread(
                libc::pid_t::try_from(std::process::id()).unwrap(),
                thread_id(),
                libc::SIGTERM,
            )
            .unwrap();
        }
        let refused = loop {
            match gro_sys::GroupGuard::reserve() {
                Ok(again) => drop(again),
                Err(refused) => break refused,
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        let group = Command::new("sleep")
            .arg("60")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap()
            .id();
        let _guard = held.arm(group).unwrap();
        eprintln!("{refused:?} {group}");
        loop {
            std::thread::park();
        }
    });
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    unreachable!("the signal didn't end the child");
}

/// Installs the handler, guards one group, releases the other's guard,
/// names a child sharing its group for a `SIGTERM`, reports all three, and
/// waits on stdin for the parent's signal.
fn child() {
    gro_sys::guard_children_on_signal().unwrap();
    let guarded = spawn_sleep(true);
    let _guard = gro_sys::guard_group(guarded).unwrap();
    let released = spawn_sleep(true);
    drop(gro_sys::guard_group(released).unwrap());
    let forward = gro_sys::TermForward::reserve().unwrap();
    let named = spawn_sleep(false);
    let _forward = forward.arm(named).unwrap();
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{guarded} {released} {named}").unwrap();
    stdout.flush().unwrap();
    drop(stdout);
    // the parent never writes, so this returns only should the signal not
    // end the process
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    unreachable!("the signal didn't end the child");
}
