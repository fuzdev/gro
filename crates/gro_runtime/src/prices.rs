//! Prices: switches that turn off one of eval's defaults each, so the
//! measurement harness can time what each default costs, read only under the
//! `prices` cargo feature.
//!
//! Without the feature, [`start`] and [`off`] read nothing and [`off`] is
//! always `false`, so a default build reads no such variable and keeps every
//! default. Release and snapshot builds never enable it; the `prices` suite
//! of `cargo xtask bench` builds a binary that does, and its `--version` says
//! so (`fgro`'s `, prices`).
//!
//! With it, the environment variable [`PRICES_OFF_ENV`] names the defaults to
//! turn off, comma-separated by their [`EvalDefault::name`]s
//! (`GRO_PRICES_OFF=landlock,seccomp`); unset or empty, every default
//! stands. A name it doesn't know, and a value that isn't Unicode, is
//! refused ([`start`]), so a typo never times the default build under
//! another name. A priced build with some
//! default off is no eval's to trust: it can run outside its box, read the
//! clock, or run forever, and exists only to be timed against the build
//! with every default on.
//!
//! The JIT has no switch here: eval runs jitless unless `--jit`, which
//! already turns it on, the box's executable mappings with it.
//!
//! The box's setup isn't priced: the pre-pass computes the link closure and
//! opens a handle to each root whatever the switches say, since the loader
//! reads its roots from it, so `landlock` and `seccomp` price only the
//! walls installed (and `seccomp` its filters' compiling). The setup falls
//! within a phases run's `prepass` phase.

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
    let names: Vec<&str> = EvalDefault::ALL.iter().map(|d| d.name()).collect();
    names.join(", ")
}

/// Parses a [`PRICES_OFF_ENV`] value: the defaults it names.
///
/// # Errors
///
/// Returns [`PricesError::Unknown`] naming a name it doesn't know.
pub fn parse(value: &str) -> Result<Vec<EvalDefault>, PricesError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            EvalDefault::ALL
                .into_iter()
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

    use super::{EvalDefault, PRICES_OFF_ENV, PricesError, parse};

    /// The defaults turned off, read once.
    static OFF: OnceLock<Result<Vec<EvalDefault>, PricesError>> = OnceLock::new();

    fn read() -> &'static Result<Vec<EvalDefault>, PricesError> {
        OFF.get_or_init(|| match std::env::var(PRICES_OFF_ENV) {
            Ok(value) => parse(&value),
            Err(VarError::NotPresent) => Ok(Vec::new()),
            Err(VarError::NotUnicode(_)) => Err(PricesError::NotUnicode),
        })
    }

    pub fn start() -> Result<(), PricesError> {
        read().as_ref().map(|_| ()).map_err(Clone::clone)
    }

    pub fn off(default: EvalDefault) -> bool {
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
pub fn off(default: EvalDefault) -> bool {
    enabled::off(default)
}

/// Whether `default` is turned off: never, without the `prices` feature.
#[cfg(not(feature = "prices"))]
#[inline]
#[expect(
    clippy::missing_const_for_fn,
    reason = "the priced build's reads the environment, and callers are the same in both"
)]
pub fn off(_default: EvalDefault) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_a_comma_separated_list() {
        assert_eq!(parse(""), Ok(Vec::new()));
        assert_eq!(parse(" , "), Ok(Vec::new()));
        assert_eq!(
            parse("landlock, seccomp"),
            Ok(vec![EvalDefault::Landlock, EvalDefault::Seccomp])
        );
        let error = parse("deny,jitless").unwrap_err().to_string();
        assert!(error.contains("`jitless`"), "{error}");
        assert!(error.contains("envelope_cap"), "{error}");
    }

    #[test]
    fn every_default_is_listed_once_by_a_distinct_name() {
        for (index, default) in EvalDefault::ALL.iter().enumerate() {
            assert_eq!(parse(default.name()), Ok(vec![*default]));
            assert!(
                !EvalDefault::ALL[..index]
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
        assert!(EvalDefault::ALL.iter().all(|default| !off(*default)));
    }
}
