//! The globals a script ported from Node, Deno, or Bun commonly reaches
//! for that `fgro` has none of, told from a run's uncaught error, so an
//! embedder can say what stands for one.
//!
//! Kind: a review signal the run's code controls. A program prints one by
//! throwing a `ReferenceError` with V8's message for a free name, so it
//! says only what the uncaught error says, never that the program needs
//! the global; it reads nothing of the run's state but the uncaught
//! error's class and message, which the run reads already, and a caught
//! one, or a `TypeError` from `globalThis.process.env`, tells nothing.

/// A global `fgro` has none of, from the set a ported script commonly
/// reaches for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeGlobal {
    /// Node's `process`.
    Process,
    /// Deno's namespace.
    Deno,
    /// Bun's namespace.
    Bun,
    /// `require`, which a module of Node's older format reads.
    Require,
    /// `__dirname`, which a module of Node's older format reads.
    Dirname,
    /// `__filename`, which a module of Node's older format reads.
    Filename,
    /// Node's `global`.
    Global,
    /// Node's `Buffer`.
    Buffer,
    /// The web's `fetch`.
    Fetch,
    /// The web's `crypto`.
    Crypto,
}

impl FreeGlobal {
    /// Every one, in the order a reader meets them.
    pub const ALL: [Self; 10] = [
        Self::Process,
        Self::Deno,
        Self::Bun,
        Self::Require,
        Self::Dirname,
        Self::Filename,
        Self::Global,
        Self::Buffer,
        Self::Fetch,
        Self::Crypto,
    ];

    /// Its name, as a program writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Deno => "Deno",
            Self::Bun => "Bun",
            Self::Require => "require",
            Self::Dirname => "__dirname",
            Self::Filename => "__filename",
            Self::Global => "global",
            Self::Buffer => "Buffer",
            Self::Fetch => "fetch",
            Self::Crypto => "crypto",
        }
    }

    /// The one an uncaught error of the class named `class` with `message`
    /// says was free: a `ReferenceError` whose message is V8's for it,
    /// `<name> is not defined`, exactly.
    pub fn of_uncaught(class: &str, message: &str) -> Option<Self> {
        if class != "ReferenceError" {
            return None;
        }
        let name = message.strip_suffix(" is not defined")?;
        Self::ALL.into_iter().find(|global| global.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::FreeGlobal;

    /// Only a `ReferenceError` with V8's message for a name of the set
    /// tells one, exactly.
    #[test]
    fn a_free_global_is_told_by_v8_s_reference_error_alone() {
        for global in FreeGlobal::ALL {
            let message = format!("{} is not defined", global.name());
            assert_eq!(
                FreeGlobal::of_uncaught("ReferenceError", &message),
                Some(global)
            );
            assert_eq!(FreeGlobal::of_uncaught("TypeError", &message), None);
            assert_eq!(FreeGlobal::of_uncaught("Error", &message), None);
        }
        for message in [
            "processx is not defined",
            "xprocess is not defined",
            "process is not defined.",
            "process  is not defined",
            "Process is not defined",
            "navigator is not defined",
            "Cannot access 'process' before initialization",
            "process",
        ] {
            assert_eq!(
                FreeGlobal::of_uncaught("ReferenceError", message),
                None,
                "{message}"
            );
        }
    }
}
