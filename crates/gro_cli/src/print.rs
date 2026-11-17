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

/// `text` with each character [`is_escaped`] names written as its `\u{…}`
/// escape, so text a program chose, like a specifier, prints as one line
/// that can't move the cursor, erase or hide a line, break one where a
/// terminal or a reader would, hide itself, or reorder what a terminal
/// shows of the text around it.
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

/// `argument`, a command-line argument a usage error names, as it's safe
/// to print: up to its first `=`, what follows marked cut with `…`, since
/// it can be a value (`--allow-env=TOKEN=…`), escaped
/// ([`escape_controls`]).
pub fn echo_argument(argument: &str) -> String {
    match argument.split_once('=') {
        Some((head, _)) => format!("{}=…", escape_controls(head)),
        None => escape_controls(argument).into_owned(),
    }
}

/// Whether [`escape_controls`] escapes `c`: a C0 or C1 control or DEL
/// (Unicode's `Cc`), the line or the paragraph separator (`Zl`, `Zp`), or a
/// format character (`Cf`: the soft hyphen, the zero-width characters, the
/// byte order mark, every `Bidi_Control` character, the tags, and the
/// rest).
///
/// The `Cf` ranges are Unicode 17.0's (`DerivedGeneralCategory.txt`, the
/// same since 15.0), kept by hand, since `std` doesn't know a character's
/// category: a format character a later version adds prints as it is until
/// it's listed here.
pub const fn is_escaped(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}'
                | '\u{2029}'
                | '\u{ad}'
                | '\u{600}'..='\u{605}'
                | '\u{61c}'
                | '\u{6dd}'
                | '\u{70f}'
                | '\u{890}'..='\u{891}'
                | '\u{8e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{echo_argument, escape_controls, is_escaped};

    /// An argument a usage error names prints up to its first `=`, escaped,
    /// and whole, escaped, when it holds none.
    #[test]
    fn an_echoed_argument_stops_at_its_first_equals_sign() {
        assert_eq!(echo_argument("--x"), "--x");
        assert_eq!(echo_argument("--allow_env=TOKEN=hunter2"), "--allow_env=…");
        assert_eq!(echo_argument("-A=\u{1b}[2J"), "-A=…");
        assert_eq!(echo_argument("=v"), "=…");
        assert_eq!(echo_argument("-\u{1b}[2J\u{202e}"), "-\\u{1b}[2J\\u{202e}");
        assert_eq!(echo_argument("--\u{1b}x=\u{1b}secret"), "--\\u{1b}x=…");
    }

    /// Controls, bidirectional formatting characters among the format
    /// characters, and the line and paragraph separators are escaped, and
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

    /// Each end of each range of format characters (`Cf`) is escaped, and
    /// the characters beside it, which aren't format characters, aren't, so
    /// a range dropped or cut short fails, as one grown past its end does;
    /// and a separator is escaped where the space beside it isn't.
    #[test]
    fn every_format_character_and_separator_is_escaped() {
        // each range's ends, as Unicode lists them
        const FORMAT: [(u32, u32); 21] = [
            (0xad, 0xad),
            (0x600, 0x605),
            (0x61c, 0x61c),
            (0x6dd, 0x6dd),
            (0x70f, 0x70f),
            (0x890, 0x891),
            (0x8e2, 0x8e2),
            (0x180e, 0x180e),
            (0x200b, 0x200f),
            (0x202a, 0x202e),
            (0x2060, 0x2064),
            (0x2066, 0x206f),
            (0xfeff, 0xfeff),
            (0xfff9, 0xfffb),
            (0x110bd, 0x110bd),
            (0x110cd, 0x110cd),
            (0x13430, 0x1343f),
            (0x1bca0, 0x1bca3),
            (0x1d173, 0x1d17a),
            (0xe0001, 0xe0001),
            (0xe0020, 0xe007f),
        ];
        let char_of = |code: u32| char::from_u32(code).unwrap();
        let in_format = |code: u32| {
            FORMAT
                .iter()
                .any(|&(start, end)| (start..=end).contains(&code))
        };
        for (start, end) in FORMAT {
            for code in [start, end] {
                assert!(is_escaped(char_of(code)), "U+{code:04X}");
            }
            for code in [start - 1, end + 1] {
                // U+2029 sits before U+202A, and is a separator
                if !in_format(code) && code != 0x2029 {
                    assert!(!is_escaped(char_of(code)), "U+{code:04X}");
                }
            }
        }
        for c in ['\u{2028}', '\u{2029}'] {
            assert!(is_escaped(c), "{c:?}");
        }
        for c in ['\u{2027}', '\u{202f}', '\u{3000}', ' '] {
            assert!(!is_escaped(c), "{c:?}");
        }
        assert_eq!(
            escape_controls("a\u{2028}b\u{2029}c\u{200b}d\u{ad}e\u{feff}f\u{e0041}"),
            "a\\u{2028}b\\u{2029}c\\u{200b}d\\u{ad}e\\u{feff}f\\u{e0041}"
        );
    }
}
