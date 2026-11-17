//! Reports of a failed run: an uncaught error as its message and V8's frames.

use std::error::Error as _;
use std::fmt;

use deno_core::error::{CoreError, CoreErrorKind, JsError, JsStackFrame};

use crate::RunError;

/// A failed run's report, as its lines.
///
/// A line can hold text the program chose, line breaks among it: an error's
/// message, a thrown value, a frame of a stack it set. The lines are the
/// report's own breaks: between a message and its frames, a frame and the
/// next, a `cause` and the frames before it, and, inside a message, before
/// a frame's `at` (a module that failed to strip names its position so),
/// so the only line a program's text can start is a frame's, as its stack
/// can. An embedder that shows the report to a person escapes each line
/// whole, its breaks included, and breaks only between lines. Displayed,
/// the lines join with `\n`, unescaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    lines: Vec<String>,
}

impl Report {
    /// The report's lines, in order, each as the run gave its text.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Appends `line` as a line of its own.
    fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    /// Appends `text`, a message, as lines broken before each frame's `at`
    /// in it, a break followed by spaces and `at `, and nowhere else.
    fn push_message(&mut self, text: &str) {
        let mut line = String::new();
        for (index, part) in text.split('\n').enumerate() {
            if index > 0 {
                if part.trim_start_matches(' ').starts_with("at ") {
                    self.lines.push(std::mem::take(&mut line));
                } else {
                    line.push('\n');
                }
            }
            line.push_str(part);
        }
        self.lines.push(line);
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.lines.join("\n"))
    }
}

impl From<Report> for String {
    fn from(report: Report) -> Self {
        report.to_string()
    }
}

/// Renders a failed run: a JS error as its message and stack frames, with
/// each `cause`; anything else by its message chain.
pub fn uncaught(error: &CoreError) -> RunError {
    RunError::Uncaught(report(error))
}

/// The report [`uncaught`] makes of `error`: [`report_js`]'s of a JS error,
/// else the error's message, then, after a blank line, `Caused by:` and its
/// source's message indented, as `deno_core` prints it.
pub fn report(error: &CoreError) -> Report {
    if let CoreErrorKind::Js(error) = &*error.0 {
        return report_js(error);
    }
    let mut report = Report { lines: Vec::new() };
    report.push_message(&error.to_string());
    if let Some(source) = error.source() {
        report.push("");
        report.push("Caused by:");
        report.push_message(&format!(
            "    {}",
            source.to_string().replace('\n', "\n    ")
        ));
    }
    report
}

/// `Uncaught Error: message`, then V8's frames, each `at
/// file:///path:line:column`, then each `cause` as `Caused by: Error: message`
/// with its frames, each a line of the report. A module's top-level throw
/// reads `Uncaught (in promise)`, since evaluating a module settles a
/// promise. A cyclic chain prints its repeated error once more and ends
/// there: `deno_core` converts the repeat without its `cause`, matching
/// objects by identity. The report doesn't mark the repeat, because the
/// converted errors are copies, and errors made at one site (a retry
/// loop's) are equal by value without being the same object.
pub fn report_js(error: &JsError) -> Report {
    let mut report = Report { lines: Vec::new() };
    report.push_message(&error.exception_message);
    push_frames(&mut report, error);
    let mut next = error.cause.as_deref();
    while let Some(cause) = next {
        report.push_message(&format!("Caused by: {}", describe(cause)));
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
fn push_frames(report: &mut Report, error: &JsError) {
    let mut pushed = false;
    if let Some(stack) = &error.stack {
        // the stack's first lines repeat the message
        for frame in stack
            .lines()
            .filter(|line| line.trim_start().starts_with("at "))
        {
            report.push(frame);
            pushed = true;
        }
    }
    if !pushed
        && let Some(location) = error
            .frames
            .first()
            .and_then(JsStackFrame::maybe_format_location)
    {
        report.push(format!("    at {location}"));
    }
}

#[cfg(test)]
mod tests {
    use super::Report;

    /// A message breaks into lines before each frame's `at`, spaces before
    /// it or none, and its other line breaks stay inside its line, a
    /// break before a tab or text merely starting with `at` among them.
    #[test]
    fn a_message_breaks_only_before_a_frame() {
        let lines = |text: &str| {
            let mut report = Report { lines: Vec::new() };
            report.push_message(text);
            report.lines
        };
        assert_eq!(lines("a"), ["a"]);
        assert_eq!(lines(""), [""]);
        assert_eq!(lines("a\nhint: b\n"), ["a\nhint: b\n"]);
        assert_eq!(
            lines("a\n    at x:1:2\nb\nat y\n\tat z\n  atom"),
            ["a", "    at x:1:2\nb", "at y\n\tat z\n  atom"]
        );
        let mut report = Report { lines: Vec::new() };
        report.push_message("m\n    at f");
        report.push("    at g");
        assert_eq!(report.lines(), ["m", "    at f", "    at g"]);
        assert_eq!(report.to_string(), "m\n    at f\n    at g");
        assert_eq!(String::from(report), "m\n    at f\n    at g");
    }
}
