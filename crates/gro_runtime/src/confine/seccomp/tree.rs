//! The seccomp tree assembler: a filter that finds a call's answer by a
//! binary search on its number, with the argument checks at the leaves.
//!
//! seccompiler compiles a filter as one comparison per call number, in
//! order, so a call far down its list runs every comparison before it. This
//! assembler emits classic BPF over seccompiler's public [`sock_filter`]
//! instead, from a classification of every call number:
//!
//! 1. The architecture check: a call from any architecture but the one the
//!    classification is for kills the process, as eval's filters do.
//! 2. The call number, loaded once, searched by unsigned `>=` comparisons
//!    over the spans of consecutive numbers that answer alike, so a call
//!    reaches its leaf in about `log2` of the spans' count. Every number
//!    above the highest classified one is one more span, answered by the
//!    caller's choice: the x32 calls (`nr | 0x40000000` on `x86_64`) and
//!    any number a kernel added after the classification land there.
//! 3. At a leaf, the answer, or a chain of checks on the call's arguments.
//!    An argument is compared by its low 32 bits, the half the kernel reads
//!    of an `int` (and of `clone`'s flags), so bits a caller sets above them
//!    change nothing.
//!
//! A conditional jump reaches at most 255 instructions ahead; a target
//! further away is reached through an unconditional jump (`JA`, whose
//! offset is 32 bits) placed beside the condition, or through a copy of the
//! return it jumps to. The program is emitted from its end backward, so
//! every jump's target is placed before the jump and its distance is known.

use seccompiler::sock_filter;

/// `BPF_LD | BPF_W | BPF_ABS`: load the 32-bit word of `seccomp_data` at
/// the offset `k` into the accumulator.
const LOAD: u16 = 0x20;
/// `BPF_ALU | BPF_AND | BPF_K`: and the accumulator with `k`.
const AND: u16 = 0x54;
/// `BPF_JMP | BPF_JA`: jump `k` instructions ahead.
const JUMP: u16 = 0x05;
/// `BPF_JMP | BPF_JEQ | BPF_K`: jump by `jt` when the accumulator equals
/// `k`, by `jf` otherwise.
const EQUAL: u16 = 0x15;
/// `BPF_JMP | BPF_JGE | BPF_K`: jump by `jt` when the accumulator is `k` or
/// above, unsigned, by `jf` otherwise.
const AT_LEAST: u16 = 0x35;
/// `BPF_RET | BPF_K`: return `k`, the filter's answer.
const RETURN: u16 = 0x06;

/// The offset of the call number in `seccomp_data`.
const NR_OFFSET: u32 = 0;
/// The offset of the architecture in `seccomp_data`.
const ARCH_OFFSET: u32 = 4;
/// The offset of the first argument in `seccomp_data`; each takes 8 bytes.
const ARGS_OFFSET: u32 = 16;

/// The most instructions the kernel takes in a filter (`BPF_MAXINSNS`).
const MAX_INSTRUCTIONS: usize = 4096;

/// The farthest a conditional jump reaches, its offsets being a byte.
const MAX_CONDITIONAL: usize = 255;

/// What a filter answers for a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// The call runs.
    Allow,
    /// The call fails with this error number, without reaching the kernel.
    Errno(u32),
    /// The process is killed.
    KillProcess,
}

impl Action {
    /// The filter's return value for this action.
    const fn ret(self) -> u32 {
        match self {
            Self::Allow => libc::SECCOMP_RET_ALLOW,
            Self::Errno(errno) => libc::SECCOMP_RET_ERRNO | (errno & libc::SECCOMP_RET_DATA),
            Self::KillProcess => libc::SECCOMP_RET_KILL_PROCESS,
        }
    }
}

/// How a call number is answered: at once, or by its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// The same answer whatever the arguments.
    Return(Action),
    /// The answer by an argument.
    Match(Box<ArgMatch>),
}

/// A check of one argument: its low 32 bits, masked by `mask`, compared
/// with each case's value in order; the first equal one decides, and none
/// equal, `otherwise` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArgMatch {
    /// The argument's index, 0 to 5.
    pub(crate) arg: u8,
    /// The bits compared; `u32::MAX` compares the whole low half.
    pub(crate) mask: u32,
    /// Each masked value and what it decides.
    pub(crate) cases: Vec<(u32, Decision)>,
    /// What decides when no case's value is equal.
    pub(crate) otherwise: Decision,
}

/// Why a classification couldn't be made or assembled into a filter.
#[derive(Debug, thiserror::Error)]
pub enum TreeError {
    /// A call number is classified twice.
    #[error("the call number {nr} is classified twice")]
    Twice {
        /// The number.
        nr: u32,
    },
    /// A number below the highest classified one has no class.
    #[error("the call number {nr} is below the highest classified one but has no class")]
    Unclassified {
        /// The number.
        nr: u32,
    },
    /// An argument index past the six a call has.
    #[error("a check reads argument {arg}, past the six a call has")]
    Argument {
        /// The index.
        arg: u8,
    },
    /// The filter is longer than the kernel takes.
    #[error("the filter is {len} instructions, past the {MAX_INSTRUCTIONS} the kernel takes")]
    TooLong {
        /// Its length.
        len: usize,
    },
}

/// A run of consecutive call numbers, from `first` to the next span's
/// first (or to `u32::MAX` for the last), that answer alike.
#[derive(Debug)]
struct Span<'a> {
    first: u32,
    decision: &'a Decision,
}

/// Assembles the filter for the architecture whose `seccomp_data` value is
/// `arch`: `calls[nr]` decides the number `nr`, and every number past the
/// last, the x32 calls among them, is answered `above`.
///
/// # Errors
///
/// Returns [`TreeError::Argument`] for a check of an argument past the
/// sixth, and [`TreeError::TooLong`] when the program passes
/// [`MAX_INSTRUCTIONS`].
pub(crate) fn assemble(
    arch: u32,
    calls: &[&Decision],
    above: Action,
) -> Result<Vec<sock_filter>, TreeError> {
    let above = Decision::Return(above);
    let mut spans: Vec<Span<'_>> = Vec::new();
    for (nr, decision) in (0..=u32::MAX).zip(calls) {
        push_span(&mut spans, nr, decision);
    }
    // a classification past `u32::MAX` leaves nothing above it
    if let Ok(past) = u32::try_from(calls.len()) {
        push_span(&mut spans, past, &above);
    }
    let mut emitter = Emitter::default();
    // the search's first instruction is the last placed, so the number's
    // load, placed next, falls through into it
    emitter.search(&spans)?;
    let number = emitter.load(NR_OFFSET);
    let kill = emitter.ret(Action::KillProcess);
    emitter.cond(EQUAL, arch, number, kill);
    emitter.load(ARCH_OFFSET);
    emitter.finish()
}

/// Adds the number `nr` to the last span when it answers alike and without
/// its arguments, or starts a span at it.
fn push_span<'a>(spans: &mut Vec<Span<'a>>, nr: u32, decision: &'a Decision) {
    let joins = matches!(decision, Decision::Return(_))
        && spans.last().is_some_and(|last| last.decision == decision);
    if !joins {
        spans.push(Span {
            first: nr,
            decision,
        });
    }
}

/// An instruction placed by the [`Emitter`], by its index from the
/// program's end.
#[derive(Debug, Clone, Copy)]
struct Label(usize);

/// The program, built from its end backward: `reversed[i]` is the
/// instruction `i` places before the end.
#[derive(Debug, Default)]
struct Emitter {
    reversed: Vec<sock_filter>,
}

impl Emitter {
    /// Places `instruction` before everything placed so far.
    fn push(&mut self, instruction: sock_filter) -> Label {
        self.reversed.push(instruction);
        Label(self.reversed.len() - 1)
    }

    /// How many instructions an instruction placed next would jump over to
    /// reach `target`.
    const fn distance(&self, target: Label) -> usize {
        self.reversed.len() - target.0 - 1
    }

    fn ret(&mut self, action: Action) -> Label {
        self.push(sock_filter {
            code: RETURN,
            jt: 0,
            jf: 0,
            k: action.ret(),
        })
    }

    fn load(&mut self, offset: u32) -> Label {
        self.push(sock_filter {
            code: LOAD,
            jt: 0,
            jf: 0,
            k: offset,
        })
    }

    /// An instruction placed next that reaches `target`, which may be past
    /// a conditional jump's reach: a copy of it when it returns, or an
    /// unconditional jump to it.
    fn reach(&mut self, target: Label) -> Label {
        let placed = &self.reversed[target.0];
        if placed.code == RETURN {
            let copy = placed.clone();
            return self.push(copy);
        }
        let k = self.distance(target);
        self.push(sock_filter {
            code: JUMP,
            jt: 0,
            jf: 0,
            // the program is at most `MAX_INSTRUCTIONS` long by the time
            // it's checked; a longer one fails that check, its jumps unread
            k: u32::try_from(k).unwrap_or(u32::MAX),
        })
    }

    /// A conditional jump `code` on `k`, to `taken` when it holds and to
    /// `not_taken` when it doesn't, each reached through [`Self::reach`]
    /// when it's past a conditional jump's reach.
    fn cond(&mut self, code: u16, k: u32, mut taken: Label, mut not_taken: Label) -> Label {
        loop {
            let (jt, jf) = (self.distance(taken), self.distance(not_taken));
            if jt > MAX_CONDITIONAL {
                taken = self.reach(taken);
            } else if jf > MAX_CONDITIONAL {
                not_taken = self.reach(not_taken);
            } else {
                // both distances are at most `MAX_CONDITIONAL` here, so each
                // fits the byte the instruction holds it in, and the
                // fallback is never taken
                debug_assert!(jt <= MAX_CONDITIONAL && jf <= MAX_CONDITIONAL);
                return self.push(sock_filter {
                    code,
                    jt: u8::try_from(jt).unwrap_or(u8::MAX),
                    jf: u8::try_from(jf).unwrap_or(u8::MAX),
                    k,
                });
            }
        }
    }

    /// The binary search over `spans`, with the call number in the
    /// accumulator: at each step, a number at or above the middle span's
    /// first goes right, any other left.
    fn search(&mut self, spans: &[Span<'_>]) -> Result<Label, TreeError> {
        match spans {
            [] => Ok(self.ret(Action::KillProcess)),
            [span] => self.decision(span.decision),
            _ => {
                let middle = spans.len() / 2;
                let right = self.search(&spans[middle..])?;
                let left = self.search(&spans[..middle])?;
                Ok(self.cond(AT_LEAST, spans[middle].first, right, left))
            }
        }
    }

    fn decision(&mut self, decision: &Decision) -> Result<Label, TreeError> {
        match decision {
            Decision::Return(action) => Ok(self.ret(*action)),
            Decision::Match(check) => self.arg_match(check),
        }
    }

    /// The check `check`: the argument's low half loaded, masked, then a
    /// comparison per case, the first case's first.
    fn arg_match(&mut self, check: &ArgMatch) -> Result<Label, TreeError> {
        if check.arg > 5 {
            return Err(TreeError::Argument { arg: check.arg });
        }
        let otherwise = self.decision(&check.otherwise)?;
        // a case answering as `otherwise` does, or as an earlier-placed
        // case does, jumps to that one's return
        let mut returns: Vec<(Action, Label)> = Vec::new();
        if let Decision::Return(action) = check.otherwise {
            returns.push((action, otherwise));
        }
        let mut targets = Vec::with_capacity(check.cases.len());
        for (_, decision) in check.cases.iter().rev() {
            let shared = match decision {
                Decision::Return(action) => returns
                    .iter()
                    .find(|(placed, _)| placed == action)
                    .map(|(_, label)| *label),
                Decision::Match(_) => None,
            };
            let target = if let Some(label) = shared {
                label
            } else {
                let label = self.decision(decision)?;
                if let Decision::Return(action) = decision {
                    returns.push((*action, label));
                }
                label
            };
            targets.push(target);
        }
        let mut next = otherwise;
        for ((value, _), target) in check.cases.iter().rev().zip(targets) {
            next = self.cond(EQUAL, *value, target, next);
        }
        if check.mask != u32::MAX {
            self.push(sock_filter {
                code: AND,
                jt: 0,
                jf: 0,
                k: check.mask,
            });
        }
        Ok(self.load(arg_low_offset(check.arg)))
    }

    /// The program in order, checked against the kernel's length limit.
    fn finish(mut self) -> Result<Vec<sock_filter>, TreeError> {
        let len = self.reversed.len();
        if len > MAX_INSTRUCTIONS {
            return Err(TreeError::TooLong { len });
        }
        self.reversed.reverse();
        Ok(self.reversed)
    }
}

/// The offset of the low 32 bits of the argument `arg` in `seccomp_data`,
/// whose arguments are 64-bit words in the architecture's byte order.
fn arg_low_offset(arg: u8) -> u32 {
    let word = ARGS_OFFSET + 8 * u32::from(arg);
    if cfg!(target_endian = "big") {
        word + 4
    } else {
        word
    }
}
