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
//! [`init_engine`] makes one, so no runtime starts on an engine nobody
//! chose, and the token names the options the engine has and its process's
//! [`Confinement`]: the engine starts only once the process is confined or
//! named unconfined, since V8's worker threads start with it and a box
//! installed after them would leave them outside. [`init_confined_engine`]
//! returns a [`ConfinedEngine`], the token [`crate::eval::eval`] takes, which
//! only a confined process's engine has, so no caller can evaluate a module
//! outside the box. A confined engine's JIT must be the one its box was
//! installed for ([`Confined::jit`]), since a jitless box refuses the
//! executable mappings the JIT makes: [`init_engine`] refuses a disagreement
//! before V8 starts.
//!
//! Whatever the options, the engine draws V8's entropy from the operating
//! system's random source (`getrandom`), not from V8's own: left alone, V8
//! opens `/dev/urandom` itself and falls back to the clock, without a word,
//! when it can't. In a confined process, where the box denies
//! `/dev/urandom`, a failed draw stops the process with a message instead
//! of letting V8 fall back to the clock; unconfined, V8's fallback stands,
//! its `/dev/urandom` read first.
//!
//! The order, before `deno_core` initializes V8 in the first
//! `JsRuntime::try_new` (its `setup::init_v8`): the entropy source, which V8
//! reads from platform init on; then V8's flags (the JIT and code generation
//! from strings), which V8 freezes once its platform starts, and to which
//! `deno_core` adds its own without touching these; then the ICU default
//! locale, which each
//! isolate reads when its `Intl` first asks. ICU's data is linked into the V8
//! library (`icu_use_data_file = false` in the `v8` crate's build), and
//! `deno_core` loads data of its own (`set_common_data_77`) only under its
//! `include_icu_data` feature, which this crate doesn't enable, so no data
//! load comes after the locale is set.

use std::sync::OnceLock;

use deno_core::v8;

use crate::confine::{Confined, Confinement, Jit};

/// The engine's process-wide options. The default is what `fgro run` uses:
/// the JIT on, code generation from strings allowed, and ICU's own locale.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOptions {
    /// The ICU default locale every runtime's `Intl` starts from, such as
    /// `en-US`. `None` keeps ICU's own default, which it takes from the
    /// host (`LC_ALL`, then `LC_MESSAGES`, then `LANG`).
    pub locale: Option<&'static str>,
    /// V8's `--jitless`: JS runs in the interpreter alone, with no code
    /// compiled at run time, and `WebAssembly` is absent from every global
    /// scope, since it needs the compilers. In a confined process it must
    /// agree with the box's [`Jit`].
    pub jitless: bool,
    /// V8's `--disallow-code-generation-from-strings`: `eval`, the
    /// `Function` constructors (async and generator ones included), and a
    /// string passed to `setTimeout` or `setInterval` throw an `EvalError`.
    pub disallow_code_generation_from_strings: bool,
}

impl EngineOptions {
    /// The V8 flags these options set, as V8 parses them: empty when they
    /// set none.
    fn v8_flags(&self) -> String {
        let mut flags = Vec::new();
        if self.jitless {
            flags.push("--jitless");
        }
        if self.disallow_code_generation_from_strings {
            flags.push("--disallow-code-generation-from-strings");
        }
        flags.join(" ")
    }
}

/// Why [`init_engine`] refused.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// The engine was set up already, with other options.
    #[error("the engine is already set up with other options: {0:?}")]
    Conflict(EngineOptions),
    /// The engine was set up already, in a process confined otherwise.
    #[error("the engine is already set up, {}", if *.confined { "confined to another box" } else { "unconfined" })]
    ConfinementConflict {
        /// Whether it was set up confined.
        confined: bool,
    },
    /// The process's box was installed for one JIT choice and the options
    /// make the other: a jitless box would fail the JIT's executable
    /// mappings, and a box for the JIT would allow them to a jitless engine.
    #[error(
        "the box was installed for an engine with the JIT {}, and the engine's options turn it {}",
        if matches!(.box_jit, Jit::On) { "on" } else { "off" },
        if matches!(.box_jit, Jit::On) { "off" } else { "on" }
    )]
    JitConflict {
        /// The JIT the box was installed for.
        box_jit: Jit,
    },
}

/// What the engine was set up with.
#[derive(Debug)]
struct Setup {
    /// Its options.
    options: EngineOptions,
    /// Its process's confinement.
    confinement: Confinement,
}

/// What the engine was set up with. A static because the engine is process
/// state by nature: V8 keeps these settings for the life of the process,
/// whichever runtime asked first.
static ENGINE: OnceLock<Setup> = OnceLock::new();

/// The engine, set up: proof that [`init_engine`] ran in this process, with
/// which options, and how the process is confined. Every runtime start takes
/// one.
#[derive(Debug, Clone, Copy)]
pub struct Engine(&'static Setup);

impl Engine {
    /// The options the engine was set up with.
    #[must_use]
    pub const fn options(self) -> &'static EngineOptions {
        &self.0.options
    }

    /// The process's confinement, as the engine was set up with it.
    #[must_use]
    pub const fn confinement(self) -> &'static Confinement {
        &self.0.confinement
    }
}

/// An [`Engine`] set up in a process held in its box.
///
/// The token [`crate::eval::eval`] takes: made only by
/// [`init_confined_engine`], from a [`Confined`] that only the box's
/// installation makes, so an engine set up unconfined never has one.
#[derive(Debug, Clone, Copy)]
pub struct ConfinedEngine {
    /// The engine.
    engine: Engine,
    /// The box its process is held in.
    confined: &'static Confined,
}

impl ConfinedEngine {
    /// The engine, for what any engine's token answers.
    #[must_use]
    pub const fn engine(self) -> Engine {
        self.engine
    }

    /// The box the engine's process is held in.
    #[must_use]
    pub const fn confined(self) -> &'static Confined {
        self.confined
    }
}

/// [`init_engine`] in a process held in the box `confined` names, returning
/// the token only a confined engine has.
///
/// # Errors
///
/// As [`init_engine`]: the box was installed for another JIT than
/// `options` make, or the engine was set up already with other options, or
/// confined otherwise (unconfined among them).
pub fn init_confined_engine(
    confined: Confined,
    options: &EngineOptions,
) -> Result<ConfinedEngine, EngineError> {
    let engine = init_engine(Confinement::Confined(confined), options)?;
    match engine.confinement() {
        Confinement::Confined(confined) => Ok(ConfinedEngine { engine, confined }),
        // `init_engine` returns only an engine confined as asked
        Confinement::Unconfined => Err(EngineError::ConfinementConflict { confined: false }),
    }
}

/// Sets the engine up with `options` in a process confined as `confinement`
/// says, once per process, before the first runtime starts, and returns the
/// token runtimes start with.
///
/// A second call with equal options and an equal confinement does nothing but
/// return the token again, so callers needn't track whether it ran.
///
/// # Errors
///
/// Returns [`EngineError::JitConflict`], before anything is set up, when
/// `confinement` is a box installed for another JIT than `options` make;
/// [`EngineError::Conflict`] when the engine was set up already with
/// options other than `options`; and [`EngineError::ConfinementConflict`]
/// when it was set up confined otherwise.
///
/// # Panics
///
/// When the locale contains a NUL byte, which no locale name does.
pub fn init_engine(
    confinement: Confinement,
    options: &EngineOptions,
) -> Result<Engine, EngineError> {
    if let Confinement::Confined(confined) = &confinement
        && confined.jit() != Jit::from_jitless(options.jitless)
    {
        return Err(EngineError::JitConflict {
            box_jit: confined.jit(),
        });
    }
    let mut confinement = Some(confinement);
    let applied = ENGINE.get_or_init(|| {
        // never a default: an engine set up unconfined in a confined process
        // would fail open
        #[expect(
            clippy::expect_used,
            reason = "the closure runs at most once, before anything else takes the confinement"
        )]
        let confinement = confinement
            .take()
            .expect("the confinement is taken once, by the engine's setup");
        apply(options, &confinement);
        Setup {
            options: options.clone(),
            confinement,
        }
    });
    let Some(confinement) = confinement else {
        return Ok(Engine(applied));
    };
    if &applied.options != options {
        Err(EngineError::Conflict(applied.options.clone()))
    } else if applied.confinement != confinement {
        Err(EngineError::ConfinementConflict {
            confined: matches!(applied.confinement, Confinement::Confined(_)),
        })
    } else {
        Ok(Engine(applied))
    }
}

/// Applies `options` to V8 in a process confined as `confinement` says, in
/// the order the module doc gives.
fn apply(options: &EngineOptions, confinement: &Confinement) {
    // each a function item, which V8's binding requires: it stores no pointer
    match confinement {
        Confinement::Confined(_) => v8::V8::set_entropy_source(fill_entropy_or_abort),
        Confinement::Unconfined => v8::V8::set_entropy_source(fill_entropy),
    }
    let flags = options.v8_flags();
    if !flags.is_empty() {
        v8::V8::set_flags_from_string(&flags);
    }
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

/// V8's entropy source in a confined process: fills `buffer` as
/// [`fill_entropy`] does, and when it can't, prints why and aborts the
/// process, since V8's fallbacks there are the clock alone.
fn fill_entropy_or_abort(buffer: &mut [u8]) -> bool {
    if fill_entropy(buffer) {
        return true;
    }
    eprintln!(
        "error: the operating system's random source failed, and in a confined process V8's \
         only other source is the clock, so the process stops"
    );
    std::process::abort();
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{EngineError, EngineOptions, init_engine};
    use crate::confine::{BoxRoots, Confined, Confinement, Jit};
    use crate::{RuntimeOptions, STACK_SIZE, run_main};

    /// How many times V8 has asked [`fill_entropy`](super::fill_entropy) in
    /// this process. Only V8 calls it, so a count above zero means V8 has the
    /// source.
    pub(super) static ENTROPY_CALLS: AtomicUsize = AtomicUsize::new(0);

    /// The unit tests share one process and the default engine, so whichever
    /// test sets it up first, a run here has V8 asking the source.
    #[test]
    fn v8_draws_its_entropy_from_the_source() {
        let engine = init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
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
        // the default sets no flag
        assert_eq!(EngineOptions::default().v8_flags(), "");
        // and the source fills, so V8 never falls back to its own seeding
        assert!(super::fill_entropy(&mut [0; 8]));
        // set up with the default options, so equal ones are accepted and
        // others refused
        assert!(init_engine(Confinement::Unconfined, &EngineOptions::default()).is_ok());
        let other = EngineOptions {
            locale: Some("en-US"),
            ..EngineOptions::default()
        };
        assert!(matches!(
            init_engine(Confinement::Unconfined, &other),
            Err(EngineError::Conflict(options)) if options == EngineOptions::default()
        ));
    }

    /// A box and an engine that disagree on the JIT are refused before
    /// anything is set up, either way round, whatever engine the process
    /// has: the check reads no state.
    #[test]
    fn a_box_for_another_jit_is_refused() {
        for (box_jit, jitless) in [(Jit::Off, false), (Jit::On, true)] {
            let confined = Confined::for_test(BoxRoots::for_test(Vec::new()), box_jit);
            let options = EngineOptions {
                jitless,
                ..EngineOptions::default()
            };
            let error = init_engine(Confinement::Confined(confined), &options).unwrap_err();
            assert!(
                matches!(error, EngineError::JitConflict { box_jit: found } if found == box_jit),
                "{error:?}"
            );
        }
        assert_eq!(Jit::from_jitless(true), Jit::Off);
        assert_eq!(Jit::from_jitless(false), Jit::On);
    }
}
