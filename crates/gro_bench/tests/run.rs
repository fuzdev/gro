//! The runner end to end, with `sh` as the runtime and shell scripts as the
//! workloads, so it needs no JS runtime installed.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

use std::collections::BTreeMap;
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use gro_bench::error::BenchError;
use gro_bench::record::{GitState, Machine, MachineQuiet, RunRecord};
use gro_bench::run::{RunContext, run};
use gro_bench::suite::{Arg, FileArg, Runtime, Suite, Workload};

fn context() -> RunContext {
    RunContext {
        git: GitState {
            sha: "0".repeat(40),
            dirty: false,
        },
        machine: Machine::detect(),
        machine_quiet: MachineQuiet::Unchecked,
        corpora: None,
        workspace: BTreeMap::new(),
    }
}

/// A fresh directory for one test, holding `files`.
fn scratch(test: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gro_bench_{test}_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        fs::write(dir.join(name), content).unwrap();
    }
    dir
}

fn sh(args: Vec<Arg>) -> Runtime {
    Runtime {
        name: "sh".to_owned(),
        program: "sh".to_owned(),
        args,
        // dash has no `--version`
        version_args: vec!["-c".to_owned(), "echo sh test".to_owned()],
        v8_args: None,
    }
}

fn suite(runtimes: Vec<Runtime>, scripts: &[&str], samples: u32) -> Suite {
    Suite {
        name: "test".to_owned(),
        version: 1,
        samples: NonZeroU32::new(samples).unwrap(),
        runtimes,
        workloads: scripts
            .iter()
            .map(|script| Workload {
                name: script.trim_end_matches(".sh").to_owned(),
                script: PathBuf::from(script),
                warmup: 1,
            })
            .collect(),
    }
}

fn run_in(dir: &Path, suite: &Suite) -> Result<RunRecord, BenchError> {
    let result = run(suite, dir, context());
    fs::remove_dir_all(dir).unwrap();
    result
}

#[test]
fn measures_every_row_and_the_noise_row() {
    let dir = scratch(
        "rows",
        &[("a.sh", "true\n"), ("b.sh", ": \"$1\"\n"), ("shim.sh", "")],
    );
    // a second runtime whose shim is a file argument
    let mut shim = sh(vec![Arg::File(FileArg {
        file: PathBuf::from("shim.sh"),
    })]);
    shim.name = "sh_shim".to_owned();
    let suite = suite(vec![sh(Vec::new()), shim], &["a.sh", "b.sh"], 4);
    let record = run_in(&dir, &suite).unwrap();

    let rows: Vec<(&str, &str)> = record
        .rows
        .iter()
        .map(|row| (row.runtime.as_str(), row.workload.as_str()))
        .collect();
    assert_eq!(
        rows,
        [("sh", "a"), ("sh_shim", "a"), ("sh", "b"), ("sh_shim", "b")]
    );
    for row in &record.rows {
        assert_eq!(row.wall_ns.samples.len(), 4);
        assert_eq!(row.wall_ns.stats.n, 4);
        assert!(row.wall_ns.samples.iter().all(|&ns| ns > 0));
        assert_eq!(row.warmup, 1);
    }
    // the noise row is the first row measured again
    assert_eq!(
        (
            record.noise.runtime.as_str(),
            record.noise.workload.as_str()
        ),
        ("sh", "a")
    );
    assert_eq!(record.noise.wall_ns.samples.len(), 4);
    assert!(record.noise.median_deviation >= 0.0);

    assert_eq!(record.runtimes.len(), 2);
    assert_eq!(record.runtimes[0].version, "sh test");
    assert_eq!(record.runtimes[0].v8, None);
    assert_eq!(record.runtimes[0].blake3.len(), 64);
    assert_eq!(record.config_hash, suite.config_hash());
    assert_eq!(record.suite.name, "test");
}

#[test]
fn a_changed_artifact_discards_the_run() {
    // the workload rewrites its own script, as a rebuild mid-run would
    let dir = scratch("changed", &[("grows.sh", "echo '#' >> \"$0\"\n")]);
    let script = dir.join("grows.sh").canonicalize().unwrap();
    let suite = suite(vec![sh(Vec::new())], &["grows.sh"], 2);
    match run_in(&dir, &suite) {
        Err(BenchError::ArtifactsChanged { paths }) => assert_eq!(paths, [script]),
        other => panic!("expected ArtifactsChanged, got {other:?}"),
    }
}

#[test]
fn a_failing_process_fails_the_run() {
    let dir = scratch("failing", &[("fails.sh", "exit 3\n")]);
    let suite = suite(vec![sh(Vec::new())], &["fails.sh"], 2);
    let error = run_in(&dir, &suite).unwrap_err();
    match &error {
        BenchError::CommandFailed { command, status } => {
            assert!(command.ends_with("fails.sh"), "{command}");
            assert_eq!(status.code(), Some(3));
        }
        other => panic!("expected CommandFailed, got {other:?}"),
    }
    // re-running unchanged fails the same way
    assert!(error.is_input_error());
}

#[test]
fn a_missing_program_is_an_input_error() {
    let dir = scratch("missing", &[("a.sh", "")]);
    let mut runtime = sh(Vec::new());
    runtime.program = "gro_bench_no_such_program".to_owned();
    let error = run_in(&dir, &suite(vec![runtime], &["a.sh"], 2)).unwrap_err();
    assert!(matches!(error, BenchError::ProgramNotFound { .. }));
    assert!(error.is_input_error());
}

#[cfg(unix)]
#[test]
fn a_script_program_is_an_input_error() {
    use std::os::unix::fs::PermissionsExt;
    // a wrapper script, as a version manager might put on `PATH`
    let dir = scratch(
        "script",
        &[("a.sh", ""), ("wrapper", "#!/bin/sh\nexec sh \"$@\"\n")],
    );
    let wrapper = dir.join("wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let mut runtime = sh(Vec::new());
    runtime.program = "./wrapper".to_owned();
    let error = run_in(&dir, &suite(vec![runtime], &["a.sh"], 2)).unwrap_err();
    match &error {
        BenchError::ProgramIsScript { path, .. } => {
            assert_eq!(path.file_name().unwrap(), "wrapper");
        }
        other => panic!("expected ProgramIsScript, got {other:?}"),
    }
    assert!(error.is_input_error());
    assert!(error.to_string().contains("name the real executable"));
}

#[test]
fn records_the_hash_of_each_suite_file() {
    let dir = scratch("artifacts", &[("a.sh", "true\n"), ("shim.sh", "")]);
    let script_hash = blake3::hash(b"true\n").to_hex().to_string();
    let shim_hash = blake3::hash(b"").to_hex().to_string();
    let runtime = sh(vec![Arg::File(FileArg {
        file: PathBuf::from("shim.sh"),
    })]);
    let record = run_in(&dir, &suite(vec![runtime], &["a.sh"], 2)).unwrap();
    assert_eq!(
        record.artifacts,
        BTreeMap::from([
            ("a.sh".to_owned(), script_hash),
            ("shim.sh".to_owned(), shim_hash),
        ])
    );
}

/// The probe sees what a measured process sees: `argv[0]` (from
/// `/proc/self/cmdline`) and the environment's variable names.
#[cfg(target_os = "linux")]
#[test]
fn processes_get_the_suite_argv0_and_an_isolated_environment() {
    let dir = scratch("isolated", &[("a.sh", "")]);
    let mut runtime = sh(Vec::new());
    // `sh` resolves to a canonical file of another name on most systems (`dash`)
    runtime.version_args = vec![
        "-c".to_owned(),
        r#"printf '%s %s\n' "$(tr '\0' '\n' < /proc/$$/cmdline | head -n 1)" "$(env | cut -d = -f 1 | sort | tr '\n' ,)""#
            .to_owned(),
    ];
    // cargo runs tests with variables like `CARGO_MANIFEST_DIR` set, which must not leak
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    let record = run_in(&dir, &suite(vec![runtime], &["a.sh"], 2)).unwrap();
    // the shell itself sets `PWD`
    assert_eq!(record.runtimes[0].version, "sh HOME,PATH,PWD,");
}
