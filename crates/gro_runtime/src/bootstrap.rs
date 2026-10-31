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
//!
//! The console's choices are the run's, never the snapshot's: which stream
//! each level writes to and whether each stream is colored come from the
//! run's [`ConsoleOptions`], which the embedder puts in the runtime's
//! `OpState` and the bootstrap's start reads once. This module reads no
//! environment and asks no stream whether it's a terminal.

use std::borrow::Cow;

use deno_core::error::JsError;
use deno_core::{
    Extension, ExtensionFileSource, JsRuntime, OpState, ascii_str, ascii_str_include, op2,
};

use crate::{ConsoleOptions, ConsoleRouting};

/// The bootstrap module's specifier.
const BOOTSTRAP_SPECIFIER: &str = "ext:gro/bootstrap.js";

/// The bootstrap module, embedded.
const BOOTSTRAP_ESM: &[ExtensionFileSource] = &[ExtensionFileSource::new(
    BOOTSTRAP_SPECIFIER,
    ascii_str_include!("../js/bootstrap.js"),
)];

/// Whether the console colors its output to stderr, or with `stderr` false
/// to stdout, as the run's [`ConsoleOptions`] say. Asked when the run starts.
#[op2(fast)]
fn op_gro_color(state: &OpState, stderr: bool) -> bool {
    let console = state.borrow::<ConsoleOptions>();
    if stderr {
        console.color_stderr
    } else {
        console.color_stdout
    }
}

/// Whether the console writes every level to stderr, as the run's
/// [`ConsoleOptions`] say. Asked when the run starts.
#[op2(fast)]
fn op_gro_console_stderr(state: &OpState) -> bool {
    state.borrow::<ConsoleOptions>().routing == ConsoleRouting::Stderr
}

deno_core::extension!(
    gro,
    deps = [deno_webidl, deno_web],
    ops = [op_gro_color, op_gro_console_stderr],
    customizer = |extension: &mut Extension| {
        // the macro's own `esm` list reads files from the build tree at runtime
        extension.esm_files = Cow::Borrowed(BOOTSTRAP_ESM);
        extension.esm_entry_point = Some(BOOTSTRAP_SPECIFIER);
    },
);

/// The runtime's extensions, in dependency order: `deno_webidl`, `deno_web`, and
/// gro's bootstrap last, since it reads both.
///
/// Each extension's lazy ES modules (`deno_web`'s `locks.js` and
/// `webtransport.js`) and synthetic ES modules (none today) are dropped from
/// its declaration, so the runtime knows none. `deno_core` resolves an import
/// its module loader refuses to the lazy ES module of that exact name when it
/// knows one, whoever the importer, and that module resolves its own `ext:`
/// imports as internal; it dispatches an import naming a synthetic ES module
/// before asking the loader at all. Either way a user module's `import` would
/// reach the module past the loader's refusal of every `ext:` specifier. gro
/// installs no global from them, so nothing else changes.
pub fn extensions() -> Vec<Extension> {
    let mut extensions = declared_extensions();
    for extension in &mut extensions {
        extension.lazy_loaded_esm_files = Cow::Borrowed(&[]);
        extension.synthetic_esm_modules = Cow::Borrowed(&[]);
    }
    extensions
}

/// The extensions as their crates declare them, before [`extensions`] drops
/// the modules user code could reach.
pub fn declared_extensions() -> Vec<Extension> {
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

/// Runs the bootstrap's per-run steps on a started runtime, from a snapshot
/// or not: what a snapshot must not carry (the time origin, and the console's
/// [`ConsoleOptions`], which must be in the runtime's `OpState` by now) and
/// what `deno_core` still reads while it starts from one (`Deno.core`, removed
/// here with `__bootstrap`).
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
