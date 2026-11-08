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
//!    reads are immutable per version. The same sources make the same
//!    snapshot, byte for byte (gro patches `deno_core` for it, see
//!    `vendor/README.md`), and a rerun leaves identical files untouched, so
//!    it relinks stage two only when the snapshot changed.
//! 2. `cargo build -p gro_cli --features snapshot`, with `GRO_SNAPSHOT_DIR`
//!    naming that directory, embeds them, and the binary is hard-linked to
//!    `<target>/gro_snapshot/<profile>/fgro`, so later plain builds, which
//!    replace `<target>/<profile>/fgro`, leave it alone. The build also
//!    embeds the checkout's commit through `GRO_COMMIT` ([`commit`]), read
//!    right before it, which `fgro --version` prints; cargo tracks the
//!    variable, so a new commit or a change to the tree's dirtiness rebuilds
//!    `gro_cli`.
//!
//! Between the stage-one build and `make-snapshot`, the V8 library copy is
//! checked against the pin ([`crate::v8_archive::verify_linked`]), before any
//! V8 code runs. `make-snapshot`'s stdout, which carries only `deno_core`'s
//! progress lines, is discarded; its errors print to stderr.
//!
//! Cargo keeps both builds of `fgro`, one per feature set, so switching
//! between them relinks nothing.
//!
//! The measurement builds are stage two with one more feature beside
//! `snapshot`, from the same snapshot ([`Measurement`]): the phases build
//! (`cargo xtask bench --phases`), linked to
//! `<target>/gro_snapshot/<profile>-phases/fgro`, and the priced build (a
//! suite with `prices`), linked to `<target>/gro_snapshot/<profile>-prices/fgro`.
//!
//! `--verify` then checks the result: the baseline suite's `fgro` size
//! budgets (the executable's in a release build only, the snapshot's in
//! both), that the snapshot `fgro`'s own `make-snapshot` reproduces the
//! snapshot it embeds byte for byte ([`reproduce`]), clippy over the
//! `snapshot` feature's code, alone and with `prices`, `fgro`'s integration
//! tests against the snapshot build, its phases and prices tests against
//! builds with each feature too, and a fixture run with the cargo registry's
//! sources (`$CARGO_HOME/registry/src`), the vendored `deno_core`, and gro's
//! own extension sources hidden (tmpfs mounts in a private user and mount
//! namespace, `unshare -rm`), its output compared with a run that can see
//! them. The fixture is TypeScript, so the strip runs off the build tree too.
//! A machine without unprivileged user namespaces fails that check rather
//! than skipping it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use gro_bench::record::GitState;

use crate::cmd::{StepResult, run};
use crate::error::XtaskError;

/// Which build of the snapshot `fgro` to make: the plain one, or a
/// measurement build with one more feature, which release never carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measurement {
    /// The build that ships.
    None,
    /// With `phases`: a timestamp at each phase boundary.
    Phases,
    /// With `prices`: the switches that turn off eval's defaults.
    Prices,
}

impl Measurement {
    /// The feature it adds, which also names its build's directory.
    const fn feature(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Phases => Some("phases"),
            Self::Prices => Some("prices"),
        }
    }
}

/// The fixture both runs of the off-build-tree check execute: a TypeScript
/// entry over the web globals, so the snapshot build proves the strip too.
const SMOKE_FIXTURE: &str = "smoke.ts";

/// The variable stage two embeds the commit from, which `gro_cli` reads with
/// `option_env!`.
const COMMIT_ENV: &str = "GRO_COMMIT";

/// A snapshot build of `fgro`: the binary, and the `GRO_COMMIT` it was built
/// with, which [`verify`]'s builds repeat so they reuse it rather than
/// rebuilding `gro_cli` without it.
pub struct SnapshotBuild {
    /// `<target>/gro_snapshot/<profile>/fgro`.
    pub binary: PathBuf,
    commit: String,
}

/// The `GRO_COMMIT` value for the checkout at `root`: its `HEAD` SHA, with
/// `-dirty` appended when the working tree differs from it, untracked files
/// included, the default bench history excepted ([`GitState::read`], the
/// state a bench record carries when `--history` isn't given). Empty,
/// which `fgro --version` prints as `unknown`, when git can't say, with a
/// warning: a source tree outside a checkout still builds.
fn commit(root: &Path) -> String {
    match GitState::read(root, &root.join(crate::bench::HISTORY)) {
        Ok(state) => commit_value(&state),
        Err(error) => {
            eprintln!(
                "[xtask] warning: the build embeds no commit, since git can't read it: {error}"
            );
            String::new()
        }
    }
}

/// Formats `state` as `GRO_COMMIT` expects it.
fn commit_value(state: &GitState) -> String {
    if state.dirty {
        format!("{}-dirty", state.sha)
    } else {
        state.sha.clone()
    }
}

/// Builds the snapshot `fgro`.
///
/// Its paths come from `target_dir`, cargo's target directory, which
/// `CARGO_TARGET_DIR` or `build.target-dir` may move off the checkout.
pub fn build(
    root: &Path,
    target_dir: &Path,
    release: bool,
    measurement: Measurement,
) -> Result<SnapshotBuild, XtaskError> {
    let profile = if release { "release" } else { "debug" };
    let profile_args: &[&str] = if release { &["--release"] } else { &[] };
    let dir = target_dir.join("gro_snapshot").join(profile);
    let dir_arg = dir.display().to_string();

    let mut stage_one = vec!["build", "--locked", "-p", "gro_cli"];
    stage_one.extend(profile_args);
    run(root, &[], "cargo", &stage_one)?;
    // before stage one runs V8 code, and after the build that wrote the copy
    crate::check::verify_linked_step(target_dir, profile)?;
    let plain = target_dir.join(profile).join("fgro");
    let stamp = dir.join("stage_one.stamp");
    if snapshot_is_current(&dir, &stamp, &plain) {
        println!(
            "\n[xtask] the snapshot in {dir_arg} was made by the current {}",
            crate::cmd::relative(root, &plain)
        );
    } else {
        // its stdout carries only deno_core's progress lines; its errors go to stderr
        crate::cmd::run_program_to(
            root,
            &[],
            &plain,
            &crate::cmd::relative(root, &plain),
            &["make-snapshot", &dir_arg],
            Stdio::null(),
        )?;
        // identical outputs aren't rewritten, so the stamp records when it ran
        fs::write(&stamp, b"").map_err(|error| {
            XtaskError::Failed(format!("failed to write {}: {error}", stamp.display()))
        })?;
    }

    let features = measurement.feature().map_or_else(
        || "snapshot".to_owned(),
        |feature| format!("snapshot,{feature}"),
    );
    let mut stage_two = vec![
        "build",
        "--locked",
        "-p",
        "gro_cli",
        "--features",
        &features,
    ];
    stage_two.extend(profile_args);
    // read last, so it names the tree this build compiles
    let commit = commit(root);
    run(
        root,
        &[("GRO_SNAPSHOT_DIR", &dir_arg), (COMMIT_ENV, &commit)],
        "cargo",
        &stage_two,
    )?;
    let built = target_dir.join(profile).join("fgro");
    let linked = if let Some(feature) = measurement.feature() {
        let dir = target_dir
            .join("gro_snapshot")
            .join(format!("{profile}-{feature}"));
        fs::create_dir_all(&dir).map_err(|error| {
            XtaskError::Failed(format!("failed to create {}: {error}", dir.display()))
        })?;
        dir.join("fgro")
    } else {
        dir.join("fgro")
    };
    let _ = fs::remove_file(&linked);
    fs::hard_link(&built, &linked).map_err(|error| {
        XtaskError::Failed(format!(
            "failed to link {} to {}: {error}",
            built.display(),
            linked.display()
        ))
    })?;
    println!("\n[xtask] built {}", linked.display());
    Ok(SnapshotBuild {
        binary: linked,
        commit,
    })
}

/// The files `make-snapshot` writes, which stage two embeds.
const SNAPSHOT_FILES: [&str; 2] = ["snapshot.bin", "residual.rs"];

/// Whether `snapshot.bin` and `residual.rs` exist in `dir` and stage one last
/// ran (the `stamp`) after the plain binary was built.
fn snapshot_is_current(dir: &Path, stamp: &Path, plain: &Path) -> bool {
    let modified = |path: &Path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
    };
    SNAPSHOT_FILES.iter().all(|file| dir.join(file).is_file())
        && matches!((modified(stamp), modified(plain)), (Some(ran), Some(built)) if ran > built)
}

/// Checks the snapshot `build`: the size budgets, that it reproduces its
/// snapshot, clippy over the `snapshot` feature, the integration tests
/// against it, then the off-build-tree run.
pub fn verify(root: &Path, release: bool, build: &SnapshotBuild) -> StepResult {
    let binary = &build.binary;
    check_budgets(root, release, binary)?;
    reproduce(root, binary)?;
    let dir = binary.parent().unwrap_or(root).display().to_string();
    // the stage-two environment, so these builds reuse its gro_cli
    let env = [
        ("GRO_SNAPSHOT_DIR", dir.as_str()),
        (COMMIT_ENV, build.commit.as_str()),
    ];
    // the workspace clippy builds without the feature, so its code goes
    // unlinted there; again with `prices`, whose code in the snapshot-only
    // tests (`tests/confine.rs`) neither build reaches, the first keeping
    // their `not(prices)` branches linted
    for features in ["snapshot", "snapshot,prices"] {
        run(
            root,
            &env,
            "cargo",
            &[
                "clippy",
                "--locked",
                "-p",
                "gro_cli",
                "--features",
                features,
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        )?;
    }
    let mut test = vec![
        "test",
        "--locked",
        "-p",
        "gro_cli",
        "--features",
        "snapshot",
        "--test",
        "run",
        "--test",
        "eval",
        "--test",
        "confine",
    ];
    if release {
        test.push("--release");
    }
    run(root, &env, "cargo", &test)?;
    // the phases build's marks, eval's written from inside its box
    let mut phases = vec![
        "test",
        "--locked",
        "-p",
        "gro_cli",
        "--features",
        "snapshot,phases",
        "--test",
        "phases",
    ];
    if release {
        phases.push("--release");
    }
    run(root, &env, "cargo", &phases)?;
    // the priced build's switches, eval's box among them
    let mut prices = vec![
        "test",
        "--locked",
        "-p",
        "gro_cli",
        "--features",
        "snapshot,prices",
        "--test",
        "prices",
        "--test",
        "confine",
    ];
    if release {
        prices.push("--release");
    }
    run(root, &env, "cargo", &prices)?;
    off_build_tree(root, binary)
}

/// The suite whose `fgro` budgets the build is held to.
const BUDGET_SUITE: &str = "crates/gro_bench/suites/baseline/suite.json";

/// Checks `binary` and the snapshot beside it against the baseline suite's
/// `fgro` budgets, failing loudly on one crossed. The executable's sizes
/// bind only a release build, the one that ships; a debug build is checked
/// on its snapshot alone, which is the same either way, and says so.
fn check_budgets(root: &Path, release: bool, binary: &Path) -> StepResult {
    let suite =
        gro_bench::suite::Suite::load(&root.join(BUDGET_SUITE)).map_err(XtaskError::Budget)?;
    let Some(budget) = suite.budgets.get("fgro") else {
        return Err(XtaskError::Budget(
            gro_bench::error::BenchError::SuiteInvalid(format!(
                "{BUDGET_SUITE} has no `fgro` budget"
            )),
        ));
    };
    let size = |path: &Path| {
        fs::metadata(path)
            .map(|metadata| metadata.len())
            .map_err(|source| {
                XtaskError::Budget(gro_bench::error::BenchError::Read {
                    path: path.to_owned(),
                    source,
                })
            })
    };
    let snapshot_bytes = Some(size(&binary.with_file_name("snapshot.bin"))?);
    let (budget, sizes) = if release {
        let sizes = gro_bench::budget::Sizes {
            bytes: Some(size(binary)?),
            gzip_bytes: Some(gro_bench::budget::gzip_size(binary).map_err(XtaskError::Budget)?),
            snapshot_bytes,
        };
        (budget.clone(), sizes)
    } else {
        let snapshot_only = gro_bench::suite::Budget {
            bytes: None,
            gzip_bytes: None,
            ..budget.clone()
        };
        let sizes = gro_bench::budget::Sizes {
            snapshot_bytes,
            ..Default::default()
        };
        (snapshot_only, sizes)
    };
    gro_bench::budget::check_budget("fgro", &budget, &sizes).map_err(XtaskError::Budget)?;
    println!(
        "[xtask] within the fgro budgets in {BUDGET_SUITE}{}",
        if release {
            ""
        } else {
            " (the snapshot's alone: the executable's bind the release build)"
        }
    );
    Ok(())
}

/// Runs `binary`'s `make-snapshot` into a fresh directory beside it and fails
/// unless each file matches, byte for byte, the one beside `binary` that its
/// build embedded (cargo rebuilds `gro_cli` when either changes). A second
/// process making the same snapshot proves it a function of its sources: a
/// difference is hash order or uninitialized memory reaching the blob.
fn reproduce(root: &Path, binary: &Path) -> StepResult {
    let embedded = binary.parent().unwrap_or(root);
    let fresh = embedded.join("reproduce");
    let fresh_arg = fresh.display().to_string();
    let _ = fs::remove_dir_all(&fresh);
    // its stdout carries only deno_core's progress lines; its errors go to stderr
    crate::cmd::run_program_to(
        root,
        &[],
        binary,
        &crate::cmd::relative(root, binary),
        &["make-snapshot", &fresh_arg],
        Stdio::null(),
    )?;
    for file in SNAPSHOT_FILES {
        let read = |path: &Path| {
            fs::read(path).map_err(|error| {
                XtaskError::Failed(format!("failed to read {}: {error}", path.display()))
            })
        };
        let (expected, made) = (embedded.join(file), fresh.join(file));
        let (expected_bytes, made_bytes) = (read(&expected)?, read(&made)?);
        if expected_bytes != made_bytes {
            let offset = expected_bytes
                .iter()
                .zip(&made_bytes)
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| expected_bytes.len().min(made_bytes.len()));
            return Err(XtaskError::Failed(format!(
                "the snapshot isn't reproducible: {} ({} bytes) differs from the embedded {} \
                 ({} bytes), first at byte {offset}; the made files are kept in {}",
                made.display(),
                made_bytes.len(),
                expected.display(),
                expected_bytes.len(),
                fresh.display()
            )));
        }
    }
    let _ = fs::remove_dir_all(&fresh);
    println!(
        "[xtask] {} reproduces its embedded snapshot byte for byte",
        crate::cmd::relative(root, binary)
    );
    Ok(())
}

/// Runs the smoke fixture with `binary` twice, once seeing the build tree and
/// once with the registry's sources, the vendored `deno_core`, and gro's
/// extension sources hidden, and fails unless both succeed with the same
/// output.
fn off_build_tree(root: &Path, binary: &Path) -> StepResult {
    let fixtures = root.join("crates/gro_cli/tests/fixtures/run");
    let gro_js = root.join("crates/gro_runtime/js");
    // the patched deno_core, a path dependency, so its sources sit in the checkout
    let deno_core = root.join("vendor/deno_core");
    // every crate's sources, the `deno_web` and `deno_webidl` JS among them
    let registry = cargo_home().join("registry/src");
    // a residual source must vanish with it, or the check proves nothing
    let residual = binary.with_file_name("residual.rs");
    let probe_source = registry_source(&residual).ok_or_else(|| {
        XtaskError::Failed(format!(
            "no residual source path in {} to check the hiding with",
            residual.display()
        ))
    })?;
    if !probe_source.starts_with(&registry) {
        return Err(XtaskError::Failed(format!(
            "{} isn't under {}, the registry sources this check hides",
            probe_source.display(),
            registry.display()
        )));
    }

    // the mechanism first, so a machine without it fails as such, not as a fixture failure
    let probe = Command::new("unshare")
        .args(["-rm", "sh", "-c", "mount -t tmpfs tmpfs \"$1\"", "sh"])
        .arg(&gro_js)
        .output();
    if !probe.as_ref().is_ok_and(|output| output.status.success()) {
        return Err(XtaskError::Failed(format!(
            "the off-build-tree check needs unprivileged user and mount namespaces \
             (`unshare -rm` with a tmpfs mount), and they're unavailable here: {}",
            probe.map_or_else(
                |error| error.to_string(),
                |output| String::from_utf8_lossy(&output.stderr).trim().to_owned()
            )
        )));
    }

    println!(
        "\n[xtask] {} run {SMOKE_FIXTURE}, seeing the build tree and then with {}, {}, and {} hidden",
        crate::cmd::relative(root, binary),
        registry.display(),
        crate::cmd::relative(root, &deno_core),
        crate::cmd::relative(root, &gro_js)
    );
    let seen = Command::new(binary)
        .args(["run", SMOKE_FIXTURE])
        .current_dir(&fixtures)
        .output();
    // tmpfs over all three, then prove the files are gone before running
    let script = "mount -t tmpfs tmpfs \"$1\" && mount -t tmpfs tmpfs \"$2\" \
                  && mount -t tmpfs tmpfs \"$3\" || exit 90\n\
                  for hidden in \"$4\" \"$5\" \"$6\"; do\n\
                  if [ -e \"$hidden\" ]; then echo \"still visible: $hidden\" >&2; exit 91; fi\n\
                  done\n\
                  exec \"$7\" run \"$8\"";
    let hidden = Command::new("unshare")
        .args(["-rm", "sh", "-c", script, "sh"])
        .arg(&registry)
        .arg(&deno_core)
        .arg(&gro_js)
        .arg(&probe_source)
        .arg(deno_core.join("lib.rs"))
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
        (seen, hidden) => Err(XtaskError::Failed(format!(
            "the off-build-tree check failed\n  seeing the build tree: {}\n  hidden: {}",
            describe(&seen),
            describe(&hidden)
        ))),
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
pub fn cargo_home() -> PathBuf {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_value_marks_a_dirty_tree() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let state = |dirty| GitState {
            sha: sha.to_owned(),
            dirty,
        };
        assert_eq!(commit_value(&state(false)), sha);
        assert_eq!(commit_value(&state(true)), format!("{sha}-dirty"));
    }
}
