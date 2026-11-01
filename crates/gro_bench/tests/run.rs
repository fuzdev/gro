//! The runner end to end, with `sh` as the runtime and shell scripts as the
//! workloads, so it needs no JS runtime installed.

// clippy's test allowance covers `#[test]` functions, not their helpers
#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use gro_bench::error::BenchError;
use gro_bench::record::{GitState, Machine, MachineQuiet, Mode, RunRecord};
use gro_bench::run::{Measure, RunContext, run};
use gro_bench::suite::{
    Arg, Budget, Cold, CorporaPin, FileArg, Project, Runtime, Stdout, Suite, Workload,
};

fn context() -> RunContext {
    RunContext {
        git: GitState {
            sha: "0".repeat(40),
            dirty: false,
        },
        machine: Machine::detect(),
        machine_quiet: MachineQuiet::Unchecked,
        // only a suite naming a collection writes here, and those tests set their own
        work_dir: std::env::temp_dir().join("gro_bench_unused_work"),
        measure: Measure::Wall,
        snapshot: None,
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
        env: BTreeMap::new(),
        cold: None,
        stdin: None,
        stdout: None,
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
                cold: false,
                weight: 1,
                collection: None,
                project: None,
            })
            .collect(),
        corpora: None,
        parity: Vec::new(),
        gate: None,
        budgets: BTreeMap::new(),
        prices: None,
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

/// Each sample's peak memory is recorded, above the floor every spawned
/// process reads at least, which the run measures too.
#[cfg(target_os = "linux")]
#[test]
fn records_each_sample_s_peak_memory_and_the_floor() {
    let dir = scratch(
        "peak",
        &[
            ("light.sh", "true\n"),
            // a 200 MB string, held by awk, a descendant sh reaps
            (
                "heavy.sh",
                "awk 'BEGIN { s = sprintf(\"%200000000s\", \"\"); print length(s) }' > /dev/null\n",
            ),
        ],
    );
    let suite = suite(vec![sh(Vec::new())], &["light.sh", "heavy.sh"], 2);
    let record = run_in(&dir, &suite).unwrap();
    let floor = record.rss_floor_kib.unwrap();
    let peak = |workload: &str| {
        record
            .rows
            .iter()
            .find(|row| row.workload == workload)
            .and_then(|row| row.peak_rss_kib.clone())
            .unwrap()
    };
    assert_eq!(peak("light").samples.len(), 2);
    assert!(floor > 0);
    assert!(peak("light").max() < 100_000, "{floor} {:?}", peak("light"));
    assert!(
        peak("heavy").stats.median > 190_000.0,
        "{:?}",
        peak("heavy")
    );
    assert!(floor < 100_000, "{floor}");
}

/// A runtime over a budget fails the run before any sample, naming it.
#[test]
fn a_size_over_budget_fails_the_run() {
    let dir = scratch("budget", &[("a.sh", "true\n")]);
    let mut suite = suite(vec![sh(Vec::new())], &["a.sh"], 2);
    suite.budgets = BTreeMap::from([(
        "sh".to_owned(),
        Budget {
            gzip_bytes: Some(1),
            ..Budget::default()
        },
    )]);
    let error = run(&suite, &dir, context()).unwrap_err();
    let message = error.to_string();
    assert!(
        matches!(
            error,
            BenchError::BudgetExceeded {
                budget: "gzip_bytes",
                limit: 1,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(
        message.contains("`sh`") && message.contains("gzip_bytes"),
        "{message}"
    );
    assert!(error.is_input_error());
    // a phases run, whose fgro is a measurement build, checks none, and
    // records no peaks
    let mut phases = context();
    phases.measure = Measure::Phases {
        runtimes: BTreeSet::new(),
    };
    let record = run(&suite, &dir, phases).unwrap();
    assert_eq!(record.mode, Mode::Phases);
    assert!(record.rows.iter().all(|row| row.peak_rss_kib.is_none()));
    assert_eq!(record.rss_floor_kib, None);
    // a snapshot budget with no snapshot to measure fails loudly too
    suite.budgets = BTreeMap::from([(
        "sh".to_owned(),
        Budget {
            snapshot_bytes: Some(1 << 30),
            ..Budget::default()
        },
    )]);
    let error = run(&suite, &dir, context()).unwrap_err();
    assert!(
        error.to_string().contains("no size was measured"),
        "{error}"
    );
    // within its limits, and the budgets left out of the config hash
    suite.budgets = BTreeMap::from([(
        "sh".to_owned(),
        Budget {
            bytes: Some(1 << 40),
            gzip_bytes: Some(1 << 40),
            snapshot_bytes: None,
        },
    )]);
    let record = run_in(&dir, &suite).unwrap();
    let mut without = suite;
    without.budgets.clear();
    assert_eq!(record.config_hash, without.config_hash());
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
    // the untimed correctness run catches a row that always fails
    let dir = scratch(
        "failing",
        &[("fails.sh", "echo out; echo err >&2; exit 3\n")],
    );
    let fails = suite(vec![sh(Vec::new())], &["fails.sh"], 2);
    let error = run_in(&dir, &fails).unwrap_err();
    match &error {
        BenchError::CheckFailed { command, reason } => {
            assert!(command.ends_with("fails.sh"), "{command}");
            assert!(reason.contains("exit status: 3"), "{reason}");
            assert!(reason.contains("stdout: out; stderr: err"), "{reason}");
        }
        other => panic!("expected CheckFailed, got {other:?}"),
    }
    // re-running unchanged fails the same way
    assert!(error.is_input_error());

    // and a timed process that fails after the check passed fails the run too
    let dir = scratch(
        "failing_later",
        &[("later.sh", "test -e \"$0.ran\" && exit 3; : > \"$0.ran\"\n")],
    );
    let later = suite(vec![sh(Vec::new())], &["later.sh"], 2);
    let error = run_in(&dir, &later).unwrap_err();
    match &error {
        BenchError::CommandFailed { command, status } => {
            assert!(command.ends_with("later.sh"), "{command}");
            assert_eq!(status.code(), Some(3));
        }
        other => panic!("expected CommandFailed, got {other:?}"),
    }
    assert!(error.is_input_error());
}

/// Every process reads the runtime's `stdin` file from its start, the
/// correctness run's, the warmups', and each sample's, and the file is
/// hashed as an artifact.
#[test]
fn each_process_reads_the_runtime_stdin_afresh() {
    let dir = scratch(
        "stdin",
        &[
            ("reads.sh", "read -r line && test \"$line\" = hello\n"),
            ("in.txt", "hello\nworld\n"),
        ],
    );
    let mut runtime = sh(Vec::new());
    runtime.stdin = Some(FileArg {
        file: PathBuf::from("in.txt"),
    });
    let record = run_in(&dir, &suite(vec![runtime.clone()], &["reads.sh"], 4)).unwrap();
    assert_eq!(record.rows[0].wall_ns.samples.len(), 4);
    assert_eq!(
        record.artifacts.keys().collect::<Vec<_>>(),
        ["in.txt", "reads.sh"]
    );
    // without it, stdin is the null device, and the read fails
    let dir = scratch("no_stdin", &[("reads.sh", "read -r line\n")]);
    runtime.stdin = None;
    let error = run_in(&dir, &suite(vec![runtime], &["reads.sh"], 2)).unwrap_err();
    assert!(matches!(error, BenchError::CheckFailed { .. }), "{error:?}");
}

/// A runtime printing `stdout` as `sh -c` does, its script argument unused.
fn printing(name: &str, stdout: &str, check: Stdout) -> Runtime {
    let mut runtime = sh(vec![
        Arg::Text("-c".to_owned()),
        Arg::Text(format!("printf '%s' '{stdout}'")),
    ]);
    name.clone_into(&mut runtime.name);
    runtime.stdout = Some(check);
    runtime
}

#[test]
fn the_correctness_run_checks_stdout_and_parity() {
    let envelope = r#"{"ok": true, "value": {"a": [1, {"b": "c"}]}}"#;
    let json = r#"{"a": [1, {"b": "c"}]}"#;
    let other = r#"{"a": [1, {"b": "d"}]}"#;
    let run_with = |test: &str, runtimes: Vec<Runtime>, parity: &[&str]| {
        let dir = scratch(test, &[("w.sh", "")]);
        let mut suite = suite(runtimes, &["w.sh"], 2);
        suite.parity = parity.iter().map(|name| (*name).to_owned()).collect();
        run_in(&dir, &suite)
    };
    // an envelope's value equal to another runtime's JSON
    let equal = vec![
        printing("fgro", envelope, Stdout::Envelope),
        printing("deno", json, Stdout::Json),
    ];
    assert!(run_with("parity_equal", equal, &["fgro", "deno"]).is_ok());
    // a value that differs, named where
    let differ = vec![
        printing("fgro", envelope, Stdout::Envelope),
        printing("deno", other, Stdout::Json),
    ];
    let error = run_with("parity_differs", differ.clone(), &["fgro", "deno"]).unwrap_err();
    match &error {
        BenchError::ParityMismatch {
            workload,
            runtime,
            against,
            difference,
        } => {
            assert_eq!(
                (workload.as_str(), runtime.as_str(), against.as_str()),
                ("w", "deno", "fgro")
            );
            assert_eq!(
                (
                    difference.at.as_str(),
                    difference.a.as_str(),
                    difference.b.as_str()
                ),
                (r#"$["a"][1]["b"]"#, r#""d""#, r#""c""#)
            );
            assert!(
                error
                    .to_string()
                    .ends_with(r#"at $["a"][1]["b"]: "d" against "c""#),
                "{error}"
            );
        }
        other => panic!("expected ParityMismatch, got {other:?}"),
    }
    assert!(error.is_input_error());
    // with no parity, differing values are fine
    assert!(run_with("parity_none", differ, &[]).is_ok());
    // an envelope that isn't ok, and stdout that isn't JSON
    let not_ok = printing(
        "fgro",
        r#"{"ok": false, "error": {"kind": "resolve"}}"#,
        Stdout::Envelope,
    );
    let error = run_with("not_ok", vec![not_ok], &[]).unwrap_err();
    assert!(
        matches!(error, BenchError::CheckFailed { ref reason, .. } if reason.contains("resolve")),
        "{error:?}"
    );
    let not_json = printing("deno", "nope", Stdout::Json);
    let error = run_with("not_json", vec![not_json], &[]).unwrap_err();
    assert!(
        matches!(error, BenchError::CheckFailed { ref reason, .. } if reason.contains("isn't one JSON value")),
        "{error:?}"
    );
}

/// Runs `git -C dir` with `args`, with a throwaway identity and no signing,
/// and returns its trimmed stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Makes `dir` a git checkout with everything in it committed.
fn commit_all(dir: &Path) {
    git(dir, &["init", "--quiet"]);
    git(dir, &["add", "."]);
    git(dir, &["commit", "--quiet", "-m", "init"]);
}

/// A project workload runs its script from a fresh copy holding the
/// committed files, the stubs, and the links.
#[cfg(unix)]
#[test]
fn a_project_workload_runs_from_a_fresh_copy() {
    // it runs only beside its copy's node_modules, with the stubs and the link
    let entry = "cd \"$(dirname \"$0\")\" && test -f node_modules/stub_0001/package.json \
        && test ! -e node_modules/stub_0002 && test -f node_modules/linked/marker \
        && test -f lib/x.sh\n";
    let dir = scratch("project", &[]);
    fs::create_dir_all(dir.join("p/lib")).unwrap();
    fs::write(dir.join("p/main.sh"), entry).unwrap();
    fs::write(dir.join("p/lib/x.sh"), "").unwrap();
    fs::create_dir_all(dir.join("checkout")).unwrap();
    fs::write(dir.join("checkout/marker"), "").unwrap();
    commit_all(&dir.join("checkout"));
    let sha = git(&dir.join("checkout"), &["rev-parse", "HEAD"]);
    let mut suite = suite(vec![sh(Vec::new())], &["p/main.sh"], 2);
    suite.workloads[0].name = "proj".to_owned();
    suite.workloads[0].project = Some(Project {
        dir: PathBuf::from("p"),
        stub_packages: 2,
        links: BTreeMap::from([("linked".to_owned(), PathBuf::from("checkout"))]),
        generate: BTreeMap::new(),
    });
    let mut context = context();
    context.work_dir = dir.join("work");
    let record = run(&suite, &dir, context.clone()).unwrap();
    let copied = dir.join("work/projects/proj/main.sh").is_file();
    // a link outside any git checkout is refused, since nothing names its code
    fs::create_dir_all(dir.join("plain")).unwrap();
    fs::write(dir.join("plain/marker"), "").unwrap();
    suite.workloads[0].project.as_mut().unwrap().links =
        BTreeMap::from([("linked".to_owned(), PathBuf::from("plain"))]);
    let unversioned = run(&suite, &dir, context.clone()).unwrap_err();
    // a missing checkout fails before anything runs
    suite.workloads[0].project.as_mut().unwrap().links =
        BTreeMap::from([("linked".to_owned(), PathBuf::from("gone"))]);
    let missing = run(&suite, &dir, context).unwrap_err();
    fs::remove_dir_all(&dir).unwrap();
    assert!(copied);
    assert_eq!(record.rows.len(), 1);
    assert_eq!(record.artifacts.keys().collect::<Vec<_>>(), ["p/main.sh"]);
    // the link's checkout, by its path as the suite writes it
    let state = &record.checkouts["checkout"];
    assert_eq!(
        (state.sha.as_str(), state.dirty, &state.changes_blake3),
        (sha.as_str(), false, &None)
    );
    assert!(
        matches!(unversioned, BenchError::ProjectLinkUnversioned { .. }),
        "{unversioned:?}"
    );
    assert!(unversioned.is_input_error());
    assert!(
        matches!(missing, BenchError::ProjectLinkMissing { ref package, .. } if package == "linked"),
        "{missing:?}"
    );
    assert!(missing.is_input_error());
}

/// A change to a linked checkout during the run discards it, even when the
/// checkout was dirty already and only an untracked file's contents move.
#[cfg(unix)]
#[test]
fn a_linked_checkout_changing_mid_run_discards_it() {
    let dir = scratch("checkout_moves", &[]);
    fs::create_dir_all(dir.join("p")).unwrap();
    fs::create_dir_all(dir.join("checkout")).unwrap();
    fs::write(dir.join("checkout/tracked"), "").unwrap();
    commit_all(&dir.join("checkout"));
    fs::write(dir.join("checkout/notes"), "").unwrap();
    // each process appends to the untracked file through the link
    fs::write(
        dir.join("p/main.sh"),
        "cd \"$(dirname \"$0\")\" && echo x >> node_modules/linked/notes\n",
    )
    .unwrap();
    let mut suite = suite(vec![sh(Vec::new())], &["p/main.sh"], 2);
    suite.workloads[0].name = "proj".to_owned();
    suite.workloads[0].project = Some(Project {
        dir: PathBuf::from("p"),
        stub_packages: 0,
        links: BTreeMap::from([("linked".to_owned(), PathBuf::from("checkout"))]),
        generate: BTreeMap::new(),
    });
    let mut context = context();
    context.work_dir = dir.join("work");
    let error = run(&suite, &dir, context).unwrap_err();
    let checkout = dir.join("checkout").canonicalize().unwrap();
    fs::remove_dir_all(&dir).unwrap();
    match &error {
        BenchError::ArtifactsChanged { paths } => assert_eq!(paths, &[checkout]),
        other => panic!("expected ArtifactsChanged, got {other:?}"),
    }
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

#[test]
fn a_cold_row_runs_with_the_runtime_env_and_its_cold_env() {
    // the workload fails unless both variables reach the timed process
    let dir = scratch(
        "cold",
        &[(
            "cold.sh",
            "test \"$GRO_BENCH_ALL\" = 1 && test \"$GRO_BENCH_COLD\" = 1\n",
        )],
    );
    let mut runtime = sh(Vec::new());
    runtime.env = BTreeMap::from([("GRO_BENCH_ALL".to_owned(), "1".to_owned())]);
    runtime.cold = Some(Cold {
        args: Vec::new(),
        env: BTreeMap::from([("GRO_BENCH_COLD".to_owned(), "1".to_owned())]),
    });
    let mut suite = suite(vec![runtime], &["cold.sh"], 2);
    suite.workloads[0].cold = true;
    let record = run_in(&dir, &suite).unwrap();
    assert_eq!(record.rows[0].wall_ns.samples.len(), 2);
    assert!(record.runtimes[0].bytes > 0);
    assert!(record.runtimes[0].gzip_bytes > 0);
}

/// Commits `files` (relative to its `collections/`) in a fresh git checkout at
/// `<dir>/corpora`.
fn corpora_checkout(dir: &Path, files: &[(&str, &str)]) {
    let corpora = dir.join("corpora");
    for (path, content) in files {
        let path = corpora.join("collections").join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    for args in [
        &["init", "--quiet"][..],
        &["add", "."],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "init",
        ],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&corpora)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
}

#[test]
fn a_corpora_checkout_off_its_pin_is_an_input_error() {
    let dir = scratch("corpora", &[("a.sh", "")]);
    corpora_checkout(&dir, &[("x.ts", "")]);
    let mut suite = suite(vec![sh(Vec::new())], &["a.sh"], 2);
    suite.corpora = Some(CorporaPin {
        path: PathBuf::from("corpora"),
        tree: "0".repeat(40),
    });
    let error = run(&suite, &dir, context()).unwrap_err();
    let BenchError::CorporaTree { found, .. } = &error else {
        panic!("expected CorporaTree, got {error:?}");
    };
    assert!(error.is_input_error());
    // at its pin, the run records the tree
    suite.corpora = Some(CorporaPin {
        path: PathBuf::from("corpora"),
        tree: found.clone(),
    });
    let record = run_in(&dir, &suite).unwrap();
    assert_eq!(record.corpora.as_ref(), Some(found));
}

/// The `collections/` tree id of the checkout `corpora_checkout` made in `dir`.
fn corpora_tree(dir: &Path) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.join("corpora"))
        .args(["rev-parse", "HEAD:collections"])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A suite whose workloads run `scripts` inside the collection `c` of a
/// corpora checkout in `dir`, holding `lib.sh` and `package.json`.
fn collection_suite(dir: &Path, scripts: &[&str]) -> (Suite, RunContext) {
    corpora_checkout(
        dir,
        &[
            ("c/lib.sh", "LIB=1\n"),
            ("c/package.json", "{}\n"),
            ("d/other.sh", ""),
        ],
    );
    let mut suite = suite(vec![sh(Vec::new())], scripts, 2);
    suite.corpora = Some(CorporaPin {
        path: PathBuf::from("corpora"),
        tree: corpora_tree(dir),
    });
    for workload in &mut suite.workloads {
        workload.collection = Some("c".to_owned());
    }
    let mut context = context();
    context.work_dir = dir.join("work");
    (suite, context)
}

#[test]
fn a_collection_workload_runs_from_a_fresh_copy() {
    // it runs only beside the collection's `lib.sh`, from the suite's directory
    let entry =
        "test \"$(pwd -P)\" = \"$SUITE\" && . \"$(dirname \"$0\")/lib.sh\" && test \"$LIB\" = 1\n";
    let dir = scratch("collection", &[("entry.sh", entry), ("plain.sh", "true\n")]);
    let (mut suite, context) = collection_suite(&dir, &["entry.sh", "plain.sh"]);
    suite.workloads[1].collection = None;
    let canonical = dir.canonicalize().unwrap();
    suite.runtimes[0].env = BTreeMap::from([("SUITE".to_owned(), canonical.display().to_string())]);
    let tree = suite.corpora.as_ref().unwrap().tree.clone();
    let copy = dir.join("work/corpora").join(&tree).join("c");
    // a stale copy, which the run replaces
    fs::create_dir_all(&copy).unwrap();
    fs::write(copy.join("stale.sh"), "").unwrap();
    let record = run(&suite, &dir, context).unwrap();
    let mut listed: Vec<String> = fs::read_dir(&copy)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    listed.sort();
    let copied_entry = fs::read_to_string(copy.join("entry.sh")).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(listed, ["entry.sh", "lib.sh", "package.json"]);
    assert_eq!(copied_entry, entry);
    assert_eq!(record.rows.len(), 2);
    // the record names the scripts by their suite paths, not the copy's
    assert_eq!(
        record.artifacts.keys().collect::<Vec<_>>(),
        ["entry.sh", "plain.sh"]
    );
    assert_eq!(record.corpora, Some(tree));
}

#[test]
fn a_path_written_into_a_copy_discards_the_run() {
    // as a runtime writing a lockfile beside the collection's `package.json` would
    let dir = scratch(
        "collection_write",
        &[("writes.sh", ": > \"$(dirname \"$0\")/lock.json\"\n")],
    );
    let (suite, context) = collection_suite(&dir, &["writes.sh"]);
    let result = run(&suite, &dir, context);
    let lock = dir
        .join("work/corpora")
        .join(&suite.corpora.as_ref().unwrap().tree)
        .join("c/lock.json")
        .canonicalize()
        .unwrap();
    fs::remove_dir_all(&dir).unwrap();
    match result {
        Err(BenchError::ArtifactsChanged { paths }) => assert_eq!(paths, [lock]),
        other => panic!("expected ArtifactsChanged, got {other:?}"),
    }
}

#[test]
fn a_file_rewritten_in_a_copy_discards_the_run() {
    // the path set is unchanged, so only the copy's hashes catch it
    let dir = scratch(
        "collection_rewrite",
        &[(
            "rewrites.sh",
            "echo LIB=2 >> \"$(dirname \"$0\")/lib.sh\"\n",
        )],
    );
    let (suite, context) = collection_suite(&dir, &["rewrites.sh"]);
    let result = run(&suite, &dir, context);
    let lib = dir
        .join("work/corpora")
        .join(&suite.corpora.as_ref().unwrap().tree)
        .join("c/lib.sh")
        .canonicalize()
        .unwrap();
    fs::remove_dir_all(&dir).unwrap();
    match result {
        Err(BenchError::ArtifactsChanged { paths }) => assert_eq!(paths, [lib]),
        other => panic!("expected ArtifactsChanged, got {other:?}"),
    }
}

#[test]
fn a_collection_entry_must_be_new_to_the_collection() {
    let dir = scratch("collection_collides", &[("lib.sh", "true\n")]);
    let (mut suite, context) = collection_suite(&dir, &["lib.sh"]);
    let collides = run(&suite, &dir, context.clone()).unwrap_err();
    // and the collection must exist in the pinned tree
    suite.workloads[0].collection = Some("nope".to_owned());
    let error = run(&suite, &dir, context).unwrap_err();
    fs::remove_dir_all(&dir).unwrap();
    assert!(
        matches!(collides, BenchError::EntryCollides { ref entry, .. } if entry == Path::new("lib.sh")),
        "{collides:?}"
    );
    assert!(collides.is_input_error());
    assert!(
        matches!(error, BenchError::CollectionNotFound { ref name, .. } if name == "nope"),
        "{error:?}"
    );
    assert!(error.is_input_error());
}

/// A phases run gives the runtimes it names the file to write, and turns each
/// sample's marks into durations from the spawn to the reap; the others run
/// as they are.
#[test]
fn a_phases_run_records_each_sample_s_phases() {
    let dir = scratch(
        "phases",
        &[(
            "marks.sh",
            "[ -n \"$GRO_PHASES_FILE\" ] || exit 0\n\
             printf 'gro_phases 1\\nmain_unix_ns %s\\nwork 1000\\nexit 3000\\n' \
             \"$(date +%s%N)\" > \"$GRO_PHASES_FILE\"\n",
        )],
    );
    let mut plain = sh(Vec::new());
    plain.name = "plain".to_owned();
    let suite = suite(vec![sh(Vec::new()), plain], &["marks.sh"], 4);
    let mut context = context();
    context.work_dir = dir.join("work");
    context.measure = Measure::Phases {
        runtimes: BTreeSet::from(["sh".to_owned()]),
    };
    let result = run(&suite, &dir, context);
    fs::remove_dir_all(&dir).unwrap();
    let record = result.unwrap();
    assert_eq!(record.mode, Mode::Phases);
    let row = &record.rows[0];
    let names: Vec<&str> = row.phases.iter().map(|phase| phase.name.as_str()).collect();
    assert_eq!(names, ["start", "work", "exit", "reap"]);
    for phase in &row.phases {
        assert_eq!(phase.ns.samples.len(), 4);
    }
    assert_eq!(row.phases[1].ns.samples, [1000; 4]);
    assert_eq!(row.phases[2].ns.samples, [2000; 4]);
    // the spans sum to about the wall time, each sample
    for (index, wall) in row.wall_ns.samples.iter().enumerate() {
        let sum: u64 = row.phases.iter().map(|phase| phase.ns.samples[index]).sum();
        assert!(sum.abs_diff(*wall) < wall / 10, "{sum} against {wall}");
    }
    assert!(record.rows[1].phases.is_empty());
    assert!(row.instructions.is_none());
}

/// A phases run of a runtime that writes no file fails, naming it.
#[test]
fn a_phases_run_of_a_plain_build_fails() {
    let dir = scratch("phases_plain", &[("plain.sh", "true\n")]);
    let suite = suite(vec![sh(Vec::new())], &["plain.sh"], 2);
    let mut context = context();
    context.work_dir = dir.join("work");
    context.measure = Measure::Phases {
        runtimes: BTreeSet::from(["sh".to_owned()]),
    };
    let result = run(&suite, &dir, context.clone());
    context.measure = Measure::Phases {
        runtimes: BTreeSet::from(["nope".to_owned()]),
    };
    let unknown = run(&suite, &dir, context);
    fs::remove_dir_all(&dir).unwrap();
    assert!(
        matches!(&result, Err(BenchError::Phases(message)) if message.contains("isn't a phases build")),
        "{result:?}"
    );
    assert!(
        matches!(&unknown, Err(BenchError::SuiteInvalid(message)) if message.contains("`nope`")),
        "{unknown:?}"
    );
}

/// A counter run counts every sample under `perf stat`, the noise row's
/// too; where perf can't count here, it says so and the test skips.
#[test]
fn a_counter_run_counts_every_sample() {
    let dir = scratch("counters", &[("counted.sh", "true\n")]);
    let suite = suite(vec![sh(Vec::new())], &["counted.sh"], 4);
    let mut context = context();
    context.work_dir = dir.join("work");
    context.measure = Measure::Counters;
    let result = run(&suite, &dir, context);
    fs::remove_dir_all(&dir).unwrap();
    let record = match result {
        Err(BenchError::Counters(why)) => {
            eprintln!("perf can't count here, so nothing to check: {why}");
            return;
        }
        other => other.unwrap(),
    };
    assert_eq!(record.mode, Mode::Counters);
    // the kernel's instructions too where perf may count them for this user
    let events = &record.counters.as_ref().unwrap().events;
    let kernel = events.len() == 3;
    assert_eq!(events[..2], ["instructions:u", "cycles:u"]);
    assert!(!kernel || events[2] == "instructions:k", "{events:?}");
    let row = &record.rows[0];
    assert_eq!(row.kernel_instructions.is_some(), kernel);
    for metric in [row.instructions.as_ref(), row.cycles.as_ref()]
        .into_iter()
        .chain(kernel.then_some(row.kernel_instructions.as_ref()))
    {
        let metric = metric.unwrap();
        assert_eq!(metric.samples.len(), 4);
        assert!(metric.samples.iter().all(|&count| count > 0));
    }
    assert!(record.noise.instructions.is_some());
    assert!(record.noise.instructions_median_deviation.is_some());
    // perf, the spawner, would mask a small row's peak: none is recorded
    assert_eq!(row.peak_rss_kib, None);
    assert_eq!(record.rss_floor_kib, None);
}
