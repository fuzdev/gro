//! The builtin table: the host modules a bare specifier names on the
//! runtime, each implemented inside the binary rather than found on disk.
//!
//! The table is the single source of the builtin mechanism. [`builtin_rows!`]
//! hands each row to a macro of its caller's, so this module holds only
//! gro's own types (the resolver stays free of `deno_core`'s) while the
//! bootstrap builds each builtin's backing script and synthetic module from
//! the same rows at compile time, and the loader reads [`BUILTINS`] for
//! each builtin's URL and grant.
//!
//! A builtin's URL is its specifier in the `builtin:` scheme, which the
//! resolver refuses from source like every scheme but `file:`
//! ([`ResolveErrorKind::UnsupportedScheme`](super::ResolveErrorKind::UnsupportedScheme)),
//! so only the bare specifier reaches one, and only through the table.

use std::fmt;

/// A host capability a builtin needs its importer to be granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// The process host module's.
    Process,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Process => "process",
        })
    }
}

/// A builtin module: a host module the runtime implements itself.
#[derive(Debug, PartialEq, Eq)]
pub struct Builtin {
    /// The bare specifier that names it, matched exactly.
    pub specifier: &'static str,
    /// Its URL, the specifier in the `builtin:` scheme: what a resolution
    /// returns and `import.meta.resolve` shows.
    pub url: &'static str,
    /// The grant its importer needs, or none for a grant-free module.
    pub grant: Option<Capability>,
    /// Its named exports, beside the `default` every builtin has.
    pub exports: &'static [&'static str],
}

/// The table's rows, each handed to `$row`, a macro taking
/// `($specifier:literal, $grant:expr, [$($export:literal),*])`, the results
/// in an array. A row's specifier is ASCII without quotes, since it's
/// spliced into the backing script's source.
macro_rules! builtin_rows {
    ($row:ident) => {
        [$row!(
            "@fuzdev/gro/process.ts",
            Some($crate::resolver::Capability::Process),
            ["process"]
        )]
    };
}
pub(crate) use builtin_rows;

/// The URL of the builtin named `$specifier`, a literal.
macro_rules! builtin_url {
    ($specifier:literal) => {
        concat!("builtin:", $specifier)
    };
}
pub(crate) use builtin_url;

/// A [`Builtin`] of a row.
macro_rules! builtin_row {
    ($specifier:literal, $grant:expr, [$($export:literal),*]) => {
        Builtin {
            specifier: $specifier,
            url: builtin_url!($specifier),
            grant: $grant,
            exports: &[$($export),*],
        }
    };
}

/// Every builtin, in the table's order.
pub const BUILTINS: &[Builtin] = &builtin_rows!(builtin_row);

/// The builtin `specifier` names, if any.
pub fn find_builtin(specifier: &str) -> Option<&'static Builtin> {
    BUILTINS
        .iter()
        .find(|builtin| builtin.specifier == specifier)
}
