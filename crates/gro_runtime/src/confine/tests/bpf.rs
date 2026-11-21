//! A classic BPF interpreter and checker over the seccomp filters the box
//! compiles, so a table can pin a policy as the kernel would run it.

use seccompiler::sock_filter;

const LOAD: u32 = libc::BPF_LD | libc::BPF_W | libc::BPF_ABS;
const AND: u32 = libc::BPF_ALU | libc::BPF_AND | libc::BPF_K;
const JUMP: u32 = libc::BPF_JMP | libc::BPF_JA;
const EQUAL: u32 = libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K;
const ABOVE: u32 = libc::BPF_JMP | libc::BPF_JGT | libc::BPF_K;
const AT_LEAST: u32 = libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K;
const RETURN: u32 = libc::BPF_RET | libc::BPF_K;

/// The size of the kernel's `seccomp_data`, past which a filter may not
/// load.
const SECCOMP_DATA_LEN: u32 = 64;

/// The `seccomp_data` architecture value of the kernel's ABI this test runs on:
/// `AUDIT_ARCH_X86_64` or `AUDIT_ARCH_AARCH64`, the architectures the box is
/// built for.
pub(super) fn audit_arch() -> u32 {
    match std::env::consts::ARCH {
        "x86_64" => 0xc000_003e,
        "aarch64" => 0xc000_00b7,
        other => panic!("no seccomp_data architecture value for {other}"),
    }
}

/// What the kernel would decide for the call `nr` with `args`, from the
/// architecture `arch`, under the compiled `filter`: [`run`]'s return value,
/// for a call number as a `c_long`.
pub(super) fn decide(filter: &[sock_filter], arch: u32, nr: libc::c_long, args: [u64; 6]) -> u32 {
    let nr = i32::try_from(nr).unwrap().cast_unsigned();
    run(filter, arch, nr, args).0
}

/// What the kernel would decide for the call `nr` with `args`, from the
/// architecture `arch`, under the compiled `filter`: a classic BPF
/// interpreter over the instructions seccompiler and the tree assembler
/// emit, run on the `seccomp_data` the kernel hands a filter (`nr` and
/// `arch`, 32 bits each, the instruction pointer, then the six arguments,
/// 64 bits each, little-endian on both architectures). Returns the
/// filter's return value, its action and data, and how many instructions
/// ran to reach it. An instruction it doesn't know panics, never passes.
pub(super) fn run(filter: &[sock_filter], arch: u32, nr: u32, args: [u64; 6]) -> (u32, usize) {
    let mut data = [0_u8; 64];
    data[..4].copy_from_slice(&nr.to_le_bytes());
    data[4..8].copy_from_slice(&arch.to_le_bytes());
    for (slot, arg) in data[16..].as_chunks_mut::<8>().0.iter_mut().zip(args) {
        slot.copy_from_slice(&arg.to_le_bytes());
    }
    let (mut pc, mut accumulator, mut steps) = (0, 0_u32, 0);
    loop {
        let instruction = &filter[pc];
        pc += 1;
        steps += 1;
        let k = instruction.k;
        let taken = |taken: bool| {
            usize::from(if taken {
                instruction.jt
            } else {
                instruction.jf
            })
        };
        match u32::from(instruction.code) {
            LOAD => {
                let at = usize::try_from(k).unwrap();
                accumulator = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
            }
            AND => accumulator &= k,
            JUMP => pc += usize::try_from(k).unwrap(),
            EQUAL => pc += taken(accumulator == k),
            ABOVE => pc += taken(accumulator > k),
            AT_LEAST => pc += taken(accumulator >= k),
            RETURN => return (k, steps),
            code => panic!("an instruction the evaluator doesn't know: {code:#x}"),
        }
    }
}

/// Checks `filter` as the kernel does before installing it (its classic
/// BPF checker and seccomp's own): a length from 1 to 4096, only the
/// instructions [`run`] knows, every jump landing inside the program, every
/// load an aligned word of `seccomp_data`, and a return last.
pub(super) fn check_classic(filter: &[sock_filter]) {
    let len = filter.len();
    assert!((1..=4096).contains(&len), "the filter's length is {len}");
    for (pc, instruction) in filter.iter().enumerate() {
        let k = instruction.k;
        let lands = |offset: usize| pc + 1 + offset < len;
        match u32::from(instruction.code) {
            LOAD => assert!(
                k < SECCOMP_DATA_LEN && k % 4 == 0,
                "{pc}: a load at {k}, outside seccomp_data or unaligned"
            ),
            AND | RETURN => {}
            JUMP => assert!(
                lands(usize::try_from(k).unwrap()),
                "{pc}: a jump past the end"
            ),
            EQUAL | ABOVE | AT_LEAST => assert!(
                lands(usize::from(instruction.jt)) && lands(usize::from(instruction.jf)),
                "{pc}: a conditional jump past the end"
            ),
            code => panic!("{pc}: an instruction the checker doesn't know: {code:#x}"),
        }
    }
    assert_eq!(
        u32::from(filter[len - 1].code),
        RETURN,
        "the filter doesn't end in a return"
    );
}
