//! gro's bootstrap extension: installs the pure web globals from `deno_web` and
//! hides the host's internals from user code.
//!
//! Its JS (`js/bootstrap.js`) is embedded in the binary at compile time. The
//! embedding (`ascii_str_include!`) asserts in a constant that the source is
//! 7-bit ASCII, so a non-ASCII character fails the build rather than reaching
//! V8, which reads extension sources as one-byte strings.
//!
//! **The builtin modules.** Each row of the resolver's builtin table
//! ([`crate::resolver::BUILTINS`]) is served as a `deno_core` synthetic
//! module (a facade) named by its `builtin:` URL, over a backing script
//! named for its specifier ([`BACKING_SPECIFIERS`]), which the row's own
//! data makes (`template.rs`): one function in two levels, the outer called
//! with `deno_core`'s bootstrap object, taking the one op, and the realm's
//! `Error` in a row that hands it over, and returning the inner, which,
//! called with the scope of an importer's grant, asks the host for the
//! builtin's namespace, whose members are native (`host/native.rs`). A
//! backing script calls both at once, so a run has one namespace of each
//! builtin; a namespace per importing origin is one the host keys by the
//! origin's scope. Whatever has identity or state, a member, a class, a
//! brand, a registry, is the host's, and no row has JS of its own. No
//! snapshot holds any of it. `deno_core` compiles and runs a backing script
//! the first time a run imports its facade, after the start, and the
//! facade's exports are the own properties of what the script returns,
//! with a `default` of `undefined`. Until then a builtin costs a run its
//! facade's name, and an eval, which imports none, never more.
//!
//! The scripts reach `deno_core` as its table of residual lazy scripts,
//! each wrapped as the function body `deno_core` compiles: a snapshot's
//! table carries them, made when the snapshot was ([`crate::snapshot`]),
//! and a runtime that starts without one reads [`source_backing_scripts`],
//! made once a process. Either way the text is the same, and nothing
//! registers a script per start. No extension declares a backing script
//! among its lazy scripts, so a module run while a snapshot is made can't
//! import a builtin: the making fails at the import, as a test in
//! `process_tests.rs` holds.
//!
//! **A first import runs on the importer's stack.** `deno_core` builds a
//! facade the first time an `import()` names it, inside that call, so the
//! backing script runs on the importer's stack, and an importer that left it
//! too little, a package's module included for a grant-free builtin, would
//! cut it short, with `deno_core`'s report of the failure made on that stack
//! too. So the first `import()` of a builtin with less than
//! [`BUILTIN_IMPORT_STACK`](crate::loader::headroom::BUILTIN_IMPORT_STACK)
//! left has its module built from the event loop, with nearly the whole
//! stack, and a backing script throws nothing at load: nothing known ends
//! a script early, since an op a level calls throws only for want of stack
//! and a run never terminates V8 (eval's limits do, but eval imports no
//! builtin). That holds by measurement, not by a bound: the amount is what
//! each build's sweep holds, and off glibc, where the stack start
//! isn't read, nothing is deferred, so a deep first import can be cut short
//! there. Should anything end a script early, `deno_core` rejects that
//! import with a `TypeError` carrying its own report of what the script
//! threw, never the error, keeps nothing of the run, and runs the whole
//! script again at the next import. So both levels of a backing script are
//! kept safe to end at any call and to run again, and a cause found later
//! costs a rerun, never a half-made module or an effect done twice:
//!
//! - Neither stores anything outside its own bindings: no property on the
//!   bootstrap object or on anything it holds. The namespace the inner
//!   level returns is the host's, held only once it's whole, and
//!   `deno_core` keeps the module once both levels have returned, so a
//!   half-made one is never published and a second run starts from
//!   nothing.
//! - State that is the run's and outlives a script, an ordered writer, a
//!   table of handles, an exit code, is the host's, in `OpState`, made by
//!   the host whole: a script reaches it through the namespace op, and what
//!   one builtin must recognize of another's (a handle) its members read
//!   natively.
//! - The op either level calls can be called again: it's a get-or-make. An
//!   effect that mustn't repeat is a member's, called by a program, never
//!   by a level.
//!
//! A builtin that can fail for a reason of the run's fails in what it
//! exports, as `cwd` does when called, never at load. `deno_core` builds one
//! module of a builtin however its loads interleave (gro's patch of it). The
//! tests start every row's first import a frame apart up from the stack's
//! limit and hold every import to one whole module; that an op is safe to
//! repeat, nothing checks.
//!
//! The loader resolves an import to a `builtin:` URL only for a bare
//! specifier in the table and only for an importer granted it
//! (`import.meta.resolve` returns the URL to any importer, and the URL
//! itself imports nothing), and refuses one written in source, so the
//! facade is reached by the table's specifier alone; `deno_core` dispatches
//! a synthetic module by the raw specifier an import writes, so a facade's
//! name must be one user code can't resolve, which the `builtin:` scheme is.
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
//! `OpState`: the bootstrap's start reads the colors once, and the console
//! writes through [`op_gro_print`], which reads the routing at each write
//! and drops a message whose reader has left, so a run piped to `head`
//! goes on to its own exit code. This module reads no environment and asks
//! no stream whether it's a terminal.
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
use deno_core::error::ModuleLoaderError as JsErrorBox;
use deno_core::{
    Extension, ExtensionFileSource, JsRuntime, OpDecl, OpState, ascii_str, ascii_str_include, op2,
    v8,
};

use crate::host::host_ops;
use crate::host::stdio;
use crate::resolver::{BUILTINS, builtin_rows, builtin_url};
use crate::{ConsoleOptions, ConsoleRouting};

/// The bootstrap module's specifier.
const BOOTSTRAP_SPECIFIER: &str = "ext:gro/bootstrap.js";

/// The bootstrap module, embedded: the extension's one module, which the
/// snapshot evaluates.
const BOOTSTRAP_ESM: &[ExtensionFileSource] = &[ExtensionFileSource::new(
    BOOTSTRAP_SPECIFIER,
    ascii_str_include!("../js/bootstrap.js"),
)];

/// The specifier of the backing script of the builtin named `$specifier`, a
/// literal.
macro_rules! backing_specifier {
    ($specifier:literal) => {
        concat!("ext:gro/builtin/", $specifier)
    };
}

/// A row's backing specifier.
macro_rules! backing_specifier_of_row {
    ($specifier:literal, $grant:expr, [$($export:literal),*]) => {
        backing_specifier!($specifier)
    };
}

/// A builtin's synthetic module: its URL, over its backing script.
macro_rules! synthetic_module {
    ($specifier:literal, $grant:expr, [$($export:literal),*]) => {
        (builtin_url!($specifier), backing_specifier!($specifier))
    };
}

/// Each row's backing specifier, in the table's order: the name
/// `deno_core` compiles the row's backing script under, which nothing
/// resolves and a stack frame shows, and the one the namespace op knows the
/// row's script by.
pub const BACKING_SPECIFIERS: &[&str] = &builtin_rows!(backing_specifier_of_row);

/// Each builtin's synthetic module, `(name, backing script)`.
const BUILTIN_MODULES: &[(&str, &str)] = &builtin_rows!(synthetic_module);

/// Each builtin's backing script ([`template::backing_script`]), by its
/// specifier, sorted by it as `deno_core`'s table of residual lazy scripts
/// must be.
pub fn backing_scripts() -> Vec<(&'static str, String)> {
    let mut scripts: Vec<(&'static str, String)> = BACKING_SPECIFIERS
        .iter()
        .zip(BUILTINS)
        .map(|(specifier, row)| (*specifier, template::backing_script(row)))
        .collect();
    scripts.sort_unstable_by_key(|(specifier, _)| *specifier);
    scripts
}

/// The backing scripts as a runtime that starts without a snapshot gives
/// them to `deno_core`, its table of residual lazy scripts: made the first
/// time the process starts such a runtime, and kept for the process's life,
/// since the table is `'static`. A start from a snapshot never calls this:
/// its table came with the snapshot.
pub fn source_backing_scripts() -> &'static [(&'static str, &'static str)] {
    static SCRIPTS: OnceLock<Box<[(&'static str, &'static str)]>> = OnceLock::new();
    SCRIPTS.get_or_init(|| {
        backing_scripts()
            .into_iter()
            .map(|(specifier, script)| (specifier, &*script.leak()))
            .collect()
    })
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

/// Writes a console message to stderr when `stderr` is true (`warn`,
/// `error`, and `trace`) or the run's [`ConsoleOptions`] send every level
/// there, as eval's do, and to stdout otherwise: the host decides, so no
/// script's argument puts a message on eval's stdout. It writes through the
/// path the stdout and stderr host modules write by
/// ([`stdio::write_all`]), so the run's output keeps program order across
/// them.
///
/// A reader that has left (`EPIPE`, a run piped to `head`, or
/// `ECONNRESET` from a socket) loses the message and the run goes on, to
/// end with its own exit code, as Gro's own writes to a closed stream do;
/// any other failure, a full disk or a descriptor open only for reading
/// among them, throws an `Error` naming the stream and the system's error.
#[op2(fast)]
fn op_gro_print(state: &OpState, #[string] message: &str, stderr: bool) -> Result<(), JsErrorBox> {
    let stderr = stderr || state.borrow::<ConsoleOptions>().routing == ConsoleRouting::Stderr;
    let stream = if stderr {
        stdio::Stream::Stderr
    } else {
        stdio::Stream::Stdout
    };
    match stdio::write_all(stream, message.as_bytes()) {
        Err(error) if !stdio::is_reader_gone(&error) => Err(JsErrorBox::generic(format!(
            "failed to write to {}: {error}",
            stream.name()
        ))),
        _ => Ok(()),
    }
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

/// The ops of gro's extension that make no check: the console's, which
/// read only the run's own settings or write its output, and eval's, which
/// refuse. Every other op the extension registers, whatever its name, is a
/// host op, declared through the choke point (`crate::host`); a test fails
/// on one that is neither.
#[cfg(test)]
pub const UNGATED_OPS: [&str; 5] = [
    "op_gro_color",
    "op_gro_console_stderr",
    "op_gro_print",
    "op_gro_deny",
    "op_gro_refused",
];

/// What [`op_gro_refused`] throws.
pub const REFUSED: &str = "a host operation eval refuses was called: eval has no timers, no \
                          clock or machine reads, no blobs, no code generation from strings, \
                          no write to stdout, and no import past its loader";

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
/// a module past the loader (`op_import_sync*`), `op_print`, which writes
/// past the console to stdout, the envelope's channel, and the blob ops,
/// which only `Blob`, `File`, and the object URLs call (eval deletes all of
/// them; a blob part's id is random). None is reachable from user code, which
/// sees no op; this is defence in depth. `op_time_origin` stays: the start
/// calls it, before any user code. A snapshot is made only of a runtime
/// where each one's function is at an address the snapshot can tell from
/// those of the ops eval keeps (`crate::snapshot`).
pub const REFUSED_OPS: [&str; 18] = [
    "op_timer_schedule",
    "op_timer_track",
    "op_timer_now",
    "op_now",
    "op_memory_usage",
    "op_eval_context",
    "op_compile_function",
    "op_import_sync",
    "op_import_sync_with_source",
    "op_print",
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
        op_gro_print,
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
/// changes. The runtime's tests import every `ext:` and `builtin:`
/// specifier the build knows and expect the refusal, and pin the kept
/// synthetic names to the table's URLs.
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

mod template;
#[cfg(test)]
pub mod web_scripts;

#[cfg(test)]
mod tests {
    use super::*;

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
    /// one by that raw name; each over its row's backing script, in the
    /// table's order; and no extension keeps a lazy
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
            assert_eq!(scripts, BACKING_SPECIFIERS);
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

    /// Each row's backing script is named for its specifier, in the
    /// table's order, in a scheme the resolver refuses from source.
    #[test]
    fn the_backing_specifiers_are_the_builtin_table_s() {
        let rows: Vec<String> = BUILTINS
            .iter()
            .map(|builtin| format!("ext:gro/builtin/{}", builtin.specifier))
            .collect();
        assert_eq!(BACKING_SPECIFIERS, rows);
    }

    /// The backing scripts are one a row, each its row's, sorted by
    /// specifier as `deno_core`'s table must be; and the table a source
    /// start reads is the same scripts, made once.
    #[test]
    fn the_backing_scripts_are_the_rows_sorted() {
        let scripts = backing_scripts();
        assert_eq!(scripts.len(), BUILTINS.len());
        assert!(scripts.windows(2).all(|pair| pair[0].0 < pair[1].0));
        for (specifier, script) in &scripts {
            let at = BACKING_SPECIFIERS
                .iter()
                .position(|backing| backing == specifier)
                .unwrap();
            assert_eq!(*script, template::backing_script(&BUILTINS[at]));
        }
        let table = source_backing_scripts();
        let again = source_backing_scripts();
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
}
