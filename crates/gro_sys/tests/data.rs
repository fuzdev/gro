//! `limit_data` and `set_default_thread_stack` from inside a child process:
//! each case runs this binary again, the child, which caps its own data
//! segment, or lowers its default thread stack, and reports what it then has.
//! The stack's cases run on glibc only, whose default the call sets and reads;
//! elsewhere it does nothing. A cap is the whole process's and can't be raised
//! again, so it's never set in this process, nor under libtest's harness, whose
//! other tests would run under it.

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
    use std::process::Command;

    use super::support;

    /// The cap the cases set: 1 GiB.
    const CAP: u64 = 1 << 30;

    /// The stack limit the stack's cases start their child under, which
    /// glibc sizes its default thread stack by: 8 MiB.
    #[cfg(target_env = "gnu")]
    const STACK_LIMIT: usize = 8 << 20;

    pub fn main() {
        support::dispatch(parent, child);
    }

    /// Runs each case in a child and checks what it reports.
    fn parent() {
        let cap = CAP;
        let lower = CAP / 2;
        #[cfg_attr(
            not(target_env = "gnu"),
            expect(unused_mut, reason = "the stack's cases are glibc's only")
        )]
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
            _ => unreachable!("no case {case}"),
        }
    }
}
