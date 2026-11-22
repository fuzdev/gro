//! The entry the binary generates for one run of a command: the module a
//! verb's run starts from, held in memory and never written to disk.
//!
//! It imports the host modules every verb run grants (`fs`, `env`, `spawn`,
//! `stdout`, `stderr`, `args`, and `exit` for `set_exit_code`), the
//! grant-free `cwd`, and Gro's `binary_main`, and makes one call, handing it
//! those, an `import_url` written here so a project module's `import()`
//! runs from the entry, whose origin is the project's, and `HOME` and the
//! binary's version as string literals the binary read natively. It awaits
//! the call at its top level, so a rejection of `binary_main` fails the run.
//!
//! `tests/fixtures/verb/main_entry.ts` holds its text for a fixed `HOME` and
//! version, which `cargo xtask check-js` typechecks against Gro's types
//! (`js/tsconfig.json`), so a change to `BinaryMainDeps` fails there until
//! the text follows.

use std::fmt::Write as _;

/// The entry's text: `home`, the `HOME` the binary read, `undefined` when
/// it's unset, and `version`, the binary's, each written as a string
/// literal ([`js_string`]).
pub fn main_entry(home: Option<&str>, version: &str) -> String {
    let home = home.map_or_else(|| "undefined".to_owned(), js_string);
    format!(
        "import {{ fs }} from '@fuzdev/gro/fs.ts';
import {{ env }} from '@fuzdev/gro/env.ts';
import {{ spawn }} from '@fuzdev/gro/spawn.ts';
import {{ stdout }} from '@fuzdev/gro/stdout.ts';
import {{ stderr }} from '@fuzdev/gro/stderr.ts';
import {{ args }} from '@fuzdev/gro/args.ts';
import {{ cwd }} from '@fuzdev/gro/cwd.ts';
import {{ set_exit_code }} from '@fuzdev/gro/exit.ts';
import {{ binary_main }} from '@fuzdev/gro/binary_main.ts';

await binary_main({{
\targs,
\tfs,
\tenv,
\tspawn,
\tstdout,
\tstderr,
\tcwd,
\tset_exit_code,
\timport_url: (url) => import(url),
\thome: {home},
\tversion: {version}
}});
",
        version = js_string(version),
    )
}

/// `text` as a single-quoted JS string literal of ASCII alone: `\` and `'`
/// escaped, and every other character outside printable ASCII written as
/// `\u` and the four hex digits of each of its UTF-16 code units, so no
/// value read from the environment can end the literal or the line.
pub fn js_string(text: &str) -> String {
    let mut literal = String::with_capacity(text.len() + 2);
    literal.push('\'');
    for c in text.chars() {
        match c {
            '\\' => literal.push_str("\\\\"),
            '\'' => literal.push_str("\\'"),
            ' '..='~' => literal.push(c),
            _ => {
                let mut units = [0_u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    // writing to a `String` never fails
                    let _ = write!(literal, "\\u{unit:04x}");
                }
            }
        }
    }
    literal.push('\'');
    literal
}

#[cfg(test)]
mod tests {
    use super::{js_string, main_entry};

    /// The fixture's `HOME` and version, which no machine's are.
    const FIXTURE_HOME: &str = "/home/fixture";
    const FIXTURE_VERSION: &str = "0.0.0-fixture";

    #[test]
    fn the_entry_is_the_fixture_check_js_typechecks() {
        let path = format!(
            "{}/tests/fixtures/verb/main_entry.ts",
            env!("CARGO_MANIFEST_DIR")
        );
        let fixture = std::fs::read_to_string(&path).unwrap();
        let entry = main_entry(Some(FIXTURE_HOME), FIXTURE_VERSION);
        assert_eq!(entry, fixture, "{path} differs from the entry:\n{entry}");
    }

    #[test]
    fn an_unset_home_is_undefined() {
        let entry = main_entry(None, "1.2.3");
        assert!(entry.contains("\thome: undefined,\n"), "{entry}");
        assert!(entry.contains("\tversion: '1.2.3'\n"), "{entry}");
    }

    /// A value read from the environment stays one literal on one line,
    /// whatever it holds.
    #[test]
    fn a_string_literal_escapes_what_could_end_it() {
        assert_eq!(js_string("a/b"), "'a/b'");
        assert_eq!(js_string("it's"), "'it\\'s'");
        assert_eq!(js_string("a\\'"), "'a\\\\\\''");
        assert_eq!(js_string("a\nb\r\u{2028}"), "'a\\u000ab\\u000d\\u2028'");
        assert_eq!(js_string("\u{e9}\u{1f600}"), "'\\u00e9\\ud83d\\ude00'");
        assert_eq!(js_string("\u{7f}\0"), "'\\u007f\\u0000'");
    }
}
