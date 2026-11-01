//! The `phases` feature from outside: a default build ignores
//! `GRO_PHASES_FILE`, and a phases build writes its marks there, in order,
//! eval's included through the handle it opened before its box. The phases
//! build's test runs from `cargo xtask build --verify`, which builds `fgro`
//! with the feature for it.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The variable naming the phases file, `gro_runtime::phases`'s.
const PHASES_FILE_ENV: &str = "GRO_PHASES_FILE";

fn fixtures(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(kind)
}

/// A fresh path for a phases file, outside every fixture project, so an
/// eval's box doesn't hold it.
fn phases_file(test: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("fgro_phases_{test}_{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

/// Runs `fgro` with `args` from the `kind` fixtures, writing phases to `file`.
fn fgro(kind: &str, args: &[&str], file: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fgro"))
        .args(args)
        .env(PHASES_FILE_ENV, file)
        // eval refuses any other time zone
        .env("TZ", "UTC")
        .current_dir(fixtures(kind))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn version() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_fgro"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[cfg(not(feature = "phases"))]
#[test]
fn a_default_build_ignores_the_phases_file() {
    assert!(!version().contains(", phases"), "{}", version());
    let file = phases_file("default_run");
    let output = fgro("run", &["run", "import_static.js"], &file);
    assert!(output.status.success(), "{output:?}");
    assert!(!file.exists());
    // eval writes an envelope, evaluating or refusing a build with no snapshot
    let file = phases_file("default_eval");
    let output = fgro("eval", &["eval", "value.js"], &file);
    assert!(!output.stdout.is_empty(), "{output:?}");
    assert!(!file.exists());
}

/// The phase names a phases file holds, in order, after checking its header
/// and that each mark comes no earlier than the one before.
#[cfg(feature = "phases")]
fn read_marks(file: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(file).unwrap();
    let _ = std::fs::remove_file(file);
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
fn a_phases_build_writes_its_marks_in_order() {
    assert!(version().contains(", phases)"), "{}", version());
    let file = phases_file("run");
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
    if !cfg!(feature = "snapshot") {
        return;
    }
    // written from inside the box, through the handle opened before it
    let file = phases_file("eval");
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
