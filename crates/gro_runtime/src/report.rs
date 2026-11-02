//! Reports of a failed run: an uncaught error as its message and V8's frames.

use deno_core::error::{CoreError, CoreErrorKind, JsError, JsStackFrame};

use crate::RunError;

/// Renders a failed run: a JS error as its message and stack frames, with
/// each `cause`; anything else by its message chain.
pub fn uncaught(error: &CoreError) -> RunError {
    RunError::Uncaught(report(error))
}

/// The report [`uncaught`] makes of `error`.
pub fn report(error: &CoreError) -> String {
    match &*error.0 {
        CoreErrorKind::Js(error) => report_js(error),
        _ => error.print_with_cause(),
    }
}

/// `Uncaught Error: message`, then V8's frames, each `at
/// file:///path:line:column`, then each `cause` as `Caused by: Error: message`
/// with its frames. A module's top-level throw reads `Uncaught (in promise)`,
/// since evaluating a module settles a promise. A cyclic chain prints its
/// repeated error once more and ends there: `deno_core` converts the repeat
/// without its `cause`, matching objects by identity. The report doesn't mark
/// the repeat, because the converted errors are copies, and errors made at one
/// site (a retry loop's) are equal by value without being the same object.
pub fn report_js(error: &JsError) -> String {
    let mut report = error.exception_message.clone();
    push_frames(&mut report, error);
    let mut next = error.cause.as_deref();
    while let Some(cause) = next {
        report.push_str("\nCaused by: ");
        report.push_str(&describe(cause));
        push_frames(&mut report, cause);
        next = cause.cause.as_deref();
    }
    report
}

/// `Name: message` for an error object, else the thrown value as V8 printed it.
fn describe(error: &JsError) -> String {
    match (&error.name, &error.message) {
        (Some(name), Some(message)) if message.is_empty() => name.clone(),
        (Some(name), Some(message)) => format!("{name}: {message}"),
        _ => error
            .exception_message
            .strip_prefix("Uncaught (in promise) ")
            .or_else(|| error.exception_message.strip_prefix("Uncaught "))
            .unwrap_or(&error.exception_message)
            .to_owned(),
    }
}

/// Appends the error's frames: V8's `at` lines from its stack, or when it has
/// none (a syntax error's stack is its message alone), the location V8
/// reported for it.
fn push_frames(report: &mut String, error: &JsError) {
    let mut pushed = false;
    if let Some(stack) = &error.stack {
        // the stack's first lines repeat the message
        for frame in stack
            .lines()
            .filter(|line| line.trim_start().starts_with("at "))
        {
            report.push('\n');
            report.push_str(frame);
            pushed = true;
        }
    }
    if !pushed
        && let Some(location) = error
            .frames
            .first()
            .and_then(JsStackFrame::maybe_format_location)
    {
        report.push_str("\n    at ");
        report.push_str(&location);
    }
}
