//! Node's built-in modules, which the resolver refuses, and the host
//! module of Gro's a refusal names for one.
//!
//! A `node:` specifier names a built-in, and so does a bare specifier that
//! is exactly one of the names Node resolves bare ([`NODE_BUILTINS`]),
//! which Node resolves ahead of any package of that name. Either is refused
//! ahead of any package lookup, so a package installed under a built-in's
//! name never stands in for it: the code meant Node's module, which there
//! is no compat layer for.

use std::fmt;

use super::builtin::{SPLIT_MODULES, find_builtin};
use super::error::write_specifiers;

/// The names Node resolves bare as its built-in modules: `builtinModules`
/// less the ones only `node:` names (`node:test`, `node:sqlite`, …) and the
/// internals starting with `_`, which Node versions add and drop, sorted
/// for [`is_node_builtin`]'s search.
///
/// The resolution differential's oracle holds it to the `builtinModules` of
/// the Node it runs under (`tests/fixtures/resolution/cases.json`'s
/// `node_builtins`).
pub const NODE_BUILTINS: &[&str] = &[
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

/// Whether Node resolves the bare `specifier` as one of its built-in
/// modules, exactly as written.
pub fn is_node_builtin(specifier: &str) -> bool {
    NODE_BUILTINS.binary_search(&specifier).is_ok()
}

/// What of Gro's stands for a Node built-in, by the advisory table of
/// `src/docs/host_modules.md`: one host module, or the process modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counterpart {
    /// One host module, by its `.ts` specifier.
    Module(&'static str),
    /// The process modules, `@fuzdev/gro/process.ts`'s members.
    ProcessModules,
}

/// The counterpart of the Node built-in `name` (`fs`, not `node:fs`).
fn counterpart(name: &str) -> Option<Counterpart> {
    match name {
        "fs" | "fs/promises" => Some(Counterpart::Module("@fuzdev/gro/fs.ts")),
        "child_process" => Some(Counterpart::Module("@fuzdev/gro/spawn.ts")),
        "process" => Some(Counterpart::ProcessModules),
        "net" | "http" | "https" | "dns" | "tls" | "dgram" => {
            Some(Counterpart::Module("@fuzdev/gro/net.ts"))
        }
        _ => None,
    }
}

/// The process modules on the binary, each by its `.ts` specifier, in
/// the table's order, and how many there are, built or not.
fn process_modules() -> (Vec<&'static str>, usize) {
    let all = SPLIT_MODULES
        .iter()
        .find(|split| split.stem == "@fuzdev/gro/process")
        .map_or(&[][..], |split| split.modules);
    let built = all
        .iter()
        .copied()
        .filter(|module| find_builtin(module).is_some())
        .collect();
    (built, all.len())
}

/// Writes the refusal of the Node built-in `name`, named by a `node:`
/// specifier or, when `bare`, by its bare name, and the fix: the host
/// module of Gro's for it, when there is one and it's on the binary; that
/// it isn't on the binary yet; or that Gro has none.
pub(super) fn write_refusal(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    bare: bool,
    binary: &str,
) -> fmt::Result {
    if bare {
        let package = name.split('/').next().unwrap_or(name);
        write!(
            f,
            "it names the Node built-in 'node:{name}', which Node resolves ahead of any package \
             named '{package}', and there is no Node compat"
        )?;
    } else {
        f.write_str("a node: specifier names a Node built-in, and there is no Node compat")?;
    }
    match counterpart(name) {
        Some(Counterpart::Module(specifier)) if find_builtin(specifier).is_some() => write!(
            f,
            ". Fix: import '{specifier}', Gro's host module for it, or run this under Node"
        ),
        Some(Counterpart::Module(specifier)) => write!(
            f,
            ". Gro's host module for it, '{specifier}', isn't on {binary} yet. Fix: run this \
             under Node"
        ),
        Some(Counterpart::ProcessModules) => {
            let (built, count) = process_modules();
            if built.is_empty() {
                return write!(
                    f,
                    ". Gro's process modules aren't on {binary} yet. Fix: run this under Node"
                );
            }
            f.write_str(". Fix: import what you use from Gro's process modules, ")?;
            if built.len() < count {
                write!(f, "of which {binary} has ")?;
                write_specifiers(f, built.iter().copied(), built.len(), "and")?;
            } else {
                write_specifiers(f, built.iter().copied(), built.len(), "or")?;
            }
            f.write_str(", or run this under Node")
        }
        None => f.write_str(
            ", and Gro has no host module for it. Fix: use code that doesn't need it, or run \
             this under Node",
        ),
    }
}
