//! The run's box from inside a process holding it: each case runs in a
//! child, this binary run again with the case's name, which takes the run's
//! steps from its main thread, its only one, as a run takes them
//! (`gro_runtime::confine::run`), with a plan of the test's own, then prints
//! what it could and couldn't do, and what its programs could. Once boxed
//! it prints `inspect` and waits on stdin while the parent reads it from
//! outside (`/proc/<pid>`: its seccomp mode, capabilities, maps, limits,
//! environment, and each of its threads), then reports both.
//!
//! The binary runs without libtest's harness, whose threads would refuse
//! the box. The child starts through `/bin/sh`, which hands it a descriptor
//! on a file outside the box and a soft limit on descriptors below the hard
//! one, so the first step's close and the open-files raise are seen.
//!
//! The network form a case expects is the host's, read by the parent from
//! the settings that refuse user namespaces: the run's own namespace where
//! nothing refuses one, else the host's network with the cause. Each case
//! named `_inet_refused` takes the host's network wherever it runs, under a
//! seccomp filter refusing `unshare` that `python3` installs before it runs
//! the child, so both forms' walls are held on every host. The parent also runs an
//! `unshare` and its maps on the host with no check first, prints what they
//! did, and fails unless that agrees with the form the settings gave
//! (`probe_agrees`), an oracle apart from the settings the box's own check
//! reads, so where user namespaces are restricted the run proves what the
//! check infers.
//!
//! Each step's refusal is held too: another thread at each step that
//! checks for one, and a descriptor held across the box that its walls
//! would miss, which leaves nothing installed: a socket above stderr; on
//! stdin, an unconnected TCP socket, a TCP stream connected to a listener
//! of the parent's (which code holding it could disconnect, then connect
//! elsewhere or listen), a UDP socket, a unix datagram socket, and an
//! `io_uring` instance. A connected unix stream pair on stdout, which
//! reaches only its peer, the parent, is allowed. Before any step, the
//! Landlock floor's probe changes nothing, and a socket on stdin is refused
//! by the stdio check with the process as it started.
//!
//! The plan's cases: the tool directories made once the run is hardened,
//! one planted as a symlink refused with nothing made where it leads; and
//! a run planned as `gen` and as `check` on a project of its own, its box
//! up, reaching what the plan admits (its write set, a tool directory,
//! `/dev/null`, `/proc`, a linked package, git's home config, `/bin/sh` on
//! a merged `/usr`, the `node` on `PATH` under a rule on its file alone)
//! and refused the rest (`.git`, `node_modules`, a new top-level file, the
//! home directory).
//!
//! A program's own box, nested in the run's (`gro_runtime::confine::program`):
//! the run's box up for a run that may start a program, writing its root
//! and naming a write flag, its rules for programs' boxes taken, and two
//! boxes built, a program of the table's and a flag's, each attacking from
//! a program it starts what the other and the run hold (a tool directory,
//! an entry the run writes, what the write flag names, the sibling's
//! temporary directory, the private directory, the run and the sibling by
//! a signal and by `/proc`, a link planted into the sibling's), beside the
//! flag's program alive in its box; then a thread in the flag's box
//! restricting itself by the table's, which widens nothing, and the run,
//! whose own domain is as it was; in the run's own namespace and, under
//! the same filter as the walls' twin, in the host's network, where the
//! run's layer refuses a program's TCP as it refuses the run's. A third
//! takes a umask that strips the owner's own bits, under which a program's
//! temporary directory is its user's alone and written all the same.
//!
//! The cases that start V8 under the box need a snapshot build, so they
//! run from `cargo xtask build --verify`: the one that shows each of its
//! threads in it, and those whose `node_modules` is a symlink into another
//! checkout, holding a project's `package.json`, a type marker, or none,
//! whose packages resolve each other's imports within it and read no file
//! of the checkout holding it. The other cases run in any build. Every
//! case needs Landlock ABI 6, and fails below it, naming the kernel's ABI.

// clippy's test allowance covers `#[test]` functions, and this binary has none
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a test's setup and its failures panic"
)]

mod support;

use std::fmt::Write as _;
use std::io::{BufRead as _, ErrorKind, Write as _};
use std::os::fd::AsFd as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use gro_runtime::confine::Threads;
use gro_runtime::confine::plan::{
    BoxFacts, BoxInputs, BoxPlan, DirFlag, GrantedFlagDir, ProgramFile, RunWriteDir, RunWriteGrant,
};
use gro_runtime::confine::program::{ProgramBox, ProgramBoxError, ProgramWrites};
use gro_runtime::confine::rules::InstalledRule;
use gro_runtime::confine::run::{
    RunClosed, RunHardened, RunNetwork, RunPrepared, check_stdio, probe_floor,
};
use gro_runtime::confine::tool_dirs::ToolDirError;

use support::{DEADLINE, KillOnDrop, Scratch};

/// The error numbers the cases name, the same on every architecture the
/// box is built for.
const EPERM: i32 = 1;
const EACCES: i32 = 13;
const EEXIST: i32 = 17;
const ENOSPC: i32 = 28;
const ENOSYS: i32 = 38;
const ENETUNREACH: i32 = 101;
const ECONNREFUSED: i32 = 111;

/// `CAP_SETPCAP`, which dropping the bounding set takes.
const CAP_SETPCAP: u32 = 8;

/// An address of the documentation range, which nothing serves.
const REMOTE: &str = "192.0.2.1:9";

/// The cases, each run in a child of its own.
const CASES: &[&str] = &[
    "walls",
    "walls_inet_refused",
    "threads_close",
    "threads_network",
    "threads_harden",
    "threads_confine",
    "socket_held",
    "stdin_tcp",
    "stdin_tcp_connected",
    "stdin_udp",
    "stdin_unix_datagram",
    "stdin_ring",
    "stdout_stream",
    "stdin_checked_first",
    "tool_dirs",
    "tool_dirs_planted",
    "planned",
    "planned_check",
    "private_dir_reused",
    "program_box",
    "program_box_inet_refused",
    "program_box_umask",
    #[cfg(feature = "snapshot")]
    "v8",
    #[cfg(feature = "snapshot")]
    "linked_named",
    #[cfg(feature = "snapshot")]
    "linked_marker",
    #[cfg(feature = "snapshot")]
    "linked_absent",
];

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("child") => {
            let case = args.next().unwrap();
            let scratch = PathBuf::from(args.next().unwrap());
            let port: u16 = args.next().unwrap().parse().unwrap();
            child(&case, &scratch, port);
        }
        // libtest's own flags (`--test-threads`, a filter) are ignored
        _ => parent(),
    }
}

/// The scratch directory: a project with a write root, a file and a unix
/// socket outside it.
fn scratch() -> Scratch {
    Scratch::new(
        "run_box",
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/main.js",
                "let sum = 0;\nfor (let i = 0; i < 1e6; i++) sum += i;\nconsole.log('sum', sum);\n",
            ),
            ("project/out/kept.txt", "kept\n"),
            ("outside/secret.txt", "outside\n"),
            ("outside/truncated.txt", "kept\n"),
        ],
    )
}

/// A `/proc/sys` setting's value, trimmed, or `unread`.
fn setting(path: &str) -> String {
    std::fs::read_to_string(path).map_or_else(|_| "unread".to_owned(), |v| v.trim().to_owned())
}

/// A field of `/proc/<pid>/status` (or a task's), its value trimmed.
fn status_field(status: &str, name: &str) -> String {
    status
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .map_or_else(|| format!("no {name}"), |value| value.trim().to_owned())
}

/// Whether the effective set in `status` holds `capability`.
fn holds(status: &str, capability: u32) -> bool {
    u64::from_str_radix(&status_field(status, "CapEff:"), 16).unwrap() & (1 << capability) != 0
}

/// The network a child's first step should take on this host, as the
/// child prints it, `filtered` when a filter refuses its `unshare`
/// `EPERM`: the run's own namespace unless a setting refuses one, as the
/// settings name the cause.
fn expected_network(filtered: bool) -> String {
    let own = std::fs::read_to_string("/proc/self/status").unwrap();
    let label = std::fs::read_to_string("/proc/self/attr/apparmor/current")
        .or_else(|_| std::fs::read_to_string("/proc/self/attr/current"))
        .unwrap_or_default();
    let restricted = setting("/proc/sys/kernel/apparmor_restrict_unprivileged_userns") == "1"
        && label.trim_end_matches(['\n', '\0']) == "unconfined"
        && !holds(&own, 21);
    if restricted {
        "inet_refused, AppArmor, none".to_owned()
    } else if filtered {
        format!("inet_refused, Filtered, {EPERM}")
    } else if setting("/proc/sys/kernel/unprivileged_userns_clone") == "0" {
        format!("inet_refused, Disabled, {EPERM}")
    } else if setting("/proc/sys/user/max_user_namespaces") == "0" {
        format!("inet_refused, NoneLeft, {ENOSPC}")
    } else {
        "namespace".to_owned()
    }
}

/// Whether the network the cases expect agrees with what an `unshare` and
/// its maps did on this host with no check first (`host_probe`), an oracle
/// apart from the settings both the box's check and [`expected_network`]
/// read: the run's own namespace only where one is entered, and the
/// host's network only where one is refused or would strand the process.
fn probe_agrees(expected: &str, probe: &str) -> bool {
    if expected == "namespace" {
        probe == "entered"
    } else if expected.starts_with("inet_refused, AppArmor") {
        probe.starts_with("stranded at") || probe.starts_with("refused")
    } else if expected.starts_with("inet_refused, Disabled") {
        probe == "refused EPERM"
    } else if expected.starts_with("inet_refused, NoneLeft") {
        probe == "refused ENOSPC"
    } else {
        false
    }
}

/// What an `unshare` of a user and network namespace, then its maps,
/// does on this host with no check first, in a `python3` of its own.
fn host_probe() -> String {
    const SCRIPT: &str = r"
import ctypes, errno, os
libc = ctypes.CDLL(None, use_errno=True)
uid, gid = os.geteuid(), os.getegid()
if libc.unshare(0x10000000 | 0x40000000) != 0:
    print('refused', errno.errorcode[ctypes.get_errno()])
    raise SystemExit
for name, text in (('setgroups', 'deny'), ('uid_map', f'{uid} {uid} 1'), ('gid_map', f'{gid} {gid} 1')):
    try:
        with open(f'/proc/self/{name}', 'w') as f:
            f.write(text)
    except OSError as e:
        print('stranded at', name, errno.errorcode[e.errno])
        raise SystemExit
print('entered')
";
    match Command::new("python3").args(["-I", "-c", SCRIPT]).output() {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        Err(error) => format!("python3 failed to start: {error}"),
    }
}

/// Runs every case in a child and checks what it and the parent report.
fn parent() {
    let scratch = scratch();
    let probe = host_probe();
    println!(
        "host: apparmor_restrict_unprivileged_userns {}, unprivileged_userns_clone {}, \
         max_user_namespaces {}; an unshare with no check first: {}",
        setting("/proc/sys/kernel/apparmor_restrict_unprivileged_userns"),
        setting("/proc/sys/kernel/unprivileged_userns_clone"),
        setting("/proc/sys/user/max_user_namespaces"),
        probe
    );
    // a listener outside the box, on the host's loopback, which no child
    // may reach, and a unix socket outside it by path
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let agent = agent_socket(std::process::id());
    let _ = std::fs::remove_file(&agent);
    let _agent = std::os::unix::net::UnixListener::bind(&agent).unwrap();
    let mut failures = Vec::new();
    let network = expected_network(false);
    if !probe_agrees(&network, &probe) {
        failures.push(format!(
            "the cases expect {network}, and an unshare with no check first did: {probe}"
        ));
    }
    for &case in CASES {
        let (status, stdout, stderr, outside) = run_case(case, &scratch.0, port);
        let expected = expected(case);
        let report = format!("{stdout}{}", outside.concat());
        let result = status.success() && report == expected;
        println!("test {case} ... {}", if result { "ok" } else { "FAILED" });
        if !result {
            failures.push(format!(
                "{case}: {status}\ngot:\n{report}\nexpected:\n{expected}\nstderr: {stderr}"
            ));
        }
    }
    match listener.accept() {
        Err(error) if error.kind() == ErrorKind::WouldBlock => {}
        accepted => failures.push(format!("the listener outside was reached: {accepted:?}")),
    }
    let _ = std::fs::remove_file(&agent);
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The unix socket outside the box the parent `parent` listens on, by a
/// path short enough for a socket's address (108 bytes) wherever the
/// checkout is: in the system's temporary directory, named by the parent.
fn agent_socket(parent: u32) -> PathBuf {
    std::env::temp_dir().join(format!("gro_run_box_{parent}.sock"))
}

/// What `case` prints, then what the parent reads of it from outside.
fn expected(case: &str) -> String {
    let network = expected_network(case.ends_with("_inet_refused"));
    let namespace = network == "namespace";
    let own = std::fs::read_to_string("/proc/self/status").unwrap();
    let uid = status_field(&own, "Uid:");
    let uid = uid.split_whitespace().nth(1).unwrap();
    let mut lines: Vec<String> = Vec::new();
    let mut line = |text: &str| lines.push(text.to_owned());
    match case {
        "walls" | "walls_inet_refused" => {
            line("inherited descriptor: closed");
            line(&format!("network: {network}"));
            if namespace {
                line("nested user namespaces: 0");
            }
            line("confined");
            line("read the project: ok");
            line("write the write root: ok");
            line("write the project: denied");
            line("write outside: denied");
            line("read outside: denied");
            line("list home: denied");
            line("create in /tmp: denied");
            let (outside, remote, loopback) = if namespace {
                ("ECONNREFUSED", "ENETUNREACH", "ok")
            } else {
                ("EPERM", "EPERM", "EPERM")
            };
            line(&format!("connect outside: {outside}"));
            line(&format!("connect a remote address: {remote}"));
            line(&format!("loopback inside: {loopback}"));
            line("unix socket by path: EPERM");
            line("truncate outside: EACCES");
            line("nested user namespace: ENOSYS");
            line("a program runs: exit 3");
            line("a program writing outside: refused");
            line("a program reading outside: refused");
            line("a program signalling outside: refused");
            line("inspect");
            // a filter of the parent's own is stacked under the box's in
            // the case `python3` starts
            let filters = if case == "walls_inet_refused" { 2 } else { 1 };
            line(&format!(
                "from outside: NoNewPrivs 1, Seccomp 2, filters {filters}"
            ));
            let bounding = if namespace || holds(&own, CAP_SETPCAP) {
                "empty"
            } else {
                "kept"
            };
            line(&format!(
                "from outside: capabilities none, bounding set {bounding}"
            ));
            line(&format!(
                "from outside: uid_map {}",
                if namespace {
                    format!("{uid} {uid} 1")
                } else {
                    "0 0 4294967295".to_owned()
                }
            ));
            line("from outside: core 0 0");
            line("from outside: open files at the hard limit");
            line("from outside: environ denied");
        }
        "threads_close" => {
            line("no_new_privs after the probe: 0");
            line("refused: 2 threads, naming a preload: true");
            line("inherited descriptor: open");
        }
        "stdin_checked_first" => {
            line("refused before any step: [0, 2]");
            line("uid_map: 0 0 4294967295");
            line("dumpable: true");
        }
        "threads_network" => {
            line("refused: 2 threads, naming a preload: false");
            line("uid_map: 0 0 4294967295");
        }
        "threads_harden" => {
            line(&format!("network: {network}"));
            line("refused: 2 threads, naming a preload: false");
            line("capabilities unchanged: true");
            line("dumpable: true");
        }
        "threads_confine" => {
            line(&format!("network: {network}"));
            line("refused: 2 threads, naming a preload: false");
            line("read outside: ok");
            line("seccomp: 0");
        }
        "socket_held" => {
            line(&format!("network: {network}"));
            line("refused: a socket held");
            line("read outside: ok");
            line("seccomp: 0");
        }
        "stdin_tcp" | "stdin_tcp_connected" | "stdin_udp" | "stdin_unix_datagram" => {
            line(&format!("network: {network}"));
            line("refused: a stdio socket on [0]");
        }
        "stdin_ring" => {
            line(&format!("network: {network}"));
            line("refused: a descriptor hiding files on [0]");
        }
        "stdout_stream" => {
            line(&format!("network: {network}"));
            line("confined");
        }
        "tool_dirs" => {
            line(&format!("network: {network}"));
            line("made: ok");
            line(
                "tool directories: .fuz/cache, .svelte-kit, node_modules/$app, node_modules/.vite, node_modules/.vite-temp",
            );
            line("the one made before: kept");
            line("a file at .svelte-kit: NotADirectory");
            line("prepared with no private directory: NoPrivateDir");
        }
        "tool_dirs_planted" => {
            line(&format!("network: {network}"));
            line(".svelte-kit linked to /tmp: Symlink");
            line("made where it leads: false");
            line("node_modules/.vite linked outside: Symlink");
            line("made where it leads: false");
            line("node_modules swapped for a link to .git: Symlink");
            line("made in .git: false");
            line(".fuz a file: NotADirectory");
            line("node_modules a file: NotADirectory");
        }
        "private_dir_reused" => {
            line(&format!("network: {network}"));
            line("prepared with a reused descriptor: PrivateDirChanged");
        }
        "program_box_umask" => {
            line("umask: 0277");
            line(&format!("network: {network}"));
            line("its temporary directory's mode: 700");
            line("write its own temporary directory: ok");
        }
        "program_box" | "program_box_inet_refused" => {
            line(&format!("network: {network}"));
            line("confined");
            line("taken once: true");
            line("temporary directories: program_0, program_1");
            line("a second box for pin 0: EEXIST");
            let loopback = if namespace { "ok" } else { "EPERM" };
            for probe in [
                "read the project: ok",
                "write a tool directory: ok",
                "write an entry the run writes: EACCES",
                "write what a write flag names: EACCES",
                "read what a write flag names: EACCES",
                "write its own run-write directory: ok",
                "read the sibling's run-write directory: EACCES",
                "write the sibling's run-write directory: EACCES",
                "write /dev/null: ok",
                "write its own temporary directory: ok",
                "run a program in it: ok",
                "write the sibling's temporary directory: EACCES",
                "read the sibling's temporary directory: EACCES",
                "read the private directory: EACCES",
                "list the private directory: EACCES",
                "run a program in the private directory: EACCES",
                "signal the run: EPERM",
                "signal the sibling: EPERM",
                "read the run's environ: EACCES",
                "read the sibling's environ: EACCES",
                "read the sibling's cwd: EACCES",
                "plant a link into the sibling's: ok",
                "write through it: EACCES",
            ] {
                line(&format!("table: {probe}"));
            }
            line(&format!("table: loopback: {loopback}"));
            for probe in [
                "read the project: ok",
                "write a tool directory: EACCES",
                "write its own run-write directory: ok",
                "write the table's run-write directory: EACCES",
                "write /dev/null: ok",
                "write its own temporary directory: ok",
                "write the table's temporary directory: EACCES",
                "write through the table's link: ok",
            ] {
                line(&format!("flag: {probe}"));
            }
            line(&format!("flag: loopback: {loopback}"));
            line("the link's write landed in the flag's own: true");
            line("the flag's box, then the table's: restricted");
            line("a tool directory written from that thread: denied");
            line("the run writes a tool directory: ok");
            line("the run writes a program's temporary directory: ok");
            line("the run writes each program's run-write directory: ok, ok");
            line("the run reads the sibling's environ: ok");
        }
        "planned" | "planned_check" => {
            let writes = if case == "planned" { "ok" } else { "denied" };
            line(&format!("network: {network}"));
            line("confined");
            line("installed as planned: true");
            line("write .git/hooks: denied");
            line("write node_modules/x: denied");
            line("write a new top-level file: denied");
            line(&format!("write src: {writes}"));
            line("write node_modules/.vite: ok");
            line(&format!("write a nested node_modules: {writes}"));
            line("write the private directory: ok");
            line("the private directory held by the box: true");
            line("/dev/null: read ok, write ok");
            line("a program runs: exit 3");
            line("read a linked package: ok");
            line("read /proc/self/status: ok");
            line("read git's home config: ok");
            line("read the home directory: denied");
            line("node runs: node ok");
        }
        "linked_named" | "linked_marker" | "linked_absent" => {
            line("confined");
            line("ran: ok");
        }
        "v8" => {
            line(&format!("network: {network}"));
            line("confined");
            line("sum 499999500000");
            line("ran: ok");
            line("read outside from the runtime's thread: denied");
            line("inspect");
            line("from outside: more than two threads, each NoNewPrivs 1, Seccomp 2, filters 1");
        }
        _ => panic!("no case {case}"),
    }
    lines.iter().fold(String::new(), |mut out, line| {
        writeln!(out, "{line}").unwrap();
        out
    })
}

/// Runs `case` in a child, which `/bin/sh` starts with a descriptor on a
/// file outside the box and a soft limit of 256 descriptors (and
/// each `_inet_refused` case's under `python3`'s filter), reading it from
/// outside when it prints `inspect`; returns its
/// status, stdout, stderr, and what the parent read.
fn run_case(
    case: &str,
    scratch: &Path,
    port: u16,
) -> (std::process::ExitStatus, String, String, Vec<String>) {
    let exe = std::env::current_exe().unwrap();
    if case.starts_with("stdin_") || case.starts_with("stdout_") {
        return run_stdio_case(case, &exe, scratch, port);
    }
    let secret = scratch.join("outside/secret.txt");
    let mut command = if case.ends_with("_inet_refused") {
        filtered()
    } else {
        Command::new("/bin/sh")
    };
    command
        // a soft limit on descriptors below the hard one, which the run's
        // steps raise
        .args([
            "-c",
            if case == "program_box_umask" {
                // a umask that takes the owner's own bits off
                r#"umask 0277 && ulimit -Sn 256 && exec "$0" child "$1" "$2" "$3" 3<"$4""#
            } else {
                r#"ulimit -Sn 256 && exec "$0" child "$1" "$2" "$3" 3<"$4""#
            },
        ])
        .arg(&exe)
        .arg(case)
        .arg(scratch)
        .arg(port.to_string())
        .arg(&secret)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = KillOnDrop(command.spawn().unwrap());
    let pid = child.id();
    let lines = support::lines(child.stdout.take().unwrap());
    let stderr = support::drain(child.stderr.take().unwrap());
    let mut stdin = child.stdin.take();
    let mut stdout = String::new();
    let mut outside = Vec::new();
    loop {
        match lines.recv_timeout(DEADLINE) {
            Ok(line) => {
                if line == "inspect\n" {
                    outside = inspect(pid, case);
                    // the child's read ends, and it exits
                    drop(stdin.take());
                }
                stdout.push_str(&line);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("{case}: no line in {DEADLINE:?}; so far:\n{stdout}")
            }
        }
    }
    let status = support::wait_bounded(case, &mut child);
    let stderr = String::from_utf8_lossy(&stderr.join().unwrap()).into_owned();
    (status, stdout, stderr, outside)
}

/// Runs `case`, whose child has a socket or an `io_uring` the parent chose
/// on its stdin, or a connected unix stream pair's end as its stdout, which the
/// parent reads; returns as [`run_case`] does, reading nothing from outside.
fn run_stdio_case(
    case: &str,
    exe: &Path,
    scratch: &Path,
    port: u16,
) -> (std::process::ExitStatus, String, String, Vec<String>) {
    use std::os::fd::OwnedFd;

    let mut command = if case == "stdin_ring" {
        ring_on_stdin()
    } else {
        Command::new(exe)
    };
    if case == "stdin_ring" {
        command.arg(exe);
    }
    command
        .args(["child", case])
        .arg(scratch)
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // a listener the case's stdin connects to, held while the child runs
    let mut held = None;
    let stdin: Stdio = match case {
        "stdin_tcp" => OwnedFd::from(
            socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap(),
        )
        .into(),
        "stdin_tcp_connected" => {
            // a listener of its own, apart from the one no child may reach
            let accepting = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let stream = std::net::TcpStream::connect(accepting.local_addr().unwrap()).unwrap();
            held = Some(accepting);
            OwnedFd::from(stream).into()
        }
        "stdin_udp" | "stdin_checked_first" => {
            OwnedFd::from(std::net::UdpSocket::bind("127.0.0.1:0").unwrap()).into()
        }
        "stdin_unix_datagram" => {
            OwnedFd::from(std::os::unix::net::UnixDatagram::unbound().unwrap()).into()
        }
        _ => Stdio::piped(),
    };
    command.stdin(stdin);
    // stderr a socket too, so the early check is seen to read past stdin
    let stderr_drained = case != "stdin_checked_first";
    if !stderr_drained {
        command.stderr(OwnedFd::from(
            std::net::UdpSocket::bind("127.0.0.1:0").unwrap(),
        ));
    }
    let reader = if case == "stdout_stream" {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        command.stdout(OwnedFd::from(theirs));
        Some(ours)
    } else {
        None
    };
    let mut child = KillOnDrop(command.spawn().unwrap());
    // the child's end of the pair is the child's alone, so its exit ends
    // the parent's reads
    drop(command);
    let stdout = reader.map_or_else(
        || support::drain(child.stdout.take().unwrap()),
        support::drain,
    );
    let stderr = stderr_drained.then(|| support::drain(child.stderr.take().unwrap()));
    let status = support::wait_bounded(case, &mut child);
    drop(held);
    let stdout = String::from_utf8_lossy(&stdout.join().unwrap()).into_owned();
    let stderr = stderr.map_or_else(String::new, |stderr| {
        String::from_utf8_lossy(&stderr.join().unwrap()).into_owned()
    });
    (status, stdout, stderr, Vec::new())
}

/// The program its first argument names, run by `python3` with an
/// `io_uring` instance on its stdin, made by `io_uring_setup`.
fn ring_on_stdin() -> Command {
    // `struct io_uring_params`, 120 bytes, zeroed: one entry, no flags
    const SCRIPT: &str = r#"
import ctypes, os, sys
libc = ctypes.CDLL(None, use_errno=True)
params = ctypes.create_string_buffer(120)
ring = libc.syscall(425, 1, params)
if ring < 0:
    sys.exit(f"io_uring_setup failed: {os.strerror(ctypes.get_errno())} (kernel.io_uring_disabled?)")
os.dup2(ring, 0)
os.close(ring)
os.execv(sys.argv[1], sys.argv[1:])
"#;
    let mut command = Command::new("python3");
    command.args(["-I", "-c", SCRIPT]);
    command
}

/// `/bin/sh` run by `python3`, which first installs a seccomp filter
/// refusing `unshare` with `EPERM` and allowing every other call, as a
/// container runtime's profile does.
fn filtered() -> Command {
    // `struct sock_filter`s: load the call number, `unshare`'s returns the
    // error, any other is allowed
    const SCRIPT: &str = r#"
import ctypes, os, struct, sys
code = struct.pack("HBBI", 0x20, 0, 0, 0)
code += struct.pack("HBBI", 0x15, 0, 1, 272)
code += struct.pack("HBBI", 0x06, 0, 0, 0x00050000 | 1)
code += struct.pack("HBBI", 0x06, 0, 0, 0x7fff0000)
buffer = ctypes.create_string_buffer(code)
class Program(ctypes.Structure):
    _fields_ = [("len", ctypes.c_ushort), ("filter", ctypes.c_void_p)]
program = Program(4, ctypes.cast(buffer, ctypes.c_void_p))
libc = ctypes.CDLL(None, use_errno=True)
ulong = ctypes.c_ulong
assert libc.prctl(38, ulong(1), ulong(0), ulong(0), ulong(0)) == 0
assert libc.prctl(22, ulong(2), ctypes.byref(program), ulong(0), ulong(0)) == 0
os.execv("/bin/sh", ["/bin/sh"] + sys.argv[1:])
"#;
    let mut command = Command::new("python3");
    command.args(["-I", "-c", SCRIPT]);
    command
}

/// What the parent reads of the boxed child `pid` from outside it.
fn inspect(pid: u32, case: &str) -> Vec<String> {
    let proc = PathBuf::from(format!("/proc/{pid}"));
    let mut lines = Vec::new();
    if case == "v8" {
        let tasks: Vec<String> = std::fs::read_dir(proc.join("task"))
            .unwrap()
            .map(|task| std::fs::read_to_string(task.unwrap().path().join("status")).unwrap())
            .collect();
        let boxed = tasks.iter().all(|status| {
            status_field(status, "NoNewPrivs:") == "1"
                && status_field(status, "Seccomp:") == "2"
                && status_field(status, "Seccomp_filters:") == "1"
        });
        lines.push(format!(
            "from outside: {} threads, each {}\n",
            if tasks.len() > 2 {
                "more than two".to_owned()
            } else {
                tasks.len().to_string()
            },
            if boxed {
                "NoNewPrivs 1, Seccomp 2, filters 1".to_owned()
            } else {
                format!("not boxed: {tasks:?}")
            }
        ));
        return lines;
    }
    let status = std::fs::read_to_string(proc.join("status")).unwrap();
    lines.push(format!(
        "from outside: NoNewPrivs {}, Seccomp {}, filters {}\n",
        status_field(&status, "NoNewPrivs:"),
        status_field(&status, "Seccomp:"),
        status_field(&status, "Seccomp_filters:")
    ));
    let none = ["CapInh:", "CapPrm:", "CapEff:", "CapAmb:"]
        .iter()
        .all(|set| status_field(&status, set) == "0000000000000000");
    let bounding = status_field(&status, "CapBnd:");
    lines.push(format!(
        "from outside: capabilities {}, bounding set {}\n",
        if none { "none" } else { "held" },
        if bounding == "0000000000000000" {
            "empty"
        } else {
            "kept"
        }
    ));
    let uid_map = std::fs::read_to_string(proc.join("uid_map")).unwrap();
    lines.push(format!(
        "from outside: uid_map {}\n",
        uid_map.split_whitespace().collect::<Vec<_>>().join(" ")
    ));
    let limits = std::fs::read_to_string(proc.join("limits")).unwrap();
    let limit = |name: &str| -> Vec<String> {
        let line = limits.lines().find(|line| line.starts_with(name)).unwrap();
        line[name.len()..]
            .split_whitespace()
            .take(2)
            .map(str::to_owned)
            .collect()
    };
    lines.push(format!(
        "from outside: core {}\n",
        limit("Max core file size").join(" ")
    ));
    let open = limit("Max open files");
    lines.push(format!(
        "from outside: open files {}\n",
        if open[0] == open[1] {
            "at the hard limit".to_owned()
        } else {
            format!("{} of {}", open[0], open[1])
        }
    ));
    lines.push(format!(
        "from outside: environ {}\n",
        outcome(std::fs::read(proc.join("environ")))
    ));
    lines
}

/// `ok`, or the error: `denied` for `EACCES` (Landlock's, or a file's
/// mode), else its number's name where a case names it.
fn outcome<T>(result: std::io::Result<T>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(error) => match error.raw_os_error() {
            Some(EACCES) => "denied".to_owned(),
            Some(EPERM) => "EPERM".to_owned(),
            Some(ENOSYS) => "ENOSYS".to_owned(),
            Some(ENOSPC) => "ENOSPC".to_owned(),
            Some(ENETUNREACH) => "ENETUNREACH".to_owned(),
            Some(ECONNREFUSED) => "ECONNREFUSED".to_owned(),
            _ => format!("{error}"),
        },
    }
}

/// The network a step took, as a case prints it.
fn network_line(network: &RunNetwork) -> String {
    match network {
        RunNetwork::Namespace => "network: namespace".to_owned(),
        RunNetwork::InetRefused { cause, errno } => format!(
            "network: inet_refused, {cause:?}, {}",
            errno.map_or_else(|| "none".to_owned(), |errno| errno.to_string())
        ),
    }
}

/// Prints `line` and flushes it, so the parent reads it before the child
/// waits or exits.
fn say(line: &str) {
    let mut stdout = std::io::stdout();
    writeln!(stdout, "{line}").unwrap();
    stdout.flush().unwrap();
}

/// The plan of the cases' box: a run of the project that writes its
/// root and starts no program, so the project is read, its top-level
/// entries written (`out` among them), and the system's directories read,
/// `/usr` executed too.
fn plan(project: &Path) -> BoxPlan {
    let inputs = BoxInputs {
        root: std::fs::canonicalize(project).unwrap(),
        writes_root: true,
        ..BoxInputs::default()
    };
    BoxPlan::for_run(&inputs, &BoxFacts::read(&inputs).unwrap()).unwrap()
}

/// The run's steps up to its box, prepared from [`plan`].
fn prepared(project: &Path) -> RunPrepared {
    let plan = plan(project);
    let networked = RunClosed::close_inherited(probe_floor().unwrap())
        .unwrap()
        .enter_network()
        .unwrap();
    say(&network_line(networked.network()));
    networked.harden().unwrap().prepare(&plan, None).unwrap()
}

/// Starts a thread that waits until the returned sender sends, for a step
/// to refuse.
fn another_thread() -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (stop, wait) = std::sync::mpsc::channel::<()>();
    let other = std::thread::spawn(move || {
        let _ = wait.recv();
    });
    (stop, other)
}

/// Runs `case` in this process, the child.
fn child(case: &str, scratch: &Path, port: u16) {
    let project = scratch.join("project");
    let outside = scratch.join("outside/secret.txt");
    match case {
        "walls" | "walls_inet_refused" => walls(&project, scratch, port),
        "threads_close" => {
            let floor = probe_floor().unwrap();
            let status = std::fs::read_to_string("/proc/self/status").unwrap();
            say(&format!(
                "no_new_privs after the probe: {}",
                status_field(&status, "NoNewPrivs:")
            ));
            let (stop, other) = another_thread();
            match RunClosed::close_inherited(floor) {
                Err(gro_runtime::confine::run::RunStepError::Threads(
                    threads @ Threads { count, .. },
                )) => {
                    say(&format!(
                        "refused: {count} threads, naming a preload: {}",
                        threads.to_string().contains("LD_PRELOAD")
                    ));
                }
                other => panic!("not refused for its threads: {other:?}"),
            }
            stop.send(()).unwrap();
            other.join().unwrap();
            say(&format!(
                "inherited descriptor: {}",
                if Path::new("/proc/self/fd/3").exists() {
                    "open"
                } else {
                    "closed"
                }
            ));
        }
        "stdin_checked_first" => {
            probe_floor().unwrap();
            match check_stdio() {
                Err(gro_runtime::confine::ConfineError::StdioSocket { fds }) => {
                    say(&format!("refused before any step: {fds:?}"));
                }
                other => panic!("not refused for stdin: {other:?}"),
            }
            let uid_map = std::fs::read_to_string("/proc/self/uid_map").unwrap();
            say(&format!(
                "uid_map: {}",
                uid_map.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
            say(&format!("dumpable: {}", gro_sys::is_dumpable().unwrap()));
        }
        "threads_network" => {
            let closed = RunClosed::close_inherited(probe_floor().unwrap()).unwrap();
            let (stop, other) = another_thread();
            match closed.enter_network() {
                Err(gro_runtime::confine::run::RunStepError::Threads(
                    threads @ Threads { count, .. },
                )) => {
                    say(&format!(
                        "refused: {count} threads, naming a preload: {}",
                        threads.to_string().contains("LD_PRELOAD")
                    ));
                }
                other => panic!("not refused for its threads: {other:?}"),
            }
            stop.send(()).unwrap();
            other.join().unwrap();
            let uid_map = std::fs::read_to_string("/proc/self/uid_map").unwrap();
            say(&format!(
                "uid_map: {}",
                uid_map.split_whitespace().collect::<Vec<_>>().join(" ")
            ));
        }
        "threads_harden" => {
            let networked = RunClosed::close_inherited(probe_floor().unwrap())
                .unwrap()
                .enter_network()
                .unwrap();
            say(&network_line(networked.network()));
            let before = capabilities();
            let (stop, other) = another_thread();
            match networked.harden() {
                Err(gro_runtime::confine::run::RunStepError::Threads(
                    threads @ Threads { count, .. },
                )) => {
                    say(&format!(
                        "refused: {count} threads, naming a preload: {}",
                        threads.to_string().contains("LD_PRELOAD")
                    ));
                }
                other => panic!("not refused for its threads: {other:?}"),
            }
            stop.send(()).unwrap();
            other.join().unwrap();
            say(&format!(
                "capabilities unchanged: {}",
                capabilities() == before
            ));
            say(&format!("dumpable: {}", gro_sys::is_dumpable().unwrap()));
        }
        "threads_confine" => {
            let prepared = prepared(&project);
            let (stop, other) = another_thread();
            match prepared.confine() {
                Err(gro_runtime::confine::ConfineError::Threads(
                    threads @ Threads { count, .. },
                )) => {
                    say(&format!(
                        "refused: {count} threads, naming a preload: {}",
                        threads.to_string().contains("LD_PRELOAD")
                    ));
                }
                other => panic!("not refused for its threads: {other:?}"),
            }
            stop.send(()).unwrap();
            other.join().unwrap();
            // nothing was installed
            say(&format!(
                "read outside: {}",
                outcome(std::fs::read(&outside))
            ));
            let status = std::fs::read_to_string("/proc/self/status").unwrap();
            say(&format!("seccomp: {}", status_field(&status, "Seccomp:")));
        }
        "socket_held" => {
            let prepared = prepared(&project);
            // a socket held across the box, on the network it was made in
            let held =
                socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
            match prepared.confine() {
                Err(gro_runtime::confine::ConfineError::SocketHeld { fds }) => {
                    assert_eq!(fds, [std::os::fd::AsRawFd::as_raw_fd(&held)]);
                    say("refused: a socket held");
                }
                other => panic!("not refused for the socket: {other:?}"),
            }
            // nothing was installed
            say(&format!(
                "read outside: {}",
                outcome(std::fs::read(&outside))
            ));
            let status = std::fs::read_to_string("/proc/self/status").unwrap();
            say(&format!("seccomp: {}", status_field(&status, "Seccomp:")));
        }
        "stdin_tcp"
        | "stdin_tcp_connected"
        | "stdin_udp"
        | "stdin_unix_datagram"
        | "stdin_ring"
        | "stdout_stream" => match prepared(&project).confine() {
            Err(gro_runtime::confine::ConfineError::StdioSocket { fds }) => {
                say(&format!("refused: a stdio socket on {fds:?}"));
            }
            Err(gro_runtime::confine::ConfineError::HidingHeld { fds }) => {
                say(&format!("refused: a descriptor hiding files on {fds:?}"));
            }
            Ok(_) => say("confined"),
            Err(error) => panic!("refused otherwise: {error}"),
        },
        "tool_dirs" => tool_dirs(scratch),
        "tool_dirs_planted" => tool_dirs_planted(scratch),
        "planned" => planned(scratch, true),
        "private_dir_reused" => private_dir_reused(scratch),
        "program_box" | "program_box_inet_refused" => program_box(scratch, case),
        "program_box_umask" => program_box_umask(scratch),
        "planned_check" => planned(scratch, false),
        #[cfg(feature = "snapshot")]
        "v8" => v8::run(&project, &outside),
        #[cfg(feature = "snapshot")]
        "linked_named" | "linked_marker" | "linked_absent" => v8::linked(case, scratch),
        _ => panic!("no case {case}"),
    }
}

/// A project of its own beneath the scratch directory, `node_modules`
/// installed, made before the run's steps, its real path.
fn tools_project(scratch: &Path, name: &str) -> PathBuf {
    let project = scratch.join(name);
    std::fs::create_dir_all(project.join("node_modules")).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::write(project.join("package.json"), "{}").unwrap();
    std::fs::canonicalize(project).unwrap()
}

/// The plan of a run of `project` granting the local `tools`, the project
/// declaring `declared`.
fn tools_plan(project: &Path, tools: &[&str], declared: &[&str]) -> BoxPlan {
    let inputs = BoxInputs {
        root: project.to_owned(),
        tools: tools.iter().map(|&tool| tool.to_owned()).collect(),
        declared: declared.iter().map(|&name| name.to_owned()).collect(),
        ..BoxInputs::default()
    };
    BoxPlan::for_run(&inputs, &BoxFacts::read(&inputs).unwrap()).unwrap()
}

/// The hardened run's steps, the network printed.
fn hardened() -> RunHardened {
    let networked = RunClosed::close_inherited(probe_floor().unwrap())
        .unwrap()
        .enter_network()
        .unwrap();
    say(&network_line(networked.network()));
    networked.harden().unwrap()
}

/// What a make of tool directories refused, by kind.
fn tool_dir_refusal(made: Result<(), ToolDirError>) -> String {
    match made {
        Ok(()) => "made".to_owned(),
        Err(ToolDirError::NotADirectory { .. }) => "NotADirectory".to_owned(),
        Err(ToolDirError::Symlink { .. }) => "Symlink".to_owned(),
        Err(error) => format!("{error}"),
    }
}

/// The tool directories a plan names, made once the run is hardened,
/// `.fuz` with `.fuz/cache` and `.svelte-kit` passing as the directory made
/// before; a file at one refused.
fn tool_dirs(scratch: &Path) {
    let project = tools_project(scratch, "tools_made");
    std::fs::create_dir(project.join(".svelte-kit")).unwrap();
    std::fs::write(project.join(".svelte-kit/kept.txt"), "").unwrap();
    let plan = tools_plan(
        &project,
        &["vitest"],
        &["@sveltejs/kit", "vite", "@fuzdev/fuz_css"],
    );
    let filed = tools_project(scratch, "tools_file");
    std::fs::write(filed.join(".svelte-kit"), "").unwrap();
    let file_plan = tools_plan(&filed, &["vitest"], &["@sveltejs/kit"]);
    let spawning_inputs = BoxInputs {
        root: project.clone(),
        spawns: true,
        ..BoxInputs::default()
    };
    let spawning =
        BoxPlan::for_run(&spawning_inputs, &BoxFacts::read(&spawning_inputs).unwrap()).unwrap();
    let hardened = hardened();
    hardened.make_tool_dirs(&plan).unwrap();
    say("made: ok");
    let mut made: Vec<String> = [
        ".fuz/cache",
        ".svelte-kit",
        "node_modules/$app",
        "node_modules/.vite",
        "node_modules/.vite-temp",
    ]
    .into_iter()
    .filter(|dir| project.join(dir).is_dir() && !project.join(dir).is_symlink())
    .map(str::to_owned)
    .collect();
    made.sort();
    say(&format!("tool directories: {}", made.join(", ")));
    say(&format!(
        "the one made before: {}",
        if project.join(".svelte-kit/kept.txt").exists() {
            "kept"
        } else {
            "emptied"
        }
    ));
    say(&format!(
        "a file at .svelte-kit: {}",
        tool_dir_refusal(hardened.make_tool_dirs(&file_plan))
    ));
    say(&format!(
        "prepared with no private directory: {}",
        match hardened.prepare(&spawning, None) {
            Err(gro_runtime::confine::rules::RuleError::NoPrivateDir { .. }) => "NoPrivateDir",
            Err(_) => "another refusal",
            Ok(_) => "prepared",
        }
    ));
}

/// A tool directory planted as a symlink, `.svelte-kit` to `/tmp` and
/// `node_modules/.vite` outside the project, and `node_modules` swapped for
/// a link to `.git` after the plan read it, each refused with nothing made
/// where it leads.
fn tool_dirs_planted(scratch: &Path) {
    let project = tools_project(scratch, "tools_planted");
    let tmp = PathBuf::from(format!("/tmp/gro_run_box_{}_x", std::process::id()));
    std::os::unix::fs::symlink(&tmp, project.join(".svelte-kit")).unwrap();
    let kit = tools_plan(&project, &["svelte-kit"], &["@sveltejs/kit"]);
    let home = scratch.join("outside/home_x");
    std::os::unix::fs::symlink(&home, project.join("node_modules/.vite")).unwrap();
    let vite = tools_plan(&project, &["vitest"], &["vitest"]);
    let swapped = tools_project(scratch, "tools_swapped");
    let swapped_plan = tools_plan(&swapped, &["vitest"], &["vitest"]);
    std::fs::rename(swapped.join("node_modules"), swapped.join("moved")).unwrap();
    std::os::unix::fs::symlink(".git", swapped.join("node_modules")).unwrap();
    // a file on the way to a tool directory: `.fuz` for `.fuz/cache`, and a
    // `node_modules` swapped for one after the plan, for `node_modules/.vite`
    let filed = tools_project(scratch, "tools_filed");
    std::fs::write(filed.join(".fuz"), "").unwrap();
    let fuz_plan = tools_plan(&filed, &["vitest"], &["@fuzdev/fuz_css"]);
    let filed_modules = tools_project(scratch, "tools_filed_modules");
    let filed_plan = tools_plan(&filed_modules, &["vitest"], &["vitest"]);
    std::fs::remove_dir(filed_modules.join("node_modules")).unwrap();
    std::fs::write(filed_modules.join("node_modules"), "").unwrap();
    let hardened = hardened();
    say(&format!(
        ".svelte-kit linked to /tmp: {}",
        tool_dir_refusal(hardened.make_tool_dirs(&kit))
    ));
    say(&format!("made where it leads: {}", tmp.exists()));
    say(&format!(
        "node_modules/.vite linked outside: {}",
        tool_dir_refusal(hardened.make_tool_dirs(&vite))
    ));
    say(&format!("made where it leads: {}", home.exists()));
    say(&format!(
        "node_modules swapped for a link to .git: {}",
        tool_dir_refusal(hardened.make_tool_dirs(&swapped_plan))
    ));
    say(&format!(
        "made in .git: {}",
        swapped.join(".git/.vite").exists() || swapped.join(".git/.vite-temp").exists()
    ));
    say(&format!(
        ".fuz a file: {}",
        tool_dir_refusal(hardened.make_tool_dirs(&fuz_plan))
    ));
    say(&format!(
        "node_modules a file: {}",
        tool_dir_refusal(hardened.make_tool_dirs(&filed_plan))
    ));
}

/// A run of a project of its own as `gen` (`writing`) or `check` plans it:
/// `svelte-kit` granted, the project declaring `@sveltejs/kit` and `vite`,
/// the `node` on `PATH` pinned, a package linked from outside, a nested
/// `node_modules`, and git's home config in a home of its own; its tool
/// directories and private directory made, its box up, then what it and
/// the programs it starts reach.
fn planned(scratch: &Path, writing: bool) {
    let case = if writing {
        "planned_write"
    } else {
        "planned_check"
    };
    let made = |path: &str| std::fs::create_dir_all(scratch.join(case).join(path)).unwrap();
    for dir in [
        "project/src",
        "project/.git/hooks",
        "project/node_modules/x",
        "project/examples/app/node_modules",
        "linked",
        "home",
        "tmp",
    ] {
        made(dir);
    }
    let at = |path: &str| {
        std::fs::canonicalize(scratch.join(case))
            .unwrap()
            .join(path)
    };
    let project = at("project");
    for (path, text) in [
        ("project/package.json", "{}"),
        ("project/CLAUDE.md", ""),
        ("linked/package.json", "{}"),
        ("home/.gitconfig", ""),
        ("home/secret.txt", ""),
    ] {
        std::fs::write(at(path), text).unwrap();
    }
    std::os::unix::fs::symlink("CLAUDE.md", project.join("AGENTS.md")).unwrap();
    std::os::unix::fs::symlink(at("linked"), project.join("node_modules/linked")).unwrap();
    let path = std::env::var("PATH").unwrap();
    let found = PathBuf::from(
        gro_runtime::find_on_path("node", Some(&path))
            .unwrap()
            .unwrap(),
    );
    let node = std::fs::canonicalize(&found).unwrap();
    let inputs = BoxInputs {
        root: project.clone(),
        writes_root: writing,
        tools: ["svelte-kit".to_owned()].into(),
        spawns: true,
        declared: ["@sveltejs/kit".to_owned(), "vite".to_owned()].into(),
        programs: vec![ProgramFile::Node {
            script: {
                let mut first = [0u8; 2];
                std::io::Read::read_exact(&mut std::fs::File::open(&node).unwrap(), &mut first)
                    .unwrap();
                &first == b"#!"
            },
            found,
            real: node.clone(),
        }],
        home: Some(at("home")),
        ..BoxInputs::default()
    };
    let plan = BoxPlan::for_run(&inputs, &BoxFacts::read(&inputs).unwrap()).unwrap();
    let hardened = hardened();
    let parent: std::os::fd::OwnedFd = std::fs::File::open(at("tmp")).unwrap().into();
    let private = hardened
        .make_private_dir(parent.as_fd(), &at("tmp"))
        .unwrap();
    drop(parent);
    hardened.make_tool_dirs(&plan).unwrap();
    let private_path = private.path().to_owned();
    let prepared = hardened.prepare(&plan, Some(private)).unwrap();
    let confined = prepared.confine().unwrap();
    say("confined");
    let planned: Vec<_> = plan.rules().iter().map(|rule| rule.source).collect();
    let installed: Vec<_> = confined.rules().iter().map(InstalledRule::source).collect();
    say(&format!(
        "installed as planned: {}",
        confined.absent().is_empty() && installed == planned
    ));
    let write = |path: PathBuf| outcome(std::fs::write(path, ""));
    say(&format!(
        "write .git/hooks: {}",
        write(project.join(".git/hooks/x"))
    ));
    say(&format!(
        "write node_modules/x: {}",
        write(project.join("node_modules/x/y"))
    ));
    say(&format!(
        "write a new top-level file: {}",
        write(project.join("new.txt"))
    ));
    say(&format!("write src: {}", write(project.join("src/a.txt"))));
    say(&format!(
        "write node_modules/.vite: {}",
        write(project.join("node_modules/.vite/x"))
    ));
    say(&format!(
        "write a nested node_modules: {}",
        write(project.join("examples/app/node_modules/x"))
    ));
    say(&format!(
        "write the private directory: {}",
        write(private_path.join("x"))
    ));
    // its lock lives in the box, past `prepare`: a sweep's exclusive lock
    // waits
    say(&format!(
        "the private directory held by the box: {}",
        std::fs::File::open(&private_path)
            .is_ok_and(|dir| matches!(dir.try_lock(), Err(std::fs::TryLockError::WouldBlock)))
    ));
    let null = Path::new("/dev/null");
    say(&format!(
        "/dev/null: read {}, write {}",
        outcome(std::fs::read(null)),
        outcome(std::fs::OpenOptions::new().write(true).open(null))
    ));
    say(&format!(
        "a program runs: {}",
        program("exit 3", &[]).map_or_else(|error| error, |code| format!("exit {code}"))
    ));
    say(&format!(
        "read a linked package: {}",
        outcome(std::fs::read(
            project.join("node_modules/linked/package.json")
        ))
    ));
    say(&format!(
        "read /proc/self/status: {}",
        outcome(std::fs::read("/proc/self/status"))
    ));
    say(&format!(
        "read git's home config: {}",
        outcome(std::fs::read(at("home/.gitconfig")))
    ));
    say(&format!(
        "read the home directory: {}",
        outcome(std::fs::read(at("home/secret.txt")))
    ));
    let ran = Command::new(&node)
        .args(["-e", "process.memoryUsage(); console.log('node ok')"])
        .stdin(Stdio::piped())
        .output()
        .map_or_else(
            |error| format!("didn't start: {error}"),
            |output| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout).trim(),
                    String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .next()
                        .unwrap_or_default()
                )
            },
        );
    say(&format!("node runs: {ran}"));
}

/// A private directory whose descriptor numbers a second chain of the
/// run's steps closed and the next opens reused: `prepare` refuses it, held
/// to the directory it made.
fn private_dir_reused(scratch: &Path) {
    let project = tools_project(scratch, "private_reused");
    let tmp = scratch.join("private_reused_tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let inputs = BoxInputs {
        root: project.clone(),
        temp: true,
        ..BoxInputs::default()
    };
    let plan = BoxPlan::for_run(&inputs, &BoxFacts::read(&inputs).unwrap()).unwrap();
    let hardened = hardened();
    let parent: std::os::fd::OwnedFd = std::fs::File::open(&tmp).unwrap().into();
    let private = hardened.make_private_dir(parent.as_fd(), &tmp).unwrap();
    drop(parent);
    // gro's own bug, a second chain: its first step closes every
    // descriptor above stderr by number, the private directory's among them
    let _second = RunClosed::close_inherited(probe_floor().unwrap()).unwrap();
    let reused: Vec<std::fs::File> = (0..4)
        .map(|_| std::fs::File::open(&project).unwrap())
        .collect();
    say(&format!(
        "prepared with a reused descriptor: {}",
        match hardened.prepare(&plan, Some(private)) {
            Err(gro_runtime::confine::rules::RuleError::PrivateDirChanged { .. }) => {
                "PrivateDirChanged"
            }
            Err(_) => "another refusal",
            Ok(_) => "prepared",
        }
    ));
    // the descriptors the directory held are the reused ones now: end here
    std::mem::forget(reused);
    std::process::exit(0);
}

/// What a program's probes print, each line `label: result`: a spec
/// `label|op|path[|target]` per argument, the result `ok` or the error's
/// name.
const PROBES: &str = r"
import errno, os, socket, stat, subprocess, sys
for spec in sys.argv[1:]:
    label, op, *args = spec.split('|')
    try:
        if op == 'read':
            open(args[0], 'rb').read()
        elif op == 'list':
            os.listdir(args[0])
        elif op == 'write':
            with open(args[0], 'w') as f:
                f.write('x')
        elif op == 'script':
            with open(args[0], 'w') as f:
                f.write('#!/bin/sh\nexit 0\n')
            os.chmod(args[0], 0o755)
            subprocess.run([args[0]], stdin=subprocess.DEVNULL, check=True)
        elif op == 'exec':
            subprocess.run([args[0]], stdin=subprocess.DEVNULL, check=True)
        elif op == 'kill':
            os.kill(int(args[0]), 0)
        elif op == 'readlink':
            os.readlink(args[0])
        elif op == 'link':
            os.symlink(args[1], args[0])
        elif op == 'loopback':
            listener = socket.socket()
            listener.bind(('127.0.0.1', 0))
            listener.listen()
            socket.create_connection(listener.getsockname()).close()
        print(f'{label}: ok')
    except OSError as error:
        print(f'{label}: {errno.errorcode.get(error.errno, error.errno)}')
    except subprocess.CalledProcessError as error:
        print(f'{label}: exit {error.returncode}')
";

/// Runs `run` on a thread of its own restricted by `program_box`, as a
/// launch does, and returns what it returns.
fn in_box<T: Send>(program_box: &ProgramBox, run: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                program_box.restrict_thread().unwrap();
                run()
            })
            .join()
            .unwrap()
    })
}

/// Prints what `python3` prints for `probes`, run in `program_box`, each
/// line after `label`.
fn probe(program_box: &ProgramBox, label: &str, probes: &[String]) {
    let output = in_box(program_box, || {
        Command::new("python3")
            .args(["-I", "-c", PROBES])
            .args(probes)
            .stdin(Stdio::piped())
            .output()
            .unwrap()
    });
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        say(&format!("{label}: {line}"));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        say(&format!("{label}: stderr {stderr}"));
    }
}

/// A program's own box, nested in the run's: a run that writes its root,
/// grants a tool of the table, and names a write flag and a directory
/// `--allow-run-write=` gives each of two programs, boxed, its child rules
/// taken, then the two programs' boxes built from their write sets, a
/// table's (pin 0) and a flag's (pin 1), each attacking from inside a
/// program it starts what the other holds, the other's run-write directory
/// among it, what a write flag names, read included, and what the run
/// holds, beside the flag's sibling alive in
/// its box; then a thread restricted by the flag's box trying to widen
/// itself by the table's, and the run's own domain, as it was. Its
/// `_inet_refused` twin takes the host's network, so a program's TCP is
/// held to each form the run's layer decides.
fn program_box(scratch: &Path, case: &str) {
    let base = scratch.join(case);
    for dir in [
        "project/src",
        "project/node_modules",
        "project/.git",
        "home",
        "tmp",
        "flagged",
        "table_writes",
        "flag_writes",
    ] {
        std::fs::create_dir_all(base.join(dir)).unwrap();
    }
    std::fs::write(base.join("project/package.json"), "{}").unwrap();
    std::fs::write(base.join("flagged/kept.txt"), "").unwrap();
    std::fs::write(base.join("flag_writes/kept.txt"), "").unwrap();
    let base = std::fs::canonicalize(base).unwrap();
    let at = |path: &str| base.join(path);
    let project = at("project");
    let inputs = BoxInputs {
        root: project.clone(),
        writes_root: true,
        tools: ["svelte-kit".to_owned()].into(),
        spawns: true,
        declared: ["@sveltejs/kit".to_owned()].into(),
        flags: vec![GrantedFlagDir {
            flag: DirFlag::Write,
            dir: "../flagged".to_owned(),
            real: at("flagged"),
        }],
        run_writes: [
            ("svelte-kit", true, "table_writes"),
            ("x", false, "flag_writes"),
        ]
        .map(|(program, table, dir)| RunWriteGrant {
            program: program.to_owned(),
            table,
            dirs: vec![RunWriteDir {
                named: program.to_owned(),
                dir: format!("../{dir}"),
                real: at(dir),
            }],
        })
        .to_vec(),
        home: Some(at("home")),
        ..BoxInputs::default()
    };
    let plan = BoxPlan::for_run(&inputs, &BoxFacts::read(&inputs).unwrap()).unwrap();
    let writes = plan.program_writes();
    assert_eq!(
        writes,
        [
            ProgramWrites::TABLE,
            ProgramWrites {
                tool_dirs: true,
                root: false,
                run_write: Some(0),
            },
            ProgramWrites {
                tool_dirs: false,
                root: false,
                run_write: Some(1),
            },
        ]
    );
    let hardened = hardened();
    let parent: std::os::fd::OwnedFd = std::fs::File::open(at("tmp")).unwrap().into();
    let private = hardened
        .make_private_dir(parent.as_fd(), &at("tmp"))
        .unwrap();
    drop(parent);
    hardened.make_tool_dirs(&plan).unwrap();
    // what the run keeps in its private directory, beside its programs'
    let private_dir = private.path().to_owned();
    std::fs::create_dir(private_dir.join("xdg_data")).unwrap();
    std::fs::write(private_dir.join("xdg_data/token"), "secret").unwrap();
    let script = private_dir.join("run.sh");
    std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut confined = hardened
        .prepare(&plan, Some(private))
        .unwrap()
        .confine()
        .unwrap();
    say("confined");
    let programs = confined.take_program_rules().unwrap();
    say(&format!(
        "taken once: {}",
        confined.take_program_rules().is_none()
    ));
    let table = programs.program_box(writes[1], 0).unwrap();
    let flag = programs.program_box(writes[2], 1).unwrap();
    let named = |program_box: &ProgramBox| {
        assert_eq!(program_box.temp_dir().parent(), Some(private_dir.as_path()));
        program_box
            .temp_dir()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    say(&format!(
        "temporary directories: {}, {}",
        named(&table),
        named(&flag)
    ));
    say(&format!(
        "a second box for pin 0: {}",
        match programs.program_box(ProgramWrites::TABLE, 0) {
            Err(ProgramBoxError::TempDir { source, .. })
                if source.raw_os_error() == Some(EEXIST) =>
            {
                "EEXIST".to_owned()
            }
            other => format!("{other:?}"),
        }
    ));
    // written by the run, the outer domain, before either program runs
    std::fs::write(flag.temp_dir().join("kept.txt"), "").unwrap();
    let mut sibling = KillOnDrop(in_box(&flag, || {
        Command::new("sleep")
            .arg("600")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap()
    }));
    let sibling_pid = sibling.id().to_string();
    let run_pid = std::process::id().to_string();
    let spec = |label: &str, op: &str, path: &Path| format!("{label}|{op}|{}", path.display());
    let svelte_kit = project.join(".svelte-kit/written.txt");
    let link = table.temp_dir().join("link");
    let victim = flag.temp_dir().join("victim");
    probe(
        &table,
        "table",
        &[
            spec("read the project", "read", &project.join("package.json")),
            spec("write a tool directory", "write", &svelte_kit),
            spec(
                "write an entry the run writes",
                "write",
                &project.join("src/a.txt"),
            ),
            spec(
                "write what a write flag names",
                "write",
                &at("flagged/written.txt"),
            ),
            spec(
                "read what a write flag names",
                "read",
                &at("flagged/kept.txt"),
            ),
            spec(
                "write its own run-write directory",
                "write",
                &at("table_writes/a.txt"),
            ),
            spec(
                "read the sibling's run-write directory",
                "read",
                &at("flag_writes/kept.txt"),
            ),
            spec(
                "write the sibling's run-write directory",
                "write",
                &at("flag_writes/a.txt"),
            ),
            spec("write /dev/null", "write", Path::new("/dev/null")),
            spec(
                "write its own temporary directory",
                "write",
                &table.temp_dir().join("a.txt"),
            ),
            spec(
                "run a program in it",
                "script",
                &table.temp_dir().join("run.sh"),
            ),
            spec(
                "write the sibling's temporary directory",
                "write",
                &flag.temp_dir().join("a.txt"),
            ),
            spec(
                "read the sibling's temporary directory",
                "read",
                &flag.temp_dir().join("kept.txt"),
            ),
            spec(
                "read the private directory",
                "read",
                &private_dir.join("xdg_data/token"),
            ),
            spec("list the private directory", "list", &private_dir),
            spec("run a program in the private directory", "exec", &script),
            format!("signal the run|kill|{run_pid}"),
            format!("signal the sibling|kill|{sibling_pid}"),
            format!("read the run's environ|read|/proc/{run_pid}/environ"),
            format!("read the sibling's environ|read|/proc/{sibling_pid}/environ"),
            format!("read the sibling's cwd|readlink|/proc/{sibling_pid}/cwd"),
            format!(
                "plant a link into the sibling's|link|{}|{}",
                link.display(),
                victim.display()
            ),
            spec("write through it", "write", &link),
            "loopback|loopback".to_owned(),
        ],
    );
    probe(
        &flag,
        "flag",
        &[
            spec("read the project", "read", &project.join("package.json")),
            spec("write a tool directory", "write", &svelte_kit),
            spec(
                "write its own run-write directory",
                "write",
                &at("flag_writes/b.txt"),
            ),
            spec(
                "write the table's run-write directory",
                "write",
                &at("table_writes/b.txt"),
            ),
            spec("write /dev/null", "write", Path::new("/dev/null")),
            spec(
                "write its own temporary directory",
                "write",
                &flag.temp_dir().join("a.txt"),
            ),
            spec(
                "write the table's temporary directory",
                "write",
                &table.temp_dir().join("b.txt"),
            ),
            spec("write through the table's link", "write", &link),
            "loopback|loopback".to_owned(),
        ],
    );
    say(&format!(
        "the link's write landed in the flag's own: {}",
        victim.exists()
    ));
    in_box(&flag, || {
        say(&format!(
            "the flag's box, then the table's: {}",
            table
                .restrict_thread()
                .map_or_else(|error| format!("{error}"), |()| "restricted".to_owned())
        ));
        say(&format!(
            "a tool directory written from that thread: {}",
            outcome(std::fs::write(project.join(".svelte-kit/widened.txt"), ""))
        ));
    });
    say(&format!(
        "the run writes a tool directory: {}",
        outcome(std::fs::write(project.join(".svelte-kit/run.txt"), ""))
    ));
    say(&format!(
        "the run writes a program's temporary directory: {}",
        outcome(std::fs::write(flag.temp_dir().join("run.txt"), ""))
    ));
    say(&format!(
        "the run writes each program's run-write directory: {}, {}",
        outcome(std::fs::write(at("table_writes/run.txt"), "")),
        outcome(std::fs::write(at("flag_writes/run.txt"), ""))
    ));
    say(&format!(
        "the run reads the sibling's environ: {}",
        outcome(std::fs::read(format!("/proc/{sibling_pid}/environ")))
    ));
    sibling.kill().unwrap();
    sibling.wait().unwrap();
}

/// A program's temporary directory under a umask that takes the owner's
/// own bits off (`0277`, which `/bin/sh` set before this process): its
/// user's alone all the same, and written from inside its box. The case's
/// own directories are given their modes as they're made.
fn program_box_umask(scratch: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    say(&format!("umask: {}", status_field(&status, "Umask:")));
    let base = scratch.join("program_box_umask");
    let made = |path: &Path| {
        std::fs::create_dir(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    made(&base);
    for dir in ["project", "home", "tmp"] {
        made(&base.join(dir));
    }
    std::fs::write(base.join("project/package.json"), "{}").unwrap();
    let base = std::fs::canonicalize(base).unwrap();
    let inputs = BoxInputs {
        root: base.join("project"),
        spawns: true,
        home: Some(base.join("home")),
        ..BoxInputs::default()
    };
    let plan = BoxPlan::for_run(&inputs, &BoxFacts::read(&inputs).unwrap()).unwrap();
    let hardened = hardened();
    let parent: std::os::fd::OwnedFd = std::fs::File::open(base.join("tmp")).unwrap().into();
    let private = hardened
        .make_private_dir(parent.as_fd(), &base.join("tmp"))
        .unwrap();
    drop(parent);
    let mut confined = hardened
        .prepare(&plan, Some(private))
        .unwrap()
        .confine()
        .unwrap();
    let programs = confined.take_program_rules().unwrap();
    let program_box = programs.program_box(ProgramWrites::FLAG, 0).unwrap();
    let mode = std::fs::metadata(program_box.temp_dir())
        .unwrap()
        .permissions()
        .mode();
    say(&format!(
        "its temporary directory's mode: {:o}",
        mode & 0o777
    ));
    let temp = program_box.temp_dir().join("a.txt");
    say(&format!(
        "write its own temporary directory: {}",
        in_box(&program_box, || outcome(std::fs::write(&temp, "")))
    ));
}

/// The process's capability sets, as `/proc/self/status` shows them.
fn capabilities() -> Vec<String> {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    ["CapInh:", "CapPrm:", "CapEff:", "CapBnd:", "CapAmb:"]
        .iter()
        .map(|set| status_field(&status, set))
        .collect()
}

/// Waits for the parent to read this process from outside.
fn wait_for_inspection() {
    say("inspect");
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
}

/// The walls case: the run's steps, each seen where it can be, the box,
/// then what this process and the programs it starts can reach.
fn walls(project: &Path, scratch: &Path, port: u16) {
    let plan = plan(project);
    let closed = RunClosed::close_inherited(probe_floor().unwrap()).unwrap();
    // descriptor 3, which `/bin/sh` opened on a file outside the box
    say(&format!(
        "inherited descriptor: {}",
        if Path::new("/proc/self/fd/3").exists() {
            "open"
        } else {
            "closed"
        }
    ));
    let networked = closed.enter_network().unwrap();
    say(&network_line(networked.network()));
    if *networked.network() == RunNetwork::Namespace {
        say(&format!(
            "nested user namespaces: {}",
            setting("/proc/sys/user/max_user_namespaces")
        ));
    }
    let prepared = networked.harden().unwrap().prepare(&plan, None).unwrap();
    let _confined = prepared.confine().unwrap();
    say("confined");
    let outside = scratch.join("outside");
    say(&format!(
        "read the project: {}",
        outcome(std::fs::read(project.join("package.json")))
    ));
    say(&format!(
        "write the write root: {}",
        outcome(std::fs::write(project.join("out/written.txt"), ""))
    ));
    say(&format!(
        "write the project: {}",
        outcome(std::fs::write(project.join("written.txt"), ""))
    ));
    say(&format!(
        "write outside: {}",
        outcome(std::fs::write(outside.join("written.txt"), ""))
    ));
    say(&format!(
        "read outside: {}",
        outcome(std::fs::read(outside.join("secret.txt")))
    ));
    let home = std::env::var_os("HOME").unwrap();
    say(&format!("list home: {}", outcome(std::fs::read_dir(home))));
    say(&format!(
        "create in /tmp: {}",
        outcome(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(format!("/tmp/gro_run_box_{}", std::process::id()))
        )
    ));
    say(&format!(
        "connect outside: {}",
        outcome(std::net::TcpStream::connect(("127.0.0.1", port)))
    ));
    say(&format!(
        "connect a remote address: {}",
        outcome(std::net::TcpStream::connect_timeout(
            &REMOTE.parse().unwrap(),
            Duration::from_secs(2)
        ))
    ));
    let loopback = std::net::TcpListener::bind("127.0.0.1:0").and_then(|listener| {
        std::net::TcpStream::connect(listener.local_addr()?).map(|stream| (listener, stream))
    });
    say(&format!("loopback inside: {}", outcome(loopback)));
    say(&format!(
        "unix socket by path: {}",
        outcome(std::os::unix::net::UnixStream::connect(agent_socket(
            std::os::unix::process::parent_id()
        )))
    ));
    // by path, which Landlock's truncate right gates apart from writing
    say(&format!(
        "truncate outside: {}",
        python(&format!(
            "import errno, os\n\
             try:\n    os.truncate({:?}, 0)\n    print('ok')\n\
             except OSError as error:\n    print(errno.errorcode[error.errno])",
            outside.join("truncated.txt").display().to_string()
        ))
    ));
    say(&format!(
        "nested user namespace: {}",
        python(
            "import ctypes, errno\n\
             libc = ctypes.CDLL(None, use_errno=True)\n\
             print('ok' if libc.unshare(0x10000000) == 0 else \
             errno.errorcode[ctypes.get_errno()])"
        )
    ));
    say(&format!(
        "a program runs: {}",
        program("exit 3", &[]).map_or_else(|error| error, |code| format!("exit {code}"))
    ));
    let written = outside.join("program.txt");
    let wrote = program(r#"echo x > "$1""#, &[written.as_os_str()]);
    say(&format!(
        "a program writing outside: {}",
        refused(wrote, written.exists())
    ));
    let read = program(r#"cat "$1""#, &[outside.join("secret.txt").as_os_str()]);
    say(&format!(
        "a program reading outside: {}",
        refused(read, false)
    ));
    let parent = std::os::unix::process::parent_id().to_string();
    let signalled = program(r#"kill -0 "$1""#, &[parent.as_ref()]);
    say(&format!(
        "a program signalling outside: {}",
        refused(signalled, false)
    ));
    wait_for_inspection();
}

/// `refused` when a program exited nonzero and left nothing (`left`),
/// else what it did.
fn refused(code: Result<i32, String>, left: bool) -> String {
    match code {
        Ok(code) if code != 0 && !left => "refused".to_owned(),
        Ok(code) => format!("exit {code}, left {left}"),
        Err(error) => error,
    }
}

/// `/bin/sh -c script` with `args` as `$1` on, its stdin a pipe (no
/// `/dev/null`, which the box doesn't grant), its output discarded;
/// returns its exit code.
fn program(script: &str, args: &[&std::ffi::OsStr]) -> Result<i32, String> {
    Command::new("/bin/sh")
        .args(["-c", script, "sh"])
        .args(args)
        .stdin(Stdio::piped())
        .output()
        .map(|output| output.status.code().unwrap_or(-1))
        .map_err(|error| format!("didn't start: {error}"))
}

/// What `python3 -I -c script` prints, trimmed, its stdin a pipe.
fn python(script: &str) -> String {
    match Command::new("python3")
        .args(["-I", "-c", script])
        .stdin(Stdio::piped())
        .output()
    {
        Ok(output) => format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!("python3 didn't start: {error}"),
    }
}

/// The case that starts V8 under the box, from the snapshot the binary
/// under test embeds.
#[cfg(feature = "snapshot")]
mod v8 {
    use std::path::Path;

    use gro_runtime::confine::Confinement;
    use gro_runtime::engine::{EngineOptions, init_engine};
    use gro_runtime::snapshot::StartupSnapshot;
    use gro_runtime::{RuntimeOptions, STACK_SIZE, run_main};

    use gro_runtime::confine::plan::{BoxFacts, BoxInputs, BoxPlan};
    use gro_runtime::confine::run::{RunClosed, probe_floor};

    use super::{outcome, prepared, say, wait_for_inspection};

    /// The snapshot the binary under test embeds, as `fgro` embeds it.
    mod embedded {
        pub static SNAPSHOT: &[u8] =
            include_bytes!(concat!(env!("GRO_SNAPSHOT_DIR"), "/snapshot.bin"));
        include!(concat!(env!("GRO_SNAPSHOT_DIR"), "/residual.rs"));
    }

    /// A project whose `node_modules` is a symlink into another checkout,
    /// whose `package.json` is a project's (`linked_named`), a type marker
    /// (`linked_marker`), or none (`linked_absent`), an ancestor's
    /// `package.json` above it either way, run in a box planned for it: the
    /// project's module imports a package there, which imports its
    /// sibling, resolved within that `node_modules`, reading nothing of the
    /// checkout holding it, which the box doesn't grant.
    pub fn linked(case: &str, scratch: &Path) {
        let base = scratch.join(case);
        let holder = match case {
            "linked_named" => Some(r#"{"name": "other", "type": "module"}"#),
            "linked_marker" => Some(r#"{"type": "module"}"#),
            _ => None,
        };
        let mut files = vec![
            ("package.json", r#"{"name": "up", "type": "module"}"#),
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/main.js",
                "import { b } from 'a';\nif (b !== 'b') throw new Error(b);\n",
            ),
            (
                "other/node_modules/a/package.json",
                r#"{"name": "a", "type": "module", "exports": "./i.js"}"#,
            ),
            ("other/node_modules/a/i.js", "export { b } from 'b';\n"),
            (
                "other/node_modules/b/package.json",
                r#"{"name": "b", "type": "module", "exports": "./i.js"}"#,
            ),
            ("other/node_modules/b/i.js", "export const b = 'b';\n"),
        ];
        if let Some(holder) = holder {
            files.push(("other/package.json", holder));
        }
        for (path, text) in files {
            let path = base.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        std::os::unix::fs::symlink(
            base.join("other/node_modules"),
            base.join("project/node_modules"),
        )
        .unwrap();
        let project = std::fs::canonicalize(base.join("project")).unwrap();
        let inputs = BoxInputs {
            root: project.clone(),
            ..BoxInputs::default()
        };
        let plan = BoxPlan::for_run(&inputs, &BoxFacts::read(&inputs).unwrap()).unwrap();
        let confined = RunClosed::close_inherited(probe_floor().unwrap())
            .unwrap()
            .enter_network()
            .unwrap()
            .harden()
            .unwrap()
            .prepare(&plan, None)
            .unwrap()
            .confine()
            .unwrap();
        say("confined");
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                let engine =
                    init_engine(Confinement::Run(confined), &EngineOptions::default()).unwrap();
                let options = RuntimeOptions {
                    snapshot: Some(StartupSnapshot {
                        bytes: embedded::SNAPSHOT,
                        residual_lazy_js: embedded::RESIDUAL_LAZY_JS,
                    }),
                    ..RuntimeOptions::default()
                };
                let outcome_ = run_main(engine, &options, Path::new("main.js"), &project);
                say(&format!(
                    "ran: {}",
                    outcome_
                        .result
                        .as_ref()
                        .map_or_else(|error| format!("{error}"), |()| "ok".to_owned())
                ));
                if let Some(runtime) = outcome_.runtime {
                    runtime.forget();
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    /// The run's box, then the engine on a thread started after it, which
    /// runs the project's module and tries a read outside the box; the
    /// parent then reads every thread V8 started.
    pub fn run(project: &Path, outside: &Path) {
        let confined = prepared(project).confine().unwrap();
        say("confined");
        let project = project.to_owned();
        let outside = outside.to_owned();
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || {
                let engine =
                    init_engine(Confinement::Run(confined), &EngineOptions::default()).unwrap();
                let options = RuntimeOptions {
                    snapshot: Some(StartupSnapshot {
                        bytes: embedded::SNAPSHOT,
                        residual_lazy_js: embedded::RESIDUAL_LAZY_JS,
                    }),
                    ..RuntimeOptions::default()
                };
                let outcome_ = run_main(engine, &options, Path::new("main.js"), &project);
                say(&format!(
                    "ran: {}",
                    outcome_
                        .result
                        .as_ref()
                        .map_or_else(|error| format!("{error}"), |()| "ok".to_owned())
                ));
                say(&format!(
                    "read outside from the runtime's thread: {}",
                    outcome(std::fs::read(&outside))
                ));
                if let Some(runtime) = outcome_.runtime {
                    runtime.forget();
                }
            })
            .unwrap()
            .join()
            .unwrap();
        wait_for_inspection();
    }
}
