//! `cargo xtask check-reproducible` as its binary runs: the refusals that
//! read the process's own environment.

use std::process::Command;

/// A variable that configures how cargo compiles, or a rustc wrapper, in
/// the process's environment is refused, exit 2, before the `--dir` (one
/// that climbs, refused next) is read.
#[test]
fn the_process_environment_is_refused_first() {
    for ((name, value), named) in [
        (
            ("CARGO_INCREMENTAL", "1"),
            "would take CARGO_INCREMENTAL and the clone's wouldn't",
        ),
        (
            ("RUSTC_WRAPPER", "sccache"),
            "the check's environment sets a rustc wrapper (RUSTC_WRAPPER)",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["check-reproducible", "--dir", "/nonexistent/../refused"])
            .env_clear()
            .env(name, value)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains(named), "{stderr}");
    }
}
