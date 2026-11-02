//! The `phases` feature from outside: a default build ignores
//! `GRO_PHASES_FILE`, and a phases build writes its marks there, in order,
//! eval's included through the handle it opened before its box. The phases
//! build's test runs from `cargo xtask build --verify`, which builds `fgro`
//! with the feature for it.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

mod support;

use std::path::Path;
use std::process::Output;

use support::{Scratch, fgro_command, spawn};

/// The variable naming the phases file, `gro_runtime::phases`'s.
const PHASES_FILE_ENV: &str = "GRO_PHASES_FILE";

/// A fresh directory for a phases file, outside every fixture project, so
/// an eval's box doesn't hold it.
fn phases_dir(test: &str) -> Scratch {
    Scratch::new(test, &[])
}

/// Runs `fgro` with `args` from the `kind` fixtures, writing phases to `file`.
fn fgro(kind: &str, args: &[&str], file: &Path) -> Output {
    let mut command = fgro_command(args);
    command
        .env(PHASES_FILE_ENV, file)
        .current_dir(support::fixtures(kind));
    spawn(command, b"")
}

fn version() -> String {
    let output = spawn(fgro_command(&["--version"]), b"");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[cfg(not(feature = "phases"))]
#[test]
fn a_default_build_ignores_the_phases_file() {
    assert!(!version().contains(", phases"), "{}", version());
    let dir = phases_dir("default_run");
    let file = dir.0.join("phases");
    let output = fgro("run", &["run", "import_static.js"], &file);
    assert!(output.status.success(), "{output:?}");
    assert!(!file.exists());
    // eval writes an envelope, evaluating or refusing a build with no snapshot
    let dir = phases_dir("default_eval");
    let file = dir.0.join("phases");
    let output = fgro("eval", &["eval", "value.js"], &file);
    assert!(!output.stdout.is_empty(), "{output:?}");
    assert!(!file.exists());
}

/// The phase names a phases file holds, in order, after checking its header
/// and that each mark comes no earlier than the one before.
#[cfg(feature = "phases")]
fn read_marks(file: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(file).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("gro_phases 1"), "{text}");
    let anchor = lines.next().unwrap();
    let wall: u128 = anchor
        .strip_prefix("main_unix_ns ")
        .unwrap()
        .parse()
        .unwrap();
    assert!(wall > 0, "{text}");
    let mut last = 0;
    lines
        .map(|line| {
            let (name, at) = line.split_once(' ').unwrap();
            let at: u128 = at.parse().unwrap();
            assert!(at >= last, "{text}");
            last = at;
            name.to_owned()
        })
        .collect()
}

#[cfg(feature = "phases")]
#[test]
fn a_phases_build_writes_runs_marks_in_order() {
    assert!(version().contains(", phases)"), "{}", version());
    let dir = phases_dir("run");
    let file = dir.0.join("phases");
    let output = fgro("run", &["run", "import_static.js"], &file);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        read_marks(&file),
        [
            "engine",
            "runtime",
            "bootstrap",
            "load",
            "evaluate",
            "teardown",
            "exit"
        ]
    );
}

/// Eval's marks, written from inside the box, through the handle opened
/// before it.
#[cfg(feature = "phases")]
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_phases_build_writes_evals_marks_in_order() {
    let dir = phases_dir("eval");
    let file = dir.0.join("phases");
    let output = fgro("eval", &["eval", "value.js"], &file);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        read_marks(&file),
        [
            "prepass",
            "confine",
            "engine",
            "runtime",
            "bootstrap",
            "guards",
            "load",
            "evaluate",
            "serialize",
            "drain",
            "teardown",
            "envelope",
            "write",
            "exit"
        ]
    );
}
