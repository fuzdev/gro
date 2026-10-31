//! The engine: V8's process-wide setup, apart from any one runtime.
//!
//! V8 takes some settings once per process, before its platform starts or its
//! first isolate exists, and every runtime the process makes shares them.
//! [`EngineOptions`] carries those settings and [`init_engine`] applies them,
//! before the first runtime; [`RuntimeOptions`](crate::RuntimeOptions) carries
//! what each runtime decides for itself.
//!
//! [`init_engine`] returns an [`Engine`], the token every runtime start takes
//! ([`crate::run_main`], [`crate::snapshot::make_snapshot`]). Only
//! [`init_engine`] makes one, so no runtime starts on an engine nobody chose,
//! and the token names the options the engine has.
//!
//! Whatever the options, the engine draws V8's entropy from the operating
//! system's random source (`getrandom`), not from V8's own: left alone, V8
//! opens `/dev/urandom` itself and falls back to the clock, without a word,
//! when it can't.
//!
//! The order, before `deno_core` initializes V8 in the first
//! `JsRuntime::try_new` (its `setup::init_v8`): the entropy source, which V8
//! reads from platform init on; then the ICU default locale, which each
//! isolate reads when its `Intl` first asks. ICU's data is linked into the V8
//! library (`icu_use_data_file = false` in the `v8` crate's build), and
//! `deno_core` loads data of its own (`set_common_data_77`) only under its
//! `include_icu_data` feature, which this crate doesn't enable, so no data
//! load comes after the locale is set.

use std::sync::OnceLock;

use deno_core::v8;

/// The engine's process-wide options. The default is what `fgro run` uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOptions {
    /// The ICU default locale every runtime's `Intl` starts from, such as
    /// `en_US_POSIX`. `None` keeps ICU's own default, which it takes from the
    /// host (`LC_ALL`, then `LC_MESSAGES`, then `LANG`).
    pub locale: Option<&'static str>,
}

/// Why [`init_engine`] refused.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The engine was set up already, with other options.
    #[error("the engine is already set up with other options: {0:?}")]
    Conflict(EngineOptions),
}

/// The options the engine was set up with. A static because the engine is
/// process state by nature: V8 keeps these settings for the life of the
/// process, whichever runtime asked first.
static ENGINE: OnceLock<EngineOptions> = OnceLock::new();

/// The engine, set up: proof that [`init_engine`] ran in this process, and
/// with which options. Every runtime start takes one.
#[derive(Debug, Clone, Copy)]
pub struct Engine(&'static EngineOptions);

impl Engine {
    /// The options the engine was set up with.
    #[must_use]
    pub const fn options(self) -> &'static EngineOptions {
        self.0
    }
}

/// Sets the engine up with `options`, once per process, before the first
/// runtime starts, and returns the token runtimes start with.
///
/// A second call with equal options does nothing but return the token again,
/// so callers needn't track whether it ran.
///
/// # Errors
///
/// Returns [`EngineError::Conflict`] when the engine was set up already with
/// options other than `options`.
///
/// # Panics
///
/// When the locale contains a NUL byte, which no locale name does.
pub fn init_engine(options: &EngineOptions) -> Result<Engine, EngineError> {
    let mut applied_here = false;
    let applied = ENGINE.get_or_init(|| {
        applied_here = true;
        apply(options);
        options.clone()
    });
    if applied_here || applied == options {
        Ok(Engine(applied))
    } else {
        Err(EngineError::Conflict(applied.clone()))
    }
}

/// Applies `options` to V8, in the order the module doc gives.
fn apply(options: &EngineOptions) {
    v8::V8::set_entropy_source(fill_entropy);
    if let Some(locale) = options.locale {
        v8::icu::set_default_locale(locale);
    }
}

/// V8's entropy source: fills `buffer` from the operating system's random
/// source, returning whether it could. V8 falls back to its own seeding
/// when it couldn't.
fn fill_entropy(buffer: &mut [u8]) -> bool {
    #[cfg(test)]
    tests::ENTROPY_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    getrandom::fill(buffer).is_ok()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{EngineError, EngineOptions, init_engine};
    use crate::{RuntimeOptions, STACK_SIZE, run_main};

    /// How many times V8 has asked [`fill_entropy`](super::fill_entropy) in
    /// this process. Only V8 calls it, so a count above zero means V8 has the
    /// source.
    pub(super) static ENTROPY_CALLS: AtomicUsize = AtomicUsize::new(0);

    /// The unit tests share one process and the default engine, so whichever
    /// test sets it up first, a run here has V8 asking the source.
    #[test]
    fn v8_draws_its_entropy_from_the_source() {
        let engine = init_engine(&EngineOptions::default()).unwrap();
        let dir = std::env::temp_dir().join(format!("gro_runtime_{}_entropy", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry: PathBuf = dir.join("main.mjs");
        std::fs::write(&entry, "Math.random();\n").unwrap();
        let cwd = dir.clone();
        let result = std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(move || run_main(engine, &RuntimeOptions::default(), &entry, &cwd))
            .unwrap()
            .join()
            .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(result.is_ok(), "{result:?}");
        assert!(ENTROPY_CALLS.load(Ordering::Relaxed) > 0);
        // and the source fills, so V8 never falls back to its own seeding
        assert!(super::fill_entropy(&mut [0; 8]));
        // set up with the default options, so equal ones are accepted and
        // others refused
        assert!(init_engine(&EngineOptions::default()).is_ok());
        let other = EngineOptions {
            locale: Some("en_US_POSIX"),
        };
        assert!(matches!(
            init_engine(&other),
            Err(EngineError::Conflict(options)) if options == EngineOptions::default()
        ));
    }
}
