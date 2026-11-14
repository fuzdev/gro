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
//! embedder's process on its own, it does so by calling a step the
//! embedder gave with the grant ([`ExitGrant`](crate::ExitGrant)): no
//! embedder that didn't grant `exit` is ever exited.
//!
//! A code is checked here, natively, as the value it is: a number that is
//! an integer from 0 to 255. Nothing converts it, so no code of the
//! caller's runs (a thenable, a `Symbol.toPrimitive`, a boxed number, and
//! a string are each refused as they are), and nothing truncates it.

use std::cell::Cell;
use std::rc::Rc;

use deno_core::v8;

use crate::RefusedImports;
use crate::loader::Loader;

/// What the `TypeError` for a code that isn't one says, the Node module's
/// message.
const NOT_A_CODE: &str = "an exit code is an integer from 0 to 255";

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
/// embedder with the grant.
///
/// It's called inside the host op, on the runtime's thread, with the
/// program's frames still on the stack, so it can't return or unwind: it
/// ends the process.
pub type ExitStep = fn(&Exit) -> !;

/// The exit host module's state for a run, in the runtime's `OpState`.
pub struct ExitState {
    /// The code `set_exit_code` last set, for a run that ends on its own
    /// and for an `exit` given none.
    code: Cell<Option<u8>>,
    /// The run's loader, which holds the `import()`s refused by rule.
    loader: Rc<Loader>,
}

impl ExitState {
    /// The state of a run over `loader`, no code set.
    pub const fn new(loader: Rc<Loader>) -> Self {
        Self {
            code: Cell::new(None),
            loader,
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

    /// What `exit` asks of the embedder, given `code` or none: the code
    /// given, else the one set, else 0, with the refusals so far.
    pub fn exit(&self, code: Option<u8>) -> Exit {
        Exit {
            code: code.or_else(|| self.code()).unwrap_or(0),
            refused: self.loader.take_refused(),
        }
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

/// Throws the `TypeError` for a value that is no exit code: the realm's
/// own, made by V8 from its intrinsic, whatever `globalThis.TypeError` has
/// become.
pub fn throw_not_a_code(scope: &v8::PinScope<'_, '_>) {
    if let Some(message) = v8::String::new(scope, NOT_A_CODE) {
        let error = v8::Exception::type_error(scope, message);
        scope.throw_exception(error);
    }
}
