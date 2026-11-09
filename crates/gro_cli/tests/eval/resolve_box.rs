//! Entry resolution and the box: entries no project owns, packages linked in from
//! outside, the confinement of the process and its descriptors, and a stdout the
//! envelope can't be written to.

#[cfg(all(feature = "snapshot", target_os = "linux"))]
use std::path::PathBuf;
#[cfg(all(feature = "snapshot", target_os = "linux"))]
use std::process::{Command, Stdio};

use serde_json::json;

use crate::eval;
#[cfg(all(feature = "snapshot", target_os = "linux"))]
use crate::support::KillOnDrop;
use crate::support::Scratch;
#[cfg(all(feature = "snapshot", target_os = "linux"))]
use crate::{command, fixtures, long_timeout, wrapped_command};

/// Waits for the driver's `ready` on `child`'s stderr, failing the test
/// when it doesn't come by the deadline.
#[cfg(all(feature = "snapshot", target_os = "linux"))]
fn wait_ready(child: &mut std::process::Child) {
    let line = crate::support::lines(child.stderr.take().unwrap())
        .recv_timeout(crate::support::DEADLINE)
        .ok();
    assert_eq!(line.as_deref(), Some("ready\n"), "the eval didn't start");
}

#[test]
fn an_entry_that_does_not_resolve_is_a_resolve_exiting_2() {
    let run = eval(&["missing.js"]);
    run.assert_failed("resolve", 2);
    assert_eq!(run.envelope["entry"]["id"], "missing.js");
    assert_eq!(run.envelope["error"]["specifier"], "missing.js");
    assert!(run.message().contains("no such file"), "{}", run.message());
}

/// A failure writing the envelope outranks the eval's outcome: stdout on
/// `/dev/full` fails the write, so a `resolve` that would exit 2 exits 1,
/// the error on stderr, with no panic.
#[cfg(target_os = "linux")]
#[test]
fn a_full_stdout_fails_the_envelope_write_exiting_1() {
    let full = std::fs::File::options()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let mut command = crate::command("eval", &["missing.js"], &[]);
    command.stdin(std::process::Stdio::null()).stdout(full);
    let output = command.output().unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("error: failed to write the envelope: "),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
}

/// Eval evaluates a project's own module: one inside `node_modules`, or one
/// with no project root above it, is refused as a `resolve` exiting 2,
/// named by its canonical path. The system's temporary directory is
/// world-writable, so the walk from a directory in it stops there.
#[test]
fn an_entry_no_project_owns_is_a_resolve_exiting_2() {
    let scratch = Scratch::new(
        "unowned",
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/node_modules/pkg/package.json",
                r#"{"name": "pkg", "exports": "./i.mjs"}"#,
            ),
            ("project/node_modules/pkg/i.mjs", "export default 1;\n"),
        ],
    );
    let entry = scratch.path("project/node_modules/pkg/i.mjs");
    let run = eval(&[&entry]);
    run.assert_failed("resolve", 2);
    assert_eq!(run.envelope["entry"]["id"], entry.as_str());
    assert!(
        run.message().contains("it is inside node_modules"),
        "{}",
        run.message()
    );

    let loose = Scratch::in_system_temp("loose", &[("main.mjs", "export default 1;\n")]);
    let entry = loose.path("main.mjs");
    let run = eval(&[&entry]);
    run.assert_failed("resolve", 2);
    assert!(
        run.message().contains("it has no project root"),
        "{}",
        run.message()
    );
}

/// A build with no snapshot refuses to evaluate once the entry resolves,
/// before it confines its process, so the same on every host: its runtime
/// would read its extensions' JS from the build tree, outside the box.
#[cfg(not(feature = "snapshot"))]
#[test]
fn a_build_with_no_snapshot_refuses_to_evaluate() {
    let run = eval(&["value.js"]);
    run.assert_failed("internal", 1);
    assert!(
        run.message()
            .starts_with("a confined eval starts only from a startup snapshot"),
        "{}",
        run.message()
    );
    assert_eq!(
        run.envelope["error"]["hint"],
        "build fgro with its snapshot: cargo xtask build"
    );
}

/// Every task of a confined eval's process, its main thread, the thread
/// that evaluates, V8's workers, and the watchdog, has `no_new_privs` and
/// seccomp's filter mode, read from outside while the driver runs.
#[cfg(all(feature = "snapshot", target_os = "linux"))]
#[test]
fn every_task_of_a_confined_eval_is_boxed() {
    use std::io::Write as _;

    let mut command = command(
        "eval",
        &["value.js", "--driver", "-", "--timeout", &long_timeout()],
        &[],
    );
    command.stdout(Stdio::null());
    let mut child = KillOnDrop(command.spawn().unwrap());
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(fixtures().join("drivers/spin.js")).unwrap())
        .unwrap();
    // the driver says it runs once every thread the eval starts with is up
    wait_ready(&mut child);
    // a task can exit between the listing and its read, which only drops it
    let tasks: Vec<(String, String)> = std::fs::read_dir(format!("/proc/{}/task", child.id()))
        .unwrap()
        .filter_map(|task| {
            let path = task.ok()?.path().join("status");
            let status = std::fs::read_to_string(&path).ok()?;
            Some((path.display().to_string(), status))
        })
        .collect();
    drop(child);
    // the main thread, the thread that evaluates, and V8's workers at least
    assert!(tasks.len() >= 3, "{tasks:?}");
    for (path, status) in &tasks {
        let field = |name: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .map(str::trim)
        };
        assert_eq!(field("NoNewPrivs:"), Some("1"), "{path}\n{status}");
        assert_eq!(field("Seccomp:"), Some("2"), "{path}\n{status}");
        assert_eq!(field("Seccomp_filters:"), Some("2"), "{path}\n{status}");
    }
}

/// A descriptor the eval inherits above stderr, a file outside the box
/// opened by the shell that starts it, is closed before the box goes up, so
/// the process no longer holds it while the driver runs; the eval still runs.
#[cfg(all(feature = "snapshot", target_os = "linux"))]
#[test]
fn an_inherited_descriptor_is_closed() {
    use std::io::Write as _;

    let scratch = Scratch::new("inherited", &[("outside/secret.txt", "outside\n")]);
    let outside = scratch.0.join("outside/secret.txt");
    // bash leaves the descriptor open across its exec of fgro, as a shell
    // that leaks one into every command does
    let mut bash = Command::new("bash");
    bash.args(["-c", r#"exec 7<"$1"; shift; exec "$@""#, "bash"])
        .arg(&outside);
    let mut command = wrapped_command(
        bash,
        &["value.js", "--driver", "-", "--timeout", &long_timeout()],
    );
    command.stdout(Stdio::null());
    let mut child = KillOnDrop(command.spawn().unwrap());
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&std::fs::read(fixtures().join("drivers/spin.js")).unwrap())
        .unwrap();
    wait_ready(&mut child);
    let held: Vec<PathBuf> = std::fs::read_dir(format!("/proc/{}/fd", child.id()))
        .unwrap()
        .filter_map(|fd| std::fs::read_link(fd.unwrap().path()).ok())
        .collect();
    drop(child);
    assert!(!held.contains(&outside), "{held:?}");
}

/// A package linked into the project's `node_modules` from outside it is in
/// the link closure, so it loads.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_package_linked_from_outside_the_project_loads() {
    let scratch = Scratch::new(
        "linked",
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/config.js",
                "import {value} from '@x/lib/value.js';\nexport default {value};\n",
            ),
            (
                "lib/package.json",
                r#"{"name": "@x/lib", "type": "module", "exports": {"./value.js": "./value.js"}}"#,
            ),
            ("lib/value.js", "export const value = 'linked';\n"),
        ],
    );
    std::fs::create_dir_all(scratch.0.join("project/node_modules/@x")).unwrap();
    std::os::unix::fs::symlink(
        "../../../lib",
        scratch.0.join("project/node_modules/@x/lib"),
    )
    .unwrap();
    let run = eval(&[&scratch.path("project/config.js")]);
    run.assert_value(&json!({"value": "linked"}));
}

/// A linked package whose link's target has a `.` or `..` after a
/// directory, or ends in a slash, loads as one written plainly: glibc's
/// `realpath` checks that the directory before such a component exists, a
/// call the box keeps.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_package_linked_through_dot_components_loads() {
    let scratch = Scratch::new(
        "linked_dots",
        &[
            ("project/package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/config.js",
                "import {value} from 'lib';\nexport default {value};\n",
            ),
            (
                "lib/package.json",
                r#"{"name": "lib", "type": "module", "exports": "./value.js"}"#,
            ),
            ("lib/value.js", "export const value = 'linked';\n"),
            ("a/b/.keep", ""),
        ],
    );
    std::fs::create_dir_all(scratch.0.join("project/node_modules")).unwrap();
    let link = scratch.0.join("project/node_modules/lib");
    let root = scratch.0.display();
    for target in [
        format!("{root}/a/b/../../lib"),
        format!("{root}/lib/"),
        format!("{root}/lib/."),
        "../../a/b/../../lib".to_owned(),
        "../../lib/".to_owned(),
    ] {
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let run = eval(&[&scratch.path("project/config.js")]);
        assert_eq!(run.code, 0, "{target}: {}", run.stdout);
        run.assert_value(&json!({"value": "linked"}));
    }
}

/// A project's `package.json` symlinked to a file outside the box is read
/// at its real path. The pre-pass reads it before the box exists; a bare
/// import's walk up passes it again, and that read is refused as outside
/// the box, though the run read the file once already, failing the eval as
/// `internal`, with or without the graph.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_package_json_linked_out_of_the_box_is_internal() {
    let scratch = Scratch::new(
        "linked_out",
        &[
            ("out/pkg.json", r#"{"name": "p", "type": "module"}"#),
            (
                "project/main.ts",
                "import x from 'foo';\nexport default x;\n",
            ),
            (
                "project/node_modules/foo/package.json",
                r#"{"name": "foo", "exports": "./i.mjs"}"#,
            ),
            ("project/node_modules/foo/i.mjs", "export default 7;\n"),
        ],
    );
    std::os::unix::fs::symlink("../out/pkg.json", scratch.0.join("project/package.json")).unwrap();
    let entry = scratch.path("project/main.ts");
    let outside = scratch.path("out/pkg.json");
    for args in [&[entry.as_str()][..], &[entry.as_str(), "--graph"]] {
        let run = eval(args);
        run.assert_failed("internal", 1);
        assert!(
            run.message().contains(&format!(
                "the eval was to read {outside}, which lies outside its box"
            )),
            "{}",
            run.message()
        );
    }
}
