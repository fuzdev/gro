//! gro's bootstrap extension: installs the pure web globals from `deno_web` and
//! hides the host's internals from user code.
//!
//! Its JS (`js/bootstrap.js`) is embedded in the binary at compile time. The
//! embedding (`ascii_str_include!`) asserts in a constant that the source is
//! 7-bit ASCII, so a non-ASCII character fails the build rather than reaching
//! V8, which reads extension sources as one-byte strings.
//!
//! `deno_webidl` and `deno_web`'s own sources aren't embedded: `deno_core` reads them
//! from their crates' paths on the build machine when they first load. A
//! startup snapshot carries the ones the bootstrap loads, and the binary that
//! starts from it embeds the rest ([`crate::snapshot`]); without one, the
//! runtime runs only from its build tree.

use std::borrow::Cow;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::sync::OnceLock;

use deno_core::error::JsError;
use deno_core::{Extension, ExtensionFileSource, JsRuntime, ascii_str, ascii_str_include, op2};

/// The bootstrap module's specifier.
const BOOTSTRAP_SPECIFIER: &str = "ext:gro/bootstrap.js";

/// The bootstrap module, embedded.
const BOOTSTRAP_ESM: &[ExtensionFileSource] = &[ExtensionFileSource::new(
    BOOTSTRAP_SPECIFIER,
    ascii_str_include!("../js/bootstrap.js"),
)];

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

/// Whether console output to the process's stdout, or with `stderr` its
/// stderr, is colored: [`color_env`]'s rule over the process environment,
/// deciding per stream. The one place every color input is read: the host
/// reads the environment and asks each stream once per process, on first use
/// (statics, so never in a snapshot); user code never sees either.
fn use_color(stderr: bool) -> bool {
    static COLOR_ENV: OnceLock<ColorEnv> = OnceLock::new();
    // stdout's, then stderr's: whether the stream is a terminal
    static IS_TERMINAL: [OnceLock<bool>; 2] = [OnceLock::new(), OnceLock::new()];
    COLOR_ENV
        .get_or_init(|| color_env(|name| std::env::var_os(name)))
        .resolve(|| {
            *IS_TERMINAL[usize::from(stderr)].get_or_init(|| {
                if stderr {
                    std::io::stderr().is_terminal()
                } else {
                    std::io::stdout().is_terminal()
                }
            })
        })
}

/// [`use_color`] for the console, asked per write (and answered from its cache).
#[op2(fast)]
fn op_gro_color(stderr: bool) -> bool {
    use_color(stderr)
}

deno_core::extension!(
    gro,
    deps = [deno_webidl, deno_web],
    ops = [op_gro_color],
    customizer = |extension: &mut Extension| {
        // the macro's own `esm` list reads files from the build tree at runtime
        extension.esm_files = Cow::Borrowed(BOOTSTRAP_ESM);
        extension.esm_entry_point = Some(BOOTSTRAP_SPECIFIER);
    },
);

/// The runtime's extensions, in dependency order: `deno_webidl`, `deno_web`, and
/// gro's bootstrap last, since it reads both.
pub fn extensions() -> Vec<Extension> {
    vec![
        deno_webidl::deno_webidl::init(),
        deno_web::deno_web::init(
            std::sync::Arc::new(deno_web::BlobStore::default()),
            None,
            false,
            deno_web::InMemoryBroadcastChannel::default(),
        ),
        gro::init(),
    ]
}

/// Runs the bootstrap's per-process steps on a started runtime, from a
/// snapshot or not: what a snapshot must not carry (the time origin) and what
/// `deno_core` still reads while it starts from one (`Deno.core`, removed here
/// with `__bootstrap`).
///
/// # Errors
///
/// Returns the JS error if the steps throw.
pub fn start(runtime: &mut JsRuntime) -> Result<(), Box<JsError>> {
    runtime
        .execute_script(
            "ext:gro/start.js",
            ascii_str!("globalThis.__bootstrap.gro_start();"),
        )
        .map(drop)
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
