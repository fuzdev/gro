//! The runtime's tests: the runtimes they start, from source and from the startup
//! snapshot, and the probes the snapshot's child process runs.

mod builtins;
mod error_classes;
mod eval_refusals;
mod host_ops;
mod native_members;
mod ops;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{Value, json};

use super::*;
use crate::STACK_SIZE;
use crate::bootstrap::web_scripts;
use crate::confine::{Confinement, Jit};
use crate::engine::{EngineOptions, init_engine};
use crate::host::error_class::{DeclaredFamily, ErrorClasses};
use crate::host::native::{
    self, Member, NativeMember, NativeNamespaces, Place, Prototypes, ValueKind, hands_over_error,
};
use crate::host::{HOST_OPS, NATIVE_MEMBERS};
use crate::loader::graph::Recording;
use crate::process_tests::{backing_scripts_file, make_startup_snapshot, run_test_in_child};
use crate::resolver::BUILTINS;
use crate::test_support::TestDir;
use crate::{ArgsGrant, EnvGrant, Grants};

/// What a test's runtime starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// The extensions' sources, as a plain build's does.
    Source,
    /// The startup snapshot, as the `fgro` that ships does. Only in
    /// [`snapshot_child`]'s process: V8 shares one read-only heap between
    /// a process's isolates, so every isolate of a process starts from
    /// the same snapshot or from none.
    Snapshot,
}

/// Both starts, for a test of what must hold from either.
const STARTS: [Start; 2] = [Start::Source, Start::Snapshot];

/// The variable naming the snapshot file [`snapshot_child`]'s runtimes
/// start from.
const SNAPSHOT_START: &str = "GRO_RUNTIME_TEST_SNAPSHOT_START";

/// What the line [`snapshot_child`] prints its results on starts with.
const RESULTS_MARK: &str = "snapshot_child results: ";

/// The variable naming the engine a [`snapshot_child`]'s process sets
/// up, one of [`EVAL_ENGINES`]' names: unset, the one `fgro run` does,
/// as in a test process's own run.
const ENGINE: &str = "GRO_RUNTIME_TEST_ENGINE";

/// Eval's engines, as [`ENGINE`] names them: jitless, as `fgro eval`
/// runs, and with the JIT, as `fgro eval --jit` does. An engine is its
/// process's, so each has a child of its own.
const EVAL_ENGINES: [(&str, Jit); 2] = [("eval_jitless", Jit::Off), ("eval_jit", Jit::On)];

/// The options of the engine this process's runtimes run on, as
/// [`ENGINE`] says.
fn engine_options() -> EngineOptions {
    let Some(name) = std::env::var_os(ENGINE) else {
        return EngineOptions::default();
    };
    let jit = EVAL_ENGINES
        .iter()
        .find_map(|(engine, jit)| (name == *engine).then_some(*jit))
        .unwrap_or_else(|| panic!("{ENGINE} names no engine: {}", name.display()));
    eval::engine_options(jit)
}

/// The variable naming the directory of the snapshot the build made,
/// `snapshot.bin` in it, which `fgro` embeds from there.
const BUILT_SNAPSHOT_DIR: &str = "GRO_SNAPSHOT_DIR";

/// The snapshot [`snapshot_child`]'s runtimes start from, read once from
/// the file [`SNAPSHOT_START`] names, with the scripts in the file
/// beside it as the residual table: the builtins' backing scripts, which
/// the process that made the snapshot wrote there, so this process
/// makes none, or none at all beside the build's snapshot. It comes
/// with no other residual script, so a runtime started from it loads no
/// web script the snapshot lacks.
fn child_snapshot() -> StartupSnapshot {
    static SNAPSHOT: OnceLock<StartupSnapshot> = OnceLock::new();
    *SNAPSHOT.get_or_init(|| {
        let file = std::env::var_os(SNAPSHOT_START)
            .expect("a runtime starts from the snapshot only in `snapshot_child`'s process");
        let scripts = std::fs::read_to_string(backing_scripts_file(Path::new(&file))).unwrap();
        let scripts: Vec<(String, String)> = serde_json::from_str(&scripts).unwrap();
        let table: Vec<(&'static str, &'static str)> = scripts
            .into_iter()
            .map(|(specifier, script)| (&*specifier.leak(), &*script.leak()))
            .collect();
        StartupSnapshot {
            bytes: Box::leak(std::fs::read(file).unwrap().into_boxed_slice()),
            residual_lazy_js: Box::leak(table.into_boxed_slice()),
        }
    })
}

/// Runs `test` on a runtime built as `mode` says, from source, as
/// [`on_runtime`] does.
fn on_new_runtime<T: Send + 'static>(
    name: &str,
    mode: Mode,
    test: impl FnOnce(&mut JsRuntime) -> T + Send + 'static,
) -> T {
    on_runtime(name, Start::Source, mode, test)
}

/// Runs `test` on a runtime started as `start` says and built as `mode`
/// says, before the bootstrap's per-run steps, so `Deno.core.ops` is
/// still there, on a thread reserving [`STACK_SIZE`].
fn on_runtime<T: Send + 'static>(
    name: &str,
    start: Start,
    mode: Mode,
    test: impl FnOnce(&mut JsRuntime) -> T + Send + 'static,
) -> T {
    use crate::resolver::DiskTree;

    init_engine(Confinement::Unconfined, &engine_options()).unwrap();
    let snapshot = (start == Start::Snapshot).then(child_snapshot);
    let dir = TestDir::on_disk(&format!("{name}_{start:?}"));
    std::fs::write(dir.path().join("main.mjs"), "").unwrap();
    let cwd = dir.path().to_owned();
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let (entry, roots) =
                crate::resolver::resolve_entry(&DiskTree, Path::new("main.mjs"), &cwd).unwrap();
            let loader = Rc::new(Loader::new(&entry, roots, Recording::Off));
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let _entered = tokio.enter();
            let create_params = (mode == Mode::Eval).then(eval_create_params);
            let mut runtime = new_runtime(snapshot, loader, mode, create_params).unwrap();
            test(&mut runtime)
        })
        .unwrap()
        .join()
        .unwrap()
}

/// A script's completion value, which must be a string of JSON, parsed.
fn script_json(runtime: &mut JsRuntime, source: impl Into<String>) -> Value {
    let value = runtime.execute_script("<test>", source.into()).unwrap();
    deno_core::scope!(scope, runtime);
    let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
    serde_json::from_str(&json).unwrap()
}

/// A probe: it makes its own runtimes, started as it's told, and
/// returns what it saw as JSON.
type Probe = fn(Start) -> Value;

/// What the child modules' tests ask of a runtime from either start, by name.
const PROBES: [(&str, Probe); 7] = [
    ("op_functions", ops::op_functions),
    ("unregistered_reads", ops::unregistered_reads),
    ("host_ops", host_ops::host_ops_results),
    ("refused_ops", eval_refusals::refused_ops_results),
    ("builtin_imports", builtins::builtin_imports),
    ("error_classes", error_classes::error_classes),
    ("native_members", native_members::native_members),
];

/// The probe eval's engines are asked: what eval refuses.
const EVAL_ENGINE_PROBE: &str = "refused_ops";

/// What the probe `name` saw of runtimes started as `start` says: from
/// source, run here; from the snapshot, as [`snapshot_child`] ran it.
/// Either way once a test process, since two tests ask for one probe
/// and its runtimes' directories are named for it.
fn probed(name: &str, start: Start) -> Value {
    static FROM_SOURCE: [OnceLock<Value>; PROBES.len()] = [const { OnceLock::new() }; PROBES.len()];
    match start {
        Start::Source => {
            let at = PROBES.iter().position(|(probe, _)| *probe == name).unwrap();
            FROM_SOURCE[at].get_or_init(|| PROBES[at].1(start)).clone()
        }
        Start::Snapshot => snapshot_results().run[name].clone(),
    }
}

/// What [`snapshot_child`] saw of runtimes started from the snapshot
/// this build's extensions make.
struct SnapshotResults {
    /// Every probe's results, by name, on the engine `fgro run` sets up.
    run: Value,
    /// [`eval_engine_results`], on each of [`EVAL_ENGINES`] in their
    /// order.
    eval: [Value; EVAL_ENGINES.len()],
}

/// The results from the snapshot this build's extensions make: a child
/// makes it, and [`snapshot_child`] runs the probes in others, one an
/// engine, once a test process.
fn snapshot_results() -> &'static SnapshotResults {
    static RESULTS: OnceLock<SnapshotResults> = OnceLock::new();
    RESULTS.get_or_init(|| {
        let dir = TestDir::in_system_temp("snapshot_start");
        let file = make_startup_snapshot(dir.path());
        SnapshotResults {
            run: snapshot_child_results(&file, None),
            eval: EVAL_ENGINES.map(|(engine, _)| eval_engine_results(&file, engine)),
        }
    })
}

/// [`EVAL_ENGINE_PROBE`]'s results on runtimes started from the snapshot
/// at `file`, on the engine `engine` names.
fn eval_engine_results(file: &Path, engine: &str) -> Value {
    snapshot_child_results(file, Some(engine))[EVAL_ENGINE_PROBE].clone()
}

/// What a [`snapshot_child`] printed, run on the snapshot at `file` and
/// the engine `engine` names, the one `fgro run` sets up with none: its
/// probes' results, by name.
fn snapshot_child_results(file: &Path, engine: Option<&str>) -> Value {
    let mut env = vec![(SNAPSHOT_START, file.as_os_str())];
    env.extend(engine.map(|engine| (ENGINE, OsStr::new(engine))));
    let output = run_test_in_child("runtime::tests::snapshot_child", &env);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let results = stdout
        .lines()
        .find_map(|line| line.strip_prefix(RESULTS_MARK))
        .unwrap();
    serde_json::from_str(results).unwrap()
}

/// The child [`snapshot_child_results`] runs, and nothing in a test
/// process's own run: the probes on runtimes started from the snapshot
/// [`SNAPSHOT_START`] names, the only runtimes of its process, on the
/// engine [`ENGINE`] names, their results printed as one line of JSON.
/// On the engine `fgro run` sets up it runs every probe, and on one of
/// eval's only [`EVAL_ENGINE_PROBE`].
#[test]
fn snapshot_child() {
    if std::env::var_os(SNAPSHOT_START).is_none() {
        return;
    }
    let eval_s = std::env::var_os(ENGINE).is_some();
    let results: serde_json::Map<String, Value> = PROBES
        .iter()
        .filter(|(name, _)| !eval_s || *name == EVAL_ENGINE_PROBE)
        .map(|(name, probe)| ((*name).to_owned(), probe(Start::Snapshot)))
        .collect();
    println!("\n{RESULTS_MARK}{}", Value::Object(results));
}

/// The name `deno_core` compiles `row`'s backing script under, which
/// the namespace op knows a row's script by.
fn backing_script(row: &str) -> &'static str {
    let at = BUILTINS
        .iter()
        .position(|builtin| builtin.specifier == row)
        .unwrap();
    bootstrap::BACKING_SPECIFIERS[at]
}

/// `source`'s completion value, a string of JSON, parsed, the script run
/// under `name`.
fn named_script_json(runtime: &mut JsRuntime, name: &'static str, source: String) -> Value {
    let value = runtime.execute_script(name, source).unwrap();
    deno_core::scope!(scope, runtime);
    let json = v8::Local::new(scope, value).to_rust_string_lossy(scope);
    serde_json::from_str(&json).unwrap()
}

/// The run's data the tests make the native members with.
fn host_data() -> HostData {
    HostData {
        args: vec!["a".to_owned()],
        cwd: Some("/work".to_owned()),
        runtime_version: "test/1.2.3".to_owned(),
        env: [("A", "1"), ("B", "2")]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .into_iter()
            .collect(),
        project: None,
        temp_dir: None,
    }
}
