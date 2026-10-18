//! The startup snapshot: the runtime's extensions evaluated once at build time
//! and serialized, so a process starts from the heap image instead of
//! evaluating their JS, and runs off its build tree.
//!
//! Two stages. [`make_snapshot`] builds the snapshot in a process of its own
//! (V8 initializes once per process, in snapshotting mode or not) from the same
//! extension list the runtime uses. Its output also names the residual lazy
//! sources: the `deno_web` and `deno_webidl` scripts that load on demand and
//! weren't loaded while the snapshot was made. The snapshot holds no copy of
//! those, so the binary that starts from it embeds them as a table
//! ([`StartupSnapshot`]); the rest of the extensions' JS lives only in the
//! snapshot.

use std::fmt;
use std::path::PathBuf;

use deno_core::snapshot::{CreateSnapshotOptions, create_snapshot};
use deno_core::{Extension, ExtensionFileSource, ExtensionFileSourceCode};

use crate::bootstrap;

/// A snapshot to start from, as the binary embeds it.
///
/// It comes with the residual lazy sources it needs. Each table is
/// `(specifier, source)` pairs sorted by specifier, and every source is 7-bit
/// ASCII ([`make_snapshot`] produces both that way).
#[derive(Clone, Copy)]
pub struct StartupSnapshot {
    /// The serialized heap.
    pub bytes: &'static [u8],
    /// The residual lazy scripts.
    pub residual_lazy_js: &'static [(&'static str, &'static str)],
    /// The residual lazy ES modules.
    pub residual_lazy_esm: &'static [(&'static str, &'static str)],
}

impl fmt::Debug for StartupSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let specifiers = |table: &'static [(&'static str, &'static str)]| -> Vec<&'static str> {
            table.iter().map(|(specifier, _)| *specifier).collect()
        };
        f.debug_struct("StartupSnapshot")
            .field("bytes", &self.bytes.len())
            .field("residual_lazy_js", &specifiers(self.residual_lazy_js))
            .field("residual_lazy_esm", &specifiers(self.residual_lazy_esm))
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
    /// The lazy ES modules not in the snapshot, sorted by specifier.
    pub residual_lazy_esm: Vec<ResidualSource>,
}

/// A lazily loaded extension source the snapshot doesn't carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidualSource {
    /// Its module specifier, such as `ext:deno_web/14_compression.js`.
    pub specifier: String,
    /// Its file on the build machine, an absolute path, for the binary to embed.
    pub path: PathBuf,
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
    /// Evaluating the extensions failed.
    #[error("failed to create the snapshot: {0}")]
    Create(String),
}

/// Builds the startup snapshot from the runtime's extensions.
///
/// Must run in a process that runs no other runtime: V8 initializes once per
/// process, and this initializes it for snapshotting.
///
/// # Errors
///
/// Returns [`SnapshotError`] when an extension source can't be read or isn't
/// ASCII, or when evaluating the extensions fails.
pub fn make_snapshot() -> Result<Snapshot, SnapshotError> {
    // a second copy of the same list, to read the sources the snapshot leaves out
    let extensions = bootstrap::extensions();
    for source in disk_sources(&extensions) {
        check_ascii(source.1)?;
    }

    let output = create_snapshot(
        CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: false,
            extensions: bootstrap::extensions(),
            extension_transpiler: None,
            with_runtime_cb: None,
        },
        None,
    )
    .map_err(|error| SnapshotError::Create(error.to_string()))?;

    // TODO: no installed global lazy-loads a residual source yet, so nothing
    // tests the table; a test that loads one from a snapshot build lands with
    // the first global that does
    let consumed = &output.consumed_lazy_specifiers;
    let residual = |files: fn(&Extension) -> &[ExtensionFileSource]| {
        let mut residual: Vec<ResidualSource> = extensions
            .iter()
            .flat_map(files)
            .filter_map(|file| match file.code {
                ExtensionFileSourceCode::LoadedFromFsDuringSnapshot(path)
                    if !consumed.iter().any(|specifier| specifier == file.specifier) =>
                {
                    Some(ResidualSource {
                        specifier: file.specifier.to_owned(),
                        path: PathBuf::from(path),
                    })
                }
                _ => None,
            })
            .collect();
        residual.sort_by(|a, b| a.specifier.cmp(&b.specifier));
        residual
    };
    Ok(Snapshot {
        bytes: output.output,
        residual_lazy_js: residual(|extension| &extension.lazy_loaded_js_files),
        residual_lazy_esm: residual(|extension| &extension.lazy_loaded_esm_files),
    })
}

/// Every extension source read from the build machine's disk, by specifier.
fn disk_sources(extensions: &[Extension]) -> impl Iterator<Item = (&'static str, &'static str)> {
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
            ExtensionFileSourceCode::LoadedFromFsDuringSnapshot(path) => {
                Some((file.specifier, path))
            }
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
