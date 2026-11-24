//! Prices: switches that turn off one of eval's defaults, or one step of a
//! command's box, each, so the measurement harness can time what each costs,
//! read only under the `prices` cargo feature.
//!
//! Without the feature, [`start`] and [`off`] read nothing and [`off`] is
//! always `false`, so a default build reads no such variable and keeps every
//! default. The binary that ships never enables it; the `prices` suite
//! of `cargo xtask bench` builds a binary that does, and its `--version` says
//! so (`fgro`'s `, prices`).
//!
//! With it, the environment variable [`PRICES_OFF_ENV`] names the defaults to
//! turn off, comma-separated by their names ([`EvalDefault::name`],
//! [`RunDefault::name`]): `GRO_PRICES_OFF=landlock,seccomp` for eval's walls,
//! `GRO_PRICES_OFF=run_landlock,run_filter` for a command's; unset or empty,
//! every default stands. A name it doesn't know, and a value that isn't
//! Unicode, is refused ([`start`]), so a typo never times the default build
//! under another name. A priced build with some default off is no eval's or
//! command's to trust: it can run outside its box, read the clock, or run
//! forever, and exists only to be timed against the build with every default
//! on.
//!
//! The JIT has no switch here: eval runs jitless unless `--jit`, which
//! already turns it on, the box's executable mappings with it.
//!
//! Eval's box's setup isn't priced: the pre-pass computes the link closure and
//! opens a handle to each root whatever the switches say, since the loader
//! reads its roots from it, so `landlock` and `seccomp` price only the
//! walls installed (and `seccomp` its filters' compiling). The setup falls
//! within a phases run's `prepass` phase.
//!
//! A command's box ([`RunDefault`]) is priced the same way: its facts and
//! plan can't be switched off, since its rules and tool directories come
//! from them, so a phases run times them as phases of their own (`facts`,
//! `plan`), and each switch prices only its step.

/// Whether this build reads the switches: the `prices` feature of this
/// crate.
pub const ENABLED: bool = cfg!(feature = "prices");

/// The variable naming the defaults to turn off.
pub const PRICES_OFF_ENV: &str = "GRO_PRICES_OFF";

/// One of eval's defaults a priced build can turn off, named by what it
/// keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalDefault {
    /// The descriptors the process inherited, closed before anything opens
    /// one.
    CloseFds,
    /// The environment cleared but `TZ`, so no library reads a variable.
    ClearEnv,
    /// The process's data segment capped (`eval::MEMORY_LIMIT`), and the
    /// default thread stack it counts lowered (`eval::DEFAULT_THREAD_STACK`).
    DataLimit,
    /// The box's Landlock rules (and the `no_new_privs` they set).
    Landlock,
    /// The box's seccomp filters: the killing filter and the allowlist,
    /// with its refusal of executable mappings in a jitless eval.
    Seccomp,
    /// The loader's own refusal of a read outside the box's roots, beneath
    /// Landlock's.
    BoxRule,
    /// `determinism: deny`: the clock, randomness, timers, and blobs refused
    /// in JS by the bootstrap.
    Deny,
    /// The op middleware refusing `bootstrap::REFUSED_OPS`.
    RefuseOps,
    /// Code generation from strings off (V8's
    /// `--disallow-code-generation-from-strings`).
    Codegen,
    /// The ICU default locale pinned to `en-US`.
    Locale,
    /// The isolate's heap capped (`eval::HEAP_LIMIT`).
    HeapCap,
    /// The near-heap-limit callback and the out-of-memory backstop.
    HeapWatch,
    /// The timeout's watchdog thread.
    Watchdog,
    /// The check that the time zone is UTC.
    UtcCheck,
    /// The loader's full record of the graph (`Recording::Full`), which the
    /// envelope's `access` and a failure's kind read.
    Recording,
    /// The envelope's cap (`eval::MAX_ENVELOPE_SIZE`), which counts the
    /// envelope's bytes before it's written.
    EnvelopeCap,
}

impl EvalDefault {
    /// Every default, in the order eval applies them.
    pub const ALL: [Self; 16] = [
        Self::CloseFds,
        Self::ClearEnv,
        Self::DataLimit,
        Self::Landlock,
        Self::Seccomp,
        Self::BoxRule,
        Self::Deny,
        Self::RefuseOps,
        Self::Codegen,
        Self::Locale,
        Self::HeapCap,
        Self::HeapWatch,
        Self::Watchdog,
        Self::UtcCheck,
        Self::Recording,
        Self::EnvelopeCap,
    ];

    /// The name [`PRICES_OFF_ENV`] lists it by.
    pub const fn name(self) -> &'static str {
        match self {
            Self::CloseFds => "close_fds",
            Self::ClearEnv => "clear_env",
            Self::DataLimit => "data_limit",
            Self::Landlock => "landlock",
            Self::Seccomp => "seccomp",
            Self::BoxRule => "box_rule",
            Self::Deny => "deny",
            Self::RefuseOps => "refuse_ops",
            Self::Codegen => "codegen",
            Self::Locale => "locale",
            Self::HeapCap => "heap_cap",
            Self::HeapWatch => "heap_watch",
            Self::Watchdog => "watchdog",
            Self::UtcCheck => "utc_check",
            Self::Recording => "recording",
            Self::EnvelopeCap => "envelope_cap",
        }
    }
}

/// One step of a command's box a priced build can turn off, named by what
/// it keeps (`gro_runtime::confine::run`'s steps, and the sweep `fgro`
/// takes between them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunDefault {
    /// The run's own user and network namespace, entered with its loopback
    /// up and nested user namespaces refused. Off, the process stays in the
    /// host's namespaces and its box is built for its own network's form
    /// all the same (`RunNetwork::Namespace`), so the steps after it do the
    /// work they do by default. On a host where the namespaces are refused,
    /// the default's form is `RunNetwork::InetRefused` and the stand-in's
    /// isn't, so the price there also counts the form's change: the TCP wall
    /// and the denylist's socket rule.
    Namespace,
    /// The sweep of the private directories of runs gone from the OS's
    /// temporary directory.
    Sweep,
    /// The box's Landlock rules installed, the run's and each program's
    /// own. Off, the rules are still opened from the plan, the handles a
    /// program's box would name still held, and the filter's install still
    /// sets `no_new_privs`, but no launch builds or installs a program's box,
    /// each child starting in the run's process as it is, its temporary
    /// variables the run's.
    Landlock,
    /// The box's seccomp denylist, assembled and installed.
    Filter,
}

impl RunDefault {
    /// Every default, in the order a command's start takes them.
    pub const ALL: [Self; 4] = [Self::Namespace, Self::Sweep, Self::Landlock, Self::Filter];

    /// The name [`PRICES_OFF_ENV`] lists it by.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Namespace => "run_namespace",
            Self::Sweep => "run_sweep",
            Self::Landlock => "run_landlock",
            Self::Filter => "run_filter",
        }
    }
}

/// A default a priced build can turn off: eval's or a command's box's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// One of eval's defaults.
    Eval(EvalDefault),
    /// One step of a command's box.
    Run(RunDefault),
}

impl Switch {
    /// Every switch, eval's then a command's, each family in its order.
    pub fn all() -> impl Iterator<Item = Self> {
        EvalDefault::ALL
            .into_iter()
            .map(Self::Eval)
            .chain(RunDefault::ALL.into_iter().map(Self::Run))
    }

    /// The name [`PRICES_OFF_ENV`] lists it by.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Eval(default) => default.name(),
            Self::Run(default) => default.name(),
        }
    }
}

impl From<EvalDefault> for Switch {
    fn from(default: EvalDefault) -> Self {
        Self::Eval(default)
    }
}

impl From<RunDefault> for Switch {
    fn from(default: RunDefault) -> Self {
        Self::Run(default)
    }
}

/// Why a [`PRICES_OFF_ENV`] value is refused, each naming every default.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PricesError {
    /// It names a default there is none of.
    #[error(
        "{} names `{name}`, which is no default a priced build turns off; the defaults are {}",
        PRICES_OFF_ENV,
        default_names()
    )]
    Unknown {
        /// The name, as listed.
        name: String,
    },
    /// It isn't Unicode, so it names nothing it can be read as.
    #[error(
        "{} is not valid Unicode, so it names no default a priced build turns off; the \
         defaults are {}",
        PRICES_OFF_ENV,
        default_names()
    )]
    NotUnicode,
}

/// Every default's name, comma-separated.
fn default_names() -> String {
    let names: Vec<&str> = Switch::all().map(Switch::name).collect();
    names.join(", ")
}

/// Parses a [`PRICES_OFF_ENV`] value: the defaults it names.
///
/// # Errors
///
/// Returns [`PricesError::Unknown`] naming a name it doesn't know.
pub fn parse(value: &str) -> Result<Vec<Switch>, PricesError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            Switch::all()
                .find(|default| default.name() == name)
                .ok_or_else(|| PricesError::Unknown {
                    name: name.to_owned(),
                })
        })
        .collect()
}

#[cfg(feature = "prices")]
mod enabled {
    use std::env::VarError;
    use std::sync::OnceLock;

    use super::{PRICES_OFF_ENV, PricesError, Switch, parse};

    /// The defaults turned off, read once.
    static OFF: OnceLock<Result<Vec<Switch>, PricesError>> = OnceLock::new();

    fn read() -> &'static Result<Vec<Switch>, PricesError> {
        OFF.get_or_init(|| match std::env::var(PRICES_OFF_ENV) {
            Ok(value) => parse(&value),
            Err(VarError::NotPresent) => Ok(Vec::new()),
            Err(VarError::NotUnicode(_)) => Err(PricesError::NotUnicode),
        })
    }

    pub fn start() -> Result<(), PricesError> {
        read().as_ref().map(|_| ()).map_err(Clone::clone)
    }

    pub fn off(default: Switch) -> bool {
        read().as_ref().is_ok_and(|off| off.contains(&default))
    }
}

/// Reads [`PRICES_OFF_ENV`]. Call it first thing in the embedder's entry, so
/// an unknown name fails before anything runs.
///
/// # Errors
///
/// Returns why the value is refused: a name it doesn't know ([`parse`]), or
/// a value that isn't Unicode.
#[cfg(feature = "prices")]
pub fn start() -> Result<(), PricesError> {
    enabled::start()
}

/// Reads the switches: nothing without the `prices` feature.
///
/// # Errors
///
/// Never, without the feature.
#[cfg(not(feature = "prices"))]
#[inline]
pub const fn start() -> Result<(), PricesError> {
    Ok(())
}

/// Whether `default` is turned off in this process: only in a priced build,
/// when [`PRICES_OFF_ENV`] names it. A value [`start`] refuses turns nothing
/// off.
#[cfg(feature = "prices")]
pub fn off(default: impl Into<Switch>) -> bool {
    enabled::off(default.into())
}

/// Whether `default` is turned off: never, without the `prices` feature.
#[cfg(not(feature = "prices"))]
#[inline]
pub fn off(_default: impl Into<Switch>) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bench's prices suite prices every default: a variant runtime
    /// named for each, turning it off by [`PRICES_OFF_ENV`].
    #[test]
    fn the_prices_suite_has_a_variant_for_every_default() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gro_bench/suites/prices/suite.json");
        let suite = gro_bench::suite::Suite::load(&path).unwrap();
        for default in EvalDefault::ALL {
            let variant = suite
                .runtimes
                .iter()
                .find(|runtime| runtime.name == default.name());
            assert_eq!(
                variant
                    .and_then(|runtime| runtime.env.get(PRICES_OFF_ENV))
                    .map(String::as_str),
                Some(default.name()),
                "{default:?}"
            );
        }
    }

    /// The bench's `verb_prices` suite prices every step of a command's box
    /// the same way.
    #[test]
    fn the_verb_prices_suite_has_a_variant_for_every_run_default() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gro_bench/suites/verb_prices/suite.json");
        let suite = gro_bench::suite::Suite::load(&path).unwrap();
        for default in RunDefault::ALL {
            let variant = suite
                .runtimes
                .iter()
                .find(|runtime| runtime.name == default.name());
            assert_eq!(
                variant
                    .and_then(|runtime| runtime.env.get(PRICES_OFF_ENV))
                    .map(String::as_str),
                Some(default.name()),
                "{default:?}"
            );
        }
    }

    #[test]
    fn parse_reads_a_comma_separated_list() {
        assert_eq!(parse(""), Ok(Vec::new()));
        assert_eq!(parse(" , "), Ok(Vec::new()));
        assert_eq!(
            parse("landlock, seccomp"),
            Ok(vec![
                Switch::Eval(EvalDefault::Landlock),
                Switch::Eval(EvalDefault::Seccomp)
            ])
        );
        // a command's steps beside eval's, by their own names
        assert_eq!(
            parse("run_landlock,deny"),
            Ok(vec![
                Switch::Run(RunDefault::Landlock),
                Switch::Eval(EvalDefault::Deny)
            ])
        );
        let error = parse("deny,jitless").unwrap_err();
        assert_eq!(
            error,
            PricesError::Unknown {
                name: "jitless".to_owned()
            }
        );
        // the message lists every default
        assert!(error.to_string().contains("envelope_cap"), "{error}");
        assert!(error.to_string().contains("run_filter"), "{error}");
    }

    #[test]
    fn every_default_is_listed_once_by_a_distinct_name() {
        let all: Vec<Switch> = Switch::all().collect();
        assert_eq!(all.len(), EvalDefault::ALL.len() + RunDefault::ALL.len());
        for (index, default) in all.iter().enumerate() {
            assert_eq!(parse(default.name()), Ok(vec![*default]));
            assert!(
                !all[..index]
                    .iter()
                    .any(|other| other.name() == default.name()),
                "{default:?}"
            );
        }
    }

    #[cfg(not(feature = "prices"))]
    #[test]
    fn a_default_build_turns_nothing_off() {
        assert_eq!(start(), Ok(()));
        assert!(Switch::all().all(|default| !off(default)));
    }
}
