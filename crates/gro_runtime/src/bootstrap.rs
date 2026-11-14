//! gro's bootstrap extension: installs the pure web globals from `deno_web` and
//! hides the host's internals from user code.
//!
//! Its JS (`js/bootstrap.js`) is embedded in the binary at compile time. The
//! embedding (`ascii_str_include!`) asserts in a constant that the source is
//! 7-bit ASCII, so a non-ASCII character fails the build rather than reaching
//! V8, which reads extension sources as one-byte strings. So are the
//! builtins' implementations ([`IMPLEMENTATIONS`]), TypeScript, held to
//! ASCII the same way.
//!
//! **The builtin modules.** Each row of the resolver's builtin table
//! ([`crate::resolver::BUILTINS`]) is served as a `deno_core` synthetic
//! module (a facade) named by its `builtin:` URL, over a backing script that
//! is the builtin's implementation, `js/builtin/<specifier>`: one function,
//! called with `deno_core`'s bootstrap object (the ops, `primordials`) and
//! the scope of the importer's grant ([`to_backing_script`]). No snapshot
//! holds any of it. `deno_core` compiles and runs a backing script the first
//! time a run imports its facade, after the start, and the facade's exports
//! are the own properties of what the script returns, with a `default` of
//! `undefined`. Until then a builtin costs a run its facade's name and its
//! ops, and an eval, which imports none, never more.
//!
//! The scripts reach `deno_core` as its table of residual lazy scripts,
//! their types stripped and each wrapped as the function body `deno_core`
//! compiles: a snapshot's table carries them, made when the snapshot was
//! ([`crate::snapshot`]), so a start from one strips nothing, and a runtime
//! that starts without one reads [`source_backing_scripts`], stripped once a
//! process. Either way the text is the same, and nothing registers a script
//! per start.
//!
//! A first import can come after user code has patched the global
//! intrinsics, so an implementation, its top level included, names no global
//! and reads no property of an intrinsic: it takes each intrinsic from
//! `primordials` and each op from `core.ops`. Its own `tsconfig.json`
//! type-checks it with no global value and no member of an intrinsic
//! declared, and the tests run every row under patched intrinsics. The scope
//! is an argument, which the script's call passes ([`scope_argument`]) and
//! an implementation reads no other way; no grant scopes anything yet, and
//! a grant-free builtin has none, so each is called with `undefined`. An
//! implementation never throws: `deno_core` rejects the import with a
//! `TypeError` carrying its own report of what the script threw, never the
//! error, and runs the whole script again at the next import, whatever the
//! first run had done by then, so a builtin that can fail fails in what it
//! exports, as `cwd` does when called. The loader resolves an import to a
//! `builtin:` URL only for a bare specifier in the table and only for an
//! importer granted it (`import.meta.resolve` returns the URL to any
//! importer, and the URL itself imports nothing), and refuses one written
//! in source, so the facade is reached by the table's specifier alone;
//! `deno_core` dispatches a synthetic module by the raw specifier an import
//! writes, so a facade's name must be one user code can't resolve, which the
//! `builtin:` scheme is.
//!
//! `deno_webidl` and `deno_web`'s own sources aren't embedded: `deno_core` reads them
//! from their crates' paths on the build machine when they first load. A
//! startup snapshot carries the ones the bootstrap loads, and the binary that
//! starts from it embeds the rest ([`crate::snapshot`]); without one, the
//! runtime runs only from its build tree.
//!
//! The bootstrap also registers how to build the error of each `DOMException`
//! class an op can raise (`atob`'s `InvalidCharacterError` among them), which
//! `deno_core` builds from the class name the op gives; with no builder, such
//! an error reaches JS as `undefined`.
//!
//! **The web ops.** `deno_web` declares ops and classes for more of the web
//! than the bootstrap installs (the geometry and stylesheet classes,
//! `ImageData`, the broadcast channel, the lock manager). The runtime
//! registers the ones on [`WEB_OPS`] and [`WEB_CLASSES`], which a script it
//! can load reads, and drops the rest from the extension before the runtime
//! or the snapshot is made ([`restrict`]), so no start makes a function for
//! an op no script reads. `web_scripts` holds the two lists to the scripts.
//!
//! The console's choices are the run's, never the snapshot's: which stream
//! each level writes to and whether each stream is colored come from the
//! run's [`ConsoleOptions`], which the embedder puts in the runtime's
//! `OpState` and the bootstrap's start reads once. This module reads no
//! environment and asks no stream whether it's a terminal.
//!
//! So does eval's [`Determinism`]: denied, the start installs
//! `determinism: deny` (the bootstrap's `deny`, which refuses the clock, randomness, and the
//! machine's facts, and deletes the timers, `Blob`, and `File`), before any
//! user or driver code runs. One snapshot serves both: the snapshot holds the
//! function, and only the run's start calls it. Beneath it, [`refuse_ops`] is
//! the op middleware an eval runtime is built with: it replaces each op in
//! [`REFUSED_OPS`] with [`op_gro_refused`], which throws, so a path past the
//! bootstrap's refusals still finds none of them.

use std::borrow::Cow;
use std::sync::OnceLock;

use deno_core::error::JsError;
use deno_core::{
    Extension, ExtensionFileSource, JsRuntime, OpDecl, OpState, ascii_str, ascii_str_include, op2,
    v8,
};
use url::Url;

use crate::host::host_ops;
use crate::loader::strip::strip_module;
use crate::resolver::{Capability, builtin_rows, builtin_url};
use crate::{ConsoleOptions, ConsoleRouting};

/// The bootstrap module's specifier.
const BOOTSTRAP_SPECIFIER: &str = "ext:gro/bootstrap.js";

/// The bootstrap module, embedded: the extension's one module, which the
/// snapshot evaluates.
const BOOTSTRAP_ESM: &[ExtensionFileSource] = &[ExtensionFileSource::new(
    BOOTSTRAP_SPECIFIER,
    ascii_str_include!("../js/bootstrap.js"),
)];

/// A builtin's implementation, embedded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Implementation {
    /// Its backing script's specifier, which nothing resolves: `deno_core`
    /// names the script by it, and a stack frame shows it.
    pub script: &'static str,
    /// Its TypeScript source, `js/builtin/<specifier>`: one function.
    pub source: &'static str,
    /// The grant its importer needs, as its row has it.
    pub grant: Option<Capability>,
}

/// The specifier of the backing script of the builtin named `$specifier`, a
/// literal.
macro_rules! backing_specifier {
    ($specifier:literal) => {
        concat!("ext:gro/builtin/", $specifier)
    };
}

/// A builtin's [`Implementation`]: the file its specifier names under
/// `js/builtin/`, which fails the build when it isn't 7-bit ASCII.
macro_rules! implementation {
    ($specifier:literal, $grant:expr, [$($export:literal),*]) => {{
        const SOURCE: &str = include_str!(concat!("../js/builtin/", $specifier));
        const _: () = assert!(
            SOURCE.is_ascii(),
            concat!("js/builtin/", $specifier, " is not 7-bit ASCII")
        );
        Implementation {
            script: backing_specifier!($specifier),
            source: SOURCE,
            grant: $grant,
        }
    }};
}

/// A builtin's synthetic module: its URL, over its backing script.
macro_rules! synthetic_module {
    ($specifier:literal, $grant:expr, [$($export:literal),*]) => {
        (builtin_url!($specifier), backing_specifier!($specifier))
    };
}

/// Each builtin's implementation, in the table's order.
pub const IMPLEMENTATIONS: &[Implementation] = &builtin_rows!(implementation);

/// Each builtin's synthetic module, `(name, backing script)`.
const BUILTIN_MODULES: &[(&str, &str)] = &builtin_rows!(synthetic_module);

/// What a backing script passes its implementation as the scope of the
/// importer's grant, a JS expression over the script's `__bootstrap`: the
/// one place a builtin's scope comes from. No grant scopes anything yet, so
/// every implementation is called with `undefined`. A grant that carries a
/// scope names here how the host hands it over, a native value the
/// implementation gives back to each op, so an op decides from the scope it
/// was called with and never from the run's grants.
const fn scope_argument(grant: Option<Capability>) -> &'static str {
    match grant {
        // a grant-free module has no grant to scope, and `args` scopes nothing
        None | Some(Capability::Args) => "undefined",
    }
}

/// How many implementations this process has stripped, for the tests that
/// hold a start from a snapshot to none.
#[cfg(test)]
pub static STRIPPED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What `deno_core` wraps a lazy script in to compile it as a function's
/// body, and takes a script that starts with [`WRAP_OPEN`] to be wrapped
/// in already, compiling it as it is, an external string V8 never copies.
/// A test holds the two to `deno_core`'s own wrapping.
const WRAP_OPEN: &str = "\"use strict\"; return (";

/// The end of the wrapping [`WRAP_OPEN`] starts.
const WRAP_CLOSE: &str = ");";

/// A builtin's backing script, the text `deno_core` compiles as a function
/// of `__bootstrap` on the builtin's first import: `implementation`'s
/// source stripped of its types, called with the bootstrap object and its
/// grant's scope ([`scope_argument`]), and wrapped as `deno_core` wraps a
/// lazy script. The comments above the function are emptied line by line
/// and the strip keeps every other character where it was, so a position
/// in the script, as a stack frame shows it, is the source's.
///
/// # Errors
///
/// Returns the strip's refusal, that the source isn't comments and then
/// one function, or that the script isn't 7-bit ASCII, which `deno_core`
/// takes on trust of its table.
pub fn to_backing_script(implementation: &Implementation) -> Result<String, String> {
    #[cfg(test)]
    STRIPPED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let script = implementation.script;
    let url = Url::parse(script).map_err(|error| format!("{script}: {error}"))?;
    let stripped = strip_module(implementation.source, &url).map_err(|refusal| refusal.message)?;
    let is_above = |line: &&str| line.trim().is_empty() || line.trim_start().starts_with("//");
    let above = stripped.lines().take_while(is_above).count();
    let function: Vec<&str> = stripped.lines().skip(above).collect();
    if !function
        .first()
        .is_some_and(|line| line.starts_with("(function "))
    {
        return Err(format!(
            "{script} is not one function, `(function (bootstrap, scope) {{ ... }})`, under \
             comments"
        ));
    }
    let function = function.join("\n");
    // the source is one expression statement
    let function = function.trim_end_matches(|c: char| c.is_whitespace() || c == ';');
    let backing = format!(
        "{WRAP_OPEN}{}{function}(__bootstrap, {}){WRAP_CLOSE}",
        "\n".repeat(above),
        scope_argument(implementation.grant)
    );
    if backing.is_ascii() {
        Ok(backing)
    } else {
        Err(format!("{script} is not 7-bit ASCII"))
    }
}

/// Each builtin's backing script, by its specifier, sorted by it as
/// `deno_core`'s table of residual lazy scripts must be.
///
/// # Errors
///
/// Returns why an implementation made no script ([`to_backing_script`]).
pub fn backing_scripts() -> Result<Vec<(&'static str, String)>, String> {
    let mut scripts = IMPLEMENTATIONS
        .iter()
        .map(|implementation| Ok((implementation.script, to_backing_script(implementation)?)))
        .collect::<Result<Vec<_>, String>>()?;
    scripts.sort_unstable_by_key(|(specifier, _)| *specifier);
    Ok(scripts)
}

/// The backing scripts as a runtime that starts without a snapshot gives
/// them to `deno_core`, its table of residual lazy scripts: made the first
/// time the process starts such a runtime, and kept for the process's life,
/// since the table is `'static`. A start from a snapshot never calls this:
/// its table came with the snapshot.
///
/// # Errors
///
/// Returns why an implementation made no script, at each call.
pub fn source_backing_scripts() -> Result<&'static [(&'static str, &'static str)], String> {
    type Table = Box<[(&'static str, &'static str)]>;
    static SCRIPTS: OnceLock<Result<Table, String>> = OnceLock::new();
    SCRIPTS
        .get_or_init(|| {
            let scripts = backing_scripts()?.into_iter();
            Ok(scripts
                .map(|(specifier, script)| (specifier, &*script.leak()))
                .collect())
        })
        .as_deref()
        .map_err(Clone::clone)
}

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

/// Whether the run may read the clock, randomness, and the machine's facts,
/// and set timers: put in the runtime's `OpState` before the bootstrap's
/// start reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Determinism {
    /// `fgro run`'s: all of them, as the web platform has them.
    Allowed,
    /// Eval's: `determinism: deny` installed, refusing them all.
    Denied,
}

/// Whether the bootstrap's start installs `determinism: deny`, as the run's
/// [`Determinism`] says.
#[op2(fast)]
fn op_gro_deny(state: &OpState) -> bool {
    *state.borrow::<Determinism>() == Determinism::Denied
}

/// The ops of gro's extension that make no check: the console's and
/// eval's, which read only the run's own settings or refuse. Every other op
/// the extension registers, whatever its name, is a host op, a grant-free
/// module's included, declared through the choke point (`crate::host`); a
/// test fails on one that is neither.
#[cfg(test)]
pub const UNGATED_OPS: [&str; 4] = [
    "op_gro_color",
    "op_gro_console_stderr",
    "op_gro_deny",
    "op_gro_refused",
];

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
/// calls it, before any user code. A snapshot is made only of a runtime
/// where each one's function is at an address the snapshot can tell from
/// those of the ops eval keeps (`crate::snapshot`).
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

/// The ops of `deno_web` the runtime registers, in the order `deno_web`
/// declares them: every one a script the runtime can load reads from
/// `core.ops`, and no other, which the tests of `web_scripts` hold by
/// reading the scripts. [`restrict`] drops every other op the web extensions
/// declare. An op [`REFUSED_OPS`] names stays on this list while a script
/// reads it, so eval refuses it by name; a test fails on a refused name the
/// runtime doesn't register.
pub const WEB_OPS: [&str; 56] = [
    "op_base64_decode",
    "op_base64_encode_from_buffer",
    "op_base64_atob",
    "op_base64_btoa",
    "op_encoding_normalize_label",
    "op_encoding_decode_single",
    "op_encoding_decode_utf8",
    "op_encoding_decode_utf8_ascii_only",
    "op_encoding_new_decoder",
    "op_encoding_decode",
    "op_encoding_encode_into",
    "op_encoding_encode_into_fallback",
    "op_blob_create_part",
    "op_blob_slice_part",
    "op_blob_read_part",
    "op_blob_remove_part",
    "op_blob_clone_part",
    "op_blob_create_object_url",
    "op_blob_revoke_object_url",
    "op_blob_from_object_url",
    "op_message_port_create_entangled",
    "op_message_port_post_message",
    "op_message_port_post_message_raw",
    "op_message_port_recv_message",
    "op_message_port_recv_message_sync",
    "op_compression_new",
    "op_compression_write",
    "op_compression_finish",
    "op_now",
    "op_time_origin",
    "op_defer",
    "op_readable_stream_resource_allocate",
    "op_readable_stream_resource_allocate_sized",
    "op_readable_stream_resource_get_sink",
    "op_readable_stream_resource_write_error",
    "op_readable_stream_resource_write_buf",
    "op_readable_stream_resource_write_sync",
    "op_readable_stream_resource_close",
    "op_readable_stream_resource_await_close",
    "op_url_reparse",
    "op_url_parse",
    "op_url_get_serialization",
    "op_url_parse_with_base",
    "op_url_parse_search_params",
    "op_url_stringify_search_params",
    "op_urlpattern_parse",
    "op_urlpattern_process_match_input",
    "op_console_inspect",
    "op_console_inspect_args",
    "op_console_format_value",
    "op_console_quote_string",
    "op_console_parse_css",
    "op_console_parse_css_color",
    "op_console_css_to_ansi",
    "op_console_get_string_width",
    "op_console_strip_vt",
];

/// The classes of `deno_web` the runtime registers, each a constructor and
/// its methods' ops, in the order `deno_web` declares them: the ones a
/// script the runtime can load reads from `core.ops`, as [`WEB_OPS`] are.
/// [`restrict`] drops every other class the web extensions declare, so the
/// runtime has no constructor for one and no op that makes one.
pub const WEB_CLASSES: [&str; 1] = ["Console"];

/// The op middleware of an eval runtime: an op in [`REFUSED_OPS`] gets
/// [`op_gro_refused`]'s implementation, keeping its name, and every other
/// op is kept as is.
fn refuse_ops(op: OpDecl) -> OpDecl {
    if REFUSED_OPS.contains(&op.name) {
        op.with_implementation_from(&op_gro_refused())
    } else {
        op
    }
}

deno_core::extension!(
    gro,
    deps = [deno_webidl, deno_web],
    ops_fn = host_ops,
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
        extension.synthetic_esm_modules = Cow::Borrowed(BUILTIN_MODULES);
    },
);

/// Whether a runtime's ops run as their extensions declare them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ops {
    /// Every op as declared: `fgro run`'s, and the snapshot's.
    Kept,
    /// The ops in [`REFUSED_OPS`] refused ([`refuse_ops`]): eval's.
    Refused,
}

/// The runtime's extensions, in dependency order: `deno_webidl`, `deno_web`, and
/// gro's bootstrap last, since it reads both. With [`Ops::Refused`], the
/// bootstrap's extension carries [`refuse_ops`], which `deno_core` applies
/// to every op, its own included; the list is the same either way.
///
/// The web extensions register the ops on [`WEB_OPS`] and the classes on
/// [`WEB_CLASSES`], and no other they declare ([`restrict`]).
///
/// Each extension's lazy ES modules (`deno_web`'s `locks.js` and
/// `webtransport.js`) and every other extension's synthetic ES modules (none
/// today) are dropped from its declaration too, so the runtime
/// knows gro's builtins and no other. `deno_core` resolves an import its
/// module loader refuses to the lazy ES module of that exact name when it
/// knows one, whoever the importer, and that module resolves its own `ext:`
/// imports as internal; and it links a static import naming a synthetic ES
/// module by that name, whatever module the loader resolved the name to, and
/// an `import()` the loader resolves to one. Either way an `import` could
/// reach a module past the loader's refusal of every `ext:` specifier and
/// its grant check. gro installs no global from them, so nothing else
/// changes.
pub fn extensions(ops: Ops) -> Vec<Extension> {
    restrict(declared_extensions(), ops)
}

/// `extensions` with each one's lazy ES modules dropped; unless it is gro's,
/// its synthetic ES modules, its ops but those on [`WEB_OPS`], and its
/// classes but those on [`WEB_CLASSES`]; and gro's carrying [`refuse_ops`]
/// when `ops` says.
fn restrict(mut extensions: Vec<Extension>, ops: Ops) -> Vec<Extension> {
    for extension in &mut extensions {
        extension.lazy_loaded_esm_files = Cow::Borrowed(&[]);
        if extension.name != "gro" {
            extension.synthetic_esm_modules = Cow::Borrowed(&[]);
            extension.ops = Cow::Owned(kept(&extension.ops, &WEB_OPS, |op| op.name));
            extension.objects =
                Cow::Owned(kept(&extension.objects, &WEB_CLASSES, |class| class.name.0));
        }
        if ops == Ops::Refused && extension.name == "gro" {
            extension.middleware_fn = Some(Box::new(refuse_ops));
        }
    }
    extensions
}

/// The `declared` items whose names `names` lists, in one pass over both:
/// `names` is in the order the items are declared in, and a name out of
/// that order, or one no item has, keeps no item and neither does any name
/// after it, which the tests of the registered ops catch.
fn kept<T: Copy>(declared: &[T], names: &[&str], name: impl Fn(&T) -> &'static str) -> Vec<T> {
    let mut names = names.iter().peekable();
    declared
        .iter()
        .filter(|item| names.next_if(|kept| **kept == name(item)).is_some())
        .copied()
        .collect()
}

/// The extensions as their crates declare them, before [`extensions`] drops
/// the modules user code could reach and the ops no script reads.
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
/// [`ConsoleOptions`] and the run's [`Determinism`], which must be in the
/// runtime's `OpState` by now, and denied, `determinism: deny`) and
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

#[cfg(test)]
pub mod web_scripts;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver::BUILTINS;

    /// Every synthetic module the runtime knows, `(name, backing script)`,
    /// across its extensions.
    fn synthetic_modules(extensions: &[Extension]) -> Vec<(&'static str, &'static str)> {
        extensions
            .iter()
            .flat_map(|extension| extension.synthetic_esm_modules.iter().copied())
            .collect()
    }

    /// The synthetic modules the runtime keeps are exactly the builtin
    /// table's, each named by its `builtin:` URL, a scheme the resolver
    /// refuses from source, since `deno_core` links a static import naming
    /// one by that raw name; each over the backing script of its row's
    /// implementation, in the table's order; and no extension keeps a lazy
    /// ES module, nor gro's a lazy script, which `deno_core` would register
    /// at every start.
    #[test]
    fn the_kept_synthetic_modules_are_the_builtin_table_s() {
        for ops in [Ops::Kept, Ops::Refused] {
            let extensions = extensions(ops);
            let modules = synthetic_modules(&extensions);
            let names: Vec<&str> = modules.iter().map(|(name, _)| *name).collect();
            let urls: Vec<&str> = BUILTINS.iter().map(|builtin| builtin.url).collect();
            assert_eq!(names, urls);
            for name in names {
                assert!(name.starts_with("builtin:"), "{name}");
            }
            let scripts: Vec<&str> = modules.iter().map(|(_, script)| *script).collect();
            let implemented: Vec<&str> = IMPLEMENTATIONS
                .iter()
                .map(|implementation| implementation.script)
                .collect();
            assert_eq!(scripts, implemented);
            for extension in &extensions {
                assert!(
                    extension.lazy_loaded_esm_files.is_empty(),
                    "{}",
                    extension.name
                );
                assert_eq!(
                    extension.lazy_loaded_js_files.is_empty(),
                    extension.name == "gro",
                    "{}",
                    extension.name
                );
            }
        }
    }

    /// The implementations are the builtin table's rows, in its order: each
    /// row's backing script is named for its specifier in a scheme the
    /// resolver refuses from source, and carries the row's grant.
    #[test]
    fn the_implementations_are_the_builtin_table_s() {
        let rows: Vec<(String, Option<Capability>)> = BUILTINS
            .iter()
            .map(|builtin| {
                (
                    format!("ext:gro/builtin/{}", builtin.specifier),
                    builtin.grant,
                )
            })
            .collect();
        let implemented: Vec<(String, Option<Capability>)> = IMPLEMENTATIONS
            .iter()
            .map(|implementation| (implementation.script.to_owned(), implementation.grant))
            .collect();
        assert_eq!(implemented, rows);
    }

    /// The backing scripts are one a row, sorted by specifier as
    /// `deno_core`'s table must be, each wrapped, so `deno_core` compiles it
    /// as it is, with its function on the line the source has it on, under
    /// the comments; and the table a source start reads is the same
    /// scripts, made once.
    #[test]
    fn the_backing_scripts_are_sorted_and_wrapped() {
        let scripts = backing_scripts().unwrap();
        assert_eq!(scripts.len(), BUILTINS.len());
        assert!(scripts.windows(2).all(|pair| pair[0].0 < pair[1].0));
        let function_line = |text: &str| {
            text.lines()
                .position(|line| line.starts_with("(function (bootstrap"))
        };
        for (specifier, script) in &scripts {
            let implementation = IMPLEMENTATIONS
                .iter()
                .find(|implementation| implementation.script == *specifier)
                .unwrap();
            assert!(script.starts_with(WRAP_OPEN), "{specifier}: {script}");
            let line = function_line(script);
            assert!(line > Some(0), "{specifier}: {script}");
            assert_eq!(line, function_line(implementation.source), "{specifier}");
        }
        let table = source_backing_scripts().unwrap();
        let again = source_backing_scripts().unwrap();
        assert!(std::ptr::eq(table, again));
        let kept: Vec<(&str, &str)> = scripts
            .iter()
            .map(|(specifier, script)| (*specifier, script.as_str()))
            .collect();
        assert_eq!(table, kept);
    }

    /// Another extension's synthetic and lazy ES modules are dropped, and
    /// its ops but the kept ones, gro's synthetic modules and ops kept.
    #[test]
    fn another_extension_s_reachable_modules_and_unlisted_ops_are_dropped() {
        const FOREIGN_LAZY: &[ExtensionFileSource] = &[ExtensionFileSource::new(
            "ext:foreign/lazy.js",
            ascii_str!("export {};"),
        )];
        let foreign = Extension {
            name: "foreign",
            synthetic_esm_modules: Cow::Borrowed(&[("builtin:foreign", "ext:foreign/x.js")]),
            lazy_loaded_esm_files: Cow::Borrowed(FOREIGN_LAZY),
            // an op of gro's stands in for an unlisted one
            ops: Cow::Owned(vec![op_gro_deny()]),
            ..Extension::default()
        };
        let restricted = restrict(vec![foreign, gro::init()], Ops::Kept);
        assert!(restricted[0].synthetic_esm_modules.is_empty());
        assert!(restricted[0].lazy_loaded_esm_files.is_empty());
        assert!(restricted[0].ops.is_empty());
        assert_eq!(restricted[1].synthetic_esm_modules, BUILTIN_MODULES);
        assert_eq!(restricted[1].ops.len(), gro::init().ops.len());
    }

    /// The web extensions register exactly the kept ops and classes, in
    /// their order, whichever way the ops run, and each kept name is one
    /// they declare: a name [`kept`] passes over, misspelled or out of the
    /// declared order, is missing here.
    #[test]
    fn the_web_extensions_register_the_kept_ops_and_classes() {
        for ops in [Ops::Kept, Ops::Refused] {
            let extensions = extensions(ops);
            let web = || {
                extensions
                    .iter()
                    .filter(|extension| extension.name != "gro")
            };
            let registered: Vec<&str> = web()
                .flat_map(|extension| extension.ops.iter().map(|op| op.name))
                .collect();
            assert_eq!(registered, WEB_OPS);
            let classes: Vec<&str> = web()
                .flat_map(|extension| extension.objects.iter().map(|class| class.name.0))
                .collect();
            assert_eq!(classes, WEB_CLASSES);
        }
    }

    /// An implementation standing in for a row's, to see what its backing
    /// script does with it.
    fn probe(source: &'static str) -> Implementation {
        Implementation {
            script: "ext:gro/builtin/probe.ts",
            source,
            grant: None,
        }
    }

    /// A backing script is its implementation without its types or the
    /// statement's end, called once with `__bootstrap` and the scope, each
    /// comment line above the function emptied, so every line below is
    /// where the source has it; an implementation that doesn't strip, isn't
    /// one function under comments, or isn't ASCII makes none.
    #[test]
    fn a_backing_script_calls_its_stripped_implementation() {
        let script = to_backing_script(&probe(
            "// a comment\n\n// another\n(function (bootstrap: Bootstrap, _scope: undefined) \
             {\n\treturn bootstrap;\n});\n",
        ))
        .unwrap();
        assert_eq!(
            script,
            "\"use strict\"; return (\n\n\n(function (bootstrap           , _scope           ) \
             {\n\treturn bootstrap;\n})(__bootstrap, undefined));"
        );
        let refused = to_backing_script(&probe("(function () {\n\tenum E { A }\n});\n"));
        assert!(refused.unwrap_err().contains("ext:gro/builtin/probe.ts:2:"));
        for other in [
            "const a = 1;\n(function () {});\n",
            "/* a block comment */\n(function () {});\n",
            "(() => 1);\n",
            "// only a comment\n",
        ] {
            let refused = to_backing_script(&probe(other)).unwrap_err();
            assert!(
                refused.contains("is not one function"),
                "{other}: {refused}"
            );
        }
        let unicode = to_backing_script(&probe("(function () {\n\treturn '\u{e9}';\n});\n"));
        assert_eq!(
            unicode.unwrap_err(),
            "ext:gro/builtin/probe.ts is not 7-bit ASCII"
        );
    }

    /// The wrapping a backing script is made with is `deno_core`'s own of a
    /// lazy script, so `deno_core` compiles a backing script as it is and
    /// never wraps one again.
    #[test]
    fn a_backing_script_is_wrapped_as_deno_core_wraps_a_lazy_script() {
        let function = "(function () {})()";
        assert_eq!(
            deno_core::wrap_lazy_ext_script(function),
            format!("{WRAP_OPEN}{function}{WRAP_CLOSE}")
        );
    }
}
