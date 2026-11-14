//! The native half of the exit host module: the exit code a program sets,
//! the check of a code it gives, and the step that ends the run.
//!
//! `exit` ends the run where it's called: no `finally` block runs, no
//! queued job, and nothing after the call. Stopping V8 doesn't give that.
//! `terminate_execution` takes effect at V8's next stack check, so the
//! code after the call runs until one; a check forced from the op stops
//! that job, but `deno_core`'s event loop goes on after a termination
//! inside a microtask checkpoint and runs the timers that come due; and a
//! terminated run reports as an `Error` a program could have thrown
//! itself. So the op ends the process, and since a library doesn't end its
//! embedder's process on its own, it does so by calling the step the
//! embedder gave ([`RuntimeOptions::exit_step`](crate::RuntimeOptions::exit_step)),
//! which a run that grants `exit` can't start without: no embedder that
//! didn't grant `exit` is ever exited.
//!
//! A code is checked here, natively, as the value it is: a number that is
//! an integer from 0 to 255. Nothing converts it, so no code of the
//! caller's runs (a thenable, a `Symbol.toPrimitive`, a boxed number, and
//! a string are each refused as they are), and nothing truncates it.

use std::cell::Cell;

use deno_core::v8;

use crate::RefusedImports;
use crate::loader::Refusals;

/// What the `TypeError` for a code that isn't one says, the Node module's
/// message.
pub const NOT_A_CODE: &str = "an exit code is an integer from 0 to 255";

/// What a program's `exit` asks of the embedder: to end the process now.
#[derive(Debug)]
pub struct Exit {
    /// The code to exit with, as the program gave it, or the one it set
    /// with `set_exit_code`, or 0.
    pub code: u8,
    /// The `import()`s refused by rule so far, which fail the run as they
    /// do one that ends on its own ([`RunOutcome`](crate::RunOutcome)).
    pub refused: RefusedImports,
}

/// The step that ends the process for a program's `exit`, given by the
/// embedder ([`RuntimeOptions::exit_step`](crate::RuntimeOptions::exit_step)).
///
/// It's called inside the host op, on the runtime's thread, with the
/// program's frames still on the stack, so it can't return or unwind: it
/// ends the process.
pub type ExitStep = fn(&Exit) -> !;

/// The exit host module's state for a run that grants it, in the runtime's
/// `OpState`.
pub struct ExitState {
    /// The code `set_exit_code` last set, for a run that ends on its own
    /// and for an `exit` given none.
    code: Cell<Option<u8>>,
    /// The step `exit` ends the process by.
    step: ExitStep,
}

impl ExitState {
    /// The state of a run whose `exit` ends the process by `step`, no code
    /// set.
    pub const fn new(step: ExitStep) -> Self {
        Self {
            code: Cell::new(None),
            step,
        }
    }

    /// The code `set_exit_code` last set, if it was called.
    pub const fn code(&self) -> Option<u8> {
        self.code.get()
    }

    /// Sets the code the run exits with when it ends on its own.
    pub fn set_code(&self, code: u8) {
        self.code.set(Some(code));
    }

    /// Ends the process by the embedder's step, given `code` or none: with
    /// the code given, else the one set, else 0, and the refusals the run
    /// recorded so far, `refusals`.
    pub fn exit(&self, code: Option<u8>, refusals: &Refusals) -> ! {
        (self.step)(&Exit {
            code: code.or_else(|| self.code()).unwrap_or(0),
            refused: refusals.take(),
        })
    }
}

/// The exit code `value` is: a number, as it is, equal to an integer from
/// 0 to 255 (`-0` among them, as `0`). No conversion is asked of the
/// value, so an object of any kind is no code, a boxed number included,
/// and no code of a caller's runs.
pub fn to_exit_code(value: v8::Local<'_, v8::Value>) -> Option<u8> {
    let number = v8::Local::<v8::Number>::try_from(value).ok()?.value();
    // every code is an `f64` exactly, so the one equal to the number is
    // it, with no cast that could truncate or wrap
    #[expect(
        clippy::float_cmp,
        reason = "an integer from 0 to 255 is exact as an f64"
    )]
    (0..=u8::MAX).find(|code| f64::from(*code) == number)
}
