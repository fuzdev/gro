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
/// can. A frame is a line up to the next frame's, so a break in its own
/// text (a module's file name holding one) stays inside it, unless what
/// follows it reads as a frame's start (four spaces and `at `), as in a
/// stack V8 formats. An embedder
/// that shows the report to a person escapes each line whole, its breaks
/// included, and breaks only between lines. Displayed, the lines join with
/// `\n`, unescaped.
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

/// Appends the error's frames: those of its stack ([`push_stack`]), or when
/// it has none (a syntax error's stack is its message alone), the location
/// V8 reported for it.
fn push_frames(report: &mut Report, error: &JsError) {
    let pushed = error.stack.as_deref().is_some_and(|stack| {
        push_stack(
            report,
            stack,
            &header(error.name.as_deref(), error.message.as_deref()),
        )
    });
    if !pushed
        && let Some(location) = error
            .frames
            .first()
            .and_then(JsStackFrame::maybe_format_location)
    {
        report.push(format!("    at {location}"));
    }
}

/// What starts each frame of a stack V8 formatted, after the line before.
const FRAME_BREAK: &str = "\n    at ";

/// Appends the frames of `stack`, an error's, whose header V8 would format
/// as `header`, returning whether it appended any.
///
/// A stack V8 formatted (`stack.rs`) is its header, then each frame after
/// [`FRAME_BREAK`]. The header repeats the message, which the report has
/// shown, a frame it names (a strip's position) included, so it's skipped;
/// and each frame runs to the next, so a line break in its text stays in
/// its line. A stack that doesn't start with the header (a program's own
/// `.stack` or `Error.prepareStackTrace` text can start otherwise) gives
/// its lines starting with `at`, and none of the rest.
fn push_stack(report: &mut Report, stack: &str, header: &str) -> bool {
    let mut pushed = false;
    match stack
        .strip_prefix(header)
        .filter(|rest| rest.is_empty() || rest.starts_with('\n'))
    {
        Some(frames) => {
            for frame in frames.split(FRAME_BREAK).skip(1) {
                report.push(format!("    at {frame}"));
                pushed = true;
            }
        }
        None => {
            for frame in stack
                .lines()
                .filter(|line| line.trim_start().starts_with("at "))
            {
                report.push(frame);
                pushed = true;
            }
        }
    }
    pushed
}

/// The header V8 formats of an error's stack, by its `name` and `message`,
/// as `stack.rs` makes it: `name: message`, `name` or `message` alone when
/// the other is empty, and `Error` for a missing `name`.
fn header(name: Option<&str>, message: Option<&str>) -> String {
    let name = name.unwrap_or("Error");
    let message = message.unwrap_or_default();
    if name.is_empty() {
        message.to_owned()
    } else if message.is_empty() {
        name.to_owned()
    } else {
        format!("{name}: {message}")
    }
}

#[cfg(test)]
mod tests {
    use super::{Report, header, push_stack};

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

    /// A stack that starts with its header gives each frame up to the
    /// next, a line break in one kept in its line, and skips the header,
    /// frames its message names included; any other stack gives its lines
    /// starting with `at`, and no other text of it.
    #[test]
    fn a_stack_gives_its_frames_after_its_header() {
        let frames = |stack: &str, header: &str| {
            let mut report = Report { lines: Vec::new() };
            let pushed = push_stack(&mut report, stack, header);
            assert_eq!(pushed, !report.lines.is_empty(), "{stack}");
            report.lines
        };
        let header = "TypeError: m\n    at file:///a.ts:1:2";
        assert_eq!(
            frames(&format!("{header}\n    at async file:///b.js:1:1"), header),
            ["    at async file:///b.js:1:1"]
        );
        assert!(frames(header, header).is_empty());
        assert_eq!(
            frames(
                "Error: m\n    at file:///odd\nhint: x.js:1:7\n    at f (file:///b.js:2:3)",
                "Error: m"
            ),
            [
                "    at file:///odd\nhint: x.js:1:7",
                "    at f (file:///b.js:2:3)"
            ]
        );
        assert_eq!(
            frames("Error: m\nnot a frame\n    at x", "Error: m"),
            ["    at x"]
        );
        assert_eq!(
            frames("Error: mm\n    at x\nnot a frame", "Error: m"),
            ["    at x"]
        );
        assert_eq!(
            frames("set\n    at x\nhint: not a frame\n  at y", "Error: m"),
            ["    at x", "  at y"]
        );
        assert!(frames("Error: other", "Error: m").is_empty());
    }

    /// A header is `name: message`, either alone when the other is empty,
    /// `Error` for a missing name, and nothing when both are empty.
    #[test]
    fn a_header_is_formatted_as_v8_s_stack_starts() {
        assert_eq!(header(Some("TypeError"), Some("m")), "TypeError: m");
        assert_eq!(header(None, Some("m")), "Error: m");
        assert_eq!(header(None, None), "Error");
        assert_eq!(header(Some("E"), Some("")), "E");
        assert_eq!(header(Some(""), Some("m")), "m");
        assert_eq!(header(Some(""), None), "");
    }
}
