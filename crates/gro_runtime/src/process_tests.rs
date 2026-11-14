//! What a test can see only from a process of its own: the console's output
//! on the process's stdout and stderr, and the engine's process-wide options;
//! and what only a process of its own can make, the startup snapshot
//! ([`make_startup_snapshot`]).
//!
//! Each case runs in a child, this test binary run again with [`CASE`] naming
//! the case and the filter selecting only [`child`] by its full path,
//! [`CHILD_TEST`]; the parent reads its exit status and output.
//! [`run_test_in_child`] runs any one test of the binary that way.

use std::cell::Cell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use deno_core::{Extension, OpDecl};

use crate::bootstrap::{self, Ops};
use crate::confine::Confinement;
use crate::engine::{Engine, EngineError, EngineOptions, init_engine};
use crate::snapshot::{OpFunction, ResidualCode, SnapshotError, make_snapshot, make_snapshot_of};
use crate::test_support::TestDir;
use crate::{ConsoleOptions, ConsoleRouting, RuntimeOptions, STACK_SIZE, run_main};

/// The full path of [`child`] in the test binary, which libtest's `--exact`
/// filter matches.
const CHILD_TEST: &str = "process_tests::child";

/// The variable naming the case a child runs.
const CASE: &str = "GRO_RUNTIME_TEST_CASE";

/// The variable naming the file the `snapshot` case writes.
const SNAPSHOT_FILE: &str = "GRO_RUNTIME_TEST_SNAPSHOT_FILE";

/// Writes a line per level, each a number, which the console colors when its
/// stream is colored.
const CONSOLE_SOURCE: &str = "console.log(1001);
console.info(1002);
console.debug(1003);
console.warn(1004);
console.error(1005);
";

/// The locale the locale case sets: one no host is likely to default to, so
/// the test can't pass by the host's choice. (`en-US`, the one eval sets,
/// is ICU's default under `LANG=C`.)
const LOCALE: &str = "fr_CA";

/// Runs `source` as an `.mjs` entry in a fresh directory on `engine`, on a
/// thread reserving [`STACK_SIZE`] as `run_main` requires.
fn run_source(engine: Engine, name: &str, source: &str, options: RuntimeOptions) {
    let dir = TestDir::in_system_temp(&format!("process_{name}"));
    let entry = dir.path().join("main.mjs");
    std::fs::write(&entry, source).unwrap();
    let cwd = dir.path().to_owned();
    let result = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || run_main(engine, &options, &entry, &cwd).result)
        .unwrap()
        .join()
        .unwrap();
    result.unwrap();
}

fn console_case(console: ConsoleOptions) {
    let options = RuntimeOptions {
        console,
        ..RuntimeOptions::default()
    };
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    run_source(engine, "console", CONSOLE_SOURCE, options);
}

/// The child: runs the case [`CASE`] names, and nothing when it's unset, as
/// in the parent's own run.
#[test]
fn child() {
    let Some(case) = std::env::var_os(CASE) else {
        return;
    };
    match case.to_str().unwrap() {
        "console_default" => console_case(ConsoleOptions::default()),
        "console_color_stderr" => console_case(ConsoleOptions {
            color_stderr: true,
            ..ConsoleOptions::default()
        }),
        "console_color_stdout" => console_case(ConsoleOptions {
            color_stdout: true,
            ..ConsoleOptions::default()
        }),
        "console_stderr" => console_case(ConsoleOptions {
            routing: ConsoleRouting::Stderr,
            color_stderr: true,
            color_stdout: false,
        }),
        "locale" => {
            let options = EngineOptions {
                locale: Some(LOCALE),
                ..EngineOptions::default()
            };
            let engine = init_engine(Confinement::Unconfined, &options).unwrap();
            assert_eq!(engine.options(), &options);
            // equal options again are accepted, others refused
            assert_eq!(
                init_engine(Confinement::Unconfined, &options)
                    .unwrap()
                    .options(),
                &options
            );
            assert!(matches!(
                init_engine(Confinement::Unconfined, &EngineOptions::default()),
                Err(EngineError::OptionsConflict(applied)) if applied == options
            ));
            run_source(
                engine,
                "locale",
                "const locales = [
\tnew Intl.NumberFormat().resolvedOptions().locale,
\tnew Intl.DateTimeFormat().resolvedOptions().locale,
];
for (const locale of locales) {
\tif (locale !== 'fr-CA') throw new Error(`the locale is ${locale}`);
}
",
                RuntimeOptions::default(),
            );
        }
        "snapshot" => {
            let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
            let snapshot = make_snapshot(engine).unwrap();
            let file = PathBuf::from(std::env::var_os(SNAPSHOT_FILE).unwrap());
            std::fs::write(&file, snapshot.bytes).unwrap();
            let scripts: Vec<(&str, &str)> = snapshot
                .residual_lazy_js
                .iter()
                .filter_map(|source| match &source.code {
                    ResidualCode::Text(script) => {
                        Some((source.specifier.as_str(), script.as_str()))
                    }
                    ResidualCode::File(_) => None,
                })
                .collect();
            std::fs::write(
                backing_scripts_file(&file),
                serde_json::to_string(&scripts).unwrap(),
            )
            .unwrap();
        }
        "snapshot_merged" => {
            let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
            let error = make_snapshot_of(engine, extensions_merging_a_refused_op).unwrap_err();
            assert!(
                matches!(
                    error,
                    SnapshotError::SharedOpFunction {
                        refused: MERGED_REFUSED,
                        refused_function: OpFunction::Slow,
                        other: MERGED_KEPT,
                        other_function: OpFunction::Slow,
                    }
                ),
                "{error}"
            );
        }
        case => panic!("unknown case {case}"),
    }
}

/// An op eval keeps, registered before [`MERGED_REFUSED`].
const MERGED_KEPT: &str = "op_void_sync";

/// An op eval refuses, which [`extensions_merging_a_refused_op`] gives
/// [`MERGED_KEPT`]'s functions.
const MERGED_REFUSED: &str = "op_now";

/// The extensions a snapshot is made from, but that [`MERGED_REFUSED`] has
/// the functions of [`MERGED_KEPT`], so the two ops' functions are at one
/// address each, as a build that merged them leaves the two.
fn extensions_merging_a_refused_op() -> Vec<Extension> {
    let mut extensions = bootstrap::extensions(Ops::Kept);
    let kept = Cell::new(None);
    let merge = move |op: OpDecl| {
        if op.name == MERGED_KEPT {
            kept.set(Some(op));
        }
        match kept.get() {
            Some(kept) if op.name == MERGED_REFUSED => op.with_implementation_from(&kept),
            _ => op,
        }
    };
    let gro = extensions
        .iter_mut()
        .find(|extension| extension.name == "gro")
        .unwrap();
    gro.middleware_fn = Some(Box::new(merge));
    extensions
}

/// Runs the child for `case`, failing unless it succeeds, and returns its
/// output.
fn run_child(case: &str) -> Output {
    run_test_in_child(CHILD_TEST, &[(CASE, OsStr::new(case))])
}

/// Runs the test at the full path `test` alone, in a child with `env` set,
/// failing unless it ran and succeeded, and returns its output.
pub fn run_test_in_child(test: &str, env: &[(&str, &OsStr)]) -> Output {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--quiet"])
        .envs(env.iter().copied())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{test} with {env:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // a filter that matches no test exits 0 as well
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(" 1 passed;"),
        "{test} with {env:?}: the child ran no test"
    );
    output
}

/// The file beside the snapshot's at `snapshot` that holds the builtins'
/// backing scripts the snapshot was made with, a JSON array of
/// `[specifier, script]` sorted by specifier.
pub fn backing_scripts_file(snapshot: &Path) -> PathBuf {
    snapshot.with_file_name("backing_scripts.json")
}

/// Makes the startup snapshot of this build's extensions in `dir` and
/// returns its file, the serialized heap alone, with the builtins' backing
/// scripts beside it ([`backing_scripts_file`]). A child makes it: V8 makes
/// a snapshot only in a process that runs no other runtime.
pub fn make_startup_snapshot(dir: &Path) -> PathBuf {
    let file = dir.join("snapshot.bin");
    run_test_in_child(
        CHILD_TEST,
        &[
            (CASE, OsStr::new("snapshot")),
            (SNAPSHOT_FILE, file.as_os_str()),
        ],
    );
    file
}

/// The lines of `output` that are one of the console's numbers, colored or
/// not, leaving out the test harness's own.
fn console_lines(output: &[u8]) -> Vec<String> {
    String::from_utf8(output.to_vec())
        .unwrap()
        .lines()
        .filter(|line| {
            let bare = line.replace("\x1b[33m", "").replace("\x1b[39m", "");
            bare.parse::<u32>()
                .is_ok_and(|n| (1001..=1005).contains(&n))
        })
        .map(ToOwned::to_owned)
        .collect()
}

/// `n` as the console prints a number, colored (yellow) or not.
fn number(n: u32, colored: bool) -> String {
    if colored {
        format!("\x1b[33m{n}\x1b[39m")
    } else {
        n.to_string()
    }
}

/// The console's lines on stdout and stderr for `case`.
fn console_output(case: &str) -> (Vec<String>, Vec<String>) {
    let output = run_child(case);
    (console_lines(&output.stdout), console_lines(&output.stderr))
}

#[test]
fn by_default_the_console_writes_by_level_uncolored() {
    let (stdout, stderr) = console_output("console_default");
    assert_eq!(stdout, ["1001", "1002", "1003"]);
    assert_eq!(stderr, ["1004", "1005"]);
}

#[test]
fn color_is_decided_per_stream() {
    let (stdout, stderr) = console_output("console_color_stderr");
    assert_eq!(stdout, ["1001", "1002", "1003"]);
    assert_eq!(stderr, [number(1004, true), number(1005, true)]);

    let (stdout, stderr) = console_output("console_color_stdout");
    let colored: Vec<String> = [1001, 1002, 1003].map(|n| number(n, true)).into();
    assert_eq!(stdout, colored);
    assert_eq!(stderr, ["1004", "1005"]);
}

/// Every level on stderr, colored by stderr's decision, stdout's ignored.
#[test]
fn the_console_can_write_every_level_to_stderr() {
    let (stdout, stderr) = console_output("console_stderr");
    assert!(stdout.is_empty(), "{stdout:?}");
    let colored: Vec<String> = (1001..=1005).map(|n| number(n, true)).collect();
    assert_eq!(stderr, colored);
}

#[test]
fn the_engine_locale_reaches_intl() {
    run_child("locale");
}

/// No snapshot is made of a runtime where an op eval refuses has its
/// function at the address of one eval keeps: the making fails, naming
/// both, on the addresses of the runtime it made.
#[test]
fn no_snapshot_is_made_of_a_refused_op_merged_with_a_kept_one() {
    run_child("snapshot_merged");
}
