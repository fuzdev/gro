//! Color decided per stream: the off variables, `TERM`, `FORCE_COLOR`, and a
//! pseudo-terminal.

use std::process::Command;

use crate::support::{COLOR_VARS, Scratch, spawn};
use crate::{fgro_with_env, fixtures};

/// Runs `console.js` on a pseudo-terminal through util-linux's `script`, with
/// `env` set and `redirect` (a shell redirection, or empty) applied inside the
/// terminal, returning what reached the terminal.
fn run_on_terminal(env: &[(&str, &str)], redirect: &str) -> String {
    // probed once per test binary, not once per call
    static SCRIPT_PROBED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    let command = format!("'{}' run console.js {redirect}", env!("CARGO_BIN_EXE_fgro"));
    let mut script = Command::new("script");
    script
        .args(["-qec", &command, "/dev/null"])
        .current_dir(fixtures());
    for name in COLOR_VARS {
        script.env_remove(name);
    }
    // a terminal that colors, unless `env` says otherwise
    script.env("TERM", "xterm").envs(env.iter().copied());
    // fail loudly rather than skip: the color policy has no other test
    SCRIPT_PROBED.get_or_init(|| {
        Command::new("script")
            .arg("--version")
            .output()
            .expect("util-linux `script` is required for the terminal color tests");
    });
    let output = spawn(script, b"");
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn color_is_decided_per_stream() {
    let scratch = Scratch::new("streams", &[]);
    // stdout on the terminal, stderr to a file
    let stderr_file = scratch.0.join("stderr");
    let terminal = run_on_terminal(&[], &format!("2>'{}'", stderr_file.display()));
    let stderr = std::fs::read_to_string(&stderr_file).unwrap();
    assert!(terminal.contains("log \x1b["), "{terminal:?}");
    assert_eq!(stderr, "warn\nerror Map(1) { \"k\" => 1 }\n");

    // stderr on the terminal, stdout to a file
    let stdout_file = scratch.0.join("stdout");
    let terminal = run_on_terminal(&[], &format!(">'{}'", stdout_file.display()));
    let stdout = std::fs::read_to_string(&stdout_file).unwrap();
    assert!(terminal.contains("error Map(1) { \x1b["), "{terminal:?}");
    assert_eq!(stdout, "log 1 { a: [ 1, \"two\" ] }\ninfo\n");
}

#[test]
fn the_off_vars_turn_color_off_when_non_empty() {
    assert!(run_on_terminal(&[], "").contains('\x1b'));
    for env in [
        [("NO_COLOR", "1")],
        [("NODE_DISABLE_COLORS", "1")],
        [("CLAUDECODE", "1")],
    ] {
        let output = run_on_terminal(&env, "");
        assert!(!output.contains('\x1b'), "{env:?}: {output:?}");
    }
    // set but empty counts as unset
    for env in [
        [("NO_COLOR", "")],
        [("NODE_DISABLE_COLORS", "")],
        [("CLAUDECODE", "")],
    ] {
        let output = run_on_terminal(&env, "");
        assert!(output.contains('\x1b'), "{env:?}: {output:?}");
    }
}

#[test]
fn term_of_dumb_turns_color_off() {
    let output = run_on_terminal(&[("TERM", "dumb")], "");
    assert!(!output.contains('\x1b'), "{output:?}");
}

#[test]
fn force_color_colors_a_pipe_and_its_other_values_do_not() {
    let output = fgro_with_env(&["run", "console.js"], &[("FORCE_COLOR", "1")]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains("log \x1b["), "{stdout:?}");
    assert!(stderr.contains("error Map(1) { \x1b["), "{stderr:?}");

    // it wins over the variables that turn color off
    let output = fgro_with_env(
        &["run", "console.js"],
        &[("FORCE_COLOR", ""), ("NO_COLOR", "1"), ("CLAUDECODE", "1")],
    );
    assert!(String::from_utf8(output.stdout).unwrap().contains('\x1b'));

    let output = fgro_with_env(&["run", "console.js"], &[("FORCE_COLOR", "0")]);
    assert!(!String::from_utf8(output.stdout).unwrap().contains('\x1b'));
    assert!(!String::from_utf8(output.stderr).unwrap().contains('\x1b'));
}

#[test]
fn force_color_of_another_value_turns_a_terminal_off() {
    let output = run_on_terminal(&[("FORCE_COLOR", "0")], "");
    assert!(!output.contains('\x1b'), "{output:?}");
}
