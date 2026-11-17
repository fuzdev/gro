//! `clear_env_except` from inside a child process: each case runs this binary
//! again, the child, with an environment the parent sets, and the child
//! reports the environment it then has. Changing the environment is only
//! sound with one thread, so it never happens in this process, nor under
//! libtest's harness, whose threads would race it.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]

mod support;

use std::process::Command;

fn main() {
    support::dispatch(parent, child);
}

/// Runs each case in a child and checks what it reports.
fn parent() {
    let exe = std::env::current_exe().unwrap();
    let set = [
        ("TZ", "UTC"),
        ("LC_ALL", "tr_TR.UTF-8"),
        ("DENO_STARTUP_PHASES", "1"),
    ];
    for (case, env, expected) in [
        ("clears", &set[..], "cleared: ok\nafter: TZ=UTC\n"),
        ("unset", &set[1..], "cleared: ok\nafter: \n"),
        (
            "threads",
            &set[..],
            "refused: the process has 2 threads, not 1, so another may read the \
             environment\nafter: DENO_STARTUP_PHASES=1 LC_ALL=tr_TR.UTF-8 TZ=UTC\n",
        ),
        (
            "names",
            &set[..],
            "refused: \"A=B\" can't name an environment variable\nafter: \
             DENO_STARTUP_PHASES=1 LC_ALL=tr_TR.UTF-8 TZ=UTC\n",
        ),
    ] {
        let mut command = Command::new(&exe);
        command
            .args(["child", case])
            .env_clear()
            .envs(env.iter().copied());
        support::run_case(case, command, expected);
    }
}

/// The process's environment, sorted, as `NAME=value` pairs.
fn environment() -> String {
    let mut vars: Vec<String> = std::env::vars_os()
        .map(|(name, value)| format!("{}={}", name.display(), value.display()))
        .collect();
    vars.sort();
    vars.join(" ")
}

/// Runs `case` in this process, the child.
fn child(case: &str) {
    match case {
        "clears" | "unset" => {
            gro_sys::clear_env_except(&["TZ"]).unwrap();
            println!("cleared: ok");
            println!("after: {}", environment());
        }
        "threads" => {
            let (stop, wait) = std::sync::mpsc::channel::<()>();
            let other = std::thread::spawn(move || wait.recv());
            let refused = gro_sys::clear_env_except(&["TZ"]).unwrap_err();
            println!("refused: {refused}");
            stop.send(()).unwrap();
            other.join().unwrap().unwrap();
            println!("after: {}", environment());
        }
        "names" => {
            let refused = gro_sys::clear_env_except(&["TZ", "A=B"]).unwrap_err();
            println!("refused: {refused}");
            println!("after: {}", environment());
        }
        _ => unreachable!("no case {case}"),
    }
}
