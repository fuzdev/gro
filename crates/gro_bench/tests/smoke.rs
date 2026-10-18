//! The smoke suite (`tests/fixtures/smoke/`, an empty script) under whichever of its
//! runtimes are installed: proves the plumbing against real runtimes at a
//! minimal sample count. It measures nothing worth keeping and writes no
//! history.

use std::path::Path;

use gro_bench::record::{GitState, Machine, MachineQuiet};
use gro_bench::run::{RunContext, find_program, run};
use gro_bench::suite::Suite;

#[test]
fn smoke_suite_runs_under_installed_runtimes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke");
    let mut suite = Suite::load(&dir.join("suite.json")).unwrap();
    suite
        .runtimes
        .retain(|runtime| find_program(&runtime.program, &dir).is_some());
    if suite.runtimes.is_empty() {
        eprintln!("no runtime of the smoke suite is installed, nothing to run");
        return;
    }
    let context = RunContext {
        git: GitState {
            sha: "0".repeat(40),
            dirty: false,
        },
        machine: Machine::detect(),
        machine_quiet: MachineQuiet::Unchecked,
    };
    let record = run(&suite, &dir, context).unwrap();
    assert_eq!(record.runtimes.len(), suite.runtimes.len());
    assert_eq!(record.rows.len(), suite.runtimes.len());
    for (row, runtime) in record.rows.iter().zip(&suite.runtimes) {
        assert_eq!(row.runtime, runtime.name);
        assert_eq!(row.wall_ns.samples.len(), 2);
    }
    for (info, runtime) in record.runtimes.iter().zip(&suite.runtimes) {
        assert!(!info.version.is_empty());
        assert_eq!(info.v8.is_some(), runtime.v8_args.is_some());
    }
    assert_eq!(record.noise.wall_ns.samples.len(), 2);
    assert_eq!(
        record.artifacts.keys().collect::<Vec<_>>(),
        ["empty.js"],
        "the workload script, by its suite path"
    );
}
