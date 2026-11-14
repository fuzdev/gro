//! gro's bootstrap extension: installs the pure web globals from `deno_web` and
//! hides the host's internals from user code.
//!
//! Its JS (`js/bootstrap.js`) is embedded in the binary at compile time. The
//! embedding (`ascii_str_include!`) asserts in a constant that the source is
//! 7-bit ASCII, so a non-ASCII character fails the build rather than reaching
//! V8, which reads extension sources as one-byte strings. So are the builtins'
//! implementations, TypeScript (one module a builtin, like `js/args.ts`, and
//! the node-free modules of `src/lib/internal/` they share with the Node
//! implementation, embedded as they are), stripped of their types by
//! [`transpile`] when the snapshot is made, or when a runtime starts with no
//! snapshot.
//!
//! **The builtin modules.** Each row of the resolver's builtin table
//! ([`crate::resolver::BUILTINS`]) is served as a `deno_core` synthetic
//! module (a facade) named by its `builtin:` URL, over a backing script made
//! from one template: it calls the builtin's factory, which its
//! implementation module stores on `deno_core`'s `internals` when the
//! snapshot evaluates it, so only extension code reaches it. The factory
//! runs once a run, when the builtin is first imported, after the start, and
//! returns the namespace: the facade's exports are its own properties, with
//! a `default` of `undefined`. Its one argument is the scope of the
//! importer's grant, which the template passes and a factory reads no other
//! way; no grant scopes anything yet, and a grant-free builtin has none, so
//! the template passes `undefined`. A factory never throws: `deno_core`
//! evaluates a backing script once, rejecting the import with its own
//! report of what the script threw and every later import of the builtin
//! with the script's absence, so a builtin that can fail fails in what it
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

use deno_core::error::{JsError, ModuleLoaderError};
use deno_core::{
    Extension, ExtensionFileSource, JsRuntime, ModuleCodeString, ModuleName, OpDecl, OpState,
    SourceMapData, ascii_str, ascii_str_include, op2, v8,
};
use url::Url;

use crate::host::host_ops;
use crate::loader::strip::strip_module;
use crate::resolver::{builtin_rows, builtin_url};
use crate::{ConsoleOptions, ConsoleRouting};

/// The bootstrap module's specifier.
const BOOTSTRAP_SPECIFIER: &str = "ext:gro/bootstrap.js";

/// The bootstrap module and the builtins' implementations, embedded.
const BOOTSTRAP_ESM: &[ExtensionFileSource] = &[
    ExtensionFileSource::new(
        BOOTSTRAP_SPECIFIER,
        ascii_str_include!("../js/bootstrap.js"),
    ),
    ExtensionFileSource::new("ext:gro/args.ts", ascii_str_include!("../js/args.ts")),
    ExtensionFileSource::new("ext:gro/cwd.ts", ascii_str_include!("../js/cwd.ts")),
    ExtensionFileSource::new("ext:gro/pid.ts", ascii_str_include!("../js/pid.ts")),
    ExtensionFileSource::new(
        "ext:gro/runtime_version.ts",
        ascii_str_include!("../js/runtime_version.ts"),
    ),
    ExtensionFileSource::new(
        "ext:gro/internal/records.ts",
        ascii_str_include!("../../../src/lib/internal/records.ts"),
    ),
];

/// A builtin's backing script, from one template: it returns the namespace
/// the builtin's factory makes, given the scope of the importer's grant,
/// `undefined` while no grant has one.
macro_rules! backing_script {
    ($specifier:literal, $grant:expr, [$($export:literal),*]) => {
        ExtensionFileSource::new(
            concat!("ext:gro/builtin/", $specifier, ".js"),
            ascii_str!(concat!(
                "(function () {\n\treturn __bootstrap.internals.gro_builtins['",
                $specifier,
                "'](undefined);\n})()\n"
            )),
        )
    };
}

/// A builtin's synthetic module: its URL, over its backing script.
macro_rules! synthetic_module {
    ($specifier:literal, $grant:expr, [$($export:literal),*]) => {
        (
            builtin_url!($specifier),
            concat!("ext:gro/builtin/", $specifier, ".js"),
        )
    };
}

/// Each builtin's backing script.
const BUILTIN_SCRIPTS: &[ExtensionFileSource] = &builtin_rows!(backing_script);

/// Each builtin's synthetic module, `(name, backing script)`.
const BUILTIN_MODULES: &[(&str, &str)] = &builtin_rows!(synthetic_module);

/// The extension transpiler: strips the types from a `.ts` source, and
/// passes every other through as it is. Set when the snapshot is made and
/// when a runtime starts without one, never on a snapshot start, where
/// `deno_core` would pass every runtime-loadable script through it at each
/// start.
///
/// # Errors
///
/// Returns the strip's refusal, which fails the snapshot or the start.
pub fn transpile(
    name: &ModuleName,
    code: ModuleCodeString,
) -> Result<(ModuleCodeString, Option<SourceMapData>), ModuleLoaderError> {
    if !std::path::Path::new(name.as_str())
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ts"))
    {
        return Ok((code, None));
    }
    let url = Url::parse(name.as_str())
        .map_err(|error| ModuleLoaderError::generic(format!("{}: {error}", name.as_str())))?;
    let stripped = strip_module(code.as_str(), &url)
        .map_err(|refusal| ModuleLoaderError::generic(refusal.message))?;
    Ok((stripped.into(), None))
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
        extension.lazy_loaded_js_files = Cow::Borrowed(BUILTIN_SCRIPTS);
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
/// Each extension's lazy ES modules (`deno_web`'s `locks.js` and
/// `webtransport.js`) and every other extension's synthetic ES modules (none
/// today) are dropped from its declaration ([`restrict`]), so the runtime
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

/// `extensions` with each one's lazy ES modules dropped, and its synthetic
/// ES modules unless it is gro's, and gro's carrying [`refuse_ops`] when
/// `ops` says.
fn restrict(mut extensions: Vec<Extension>, ops: Ops) -> Vec<Extension> {
    for extension in &mut extensions {
        extension.lazy_loaded_esm_files = Cow::Borrowed(&[]);
        if extension.name != "gro" {
            extension.synthetic_esm_modules = Cow::Borrowed(&[]);
        }
        if ops == Ops::Refused && extension.name == "gro" {
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
    /// one by that raw name; each over a backing script of gro's own; and
    /// no extension keeps a lazy ES module.
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
            let gro = extensions
                .iter()
                .find(|extension| extension.name == "gro")
                .unwrap();
            for (name, script) in modules {
                assert!(
                    gro.lazy_loaded_js_files
                        .iter()
                        .any(|file| file.specifier == script && script.starts_with("ext:gro/")),
                    "{name}: {script}"
                );
            }
            for extension in &extensions {
                assert!(
                    extension.lazy_loaded_esm_files.is_empty(),
                    "{}",
                    extension.name
                );
            }
        }
    }

    /// Another extension's synthetic and lazy ES modules are dropped, gro's
    /// synthetic modules kept.
    #[test]
    fn another_extension_s_reachable_modules_are_dropped() {
        const FOREIGN_LAZY: &[ExtensionFileSource] = &[ExtensionFileSource::new(
            "ext:foreign/lazy.js",
            ascii_str!("export {};"),
        )];
        let foreign = Extension {
            name: "foreign",
            synthetic_esm_modules: Cow::Borrowed(&[("builtin:foreign", "ext:foreign/x.js")]),
            lazy_loaded_esm_files: Cow::Borrowed(FOREIGN_LAZY),
            ..Extension::default()
        };
        let restricted = restrict(vec![foreign, gro::init()], Ops::Kept);
        assert!(restricted[0].synthetic_esm_modules.is_empty());
        assert!(restricted[0].lazy_loaded_esm_files.is_empty());
        assert_eq!(restricted[1].synthetic_esm_modules, BUILTIN_MODULES);
    }

    /// The transpiler strips a `.ts` source and passes every other through
    /// as it is.
    #[test]
    fn the_transpiler_strips_only_typescript() {
        let code = |source: &'static str| ModuleCodeString::from_static(source);
        let (stripped, map) = transpile(
            &ModuleName::from_static("ext:gro/x.ts"),
            code("export const a: number = 1;\n"),
        )
        .unwrap();
        assert_eq!(stripped.as_str(), "export const a         = 1;\n");
        assert!(map.is_none());
        let (kept, _) = transpile(
            &ModuleName::from_static("ext:gro/x.js"),
            code("export const a: number = 1;\n"),
        )
        .unwrap();
        assert_eq!(kept.as_str(), "export const a: number = 1;\n");
        assert!(
            transpile(
                &ModuleName::from_static("ext:gro/x.ts"),
                code("enum E { A }\n")
            )
            .is_err()
        );
    }
}
