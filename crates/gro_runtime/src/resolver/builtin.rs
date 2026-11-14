//! The builtin table: the host modules a bare specifier names on the
//! runtime, each implemented inside the binary rather than found on disk.
//!
//! The table is the single source of the builtin mechanism. [`builtin_rows!`]
//! hands each row to a macro of its caller's, so this module holds only
//! gro's own types (the resolver stays free of `deno_core`'s) while the
//! bootstrap embeds each builtin's implementation and names its backing
//! script and synthetic module from the same rows at compile time, and the
//! loader reads [`BUILTINS`] for each builtin's URL and grant.
//!
//! The table also reserves specifiers it doesn't serve ([`find_reserved`]):
//! the `.js` spelling of each row's specifier, the host modules not built
//! yet ([`UNBUILT_HOST_MODULES`]), the modules Gro has none of whose
//! members are host modules of their own ([`SPLIT_MODULES`]), and every
//! respelling of these or of a row's specifier that a package's `"exports"`
//! would turn into the same path. The resolver refuses one ahead of any
//! package lookup, so a host module's specifier never resolves to a
//! package, and none changes meaning when its builtin lands. A reserved
//! specifier is data only: it has no implementation, backing script, or
//! synthetic module.
//!
//! A builtin's URL is its specifier in the `builtin:` scheme, which the
//! resolver refuses from source like every scheme but `file:`
//! ([`ResolveErrorKind::UnsupportedScheme`](super::ResolveErrorKind::UnsupportedScheme)),
//! so only the bare specifier reaches one, and only through the table.

use std::fmt;

use super::specifier::to_plain_spelling;

/// A host capability a builtin needs its importer to be granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// The args host module's: the script's arguments.
    Args,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Args => "args",
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
    /// The grant its importer needs, or none for a grant-free module, which
    /// every importer of a run may import, the project's and a package's
    /// alike.
    pub grant: Option<Capability>,
    /// Its named exports, beside the `default` every builtin has.
    pub exports: &'static [&'static str],
}

/// The table's rows, each handed to `$row`, a macro taking
/// `($specifier:literal, $grant:expr, [$($export:literal),*])`, the results
/// in an array. A row's specifier is the path of its implementation under
/// `js/builtin/`, which the bootstrap embeds.
macro_rules! builtin_rows {
    ($row:ident) => {
        [
            $row!(
                "@fuzdev/gro/args.ts",
                Some($crate::resolver::Capability::Args),
                ["args"]
            ),
            $row!("@fuzdev/gro/cwd.ts", None, ["cwd"]),
            $row!("@fuzdev/gro/pid.ts", None, ["pid"]),
            $row!("@fuzdev/gro/runtime_version.ts", None, ["runtime_version"]),
        ]
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

/// The host modules not built as builtin modules yet, each by its specifier
/// without the extension, reserved under `.ts` and `.js`. A module joins
/// the table's rows when it's built, and leaves this list.
pub const UNBUILT_HOST_MODULES: &[&str] = &[
    "@fuzdev/gro/fs",
    "@fuzdev/gro/spawn",
    "@fuzdev/gro/net",
    "@fuzdev/gro/watch",
    "@fuzdev/gro/env",
    "@fuzdev/gro/exit",
    "@fuzdev/gro/stdin",
    "@fuzdev/gro/stdout",
    "@fuzdev/gro/stderr",
];

/// A module Gro has none of, whose members are modules of their own.
#[derive(Debug, PartialEq, Eq)]
pub struct SplitModule {
    /// Its specifier without the extension, reserved under `.ts` and `.js`.
    pub stem: &'static str,
    /// The modules that hold its members, each by its `.ts` specifier.
    pub modules: &'static [&'static str],
}

/// The modules Gro has none of whose members are modules of their own.
///
/// `process` is one host module per kind of authority, and its errors one
/// grant-free module per family. Each is reserved so it never resolves to
/// a package, its refusal naming the modules to import instead.
pub const SPLIT_MODULES: &[SplitModule] = &[
    SplitModule {
        stem: "@fuzdev/gro/process",
        modules: &[
            "@fuzdev/gro/env.ts",
            "@fuzdev/gro/args.ts",
            "@fuzdev/gro/exit.ts",
            "@fuzdev/gro/stdin.ts",
            "@fuzdev/gro/stdout.ts",
            "@fuzdev/gro/stderr.ts",
            "@fuzdev/gro/cwd.ts",
            "@fuzdev/gro/pid.ts",
            "@fuzdev/gro/runtime_version.ts",
        ],
    },
    SplitModule {
        stem: "@fuzdev/gro/process_error",
        modules: &["@fuzdev/gro/env_error.ts", "@fuzdev/gro/stdio_error.ts"],
    },
];

/// A specifier the table reserves without serving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reserved {
    /// The `.js` spelling of a builtin's specifier, which is its `.ts` one.
    JsSpelling(&'static Builtin),
    /// A builtin's specifier respelled: with a query or fragment,
    /// percent-encoding, or what else URL parsing drops, which a package's
    /// `"exports"` would turn into the same path.
    Respelled(&'static Builtin),
    /// A host module not built yet, by its specifier without the extension
    /// ([`UNBUILT_HOST_MODULES`]).
    Unbuilt(&'static str),
    /// A module Gro has none of, whose members are modules of their own
    /// ([`SPLIT_MODULES`]).
    Split(&'static SplitModule),
}

/// What the table reserves `specifier` as, if anything: a builtin's
/// specifier is no reserved one, since [`find_builtin`] names it.
///
/// A respelling of a builtin's specifier or a reserved one is reserved
/// too: a specifier whose subpath URL parsing and the path's decoding turn
/// into the subpath of one, the same two steps a package's `"exports"`
/// target takes, whatever the package's patterns.
pub fn find_reserved(specifier: &str) -> Option<Reserved> {
    // every reserved specifier is one of gro's, which most bare ones aren't
    if !specifier.starts_with("@fuzdev/gro/") {
        return None;
    }
    if let Some(reserved) = find_reserved_exact(specifier) {
        return Some(reserved);
    }
    let plain = to_plain_spelling(specifier)?;
    find_builtin(&plain)
        .map(Reserved::Respelled)
        .or_else(|| find_reserved_exact(&plain))
}

/// What the table reserves `specifier`, one of gro's, as, spelled exactly.
fn find_reserved_exact(specifier: &str) -> Option<Reserved> {
    if let Some(stem) = specifier.strip_suffix(".js")
        && let Some(builtin) = BUILTINS
            .iter()
            .find(|builtin| builtin.specifier.strip_suffix(".ts") == Some(stem))
    {
        return Some(Reserved::JsSpelling(builtin));
    }
    let stem = specifier
        .strip_suffix(".ts")
        .or_else(|| specifier.strip_suffix(".js"))?;
    UNBUILT_HOST_MODULES
        .iter()
        .find(|unbuilt| **unbuilt == stem)
        .map(|unbuilt| Reserved::Unbuilt(unbuilt))
        .or_else(|| {
            SPLIT_MODULES
                .iter()
                .find(|split| split.stem == stem)
                .map(Reserved::Split)
        })
}
