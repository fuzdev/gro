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
//!
//! So does eval's [`Deny`]: when it's on, the start installs
//! `determinism: deny` (the bootstrap's `deny`, which refuses the clock,
//! randomness, and the machine's facts, and deletes the timers, `Blob`,
//! and `File`), before
//! any user or driver code runs. One snapshot serves both: the snapshot
//! holds the function, and only the run's start calls it. Beneath it,
//! [`refuse_ops`] is the op middleware an eval runtime is built with: it
//! replaces each op in [`REFUSED_OPS`] with [`op_gro_refused`], which
//! throws, so a path past the bootstrap's refusals still finds none of
//! them.

use std::borrow::Cow;

use deno_core::error::JsError;
use deno_core::{
    Extension, ExtensionFileSource, JsRuntime, OpDecl, OpState, ascii_str, ascii_str_include, op2,
    v8,
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

/// Whether the run is an eval, which denies the clock, randomness, the
/// machine's facts, and timers: put in the runtime's `OpState` before the
/// bootstrap's start reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deny(pub bool);

/// Whether the bootstrap's start installs `determinism: deny`, as the run's
/// [`Deny`] says.
#[op2(fast)]
fn op_gro_deny(state: &OpState) -> bool {
    state.borrow::<Deny>().0
}

/// What [`op_gro_refused`] throws.
pub const REFUSED: &str = "a host operation eval refuses was called: eval has no timers, no \
                          clock or machine reads, no blobs, no code generation from strings, \
                          and no import past its loader";

/// The implementation [`refuse_ops`] puts in place of each op in
/// [`REFUSED_OPS`]: it throws an `Error` saying so, whatever it's passed.
/// Slow only (it takes the scope, which a fast call has none of), so no
/// fast call can skip the throw, as `OpDecl::disable`'s fast path does.
#[op2]
fn op_gro_refused(scope: &v8::PinScope<'_, '_>) {
    if let Some(message) = v8::String::new(scope, REFUSED) {
        let error = v8::Exception::error(scope, message);
        scope.throw_exception(error);
    }
}

/// The ops an eval runtime refuses, each `deno_core`'s or `deno_web`'s:
/// the timers (`setTimeout`'s and `AbortSignal.timeout`'s path), the
/// clock and memory reads, the ops that compile code past the
/// code-generation flag (`op_eval_context`, `op_compile_function`) or load
/// a module past the loader (`op_import_sync*`), and the blob ops, which
/// only `Blob`, `File`, and the object URLs call (eval deletes all of them;
/// a blob part's id is random). None is reachable from user code, which
/// sees no op; this is defence in depth. `op_time_origin` stays: the start
/// calls it, before any user code.
pub const REFUSED_OPS: [&str; 17] = [
    "op_timer_schedule",
    "op_timer_track",
    "op_timer_now",
    "op_now",
    "op_memory_usage",
    "op_eval_context",
    "op_compile_function",
    "op_import_sync",
    "op_import_sync_with_source",
    "op_blob_create_part",
    "op_blob_slice_part",
    "op_blob_read_part",
    "op_blob_remove_part",
    "op_blob_clone_part",
    "op_blob_create_object_url",
    "op_blob_revoke_object_url",
    "op_blob_from_object_url",
];

/// The op middleware of an eval runtime: an op in [`REFUSED_OPS`] gets
/// [`op_gro_refused`]'s implementation, keeping its name, and every other
/// op is kept as is.
pub fn refuse_ops(op: OpDecl) -> OpDecl {
    if REFUSED_OPS.contains(&op.name) {
        op.with_implementation_from(&op_gro_refused())
    } else {
        op
    }
}

deno_core::extension!(
    gro,
    deps = [deno_webidl, deno_web],
    ops = [
        op_gro_color,
        op_gro_console_stderr,
        op_gro_deny,
        op_gro_refused
    ],
    customizer = |extension: &mut Extension| {
        // the macro's own `esm` list reads files from the build tree at runtime
        extension.esm_files = Cow::Borrowed(BOOTSTRAP_ESM);
        extension.esm_entry_point = Some(BOOTSTRAP_SPECIFIER);
    },
);

/// The runtime's extensions, in dependency order: `deno_webidl`, `deno_web`, and
/// gro's bootstrap last, since it reads both. With `refuse`, the bootstrap's
/// extension carries [`refuse_ops`], which `deno_core` applies to every
/// op, its own included.
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
pub fn extensions(refuse: bool) -> Vec<Extension> {
    let mut extensions = declared_extensions();
    for extension in &mut extensions {
        extension.lazy_loaded_esm_files = Cow::Borrowed(&[]);
        extension.synthetic_esm_modules = Cow::Borrowed(&[]);
        if refuse && extension.name == "gro" {
            extension.middleware_fn = Some(Box::new(refuse_ops));
        }
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
/// or not: what a snapshot must not carry (the time origin, the console's
/// [`ConsoleOptions`] and the run's [`Deny`], which must be in the runtime's
/// `OpState` by now, and with it on, `determinism: deny`) and
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
