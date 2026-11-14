//! The startup snapshot: the runtime's extensions evaluated once at build time
//! and serialized, so a process starts from the heap image instead of
//! evaluating their JS, and runs off its build tree.
//!
//! Two stages. [`make_snapshot`] builds the snapshot in a process of its own
//! (V8 initializes once per process, in snapshotting mode or not) from the same
//! extension list the runtime uses. Its output also names the residual lazy
//! scripts, the ones a start from the snapshot can still load, of which the
//! snapshot holds no copy: the `deno_web` and `deno_webidl` scripts that
//! load on demand and weren't loaded while the snapshot was made, each by
//! its file on the build machine, and the builtins' backing scripts, each by
//! its text, the builtin's implementation with its TypeScript stripped here,
//! once (`bootstrap::to_backing_script`). The binary that starts from the
//! snapshot embeds them as a table ([`StartupSnapshot`]) `deno_core` reads
//! on a script's first load, so a start registers no script and strips
//! nothing; the rest of the extensions' JS lives only in the snapshot. The
//! extension list declares no lazy ES modules (see `bootstrap::extensions`),
//! so there is no table of those.
//!
//! [`Snapshot::write_to`] writes a made snapshot as the two files the binary
//! embeds, `snapshot.bin` and `residual.rs`.
//!
//! **The op functions' addresses.** A snapshot holds each op's function,
//! and V8 writes a function there as an index into the runtime's table of
//! external references, which `deno_core` builds from the ops in order,
//! each op's slow function and its fast one when it has one. A start reads
//! the index in its own table, where eval has put its refusal at each
//! refused op's slow function and nothing at its fast one
//! (`bootstrap::REFUSED_OPS`). V8 finds the index by the function's
//! address, taking the first reference that holds it, and a release build
//! merges functions that compile to the same code into one address. So an
//! op whose function shares its address is written by another op's place,
//! and runs at a start as whatever that start put there: eval could run a
//! refused op as one it keeps, an op it keeps as the refusal, or either as
//! nothing. A snapshot's op functions are slow ones alone, since
//! `deno_core` gives a function no fast call while it makes a snapshot.
//!
//! [`make_snapshot`] reads the addresses of the runtime it makes the
//! snapshot of (`deno_core` is patched to give them, `vendor/README.md`)
//! and makes none unless each refused op's slow function shares its address
//! with no function but other refused ops' slow ones, and its fast function
//! with no slow one (`check_refused_functions`). The addresses are that
//! process's, which is where the indexes are decided: a start reads them by
//! position, whatever its own addresses are. The check reads the ops'
//! references alone: `deno_core`'s own callbacks in the table, and the
//! filler it puts where an op has no fast function, are out of its reach.

use std::cell::RefCell;
use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use deno_core::snapshot::{CreateSnapshotOptions, create_snapshot};
use deno_core::{Extension, ExtensionFileSourceCode, JsRuntime};

use crate::bootstrap::{self, Ops};
use crate::engine::Engine;

/// A snapshot to start from, as the binary embeds it.
///
/// It comes with the residual lazy scripts it needs: `(specifier, source)`
/// pairs sorted by specifier, every source 7-bit ASCII ([`make_snapshot`]
/// produces them that way).
#[derive(Clone, Copy)]
pub struct StartupSnapshot {
    /// The serialized heap.
    pub bytes: &'static [u8],
    /// The residual lazy scripts.
    pub residual_lazy_js: &'static [(&'static str, &'static str)],
}

impl fmt::Debug for StartupSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let specifiers: Vec<&str> = self
            .residual_lazy_js
            .iter()
            .map(|(specifier, _)| *specifier)
            .collect();
        f.debug_struct("StartupSnapshot")
            .field("bytes", &self.bytes.len())
            .field("residual_lazy_js", &specifiers)
            .finish()
    }
}

/// A snapshot as [`make_snapshot`] builds it.
#[derive(Debug)]
pub struct Snapshot {
    /// The serialized heap.
    pub bytes: Box<[u8]>,
    /// The lazy scripts not in the snapshot, sorted by specifier.
    pub residual_lazy_js: Vec<ResidualSource>,
}

impl Snapshot {
    /// Writes `snapshot.bin` (the heap) and `residual.rs` (the residual
    /// table, as Rust source) into `dir`, which must exist. A file already
    /// holding the same bytes is left as it is, so its unchanged modification
    /// time doesn't relink what embeds it: the same sources make the same
    /// snapshot.
    ///
    /// `residual.rs` defines the static `RESIDUAL_LAZY_JS` that
    /// [`StartupSnapshot`]'s table comes from, one entry a line: a script
    /// read from the build machine as `(specifier, include_str!(path))`, a
    /// builtin's backing script as `(specifier, "text")`. It has a second
    /// reader, `cargo xtask build --verify`, which parses the first
    /// `include_str!("…")` path out of it, so a change to the format
    /// updates that parser too.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::Write`] when a file can't be written, and
    /// [`SnapshotError::NotUtf8`] when a source path isn't UTF-8.
    pub fn write_to(&self, dir: &Path) -> Result<(), SnapshotError> {
        let residual_rs = format!(
            "// generated by `fgro make-snapshot`: the lazily loaded extension sources the\n\
             // snapshot doesn't carry, sorted by specifier, the web extensions' embedded from\n\
             // the build machine and the builtins' backing scripts written here\n\
             pub static RESIDUAL_LAZY_JS: &[(&str, &str)] = &[\n{}];\n",
            residual_entries(&self.residual_lazy_js)?,
        );
        for (name, content) in [
            ("snapshot.bin", &self.bytes[..]),
            ("residual.rs", residual_rs.as_bytes()),
        ] {
            let path = dir.join(name);
            if std::fs::read(&path).is_ok_and(|existing| existing == content) {
                continue;
            }
            std::fs::write(&path, content)
                .map_err(|source| SnapshotError::Write { path, source })?;
        }
        Ok(())
    }
}

/// The table's entries as Rust source, one a line: `(specifier,
/// include_str!(path))` for a file, `(specifier, "text")` for a text.
fn residual_entries(sources: &[ResidualSource]) -> Result<String, SnapshotError> {
    let mut entries = String::new();
    for source in sources {
        // `{:?}` quotes and escapes each as a Rust string literal
        let _ = match &source.code {
            ResidualCode::File(path) => {
                let path = path
                    .to_str()
                    .ok_or_else(|| SnapshotError::NotUtf8(path.clone()))?;
                writeln!(
                    entries,
                    "    ({:?}, include_str!({path:?})),",
                    source.specifier
                )
            }
            ResidualCode::Text(text) => {
                writeln!(entries, "    ({:?}, {text:?}),", source.specifier)
            }
        };
    }
    Ok(entries)
}

/// A lazily loaded extension source the snapshot doesn't carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidualSource {
    /// Its specifier, such as `ext:deno_web/14_compression.js`.
    pub specifier: String,
    /// Where the binary embeds it from.
    pub code: ResidualCode,
}

/// Where a residual source's code is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResidualCode {
    /// In its file on the build machine, an absolute path: a web
    /// extension's script, as its crate has it.
    File(PathBuf),
    /// Here: a builtin's backing script, 7-bit ASCII.
    Text(String),
}

/// Why a snapshot couldn't be made.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// An extension source couldn't be read.
    #[error("failed to read the extension source {path}: {source}")]
    Read {
        /// The source file.
        path: PathBuf,
        /// The read failure.
        source: std::io::Error,
    },
    /// An extension source isn't 7-bit ASCII, which V8 would read as Latin-1
    /// and silently corrupt.
    #[error("the extension source {0} is not 7-bit ASCII")]
    NotAscii(PathBuf),
    /// A builtin's implementation made no backing script.
    #[error("failed to make a builtin's backing script: {0}")]
    BackingScript(String),
    /// Evaluating the extensions failed.
    #[error("failed to create the snapshot: {0}")]
    Create(#[source] deno_core::error::CoreError),
    /// A residual source's path isn't UTF-8, so `residual.rs` can't name it.
    #[error("{0} is not a UTF-8 path")]
    NotUtf8(PathBuf),
    /// A snapshot file couldn't be written.
    #[error("failed to write {path}: {source}")]
    Write {
        /// The file.
        path: PathBuf,
        /// The write failure.
        source: std::io::Error,
    },
    /// An op eval refuses isn't one the runtime registers, so it's refused
    /// in name only.
    #[error("eval refuses the op {0}, and the runtime registers no op of that name")]
    RefusedOpUnregistered(&'static str),
    /// A function of an op eval refuses is at the address of another op's
    /// function, so the snapshot would write one by the other's place among
    /// the external references.
    #[error(
        "the {refused_function} function of {refused}, an op eval refuses, is at the address of \
         the {other_function} function of {other}: the build merged the two, and a snapshot \
         writes a function by the first external reference that holds its address, so a start \
         from this one could run either op as the other, a refused one as one eval keeps"
    )]
    SharedOpFunction {
        /// The refused op.
        refused: &'static str,
        /// Which of its functions shares its address.
        refused_function: OpFunction,
        /// The op it shares it with.
        other: &'static str,
        /// Which of that op's functions it is.
        other_function: OpFunction,
    },
}

/// Which of an op's functions: the one every call can take, or the one V8's
/// optimized code calls directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpFunction {
    /// The slow function, a V8 function callback.
    Slow,
    /// The fast function.
    Fast,
}

impl fmt::Display for OpFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Slow => "slow",
            Self::Fast => "fast",
        })
    }
}

/// An op's functions, by the addresses its runtime's external references
/// hold for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpFunctions {
    /// The op's name.
    pub name: &'static str,
    /// Its slow function's address.
    pub slow: usize,
    /// Its fast function's, when it has one.
    pub fast: Option<usize>,
}

/// Every op of `runtime` with its functions' addresses, in the order of its
/// external references.
pub(crate) fn op_functions(runtime: &JsRuntime) -> Vec<OpFunctions> {
    runtime
        .op_function_addresses()
        .into_iter()
        .map(|(name, slow, fast)| OpFunctions { name, slow, fast })
        .collect()
}

/// Fails unless each op `refused` names is among `ops`, its slow function
/// sharing its address with no function but the slow ones of other refused
/// ops, and its fast function with no slow one: what the snapshot of a
/// runtime with these `ops` needs for eval's refusals to land on the
/// refused ops and no other (the module doc has why).
///
/// # Errors
///
/// Returns [`SnapshotError::RefusedOpUnregistered`] for a name no op has,
/// and [`SnapshotError::SharedOpFunction`], naming both ops, for an address
/// shared otherwise.
pub(crate) fn check_refused_functions(
    ops: &[OpFunctions],
    refused: &[&'static str],
) -> Result<(), SnapshotError> {
    if let Some(name) = refused
        .iter()
        .find(|name| !ops.iter().any(|op| op.name == **name))
    {
        return Err(SnapshotError::RefusedOpUnregistered(name));
    }
    let is_refused = |op: &OpFunctions| refused.contains(&op.name);
    for (at, op) in ops.iter().enumerate().filter(|(_, op)| is_refused(op)) {
        for (other_at, other) in ops.iter().enumerate() {
            let functions = if other_at == at {
                None
            } else if other.slow == op.slow && !is_refused(other) {
                Some((OpFunction::Slow, OpFunction::Slow))
            } else if other.fast == Some(op.slow) {
                Some((OpFunction::Slow, OpFunction::Fast))
            } else if op.fast == Some(other.slow) {
                Some((OpFunction::Fast, OpFunction::Slow))
            } else {
                None
            };
            if let Some((refused_function, other_function)) = functions {
                return Err(SnapshotError::SharedOpFunction {
                    refused: op.name,
                    refused_function,
                    other: other.name,
                    other_function,
                });
            }
        }
    }
    Ok(())
}

/// Builds the startup snapshot from the runtime's extensions.
///
/// Must run in a process that runs no other runtime: V8 initializes once per
/// process, and this initializes it for snapshotting, on the engine `_engine`
/// names ([`crate::engine`]).
///
/// # Errors
///
/// Returns [`SnapshotError`] when an extension source can't be read or isn't
/// ASCII, when a builtin's implementation makes no backing script, when
/// evaluating the extensions fails, or when an op eval refuses is at an
/// address the snapshot can't tell from another op's, or isn't registered
/// (`check_refused_functions`).
pub fn make_snapshot(engine: Engine) -> Result<Snapshot, SnapshotError> {
    make_snapshot_of(engine, || bootstrap::extensions(Ops::Kept))
}

/// [`make_snapshot`], from the extensions `listed` returns, the same list
/// at each call: the runtime's own, but in a test of what a snapshot isn't
/// made from.
pub(crate) fn make_snapshot_of(
    _engine: Engine,
    listed: impl Fn() -> Vec<Extension>,
) -> Result<Snapshot, SnapshotError> {
    // a second copy of the same list, to read the sources the snapshot leaves out
    let extensions = listed();
    for path in disk_sources(&extensions) {
        check_ascii(path)?;
    }

    let functions = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&functions);
    let output = create_snapshot(
        CreateSnapshotOptions {
            // deno_core never reads it, and `env!("CARGO_MANIFEST_DIR")` would
            // bake the checkout's path into the binary
            cargo_manifest_dir: "",
            startup_snapshot: None,
            skip_op_registration: false,
            extensions: listed(),
            // every embedded source is JS: a builtin's TypeScript is no
            // extension source, and is stripped below
            extension_transpiler: None,
            // the addresses this runtime's snapshot names its op functions by
            with_runtime_cb: Some(Box::new(move |runtime| {
                *seen.borrow_mut() = op_functions(runtime);
            })),
        },
        None,
    )
    .map_err(SnapshotError::Create)?;
    check_refused_functions(&functions.borrow(), &bootstrap::REFUSED_OPS)?;

    let consumed = &output.consumed_lazy_specifiers;
    let files = extensions
        .iter()
        .flat_map(|extension| extension.lazy_loaded_js_files.iter())
        .filter_map(|file| match file.code {
            ExtensionFileSourceCode::LoadedFromFsDuringSnapshot(path)
                if !consumed.iter().any(|specifier| specifier == file.specifier) =>
            {
                Some(ResidualSource {
                    specifier: file.specifier.to_owned(),
                    code: ResidualCode::File(PathBuf::from(path)),
                })
            }
            _ => None,
        });
    // no snapshot holds a builtin's implementation, so each is residual
    let scripts = bootstrap::backing_scripts().map_err(SnapshotError::BackingScript)?;
    let scripts = scripts
        .into_iter()
        .map(|(specifier, script)| ResidualSource {
            specifier: specifier.to_owned(),
            code: ResidualCode::Text(script),
        });
    let mut residual_lazy_js: Vec<ResidualSource> = files.chain(scripts).collect();
    residual_lazy_js.sort_by(|a, b| a.specifier.cmp(&b.specifier));
    Ok(Snapshot {
        bytes: output.output,
        residual_lazy_js,
    })
}

/// The path of every extension source read from the build machine's disk.
fn disk_sources(extensions: &[Extension]) -> impl Iterator<Item = &'static str> {
    extensions
        .iter()
        .flat_map(|extension| {
            [
                &extension.js_files,
                &extension.esm_files,
                &extension.lazy_loaded_js_files,
                &extension.lazy_loaded_esm_files,
            ]
        })
        .flat_map(|files| files.iter())
        .filter_map(|file| match file.code {
            ExtensionFileSourceCode::LoadedFromFsDuringSnapshot(path) => Some(path),
            _ => None,
        })
}

/// Fails unless the file at `path` is 7-bit ASCII.
fn check_ascii(path: &str) -> Result<(), SnapshotError> {
    let bytes = std::fs::read(path).map_err(|source| SnapshotError::Read {
        path: PathBuf::from(path),
        source,
    })?;
    if bytes.is_ascii() {
        Ok(())
    } else {
        Err(SnapshotError::NotAscii(PathBuf::from(path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An op with a slow function alone.
    const fn slow(name: &'static str, slow: usize) -> OpFunctions {
        OpFunctions {
            name,
            slow,
            fast: None,
        }
    }

    /// An op with both functions.
    const fn fast(name: &'static str, slow: usize, fast: usize) -> OpFunctions {
        OpFunctions {
            name,
            slow,
            fast: Some(fast),
        }
    }

    /// The ops the cases below call refused.
    const REFUSED: [&str; 2] = ["refused_a", "refused_b"];

    /// What the check makes of `ops`, with [`REFUSED`] refused: nothing, or
    /// the refused op, its function, the other op, and its function.
    fn shared(ops: &[OpFunctions]) -> Option<(&'static str, OpFunction, &'static str, OpFunction)> {
        match check_refused_functions(ops, &REFUSED) {
            Ok(()) => None,
            Err(SnapshotError::SharedOpFunction {
                refused,
                refused_function,
                other,
                other_function,
            }) => Some((refused, refused_function, other, other_function)),
            Err(error) => panic!("{error}"),
        }
    }

    /// Addresses a refused op may share: none, or another refused op's slow
    /// function with its own slow one, which eval refuses alike, and
    /// another op's fast function with its own fast one, which no snapshot
    /// holds. Ops eval keeps share theirs freely, as a release build's do.
    #[test]
    fn a_refused_op_shares_its_slow_function_with_refused_ops_alone() {
        use OpFunction::{Fast, Slow};

        let apart = [
            fast("kept_a", 1, 2),
            slow("kept_b", 3),
            fast("refused_a", 4, 5),
            slow("refused_b", 6),
        ];
        assert_eq!(shared(&apart), None);
        let harmless = [
            // two kept ops, every function shared
            fast("kept_a", 1, 2),
            fast("kept_b", 1, 2),
            // two refused ops' slow functions, and their fast ones with a kept op's
            fast("refused_a", 4, 2),
            fast("refused_b", 4, 2),
        ];
        assert_eq!(shared(&harmless), None);

        // a kept op's slow function, registered before the refused op or after
        let hole = [
            slow("kept_a", 1),
            slow("refused_a", 1),
            slow("refused_b", 2),
        ];
        assert_eq!(shared(&hole), Some(("refused_a", Slow, "kept_a", Slow)));
        let closed = [
            slow("refused_a", 2),
            slow("refused_b", 1),
            slow("kept_a", 1),
        ];
        assert_eq!(shared(&closed), Some(("refused_b", Slow, "kept_a", Slow)));
        // any op's fast function at a refused op's slow one, a refused op's too
        let kept_fast = [
            fast("kept_a", 1, 7),
            slow("refused_a", 7),
            slow("refused_b", 2),
        ];
        assert_eq!(
            shared(&kept_fast),
            Some(("refused_a", Slow, "kept_a", Fast))
        );
        let refused_fast = [fast("refused_a", 1, 7), slow("refused_b", 7)];
        assert_eq!(
            shared(&refused_fast),
            // the first refused op in the table is the one named as refused
            Some(("refused_a", Fast, "refused_b", Slow))
        );
        // a refused op's fast function at a kept op's slow one
        let fast_slow = [
            fast("refused_a", 1, 7),
            slow("refused_b", 2),
            slow("kept_a", 7),
        ];
        assert_eq!(
            shared(&fast_slow),
            Some(("refused_a", Fast, "kept_a", Slow))
        );
        // an op's own two functions are its own business
        let own = [fast("refused_a", 7, 7), slow("refused_b", 2)];
        assert_eq!(shared(&own), None);
    }

    /// A refused name no op has fails the check, an empty table included,
    /// so the check never passes for want of anything to read.
    #[test]
    fn a_refused_op_must_be_registered() {
        for ops in [&[][..], &[slow("refused_a", 1), slow("kept_a", 2)]] {
            let error = check_refused_functions(ops, &REFUSED).unwrap_err();
            let expected = if ops.is_empty() {
                "refused_a"
            } else {
                "refused_b"
            };
            assert!(
                matches!(error, SnapshotError::RefusedOpUnregistered(name) if name == expected),
                "{error}"
            );
        }
    }

    /// The failure names both ops and both functions.
    #[test]
    fn a_shared_function_s_failure_names_both_ops() {
        let ops = [
            slow("kept_a", 1),
            slow("refused_a", 1),
            slow("refused_b", 2),
        ];
        assert_eq!(
            check_refused_functions(&ops, &REFUSED)
                .unwrap_err()
                .to_string(),
            "the slow function of refused_a, an op eval refuses, is at the address of the slow \
             function of kept_a: the build merged the two, and a snapshot writes a function by \
             the first external reference that holds its address, so a start from this one could \
             run either op as the other, a refused one as one eval keeps"
        );
    }
}
