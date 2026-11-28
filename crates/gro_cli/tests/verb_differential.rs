//! The verb's differential: each case of
//! `tests/fixtures/verb_differential/cases.json` run by `fgro` and held to
//! what the Node gro printed for it, `expected.json`, which `oracle.js`
//! there writes under Node and Gro's own tests check against Node
//! (`src/test/verb_differential.test.ts`), so neither side needs the other's
//! runtime. Both outputs are normalized alike, the project's root, gro's
//! version, and each duration. Where the binary differs by design, the
//! case's `differs` names each declared difference, a line of
//! `src/docs/runtime.md`'s list ([`Difference`]), and only those normalize
//! the binary's output, each required to change it, so a difference that's
//! gone fails here until its declaration goes too; a difference that
//! replaces Node's output holds the binary's to the case's `binary`.
//! Everything else, the exit code and both streams, is equal, and no case's
//! run on the binary changes the project.
//!
//! A command of Gro's runs on the binary only from a snapshot build, so this
//! runs from `cargo xtask build --verify` (CI's rust job, in its "Check"
//! step) and is ignored in a plain `cargo test`. It needs Landlock ABI 6,
//! the checkout's `node_modules` (`npm ci`), which each project links whole,
//! where `check`'s `tsc` and Gro's peers are installed, and a `node` on
//! `PATH` that is the runtime itself, not a version manager's shim. Each
//! project sits under `/tmp`, whatever `TMPDIR` says, under no
//! `package.json`, with an empty home directory of its own, as the oracle's
//! do, so its root's path is short on both sides: both consoles break an
//! inspected array whose string reaches [`WRAP_BOUND`] characters onto lines
//! of their own, which `{root}` doesn't hide, and a root that would reach it
//! fails the test before anything runs.

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "a test's setup fails by panicking"
)]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use gro_runtime::BINARY_NAME;
use serde_json::Value;
use support::{Scratch, fgro_command, spawn, version};

/// The length of a string at which Node's console, and the binary's, break
/// an inspected array holding it onto lines of their own.
const WRAP_BOUND: usize = 66;

/// The longest path under a project's root the cases print in an inspected
/// array, `gen`'s list of its genfiles.
const LONGEST_PRINTED: &str = "/src/lib/w.gen.ts";

/// The fixtures directory.
fn fixtures() -> PathBuf {
    support::fixtures("verb_differential")
}

/// A fixture file, parsed as JSON.
fn read_json(file: &str) -> Value {
    let path = fixtures().join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}. `node tests/fixtures/verb_differential/oracle.js --write` writes \
             expected.json",
            path.display()
        )
    });
    serde_json::from_str(&text).unwrap()
}

/// The checkout's `node_modules`, canonical, as the box's note names it.
fn checkout_node_modules() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules")
        .canonicalize()
        .unwrap()
}

/// A run's exit code and output, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Printed {
    code: i64,
    stdout: String,
    stderr: String,
}

impl Printed {
    fn from_json(value: &Value) -> Self {
        Self {
            code: value["code"].as_i64().unwrap(),
            stdout: value["stdout"].as_str().unwrap().to_owned(),
            stderr: value["stderr"].as_str().unwrap().to_owned(),
        }
    }

    /// Both streams mapped by `map`.
    fn map(&self, map: impl Fn(&str) -> String) -> Self {
        Self {
            code: self.code,
            stdout: map(&self.stdout),
            stderr: map(&self.stderr),
        }
    }
}

/// Whether `case`'s command may start a program: `check`'s and `gen`'s,
/// whose closures grant one (svelte-kit).
fn starts_programs(case: &Value) -> bool {
    matches!(case["args"][0].as_str(), Some("check" | "gen"))
}

/// A declared difference of the binary from the Node gro, each a line of
/// `src/docs/runtime.md`'s list, by the name a case's `differs` gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Difference {
    /// "A `node_modules` that's a symlink to another checkout's
    /// `node_modules` is followed, ..., with a `note:` line naming where
    /// it leads": the line, first on stderr, removed.
    LinkedNodeModules,
    /// "Gro's logger colors its output even under `NO_COLOR` or when
    /// piped": every SGR sequence removed.
    LoggerColors,
    /// "The console prints a string inside an array or object with double
    /// quotes, where Node's prints single ones": an inspected array's
    /// strings requoted.
    ConsoleQuotes,
    /// "`fgro --version` prints the binary's own line": held to its shape,
    /// in Node's place.
    BinaryVersion,
    /// "A forwarded section that turns off a check its command runs ... is
    /// refused before anything runs, exit 2": the case's `binary`, in
    /// Node's place.
    CheckTurnedOffRefused,
    /// "`-A` is refused, exit 2": the case's `binary`, in Node's place.
    AllowAllRefused,
    /// "The builtins' listing, bare `fgro`, names the binary in its usage
    /// lines, `fgro [name]`, where the Node Gro's names `gro`": the
    /// binary's name in the usage lines read as Node's.
    CliName,
}

impl Difference {
    fn parse(name: &str) -> Self {
        match name {
            "linked_node_modules" => Self::LinkedNodeModules,
            "logger_colors" => Self::LoggerColors,
            "console_quotes" => Self::ConsoleQuotes,
            "binary_version" => Self::BinaryVersion,
            "check_turned_off_refused" => Self::CheckTurnedOffRefused,
            "allow_all_refused" => Self::AllowAllRefused,
            "cli_name" => Self::CliName,
            name => panic!("no declared difference is named {name}"),
        }
    }

    /// Whether it replaces Node's output whole, held by the case's own
    /// expectation rather than normalized to Node's.
    const fn replaces(self) -> bool {
        matches!(
            self,
            Self::BinaryVersion | Self::CheckTurnedOffRefused | Self::AllowAllRefused
        )
    }
}

/// `text` with the project's `root`, gro's `version`, and each duration
/// normalized, as the oracle normalizes the Node gro's.
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "the placeholders are the oracle's, never formatted"
)]
fn normalize(text: &str, root: &str, version: &str) -> String {
    normalize_durations(&text.replace(root, "{root}").replace(version, "{version}"))
}

/// `text` with each duration, digits and commas, an optional fraction, and
/// `ms`, as `{ms}`: the oracle's `/\d[\d,]*(?:\.\d+)?ms/g`.
fn normalize_durations(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b',') {
            end += 1;
        }
        if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
            end += 2;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
        }
        if bytes[end..].starts_with(b"ms") {
            out.push_str(&text[copied..index]);
            out.push_str("{ms}");
            copied = end + 2;
            index = end + 2;
        } else {
            index += 1;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// `text` without its SGR sequences, `ESC [ digits and ; m`.
fn strip_sgr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("\x1b[") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let params = after
            .find(|c: char| !(c.is_ascii_digit() || c == ';'))
            .unwrap_or(after.len());
        if after[params..].starts_with('m') {
            rest = &after[params + 1..];
        } else {
            out.push_str("\x1b[");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// `text` with each string of an inspected array, `"..."` after `[ ` or
/// `, ` and before ` ]` or `,`, holding no quote or backslash, in the
/// single quotes Node's console prints.
fn requote_inspected(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let (before, quoted) = rest.split_at(start);
        out.push_str(before);
        let opens = out.ends_with("[ ") || out.ends_with(", ");
        let close = quoted[1..].find(['"', '\'', '\\']).map(|end| end + 1);
        match close {
            Some(end)
                if opens
                    && quoted.as_bytes()[end] == b'"'
                    && (quoted[end + 1..].starts_with(" ]")
                        || quoted[end + 1..].starts_with(',')) =>
            {
                out.push('\'');
                out.push_str(&quoted[1..end]);
                out.push('\'');
                rest = &quoted[end + 1..];
            }
            _ => {
                out.push('"');
                rest = &quoted[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// What the binary's run of a case printed, `printed`, with the differences
/// `differs` declares applied, and what to hold it to: the Node gro's
/// `expected`, or, for a difference that replaces it, the output the case
/// gives. Each difference that normalizes must change the output it's
/// applied to.
fn apply(
    differs: &[Difference],
    printed: &Printed,
    case: &Value,
    expected: Printed,
    version: &str,
) -> (Printed, Printed) {
    let mut actual = printed.clone();
    let mut expected = expected;
    for &difference in differs {
        let before = actual.clone();
        match difference {
            Difference::LinkedNodeModules => {
                // read for a run that starts no program, and executed too
                // for one whose closure grants one, `check`'s and `gen`'s
                // (svelte-kit), whose tools write nothing there; the line's
                // flags for those tools are the plan's to pin
                let link = format!(
                    "note: node_modules is a symlink to {}, which the box follows to read",
                    checkout_node_modules().display()
                );
                let note = if starts_programs(case) {
                    format!("{link} and execute, and no tool writes in it")
                } else {
                    format!("{link}\n")
                };
                if actual.stderr.starts_with(note.as_str()) {
                    let line = actual
                        .stderr
                        .find('\n')
                        .map_or(actual.stderr.len(), |at| at + 1);
                    actual.stderr = actual.stderr[line..].to_owned();
                }
            }
            Difference::LoggerColors => actual = actual.map(strip_sgr),
            Difference::CliName => {
                actual = actual.map(|text| {
                    text.replace(&format!("{BINARY_NAME} [name]"), "gro [name]")
                        .replace(&format!("{BINARY_NAME} task ["), "gro task [")
                });
            }
            Difference::ConsoleQuotes => actual = actual.map(requote_inspected),
            Difference::BinaryVersion => {
                let prefix = format!("{BINARY_NAME} {version} (commit ");
                assert!(
                    actual.code == 0
                        && actual.stdout.starts_with(&prefix)
                        && actual.stdout.ends_with(")\n")
                        && actual.stdout.lines().count() == 1
                        && actual.stderr.is_empty(),
                    "the binary's version line: {actual:?}"
                );
                expected = actual.clone();
            }
            Difference::CheckTurnedOffRefused | Difference::AllowAllRefused => {
                expected = Printed::from_json(&case["binary"])
                    .map(|text| text.replace("{binary}", BINARY_NAME));
            }
        }
        if !difference.replaces() {
            assert_ne!(
                before, actual,
                "{difference:?} is declared for {} and didn't change the binary's output",
                case["name"]
            );
        }
    }
    (actual, expected)
}

/// Every file under `dir`, but `node_modules`, by its path relative to it,
/// with its content.
fn files_under(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path == dir.join("node_modules") {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else {
                let content = std::fs::read(&path).unwrap();
                files.insert(path.strip_prefix(dir).unwrap().to_owned(), content);
            }
        }
    }
    files
}

/// Each case on the binary prints what the Node gro printed, but for its
/// declared differences, and changes nothing in the project.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs on the binary only from a snapshot build"
)]
fn each_case_prints_what_the_node_gro_printed() {
    let fixture = read_json("cases.json");
    let expected = read_json("expected.json");
    let version = version();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(
        cases.len(),
        expected.as_object().unwrap().len(),
        "expected.json is stale: regenerate it with `node oracle.js --write`"
    );
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let mut files: BTreeMap<String, String> = BTreeMap::new();
        for source in [&fixture["project"], &case["files"]] {
            for (path, content) in source.as_object().into_iter().flatten() {
                files.insert(format!("p/{path}"), content.as_str().unwrap().to_owned());
            }
        }
        let listed: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, content)| (path.as_str(), content.as_str()))
            .collect();
        // one case at a time, each dropping its scratch before the next makes its own
        let scratch = Scratch::in_tmp("vd", &listed);
        let root = scratch.0.join("p");
        let longest = root.display().to_string().len() + LONGEST_PRINTED.len();
        assert!(
            longest < WRAP_BOUND,
            "{}: the project's root is too long for the cases' inspected arrays, which break \
             onto lines of their own at {WRAP_BOUND} characters, and `{{root}}` can't hide that",
            root.display()
        );
        let home = scratch.0.join("home");
        std::fs::create_dir(&home).unwrap();
        std::os::unix::fs::symlink(checkout_node_modules(), root.join("node_modules")).unwrap();
        let before = files_under(&root);
        let args: Vec<&str> = case["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        let mut command = fgro_command(&args);
        command
            .current_dir(&root)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home);
        let Output {
            status,
            stdout,
            stderr,
        } = spawn(command, b"");
        let printed = Printed {
            code: status.code().map_or(-1, i64::from),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        };
        let differs: Vec<Difference> = case["differs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|name| Difference::parse(name.as_str().unwrap()))
            .collect();
        // the boxed run of a command that may start a program says first,
        // where the host refuses its box a network namespace, that it does,
        // which is the host's, not a difference from the Node gro; every
        // case whose run gets to its box declares the project's linked
        // `node_modules`, which the box notes
        let boxed = if starts_programs(case) && differs.contains(&Difference::LinkedNodeModules) {
            Printed {
                stderr: support::past_network_note(&printed.stderr),
                ..printed.clone()
            }
        } else {
            printed.clone()
        };
        let node = Printed::from_json(&expected[name]);
        let (actual, expected) = apply(&differs, &boxed, case, node, &version);
        let root_text = root.display().to_string();
        let actual = actual.map(|text| normalize(text, &root_text, &version));
        let expected = expected.map(|text| normalize(text, &root_text, &version));
        assert_eq!(
            actual, expected,
            "{name}: the binary's output, its declared differences applied, isn't the Node gro's \
             (`expected.json`, which `node oracle.js --write` regenerates); as printed: \
             {printed:?}"
        );
        assert_eq!(
            files_under(&root),
            before,
            "{name}: the binary's run changed the project"
        );
    }
}

#[test]
fn durations_normalize_as_the_oracle_s_pattern() {
    assert_eq!(
        normalize_durations("a 0.00ms b 1,234ms c 5.5ms d v0.1.2 e 3.ms f 12"),
        "a {ms} b {ms} c {ms} d v0.1.2 e 3.ms f 12"
    );
}

#[test]
fn an_inspected_array_s_strings_are_requoted() {
    assert_eq!(
        requote_inspected(r#"files [ "/a b", "/c" ] and "x" ["y"] [ "it's" ]"#),
        r#"files [ '/a b', '/c' ] and "x" ["y"] [ "it's" ]"#
    );
}

#[test]
fn sgr_sequences_are_stripped() {
    assert_eq!(
        strip_sgr("\x1b[90m[\x1b[39mgro\x1b[1;2m]\x1b[K"),
        "[gro]\x1b[K"
    );
}
