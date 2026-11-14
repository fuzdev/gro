//! Writes to stdout and stderr that never panic: std's `println!` and
//! `eprintln!` panic when their stream fails (a closed pipe, a full disk).

use std::borrow::Cow;
use std::fmt::Display;
use std::io::Write as _;

/// Writes `line` and a newline to stdout, flushed.
///
/// # Errors
///
/// Returns the write's failure, which the caller reports and exits by.
pub fn stdout_line(line: impl Display) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()
}

/// Writes `line` and a newline to stderr, ignoring a failure, which has
/// nowhere left to be reported.
pub fn stderr_line(line: impl Display) {
    // stderr is unbuffered, so there's nothing to flush
    drop(writeln!(std::io::stderr().lock(), "{line}"));
}

/// `text` with each control character and each bidirectional formatting
/// character written as its `\u{…}` escape, so text a program chose, like a
/// specifier, prints as one line that can't move the cursor, erase or hide
/// a line, or reorder what a terminal shows of the text around it.
pub fn escape_controls(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_escaped) {
        return Cow::Borrowed(text);
    }
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if is_escaped(c) {
            escaped.extend(c.escape_unicode());
        } else {
            escaped.push(c);
        }
    }
    Cow::Owned(escaped)
}

/// Whether [`escape_controls`] escapes `c`: a C0 or C1 control or DEL, or a
/// character of Unicode's `Bidi_Control` property (the marks, embeddings,
/// overrides, and isolates).
const fn is_escaped(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::escape_controls;

    /// Controls and bidirectional formatting characters are escaped, and
    /// text with none is borrowed as it is.
    #[test]
    fn controls_and_bidi_formatting_are_escaped() {
        assert!(matches!(
            escape_controls("cannot import 'x\u{e9}'"),
            Cow::Borrowed(_)
        ));
        assert_eq!(
            escape_controls(
                "a\x1b[2K\r\n\t\u{7f}\u{9b}\u{61c}\u{200e}\u{200f}\u{202a}\u{202e}\u{2066}\u{2069}\u{e9}"
            ),
            "a\\u{1b}[2K\\u{d}\\u{a}\\u{9}\\u{7f}\\u{9b}\\u{61c}\\u{200e}\\u{200f}\\u{202a}\\u{202e}\\u{2066}\\u{2069}\u{e9}"
        );
    }
}
