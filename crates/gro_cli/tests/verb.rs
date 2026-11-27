//! End-to-end tests of a command of gro's on `fgro`: its routing, a run on
//! the binary from the generated entry with its closure's grants, the
//! fallback to the Node Gro the project installed, and the refusals before
//! anything runs. A run on the binary starts in the run's kernel box, which
//! only a snapshot build does, so those tests run from `cargo xtask build
//! --verify`, and a plain build's refusal is held here instead; the box's
//! walls are `verb_box`'s.
//!
//! A project that runs on the binary links the checkout's `node_modules`
//! whole, where Gro's embedded library finds its peers. One that falls back
//! plants its own `@fuzdev/gro` at the binary's version, and finds on
//! `PATH` a committed `node` that records what it was run with.

#![expect(clippy::unwrap_used, reason = "a test's setup fails by panicking")]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use gro_runtime::BINARY_NAME;
use support::{Scratch, fgro_command, spawn, version};

/// The variables the verb reads, cleared so the harness's own don't leak in.
const VERB_VARS: [&str; 3] = ["GRO_FALLBACK", "NODE_ENV", "FAKE_NODE_RECORD"];

/// The checkout's `node_modules`, where Gro's peers are installed.
fn checkout_node_modules() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules")
        .canonicalize()
        .unwrap()
}

/// A project named `name` with `files`, its `package.json` a project's,
/// linking the checkout's `node_modules` whole.
fn linked_project(name: &str, files: &[(&str, &str)]) -> Scratch {
    let mut all = vec![("package.json", r#"{"name": "p", "type": "module"}"#)];
    all.extend_from_slice(files);
    let project = Scratch::new(name, &all);
    std::os::unix::fs::symlink(checkout_node_modules(), project.0.join("node_modules")).unwrap();
    project
}

/// `fgro` with `args` in `dir`, the verb's variables cleared.
fn fgro_in(dir: &Path, args: &[&str]) -> Command {
    let mut command = fgro_command(args);
    command.current_dir(dir);
    for name in VERB_VARS {
        command.env_remove(name);
    }
    command
}

fn run(command: Command) -> (Option<i32>, String, String) {
    let Output {
        status,
        stdout,
        stderr,
    } = spawn(command, b"");
    (
        status.code(),
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    )
}

/// Bare `fgro` lists gro's commands through the embed, `-v` prints gro's
/// version, and `--version` keeps the binary's own line.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs on the binary only from a snapshot build"
)]
fn every_first_argument_but_the_binary_s_own_is_a_command_of_gro_s() {
    let project = linked_project("verb_routing", &[]);
    let (code, stdout, stderr) = run(fgro_in(&project.0, &[]));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(
        format!("{stdout}{stderr}").contains("builtin tasks"),
        "{stdout}{stderr}"
    );
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["-v"]));
    assert_eq!(code, Some(0), "{stderr}");
    assert!(
        format!("{stdout}{stderr}").contains(&format!("v{}", version())),
        "{stdout}{stderr}"
    );
    let (code, stdout, _) = run(fgro_in(&project.0, &["--version"]));
    assert_eq!(code, Some(0));
    assert!(stdout.starts_with(&format!("{BINARY_NAME} ")), "{stdout}");
    let (code, stdout, _) = run(fgro_in(&project.0, &["--help"]));
    assert_eq!(code, Some(0));
    assert!(stdout.contains("<command>"), "{stdout}");
}

/// `format --check` reads the project with no write, and fails on a file
/// it would format; the grant flags never reach gro's args, where the
/// Node Gro would warn of them.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs on the binary only from a snapshot build"
)]
fn format_check_runs_on_the_binary() {
    let project = linked_project("verb_format", &[("src/lib/a.ts", "export const a = 1;\n")]);
    let (code, stdout, stderr) = run(fgro_in(
        &project.0,
        &["format", "--allow-env=HOME", "--check"],
    ));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(!stderr.contains("grant flags"), "{stderr}");
    std::fs::write(project.0.join("src/lib/b.ts"), "export const   b=1\n").unwrap();
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["format", "--check"]));
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert_eq!(
        std::fs::read_to_string(project.0.join("src/lib/b.ts")).unwrap(),
        "export const   b=1\n"
    );
}

/// `gen --check` runs the project's genfiles with no write to the
/// project, which `gen` grants: a genfile's own write is refused there,
/// failing the run, and made under `gen`, beneath a top-level directory,
/// which the box writes.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs on the binary only from a snapshot build"
)]
fn gen_check_holds_a_genfile_to_reads() {
    let genfile = "import { fs } from '@fuzdev/gro/fs.ts';\n\
                   export const gen = async () => {\n\
                   \tawait fs.project.write('src/lib/wrote.txt', 'x');\n\
                   \treturn '// w\\n';\n\
                   };\n";
    let project = linked_project(
        "verb_gen",
        &[("src/lib/w.gen.ts", genfile), ("src/lib/w.ts", "// w\n")],
    );
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["gen", "--check"]));
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(stderr.contains("fs refused a write"), "{stdout}{stderr}");
    assert!(!project.0.join("src/lib/wrote.txt").exists());
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["gen"]));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(project.0.join("src/lib/wrote.txt").exists());
}

/// A build with no snapshot refuses a command of Gro's that would run on
/// the binary, before anything runs: started from source, the runtime
/// reads its JS from the build tree, which the run's box doesn't admit.
#[cfg(not(feature = "snapshot"))]
#[test]
fn a_plain_build_refuses_a_command_of_gro_s() {
    let project = linked_project("verb_plain", &[]);
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["format", "--check"]));
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert_eq!(
        stderr,
        format!(
            "error: {BINARY_NAME} format runs in a kernel box, which starts only from a startup \
             snapshot embedded in the binary, and this build has none: started from source, the \
             runtime reads its JS from the build tree, outside the box. Build {BINARY_NAME} with \
             its snapshot: cargo xtask build\n"
        )
    );
}

/// A project that installs `@fuzdev/gro` at `gro_version`, built or not.
fn planted_project(name: &str, gro_version: &str, built: bool) -> Scratch {
    let package = format!(r#"{{"name": "@fuzdev/gro", "version": "{gro_version}"}}"#);
    let mut files = vec![
        ("package.json", r#"{"name": "p", "type": "module"}"#),
        ("node_modules/@fuzdev/gro/package.json", package.as_str()),
    ];
    if built {
        files.push(("node_modules/@fuzdev/gro/dist/invoke.js", "throw 1;\n"));
    }
    Scratch::new(name, &files)
}

/// The directory of the fake `node`, which records its args and its
/// working directory in the file `FAKE_NODE_RECORD` names, a variable only
/// the whole environment passes, then exits 7. It's committed, not written
/// by the test, since a file just written can't run while another
/// thread's fork holds it open for writing (`ETXTBSY`).
fn fake_node_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/verb/bin")
}

/// `fgro` with `args` in the planted `project`, `PATH` the fake `node`'s
/// directory alone, recording in the project's `record.txt`.
fn planted_fgro(project: &Scratch, args: &[&str]) -> Command {
    let mut command = fgro_in(&project.0, args);
    command
        .env("PATH", fake_node_dir())
        .env("FAKE_NODE_RECORD", project.0.join("record.txt"));
    command
}

/// A command the binary leaves to Node execs `node` from `PATH` over the
/// installed Gro's `dist/invoke.js` with its loader registered, the
/// command line as written, grant flags included, in the same directory
/// and with the whole environment, after one line saying why.
#[test]
fn a_fallback_execs_the_installed_node_gro() {
    let project = planted_project("verb_fallback", &version(), true);
    let (code, stdout, stderr) = run(planted_fgro(&project, &["dev", "--allow-read=.", "--x"]));
    assert_eq!(code, Some(7), "{stdout}{stderr}");
    let gro = project.0.join("node_modules/@fuzdev/gro");
    assert_eq!(
        stderr,
        format!(
            "note: {BINARY_NAME} dev runs on the Node gro in node_modules/@fuzdev/gro {}, which \
             confines nothing, because `dev` declares no grants: it serves and watches the \
             project with Vite and its plugins\n",
            version()
        )
    );
    let record = std::fs::read_to_string(project.0.join("record.txt")).unwrap();
    let lines: Vec<&str> = record.lines().collect();
    assert_eq!(lines[0], "--import");
    assert!(
        lines[1].starts_with("data:text/javascript,import%20%7Bregister%7D"),
        "{record}"
    );
    let invoke = gro.join("dist/invoke.js");
    assert_eq!(
        lines[2..],
        [
            "--experimental-import-meta-resolve",
            "-C",
            "development",
            invoke.to_str().unwrap(),
            "dev",
            "--allow-read=.",
            "--x",
            project.0.to_str().unwrap(),
        ]
    );
}

/// `GRO_FALLBACK=command` refuses a fallback another installed Gro decides,
/// running nothing; any value it doesn't take is refused before anything
/// runs; and a fallback with no built Gro, or none at all, is refused.
#[test]
fn a_fallback_is_refused_before_anything_runs() {
    let older = planted_project("verb_strict", "0.0.1-older", true);
    let mut command = planted_fgro(&older, &["check"]);
    command.env("GRO_FALLBACK", "command");
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(2), "{stdout}{stderr}");
    assert!(
        stderr.starts_with("error: GRO_FALLBACK=command refuses running the project's own gro"),
        "{stderr}"
    );
    assert!(!older.0.join("record.txt").exists());
    // the default runs it
    let (code, _, stderr) = run(planted_fgro(&older, &["check"]));
    assert_eq!(code, Some(7), "{stderr}");
    for value in ["", "none"] {
        let mut command = planted_fgro(&older, &["dev"]);
        command.env("GRO_FALLBACK", value);
        let (code, _, stderr) = run(command);
        assert_eq!(code, Some(2), "{value:?}: {stderr}");
        assert_eq!(
            stderr,
            format!("error: GRO_FALLBACK is \"{value}\", and it takes all or command\n")
        );
    }
    let unbuilt = planted_project("verb_unbuilt", &version(), false);
    let (code, _, stderr) = run(planted_fgro(&unbuilt, &["dev"]));
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("dist/invoke.js is missing"), "{stderr}");
    assert!(!unbuilt.0.join("record.txt").exists());
    let none = Scratch::new("verb_no_gro", &[("package.json", "{\"name\": \"p\"}")]);
    let (code, _, stderr) = run(fgro_in(&none.0, &["dev"]));
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.ends_with("this project installs none: npm i -D @fuzdev/gro\n"),
        "{stderr}"
    );
}

/// The working directory is the project's root: one with no
/// `package.json`, or one that only marks a module type, is refused.
#[test]
fn the_working_directory_holds_a_project() {
    let empty = Scratch::new("verb_no_package", &[("a.txt", "")]);
    let (code, _, stderr) = run(fgro_in(&empty.0, &["check"]));
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.ends_with("holds no package.json\n"), "{stderr}");
    let marker = Scratch::new("verb_marker", &[("package.json", r#"{"type": "module"}"#)]);
    let (code, _, stderr) = run(fgro_in(&marker.0, &[]));
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("only marks a module type"), "{stderr}");
}

/// The fallback runs the `node` its `PATH` walk finds, never one in a
/// `node_modules/.bin` entry, where a project can plant one.
#[test]
fn a_fallback_skips_a_node_the_project_plants() {
    let project = planted_project("verb_planted_node", &version(), true);
    std::fs::create_dir_all(project.0.join("node_modules/.bin")).unwrap();
    std::fs::write(
        project.0.join("node_modules/.bin/node"),
        "#!/bin/sh\nexit 9\n",
    )
    .unwrap();
    project.set_mode("node_modules/.bin/node", 0o755);
    let mut command = planted_fgro(&project, &["dev"]);
    command.env(
        "PATH",
        format!(
            "{}:{}",
            project.0.join("node_modules/.bin").display(),
            fake_node_dir().display()
        ),
    );
    let (code, stdout, stderr) = run(command);
    assert_eq!(code, Some(7), "{stdout}{stderr}");
}

/// A command's run matches the `development` condition unless `NODE_ENV`
/// is `production`, as the Node Gro's loader does, and `fgro run` never
/// does.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs on the binary only from a snapshot build"
)]
fn a_command_s_run_matches_the_development_condition() {
    let package = r##"{"name": "p", "type": "module", "imports": {"#c": {"development": "./dev.js", "default": "./prod.js"}}}"##;
    let report = "import { which } from '#c';\nconsole.error(`which: ${which}`);\n";
    let config = format!("{report}export default (config: unknown) => config;\n");
    let project = Scratch::new(
        "verb_development",
        &[
            ("package.json", package),
            ("dev.js", "export const which = 'dev';\n"),
            ("prod.js", "export const which = 'prod';\n"),
            ("gro.config.ts", config.as_str()),
            ("r.js", report),
        ],
    );
    std::os::unix::fs::symlink(checkout_node_modules(), project.0.join("node_modules")).unwrap();
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["-v"]));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(stderr.contains("which: dev"), "{stderr}");
    let mut production = fgro_in(&project.0, &["-v"]);
    production.env("NODE_ENV", "production");
    let (code, stdout, stderr) = run(production);
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(stderr.contains("which: prod"), "{stderr}");
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["run", "r.js"]));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(stderr.contains("which: prod"), "{stderr}");
}

/// A run that fails after a Node built-in was refused, caught or not, a
/// bare name Node resolves as one included, says how to run the command on
/// the Node Gro; one that fails otherwise doesn't.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs on the binary only from a snapshot build"
)]
fn a_refused_node_builtin_hints_at_the_node_gro() {
    let genfile = "export const gen = async () => {\n\
                   \ttry { await import('events'); } catch {}\n\
                   \tthrow new Error('the genfile failed');\n\
                   };\n";
    let project = linked_project("verb_node_hint", &[("src/lib/a b/n.gen.ts", genfile)]);
    let (code, stdout, stderr) = run(fgro_in(
        &project.0,
        &["gen", "--allow-env=HOME", "src/lib/a b", "--check"],
    ));
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(
        stderr.ends_with(
            "hint: a Node built-in's import was refused in this run; the Node gro runs it, unconfined: \
             node_modules/.bin/gro gen 'src/lib/a b' --check\n"
        ),
        "{stdout}{stderr}"
    );
    std::fs::write(
        project.0.join("src/lib/a b/n.gen.ts"),
        "export const gen = () => { throw new Error('the genfile failed'); };\n",
    )
    .unwrap();
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["gen"]));
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(!stderr.contains("hint:"), "{stderr}");
}

/// A flag's directory refused under a command of gro's names no `fgro run`
/// help, and the command word is the closure's, past a flag before it.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs on the binary only from a snapshot build"
)]
fn a_command_s_refusals_name_the_command() {
    let project = linked_project("verb_refusals", &[]);
    let (code, _, stderr) = run(fgro_in(
        &project.0,
        &["format", "--allow-read=missing", "--check"],
    ));
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.starts_with("error: a --allow-read directory doesn't exist: "),
        "{stderr}"
    );
    assert!(!stderr.contains("--help"), "{stderr}");
    let (code, _, stderr) = run(fgro_in(&project.0, &["--x=1", "task"]));
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.starts_with(&format!(
            "error: {BINARY_NAME} task runs on the Node gro, because `task` lists the project's tasks"
        )),
        "{stderr}"
    );
}

/// The `node:` hint follows only a failed command of Gro's: a run that
/// refused a `node:` specifier and still passed prints none, and so does
/// `fgro run`, which has no Node Gro to name.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a command of Gro's runs on the binary only from a snapshot build"
)]
fn the_node_hint_follows_only_a_failed_command() {
    let config = "try { await import('node:fs'); } catch {}\n\
                  export default (config: unknown) => config;\n";
    let project = linked_project("verb_node_hint_passed", &[("gro.config.ts", config)]);
    let (code, stdout, stderr) = run(fgro_in(&project.0, &["-v"]));
    assert_eq!(code, Some(0), "{stdout}{stderr}");
    assert!(!stderr.contains("hint:"), "{stderr}");
    let script = Scratch::new(
        "run_node_hint",
        &[(
            "a.mjs",
            "try { await import('node:fs'); } catch {}\nthrow new Error('x');\n",
        )],
    );
    let (code, stdout, stderr) = run(fgro_in(&script.0, &["run", "a.mjs"]));
    assert_eq!(code, Some(1), "{stdout}{stderr}");
    assert!(!stderr.contains("hint:"), "{stderr}");
}
