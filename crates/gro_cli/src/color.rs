//! The console's color, decided from the environment and each stream: the
//! one place `fgro` reads a color input. The runtime takes the decision as
//! `ConsoleOptions` and reads no environment itself.

use std::ffi::OsString;
use std::io::IsTerminal;

use gro_runtime::ConsoleOptions;

/// The environment's verdict on console color, before any stream is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorEnv {
    /// Colored whatever the stream is.
    On,
    /// Uncolored whatever the stream is.
    Off,
    /// Colored only when the stream is a terminal.
    Terminal,
}

/// The `FORCE_COLOR` values that turn color on, as in Node's `node:tty`; any
/// other value turns it off.
const FORCE_COLOR_ON: &[&str] = &["", "1", "2", "3", "true"];

/// Decides [`ColorEnv`] from `var`, a lookup of an environment variable, in
/// order:
///
/// - `FORCE_COLOR` set, empty included: on for `''`, `'1'`, `'2'`, `'3'`, or
///   `'true'`, otherwise off
/// - `NO_COLOR`, `NODE_DISABLE_COLORS`, or `CLAUDECODE` set non-empty, or
///   `TERM` of `dumb`: off (an agent's transcript shouldn't carry ANSI codes)
/// - otherwise colored only on a terminal
///
/// `CLICOLOR`, `CLICOLOR_FORCE`, and `COLORTERM` aren't read. A value that
/// isn't UTF-8 is set and matches none of the values named, so a
/// `FORCE_COLOR` of one turns color off.
fn color_env(var: impl Fn(&str) -> Option<OsString>) -> ColorEnv {
    if let Some(force) = var("FORCE_COLOR") {
        return if FORCE_COLOR_ON.iter().any(|on| force == *on) {
            ColorEnv::On
        } else {
            ColorEnv::Off
        };
    }
    let set = |name| var(name).is_some_and(|value| !value.is_empty());
    if set("NO_COLOR")
        || set("NODE_DISABLE_COLORS")
        || set("CLAUDECODE")
        || var("TERM").is_some_and(|term| term == "dumb")
    {
        return ColorEnv::Off;
    }
    ColorEnv::Terminal
}

impl ColorEnv {
    /// Whether a stream is colored, asking `is_terminal` only when the
    /// environment leaves it to the stream.
    fn resolve(self, is_terminal: impl FnOnce() -> bool) -> bool {
        match self {
            Self::On => true,
            Self::Off => false,
            Self::Terminal => is_terminal(),
        }
    }
}

/// The console options for `fgro run`: [`color_env`]'s rule over the process
/// environment, deciding per stream, each stream asked whether it's a
/// terminal only when the environment leaves it to the stream; every level
/// writes to its default stream.
pub fn console_options() -> ConsoleOptions {
    let env = color_env(|name| std::env::var_os(name));
    ConsoleOptions {
        color_stdout: env.resolve(|| std::io::stdout().is_terminal()),
        color_stderr: env.resolve(|| std::io::stderr().is_terminal()),
        ..ConsoleOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use super::{ColorEnv, color_env};

    /// Whether a stream is colored with only `vars` set, on a terminal or not.
    fn colored(vars: &[(&str, OsString)], is_terminal: bool) -> bool {
        color_env(|name| {
            vars.iter()
                .find(|(set, _)| *set == name)
                .map(|(_, value)| value.clone())
        })
        .resolve(|| is_terminal)
    }

    fn var(name: &'static str, value: &str) -> (&'static str, OsString) {
        (name, OsString::from(value))
    }

    #[test]
    fn with_no_vars_only_a_terminal_is_colored() {
        assert!(colored(&[], true));
        assert!(!colored(&[], false));
    }

    #[test]
    fn force_color_turns_color_on_without_a_terminal() {
        for value in ["", "1", "2", "3", "true"] {
            assert!(colored(&[var("FORCE_COLOR", value)], false), "{value:?}");
        }
    }

    #[test]
    fn force_color_turns_color_off_on_a_terminal_for_other_values() {
        for value in ["0", "false", "yes", "TRUE", " 1"] {
            assert!(!colored(&[var("FORCE_COLOR", value)], true), "{value:?}");
        }
    }

    #[test]
    fn force_color_takes_precedence_over_the_off_vars() {
        let vars = [
            var("FORCE_COLOR", "1"),
            var("NO_COLOR", "1"),
            var("NODE_DISABLE_COLORS", "1"),
            var("CLAUDECODE", "1"),
            var("TERM", "dumb"),
        ];
        assert!(colored(&vars, false));
    }

    #[test]
    fn the_off_vars_turn_color_off_only_when_non_empty() {
        for name in ["NO_COLOR", "NODE_DISABLE_COLORS", "CLAUDECODE"] {
            assert!(!colored(&[var(name, "1")], true), "{name}");
            assert!(!colored(&[var(name, "0")], true), "{name}");
            // set but empty counts as unset
            assert!(colored(&[var(name, "")], true), "{name}");
        }
    }

    #[test]
    fn term_of_dumb_turns_color_off() {
        assert!(!colored(&[var("TERM", "dumb")], true));
        assert!(colored(&[var("TERM", "xterm-256color")], true));
        assert!(colored(&[var("TERM", "")], true));
    }

    #[test]
    fn the_unhonored_vars_change_nothing() {
        for name in ["CLICOLOR", "CLICOLOR_FORCE", "COLORTERM"] {
            for value in ["0", "1"] {
                assert!(colored(&[var(name, value)], true), "{name}={value}");
                assert!(!colored(&[var(name, value)], false), "{name}={value}");
            }
        }
    }

    #[test]
    fn a_value_that_is_not_utf_8_is_set_and_matches_nothing() {
        let invalid = || OsString::from_vec(vec![0xff]);
        assert!(!colored(&[("FORCE_COLOR", invalid())], true));
        assert!(!colored(&[("NO_COLOR", invalid())], true));
        assert!(colored(&[("TERM", invalid())], true));
    }

    #[test]
    fn the_terminal_is_asked_only_when_the_environment_leaves_it() {
        let unasked = || panic!("the terminal was asked");
        assert!(ColorEnv::On.resolve(unasked));
        assert!(!ColorEnv::Off.resolve(unasked));
        assert!(ColorEnv::Terminal.resolve(|| true));
    }
}
