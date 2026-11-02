//! What the harness-off test binaries share: the dispatch between the parent
//! and its child, a child run bounded in time, and the check of a case's
//! output.

#![allow(dead_code, reason = "each test binary uses only part of this module")]
// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]
#![expect(clippy::panic, reason = "a hung child fails its test by panicking")]

use std::io::{BufRead as _, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long a child may run before it's killed and its case fails, generous
/// for a debug build on a loaded machine.
pub const DEADLINE: Duration = Duration::from_secs(60);

/// Runs `child` with its case, the argument after `child`, or empty when
/// there is none, when this binary was started as the child, else `parent`.
/// libtest's own flags (`--test-threads`, a filter) are ignored.
pub fn dispatch(parent: impl FnOnce(), child: impl FnOnce(&str)) {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("child") => child(&args.next().unwrap_or_default()),
        _ => parent(),
    }
}

/// Runs `command`, the child for `case`, bounded by the [`DEADLINE`], and
/// checks that it succeeded with `expected` as its stdout, then reports the
/// case as libtest would.
pub fn run_case(case: &str, command: Command, expected: &str) {
    let output = run_bounded(case, command);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout == expected,
        "{case}: {}\nstdout: {stdout}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    println!("test {case} ... ok");
}

/// Runs `command` with stdout and stderr read on threads of their own while
/// it runs, so output larger than a pipe's buffer can't block it, killing it
/// at the [`DEADLINE`] and failing, naming `case`, so a child that never ends
/// fails its case instead of hanging the suite.
pub fn run_bounded(case: &str, mut command: Command) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > DEADLINE {
            child.kill().unwrap();
            child.wait().unwrap();
            // not joined: a grandchild still holding a pipe would block the
            // drains past the deadline
            panic!("{case}: killed after {:?}: {command:?}", start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

/// Reads the first line of `child`'s stdout, which it takes, waiting no longer
/// than the [`DEADLINE`]: past it the child is killed and `case` fails.
pub fn read_line_bounded(case: &str, child: &mut Child) -> String {
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    // the thread outlives a timeout only until the kill closes the pipe
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = send.send(result);
    });
    if let Ok(line) = receive.recv_timeout(DEADLINE) {
        return line.unwrap();
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("{case}: the child reported nothing in {DEADLINE:?}");
}

/// Waits for `child` to exit, no longer than the [`DEADLINE`]: past it the
/// child is killed and `case` fails.
pub fn wait_bounded(case: &str, child: &mut Child) -> ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if start.elapsed() > DEADLINE {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("{case}: the child outlived {DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Reads `pipe` to its end on a thread of its own.
fn drain(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}
