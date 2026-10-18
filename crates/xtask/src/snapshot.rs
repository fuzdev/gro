//! `cargo xtask build`: `fgro` with an embedded startup snapshot, in two
//! stages, and the checks that it runs off its build tree. `<target>` is
//! cargo's target directory (`target/` unless `CARGO_TARGET_DIR` or
//! `build.target-dir` moves it).
//!
//! 1. `cargo build -p gro_cli` builds the plain `fgro`, which runs
//!    `fgro make-snapshot <target>/gro_snapshot/<profile>/`. That writes
//!    `snapshot.bin` and `residual.rs` (the lazily loaded sources the snapshot
//!    doesn't carry, embedded by absolute path). The step is skipped when it
//!    last ran after the plain binary was built (a stamp file records the
//!    run), since nothing else feeds the snapshot: the registry sources it
//!    reads are immutable per version. Snapshots aren't byte-identical from
//!    run to run, so a rerun always relinks stage two.
//! 2. `cargo build -p gro_cli --features snapshot`, with `GRO_SNAPSHOT_DIR`
//!    naming that directory, embeds them, and the binary is hard-linked to
//!    `<target>/gro_snapshot/<profile>/fgro`, so later plain builds, which
//!    replace `<target>/<profile>/fgro`, leave it alone.
//!
//! Between the stage-one build and `make-snapshot`, the V8 library copy is
//! checked against the pin ([`crate::v8_archive::verify_linked`]), before any
//! V8 code runs. `make-snapshot`'s stdout, which carries only `deno_core`'s
//! progress lines, is discarded; its errors print to stderr.
//!
//! Cargo keeps both builds of `fgro`, one per feature set, so switching
//! between them relinks nothing.
//!
//! `--verify` then checks the result: clippy over the `snapshot` feature's
//! code, `fgro`'s integration tests against the snapshot build, and a fixture
//! run with the cargo registry's sources (`$CARGO_HOME/registry/src`) and
//! gro's own extension sources hidden (tmpfs mounts in a private user and
//! mount namespace, `unshare -rm`), its output compared with a run that can
//! see them. The fixture is TypeScript, so the strip runs off the build tree
//! too. A machine without unprivileged user namespaces fails that check
//! rather than skipping it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Output, Stdio};

use crate::{StepResult, run};

/// The fixture both runs of the off-build-tree check execute: a TypeScript
/// entry over the web globals, so the snapshot build proves the strip too.
const SMOKE_FIXTURE: &str = "smoke.ts";

/// Builds the snapshot `fgro` and returns its path.
///
/// Its paths come from `target_dir`, cargo's target directory, which
/// `CARGO_TARGET_DIR` or `build.target-dir` may move off the checkout.
pub fn build(root: &Path, target_dir: &Path, release: bool) -> Result<PathBuf, ExitCode> {
    let profile = if release { "release" } else { "debug" };
    let profile_args: &[&str] = if release { &["--release"] } else { &[] };
    let dir = target_dir.join("gro_snapshot").join(profile);
    let dir_arg = dir.display().to_string();

    let mut stage_one = vec!["build", "--locked", "-p", "gro_cli"];
    stage_one.extend(profile_args);
    run(root, &[], "cargo", &stage_one)?;
    // before stage one runs V8 code, and after the build that wrote the copy
    crate::verify_linked_step(target_dir, profile)?;
    let plain = target_dir.join(profile).join("fgro");
    let stamp = dir.join("stage_one.stamp");
    if snapshot_is_current(&dir, &stamp, &plain) {
        println!(
            "\n[xtask] the snapshot in {dir_arg} was made by the current {}",
            crate::relative(root, &plain)
        );
    } else {
        // its stdout carries only deno_core's progress lines; its errors go to stderr
        crate::run_program_to(
            root,
            &[],
            &plain,
            &crate::relative(root, &plain),
            &["make-snapshot", &dir_arg],
            Stdio::null(),
        )?;
        // the outputs are rewritten on every run and differ each time, so the stamp records when it ran
        fs::write(&stamp, b"").map_err(|error| {
            eprintln!("[xtask] failed to write {}: {error}", stamp.display());
            ExitCode::FAILURE
        })?;
    }

    let mut stage_two = vec![
        "build",
        "--locked",
        "-p",
        "gro_cli",
        "--features",
        "snapshot",
    ];
    stage_two.extend(profile_args);
    run(root, &[("GRO_SNAPSHOT_DIR", &dir_arg)], "cargo", &stage_two)?;
    let built = target_dir.join(profile).join("fgro");
    let linked = dir.join("fgro");
    let _ = fs::remove_file(&linked);
    fs::hard_link(&built, &linked).map_err(|error| {
        eprintln!(
            "[xtask] failed to link {} to {}: {error}",
            built.display(),
            linked.display()
        );
        ExitCode::FAILURE
    })?;
    println!("\n[xtask] built {}", linked.display());
    Ok(linked)
}

/// Whether `snapshot.bin` and `residual.rs` exist in `dir` and stage one last
/// ran (the `stamp`) after the plain binary was built.
fn snapshot_is_current(dir: &Path, stamp: &Path, plain: &Path) -> bool {
    let modified = |path: &Path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    };
    ["snapshot.bin", "residual.rs"]
        .iter()
        .all(|file| dir.join(file).is_file())
        && matches!((modified(stamp), modified(plain)), (Some(ran), Some(built)) if ran > built)
}

/// Checks the snapshot build at `binary`: the integration tests against it,
/// then the off-build-tree run.
pub fn verify(root: &Path, release: bool, binary: &Path) -> StepResult {
    let dir = binary.parent().unwrap_or(root).display().to_string();
    // the workspace clippy builds without the feature, so its code goes unlinted there
    run(
        root,
        &[("GRO_SNAPSHOT_DIR", &dir)],
        "cargo",
        &[
            "clippy",
            "--locked",
            "-p",
            "gro_cli",
            "--features",
            "snapshot",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    let mut test = vec![
        "test",
        "--locked",
        "-p",
        "gro_cli",
        "--features",
        "snapshot",
        "--test",
        "run",
    ];
    if release {
        test.push("--release");
    }
    run(root, &[("GRO_SNAPSHOT_DIR", &dir)], "cargo", &test)?;
    off_build_tree(root, binary)
}

/// Runs the smoke fixture with `binary` twice, once seeing the build tree and
/// once with the registry's sources and gro's extension sources hidden, and
/// fails unless both succeed with the same output.
fn off_build_tree(root: &Path, binary: &Path) -> StepResult {
    let fixtures = root.join("crates/gro_cli/tests/fixtures/run");
    let gro_js = root.join("crates/gro_runtime/js");
    // every crate's sources, the `deno_web` and `deno_webidl` JS among them
    let registry = cargo_home().join("registry/src");
    // a residual source must vanish with it, or the check proves nothing
    let residual = binary.with_file_name("residual.rs");
    let probe_source = registry_source(&residual).ok_or_else(|| {
        eprintln!(
            "[xtask] no residual source path in {} to check the hiding with",
            residual.display()
        );
        ExitCode::FAILURE
    })?;
    if !probe_source.starts_with(&registry) {
        eprintln!(
            "[xtask] {} isn't under {}, the registry sources this check hides",
            probe_source.display(),
            registry.display()
        );
        return Err(ExitCode::FAILURE);
    }

    // the mechanism first, so a machine without it fails as such, not as a fixture failure
    let probe = Command::new("unshare")
        .args(["-rm", "sh", "-c", "mount -t tmpfs tmpfs \"$1\"", "sh"])
        .arg(&gro_js)
        .output();
    if !probe.as_ref().is_ok_and(|output| output.status.success()) {
        eprintln!(
            "[xtask] the off-build-tree check needs unprivileged user and mount namespaces \
             (`unshare -rm` with a tmpfs mount), and they're unavailable here: {}",
            probe.map_or_else(
                |error| error.to_string(),
                |output| String::from_utf8_lossy(&output.stderr).trim().to_owned()
            )
        );
        return Err(ExitCode::FAILURE);
    }

    println!(
        "\n[xtask] {} run {SMOKE_FIXTURE}, seeing the build tree and then with {} and {} hidden",
        crate::relative(root, binary),
        registry.display(),
        crate::relative(root, &gro_js)
    );
    let seen = Command::new(binary)
        .args(["run", SMOKE_FIXTURE])
        .current_dir(&fixtures)
        .output();
    // tmpfs over both, then prove the files are gone before running
    let script = "mount -t tmpfs tmpfs \"$1\" && mount -t tmpfs tmpfs \"$2\" || exit 90\n\
                  for hidden in \"$3\" \"$4\"; do\n\
                  if [ -e \"$hidden\" ]; then echo \"still visible: $hidden\" >&2; exit 91; fi\n\
                  done\n\
                  exec \"$5\" run \"$6\"";
    let hidden = Command::new("unshare")
        .args(["-rm", "sh", "-c", script, "sh"])
        .arg(&registry)
        .arg(&gro_js)
        .arg(&probe_source)
        .arg(gro_js.join("bootstrap.js"))
        .arg(binary)
        .arg(SMOKE_FIXTURE)
        .current_dir(&fixtures)
        .output();
    match (seen, hidden) {
        (Ok(seen), Ok(hidden))
            if seen.status.success() && hidden.status.success() && seen.stdout == hidden.stdout =>
        {
            println!("[xtask] both runs succeeded with the same output");
            Ok(())
        }
        (seen, hidden) => {
            eprintln!(
                "[xtask] the off-build-tree check failed\n  seeing the build tree: {}\n  hidden: {}",
                describe(&seen),
                describe(&hidden)
            );
            Err(ExitCode::FAILURE)
        }
    }
}

fn describe(output: &std::io::Result<Output>) -> String {
    match output {
        Ok(output) => format!(
            "{}, stdout {:?}, stderr {:?}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(error) => error.to_string(),
    }
}

/// The first source path `residual.rs` embeds, a file in the cargo registry,
/// to check the hiding with. It parses the format
/// `gro_runtime::snapshot::Snapshot::write_to` writes, which names this as its
/// second reader.
fn registry_source(residual: &Path) -> Option<PathBuf> {
    let residual = fs::read_to_string(residual).ok()?;
    let start = residual.find("include_str!(\"")? + "include_str!(\"".len();
    let end = start + residual[start..].find("\")")?;
    Some(PathBuf::from(&residual[start..end]))
}

/// Cargo's home: `CARGO_HOME`, else `~/.cargo`.
fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME").map_or_else(
        || {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".cargo")
        },
        PathBuf::from,
    )
}
