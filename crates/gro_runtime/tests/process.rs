//! What a test can see only from a process of its own: the console's output
//! on the process's stdout and stderr, and the engine's process-wide options.
//!
//! Each case runs in a child, this test binary run again with [`CASE`] naming
//! the case and the filter selecting only [`child`]; the parent reads its
//! exit status and output.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

use std::path::Path;
use std::process::{Command, Output};

use gro_runtime::engine::{EngineError, EngineOptions, init_engine};
use gro_runtime::{ConsoleOptions, ConsoleRouting, RuntimeOptions, STACK_SIZE, run_main};

/// The variable naming the case a child runs.
const CASE: &str = "GRO_RUNTIME_TEST_CASE";

/// Writes a line per level, each a number, which the console colors when its
/// stream is colored.
const CONSOLE_SOURCE: &str = "console.log(1001);
console.info(1002);
console.debug(1003);
console.warn(1004);
console.error(1005);
";

/// The locale the locale case sets: one no host is likely to default to, so
/// the test can't pass by the host's choice. (`en_US_POSIX`, the one eval
/// sets, is ICU's default under `LANG=C`.)
const LOCALE: &str = "fr_CA";

/// Runs `source` as an `.mjs` entry in a fresh directory, on a thread
/// reserving [`STACK_SIZE`] as `run_main` requires.
fn run_source(name: &str, source: &str, options: RuntimeOptions) {
    let dir =
        std::env::temp_dir().join(format!("gro_runtime_process_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.mjs");
    std::fs::write(&entry, source).unwrap();
    let cwd = dir.clone();
    let result = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || run_main(&options, &entry, &cwd))
        .unwrap()
        .join()
        .unwrap();
    std::fs::remove_dir_all(Path::new(&dir)).unwrap();
    result.unwrap();
}

fn console_case(console: ConsoleOptions) {
    let options = RuntimeOptions {
        console,
        ..RuntimeOptions::default()
    };
    run_source("console", CONSOLE_SOURCE, options);
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
            };
            init_engine(&options).unwrap();
            // equal options again are accepted, others refused
            init_engine(&options).unwrap();
            assert!(matches!(
                init_engine(&EngineOptions::default()),
                Err(EngineError::Conflict(applied)) if applied == options
            ));
            run_source(
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
        case => panic!("unknown case {case}"),
    }
}

/// Runs the child for `case`, failing unless it succeeds, and returns its
/// output.
fn run_child(case: &str) -> Output {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["child", "--exact", "--nocapture", "--quiet"])
        .env(CASE, case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{case}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // a filter that matches no test exits 0 as well
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(" 1 passed;"),
        "{case}: the child ran no test"
    );
    output
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
