//! What a test can see only from a process of its own: the console's output
//! on the process's stdout and stderr, and the engine's process-wide options;
//! and what only a process of its own can make, the startup snapshot
//! ([`make_startup_snapshot`]).
//!
//! Each case runs in a child, this test binary run again with [`CASE`] naming
//! the case and the filter selecting only [`child`] by its full path,
//! [`CHILD_TEST`]; the parent reads its exit status and output.
//! [`run_test_in_child`] runs any one test of the binary that way.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use deno_core::{Extension, ExtensionFileSource, OpDecl};

use crate::bootstrap::{self, Ops};
use crate::confine::Confinement;
use crate::engine::{Engine, EngineError, EngineOptions, init_engine};
use crate::resolver::BUILTINS;
use crate::snapshot::{OpFunction, ResidualCode, SnapshotError, make_snapshot, make_snapshot_of};
use crate::test_support::TestDir;
use crate::{
    Authority, ConsoleOptions, ConsoleRouting, Grants, PassEnvGrant, ProgramGrant, RuntimeOptions,
    STACK_SIZE, SpawnGrant, run_main,
};

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

/// Runs `env` granted the whole environment but `GRO_OMITTED`, its policy
/// fixing no value, asking for the whole environment but that one, and
/// prints what its child was given, a line per variable; then made with no
/// options, a list of none, and prints what that child was given, after a
/// line of `---`.
const SPAWN_ALL_SOURCE: &str = "import { spawn } from '@fuzdev/gro/spawn.ts';
const env = spawn.program('env', { pass_env: 'all', omit_env: ['GRO_OMITTED'] });
const result = await env.out([]);
if (result.code !== 0) throw new Error(`env exited ${result.code}`);
console.log(result.stdout);
const bare = await spawn.program('env').out([]);
if (bare.code !== 0) throw new Error(`env exited ${bare.code}`);
console.log(`---\\n${bare.stdout}`);
";

/// A run granting `env` as the grants table grants a program the whole
/// environment, handed `vars` for children.
fn spawn_all_case() {
    let grant = ProgramGrant::Table {
        pass_env: PassEnvGrant::All {
            omit_env: BTreeSet::from(["GRO_OMITTED".to_owned()]),
        },
        env: BTreeMap::new(),
    };
    let options = RuntimeOptions {
        authority: Authority::Confined(Grants {
            spawn: Some(SpawnGrant {
                programs: BTreeMap::from([("env".to_owned(), grant)]),
                exec_roots: BTreeMap::new(),
            }),
            ..Grants::default()
        }),
        spawn_env: [
            ("PATH", "/usr/bin:/bin"),
            ("GRO_UNLISTED", "passed"),
            ("GRO_OMITTED", "left out"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect(),
        ..RuntimeOptions::default()
    };
    let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    run_source(engine, "spawn_all", SPAWN_ALL_SOURCE, options);
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
        "spawn_all" => spawn_all_case(),
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
        "snapshot_importing_a_builtin" => {
            let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
            // `deno_core` panics on a failed evaluation while it makes a
            // snapshot, rather than returning the error
            let made = std::panic::catch_unwind(move || {
                make_snapshot_of(engine, extensions_importing_a_builtin)
            });
            let failure = match made {
                Ok(Ok(_)) => panic!("a snapshot was made of a runtime that imported a builtin"),
                Ok(Err(error)) => error.to_string(),
                Err(payload) => payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_default(),
            };
            assert!(
                failure.contains(bootstrap::BACKING_SPECIFIERS[0])
                    && failure.contains("cannot be lazy-loaded as it was not included"),
                "{failure}"
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

/// The extensions a snapshot is made from, and one more whose module, run
/// as the snapshot is made, imports the builtin table's first row.
fn extensions_importing_a_builtin() -> Vec<Extension> {
    const ENTRY: &str = "ext:test_builtin_import/main.js";
    let source = format!(
        "import * as namespace from '{}';\nglobalThis.namespace = namespace;\n",
        BUILTINS[0].url
    );
    let mut extensions = bootstrap::extensions(Ops::Kept);
    extensions.push(Extension {
        name: "test_builtin_import",
        esm_files: Cow::Owned(vec![ExtensionFileSource::new_computed(
            ENTRY,
            source.into(),
        )]),
        esm_entry_point: Some(ENTRY),
        ..Extension::default()
    });
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

/// A program granted the whole environment is passed it, a variable its
/// grant names nowhere among it, but what its grant omits, and nothing
/// the base set would add (`LC_ALL=C`); made with no options, it's given
/// the base set and `LC_ALL=C`, as any list's child is, and nothing else.
#[test]
fn a_program_granted_the_whole_environment_is_passed_it() {
    let output = run_child("spawn_all");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (all, bare) = stdout.split_once("---\n").unwrap();
    let lines: Vec<&str> = all.lines().collect();
    assert!(lines.contains(&"GRO_UNLISTED=passed"), "{stdout}");
    assert!(lines.contains(&"PATH=/usr/bin:/bin"), "{stdout}");
    assert!(
        !lines
            .iter()
            .any(|line| line.starts_with("GRO_OMITTED=") || line.starts_with("LC_ALL=")),
        "{stdout}"
    );
    // the harness's own lines hold no `=`
    let mut bare: Vec<&str> = bare.lines().filter(|line| line.contains('=')).collect();
    bare.sort_unstable();
    assert_eq!(bare, ["LC_ALL=C", "PATH=/usr/bin:/bin"], "{stdout}");
}

#[test]
fn the_engine_locale_reaches_intl() {
    run_child("locale");
}

/// No snapshot holds a builtin: a module the snapshot runs can't import
/// one, since no script the snapshot is made with is a backing script,
/// which reaches `deno_core` only in a start's table of residual scripts
/// (`bootstrap`'s module doc), so the making fails at the import, naming
/// the row's backing script. A builtin's namespace is made from the run's
/// own state, which no snapshot may carry into every run.
#[test]
fn no_snapshot_is_made_of_a_runtime_importing_a_builtin() {
    run_child("snapshot_importing_a_builtin");
}

/// No snapshot is made of a runtime where an op eval refuses has its
/// function at the address of one eval keeps: the making fails, naming
/// both, on the addresses of the runtime it made.
#[test]
fn no_snapshot_is_made_of_a_refused_op_merged_with_a_kept_one() {
    run_child("snapshot_merged");
}
