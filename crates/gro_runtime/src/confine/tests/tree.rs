//! Tests for the seccomp tree assembler, each filter checked as the kernel
//! checks one and read by the BPF interpreter.

use super::bpf::{check_classic, run};
use crate::confine::seccomp::tree::{Action, ArgMatch, Decision, TreeError, assemble};

const ARCH: u32 = 0xc000_003e;
const ALLOW: u32 = libc::SECCOMP_RET_ALLOW;
const KILL: u32 = libc::SECCOMP_RET_KILL_PROCESS;

const fn errno(errno: u32) -> u32 {
    libc::SECCOMP_RET_ERRNO | errno
}

fn ret(action: Action) -> Decision {
    Decision::Return(action)
}

fn check(arg: u8, mask: u32, cases: Vec<(u32, Decision)>, otherwise: Decision) -> Decision {
    Decision::Match(Box::new(ArgMatch {
        arg,
        mask,
        cases,
        otherwise,
    }))
}

/// The filter's return value for `action`, as the kernel reads it.
const fn expected(action: Action) -> u32 {
    match action {
        Action::Allow => ALLOW,
        Action::Errno(number) => errno(number),
        Action::KillProcess => KILL,
    }
}

/// The assembled filter for `calls`, each number's decision by its index.
fn assembled(
    calls: &[Decision],
    above: Action,
) -> Result<Vec<seccompiler::sock_filter>, TreeError> {
    let calls: Vec<&Decision> = calls.iter().collect();
    assemble(ARCH, &calls, above)
}

/// The answer `filter` gives the call `nr` with no arguments.
fn answer(filter: &[seccompiler::sock_filter], nr: u32) -> u32 {
    run(filter, ARCH, nr, [0; 6]).0
}

/// A classification of every number from 0 to 9, the spans and checks the
/// assembler meets: runs of a return, a lone return between them, and a
/// check of an argument's low half, masked, with nested checks and cases
/// sharing a return; every number above 9 fails with `ENOSYS`, the x32
/// numbers among them, and another architecture kills.
#[test]
fn a_small_classification_decides_by_number_and_arguments() {
    let calls = [
        ret(Action::Allow),
        ret(Action::Allow),
        ret(Action::Errno(1)),
        ret(Action::Allow),
        check(
            1,
            u32::MAX,
            vec![(7, ret(Action::Errno(2))), (8, ret(Action::Errno(2)))],
            ret(Action::Allow),
        ),
        check(
            0,
            0xf0,
            vec![(0, ret(Action::Allow))],
            ret(Action::Errno(1)),
        ),
        check(
            0,
            u32::MAX,
            vec![(
                1,
                check(5, 0xf, vec![(2, ret(Action::Allow))], ret(Action::Errno(3))),
            )],
            ret(Action::Errno(4)),
        ),
        ret(Action::KillProcess),
        ret(Action::Errno(38)),
        ret(Action::Errno(38)),
    ];
    let filter = assembled(&calls, Action::Errno(38)).unwrap();
    check_classic(&filter);
    let high = 0xdead_0000_0000_0000_u64;
    let cases: [(u32, [u64; 6], u32); 27] = [
        (0, [0; 6], ALLOW),
        (1, [u64::MAX; 6], ALLOW),
        (2, [0; 6], errno(1)),
        (3, [0; 6], ALLOW),
        // the request, the second argument, by its low half
        (4, [7, 0, 0, 0, 0, 0], ALLOW),
        (4, [0, 7, 0, 0, 0, 0], errno(2)),
        (4, [0, high | 8, 0, 0, 0, 0], errno(2)),
        (4, [0, 7 << 32, 0, 0, 0, 0], ALLOW),
        (4, [0, 9, 0, 0, 0, 0], ALLOW),
        // the first argument's masked bits clear
        (5, [0x0f, 0, 0, 0, 0, 0], ALLOW),
        (5, [high | 0x0f, 0, 0, 0, 0, 0], ALLOW),
        (5, [0x10, 0, 0, 0, 0, 0], errno(1)),
        (5, [0x1f, 0, 0, 0, 0, 0], errno(1)),
        // the first argument, then the sixth's low four bits
        (6, [1, 0, 0, 0, 0, 2], ALLOW),
        (6, [1, 0, 0, 0, 0, 0xf2], ALLOW),
        (6, [high | 1, 0, 0, 0, 0, high | 2], ALLOW),
        (6, [1, 0, 0, 0, 0, 3], errno(3)),
        (6, [2, 0, 0, 0, 0, 2], errno(4)),
        (7, [0; 6], KILL),
        (8, [0; 6], errno(38)),
        (9, [0; 6], errno(38)),
        // above the classification
        (10, [0; 6], errno(38)),
        (11, [0; 6], errno(38)),
        (0x4000_0000, [0; 6], errno(38)),
        (0x4000_0003, [0; 6], errno(38)),
        (0x8000_0000, [0; 6], errno(38)),
        (u32::MAX, [0; 6], errno(38)),
    ];
    for (nr, args, expected) in cases {
        assert_eq!(run(&filter, ARCH, nr, args).0, expected, "{nr} {args:x?}");
    }
    for nr in [0, 3, 10] {
        assert_eq!(run(&filter, ARCH ^ 1, nr, [0; 6]).0, KILL);
        assert_eq!(run(&filter, 0x4000_0003, nr, [0; 6]).0, KILL);
    }
}

/// A classification whose every number answers unlike its neighbors, so
/// the program is far longer than a conditional jump reaches: every jump
/// past that reach goes through an unconditional one or a return's copy,
/// every number still decides by its class, and the search stays
/// logarithmic.
#[test]
fn far_jumps_are_reached_through_unconditional_ones() {
    let class = |nr: u32| match nr % 4 {
        0 => Action::Allow,
        1 => Action::Errno(1),
        2 => Action::Errno(38),
        _ => Action::Errno(nr % 200),
    };
    let calls: Vec<Decision> = (0..1200)
        .map(|nr| {
            if nr % 97 == 0 {
                check(
                    2,
                    u32::MAX,
                    vec![(nr + 1000, ret(Action::Errno(5)))],
                    ret(class(nr)),
                )
            } else {
                ret(class(nr))
            }
        })
        .collect();
    let filter = assembled(&calls, Action::KillProcess).unwrap();
    check_classic(&filter);
    assert!(filter.len() > 2 * 255, "{}", filter.len());
    assert!(
        filter.iter().any(|instruction| instruction.code == 0x05),
        "no unconditional jump"
    );
    let mut deepest = 0;
    for nr in 0..1200 {
        let (answer, steps) = run(&filter, ARCH, nr, [0; 6]);
        assert_eq!(answer, expected(class(nr)), "{nr}");
        deepest = deepest.max(steps);
        if nr % 97 == 0 {
            let (answer, _) = run(&filter, ARCH, nr, [0, 0, u64::from(nr + 1000), 0, 0, 0]);
            assert_eq!(answer, errno(5), "{nr}");
        }
    }
    for nr in [1200, 1201, 0x4000_0000, u32::MAX] {
        assert_eq!(answer(&filter, nr), KILL, "{nr}");
    }
    // the arch check and the number's load, a comparison per level of a
    // search over 1201 spans, an unconditional jump at most per level, and
    // a check's load, comparison, and return
    assert!(deepest <= 3 + 2 * 11 + 3, "{deepest}");
}

/// A classification of a single number answers every number above it by
/// `above`, and an empty one answers every number so.
#[test]
fn the_classification_bounds_the_spans() {
    let filter = assembled(&[ret(Action::Allow)], Action::Errno(38)).unwrap();
    check_classic(&filter);
    assert_eq!(answer(&filter, 0), ALLOW);
    assert_eq!(answer(&filter, 1), errno(38));
    assert_eq!(answer(&filter, u32::MAX), errno(38));
    let filter = assembled(&[], Action::Errno(38)).unwrap();
    check_classic(&filter);
    assert_eq!(answer(&filter, 0), errno(38));
}

/// A check of a seventh argument, and a program longer than the kernel
/// takes, are refused.
#[test]
fn a_classification_that_cant_be_assembled_is_refused() {
    let seventh = [check(6, u32::MAX, Vec::new(), ret(Action::Allow))];
    match assembled(&seventh, Action::Allow) {
        Err(TreeError::Argument { arg: 6 }) => {}
        other => panic!("expected the seventh argument refused, got {other:?}"),
    }
    let long: Vec<Decision> = (0..3000).map(|nr| ret(Action::Errno(nr % 2))).collect();
    match assembled(&long, Action::Allow) {
        Err(TreeError::TooLong { len }) => assert!(len > 4096, "{len}"),
        other => panic!("expected the program refused as too long, got {other:?}"),
    }
}
/// A classification's answer read directly from it, apart from the
/// assembler: the low half of a checked argument, masked, against each
/// case's value in order.
fn read_directly(calls: &[Decision], above: Action, nr: u32, args: &[u64; 6]) -> u32 {
    fn decide(decision: &Decision, args: &[u64; 6]) -> u32 {
        match decision {
            Decision::Return(action) => expected(*action),
            Decision::Match(check) => {
                let low = args[usize::from(check.arg)] & u64::from(u32::MAX);
                let value = u32::try_from(low).unwrap() & check.mask;
                check
                    .cases
                    .iter()
                    .find(|(case, _)| *case == value)
                    .map_or_else(|| decide(&check.otherwise, args), |(_, d)| decide(d, args))
            }
        }
    }
    usize::try_from(nr)
        .ok()
        .and_then(|nr| calls.get(nr))
        .map_or_else(|| expected(above), |decision| decide(decision, args))
}

/// A seeded xorshift, so a failure names a reproducible classification.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
    fn low(&mut self) -> u32 {
        u32::try_from(self.next() >> 32).unwrap()
    }
}

/// A random decision: a return, or a check whose cases share values from
/// `pool` (some outside the mask, which never match), nested to depth 3,
/// and now and then hundreds of cases long, so its jumps pass a
/// conditional jump's reach.
fn random_decision(random: &mut Random, pool: &[u32], depth: u32) -> Decision {
    let action = |random: &mut Random| match random.below(5) {
        0 => Action::Allow,
        1 => Action::KillProcess,
        n => Action::Errno(u32::try_from(n).unwrap()),
    };
    if depth == 3 || random.below(4) != 0 {
        return ret(action(random));
    }
    let mask = match random.below(3) {
        0 => 0xf,
        1 => random.low() | 1,
        _ => u32::MAX,
    };
    let count = if random.below(20) == 0 {
        100 + random.below(200)
    } else {
        random.below(5)
    };
    let cases = (0..count)
        .map(|_| {
            let value = pool[usize::try_from(random.below(pool.len() as u64)).unwrap()];
            let value = if random.below(5) == 0 {
                value
            } else {
                value & mask
            };
            (value, random_decision(random, pool, depth + 1))
        })
        .collect();
    let otherwise = random_decision(random, pool, depth + 1);
    check(
        u8::try_from(random.below(6)).unwrap(),
        mask,
        cases,
        otherwise,
    )
}

/// Random classifications, runs of equal answers among them, each
/// assembled, checked as the kernel checks it, and run for every number,
/// the x32 numbers, and argument vectors aimed at each case's value with
/// random upper halves, against the classification read directly.
#[test]
fn random_classifications_decide_as_read_directly() {
    for seed in 1..=150_u64 {
        let mut random = Random(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let pool: Vec<u32> = (0..=random.below(8)).map(|_| random.low()).collect();
        let len = match random.below(4) {
            0 => random.below(16),
            1 | 2 => random.below(300),
            _ => 300 + random.below(900),
        };
        let mut calls: Vec<Decision> = Vec::new();
        for _ in 0..len {
            let decision = match calls.last() {
                Some(last) if random.below(2) == 0 => last.clone(),
                _ => random_decision(&mut random, &pool, 0),
            };
            calls.push(decision);
        }
        let above = Action::Errno(38);
        let filter = match assembled(&calls, above) {
            Ok(filter) => filter,
            Err(TreeError::TooLong { len }) => {
                assert!(len > 4096, "seed {seed}: {len}");
                continue;
            }
            Err(other) => panic!("seed {seed}: {other}"),
        };
        check_classic(&filter);
        let len = u32::try_from(calls.len()).unwrap();
        let numbers = (0..len + 2).chain([len | 0x4000_0000, 0x4000_0000, u32::MAX]);
        for nr in numbers {
            for _ in 0..6 {
                let args: [u64; 6] = std::array::from_fn(|_| {
                    let low = pool[usize::try_from(random.below(pool.len() as u64)).unwrap()];
                    let low = if random.below(3) == 0 {
                        random.low()
                    } else {
                        low
                    };
                    (random.next() & 0xffff_ffff_0000_0000) | u64::from(low)
                });
                assert_eq!(
                    run(&filter, ARCH, nr, args).0,
                    read_directly(&calls, above, nr, &args),
                    "seed {seed}: {nr:#x} {args:x?}"
                );
            }
        }
        assert_eq!(run(&filter, ARCH ^ 1, 0, [0; 6]).0, KILL, "seed {seed}");
    }
}

/// A check whose every case answers its own error, so each case's jump
/// reaches its return across the other cases' comparisons and returns: 256
/// cases put every such jump at exactly a conditional jump's reach (255
/// instructions over), 257 one past it, where each goes through a copy of
/// its return.
#[test]
fn a_jump_at_a_conditional_jumps_reach_and_one_past_it() {
    for count in [255_u32, 256, 257, 258] {
        let cases = (0..count)
            .map(|value| (value, ret(Action::Errno(value + 1))))
            .collect();
        let calls = [check(0, u32::MAX, cases, ret(Action::Allow))];
        let filter = assembled(&calls, Action::Errno(38)).unwrap();
        check_classic(&filter);
        for value in 0..count {
            let args = [0xdead_0000_0000_0000 | u64::from(value), 0, 0, 0, 0, 0];
            assert_eq!(
                run(&filter, ARCH, 0, args).0,
                errno(value + 1),
                "{count}: {value}"
            );
        }
        assert_eq!(
            run(&filter, ARCH, 0, [u64::from(count), 0, 0, 0, 0, 0]).0,
            ALLOW
        );
    }
}
