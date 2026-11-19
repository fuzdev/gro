//! `limit_data`, `set_default_thread_stack`, and `raise_open_files_limit`
//! from inside a child process: each case runs this binary again, the
//! child, which caps its own data segment, lowers its default thread stack,
//! or raises its soft limit on open descriptors, and reports what it then
//! has.
//! The stack's cases run on glibc only, whose default the call sets and reads;
//! elsewhere it does nothing. A cap is the whole process's and can't be raised
//! again, so it's never set in this process, nor under libtest's harness, whose
//! other tests would run under it.

#![expect(clippy::unwrap_used, reason = "a test's setup and its failures panic")]

mod support;

use std::process::Command;

/// The cap the cases set: 1 GiB.
const CAP: u64 = 1 << 30;

/// The soft limit on open descriptors the first open-files case starts its
/// child under, a shell's common default.
const OPEN_FILES_SOFT: u64 = 1024;

/// The hard limit on open descriptors the open-files cases start their
/// child under, which an unprivileged `prlimit` may lower it to.
const OPEN_FILES_HARD: u64 = 4096;

/// The stack limit the stack's cases start their child under, which
/// glibc sizes its default thread stack by: 8 MiB.
#[cfg(target_env = "gnu")]
const STACK_LIMIT: usize = 8 << 20;

fn main() {
    support::dispatch(parent, child);
}

/// Runs each case in a child and checks what it reports.
fn parent() {
    let cap = CAP;
    let lower = CAP / 2;
    let mut cases = vec![
        (
            "caps",
            None,
            format!("set: {cap}\nlimit: {cap} {cap}\npast: refused\nwithin: ok\n"),
        ),
        (
            "lower",
            None,
            format!("set: {lower}\nset: {lower}\nlimit: {lower} {lower}\n"),
        ),
        // the parent's lower soft limit, under an unlimited hard one,
        // set by util-linux's `prlimit` before it runs the child
        (
            "soft",
            Some(format!("--data={lower}:unlimited")),
            format!("set: {lower}\nlimit: {lower} {lower}\n"),
        ),
    ];
    cases.extend([
        // a soft limit below the hard one is raised to it
        (
            "open_files",
            Some(format!("--nofile={OPEN_FILES_SOFT}:{OPEN_FILES_HARD}")),
            format!(
                "before: {OPEN_FILES_SOFT} {OPEN_FILES_HARD}\nraised: {OPEN_FILES_HARD}\nafter: \
                 {OPEN_FILES_HARD} {OPEN_FILES_HARD}\n"
            ),
        ),
        // one at the hard limit stays
        (
            "open_files",
            Some(format!("--nofile={OPEN_FILES_HARD}:{OPEN_FILES_HARD}")),
            format!(
                "before: {OPEN_FILES_HARD} {OPEN_FILES_HARD}\nraised: {OPEN_FILES_HARD}\nafter: \
                 {OPEN_FILES_HARD} {OPEN_FILES_HARD}\n"
            ),
        ),
    ]);
    #[cfg(target_env = "gnu")]
    {
        let limit = STACK_LIMIT;
        let half = STACK_LIMIT / 2;
        cases.extend([
            // a smaller size lowers the default glibc took from the limit
            (
                "stack_lower",
                Some(format!("--stack={limit}:{limit}")),
                format!("before: {limit}\nafter: {half}\n"),
            ),
            // a larger one never raises it
            (
                "stack_higher",
                Some(format!("--stack={limit}:{limit}")),
                format!("before: {limit}\nafter: {limit}\n"),
            ),
            // under an unlimited limit, glibc's own default, whatever
            // its size, is lowered to the limit's or kept
            (
                "stack_unlimited",
                Some("--stack=unlimited:unlimited".to_owned()),
                "lowered only: true\n".to_owned(),
            ),
        ]);
    }
    let exe = std::env::current_exe().unwrap();
    for (case, limit, expected) in cases {
        let mut command = limit.map_or_else(
            || Command::new(&exe),
            |limit| {
                let mut command = Command::new("prlimit");
                command.arg(limit).arg(&exe);
                command
            },
        );
        command.args(["child", case]);
        support::run_case(case, command, &expected);
    }
}

/// Runs `case` in this process, the child.
fn child(case: &str) {
    match case {
        // soft and hard both at the cap, an allocation past it refused
        // and one well within it made
        "caps" => {
            println!("set: {}", gro_sys::limit_data(CAP).unwrap());
            let (soft, hard) = gro_sys::data_limit().unwrap();
            println!("limit: {soft} {hard}");
            let mut past: Vec<u8> = Vec::new();
            let past_len = usize::try_from(CAP).unwrap();
            println!(
                "past: {}",
                if past.try_reserve_exact(past_len).is_err() {
                    "refused"
                } else {
                    "made"
                }
            );
            let mut within: Vec<u8> = Vec::new();
            within.try_reserve_exact(past_len / 8).unwrap();
            println!("within: ok");
        }
        // a lower limit stands: the cap is never raised
        "lower" => {
            println!("set: {}", gro_sys::limit_data(CAP / 2).unwrap());
            println!("set: {}", gro_sys::limit_data(CAP).unwrap());
            let (soft, hard) = gro_sys::data_limit().unwrap();
            println!("limit: {soft} {hard}");
        }
        // a lower soft limit stands, though the hard one is higher
        "soft" => {
            println!("set: {}", gro_sys::limit_data(CAP).unwrap());
            let (soft, hard) = gro_sys::data_limit().unwrap();
            println!("limit: {soft} {hard}");
        }
        // the default a thread started without a size gets, before and
        // after a smaller size is set, and a larger one
        #[cfg(target_env = "gnu")]
        "stack_lower" | "stack_higher" => {
            let before = gro_sys::default_thread_stack().unwrap();
            println!("before: {before}");
            let bytes = if case == "stack_lower" {
                STACK_LIMIT / 2
            } else {
                STACK_LIMIT * 2
            };
            gro_sys::set_default_thread_stack(bytes).unwrap();
            println!("after: {}", gro_sys::default_thread_stack().unwrap());
        }
        #[cfg(target_env = "gnu")]
        "stack_unlimited" => {
            let before = gro_sys::default_thread_stack().unwrap();
            gro_sys::set_default_thread_stack(STACK_LIMIT).unwrap();
            let after = gro_sys::default_thread_stack().unwrap();
            println!("lowered only: {}", after == before.min(STACK_LIMIT));
        }
        // the soft and hard limits on open descriptors, before and after
        "open_files" => {
            let (soft, hard) = open_files_limit();
            println!("before: {soft} {hard}");
            println!("raised: {}", gro_sys::raise_open_files_limit().unwrap());
            let (soft, hard) = open_files_limit();
            println!("after: {soft} {hard}");
        }
        _ => unreachable!("no case {case}"),
    }
}

/// The process's soft and hard limits on open descriptors, from
/// `/proc/self/limits`, `gro_sys` having no reader of them.
fn open_files_limit() -> (String, String) {
    let limits = std::fs::read_to_string("/proc/self/limits").unwrap();
    let line = limits
        .lines()
        .find(|line| line.starts_with("Max open files"))
        .unwrap();
    let mut fields = line["Max open files".len()..].split_whitespace();
    (
        fields.next().unwrap().to_owned(),
        fields.next().unwrap().to_owned(),
    )
}
