//! Reading a resolved module: the bytes, its format, and its types stripped.

use std::io;
use std::path::Path;

use deno_core::{ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType, RequestedModuleType};

use super::Refused;
use super::graph::{FailureReason, LoadFailure, Loaded, Recording};
use super::strip::strip_module;
use crate::resolver::{self, FileId, MAX_MODULE_SIZE, ModuleFormat, ReadRefusal};

/// Reads the module at `specifier`, a URL the resolver produced for the file
/// `file_id` names, once `check` allows its path, stripping a TypeScript
/// module's types, and returns it with, when `recording` is full, what was
/// read: its format and the hash of the bytes read.
pub(super) fn load(
    specifier: &ModuleSpecifier,
    requested: &RequestedModuleType,
    file_id: FileId,
    recording: Recording,
    check: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(ModuleSource, Option<Loaded>), Refused> {
    let refused = |failure, message| Refused::new(FailureReason::Load(failure), message);
    // TODO: JSON modules (`with {type: 'json'}`) join with the module formats past JS
    if *requested != RequestedModuleType::None {
        return Err(refused(
            LoadFailure::ImportAttributes,
            format!("cannot load {specifier}: import attributes are not supported yet"),
        ));
    }
    let path = specifier.to_file_path().map_err(|()| {
        refused(
            LoadFailure::NoLocalPath,
            format!("cannot load {specifier}: not a local file"),
        )
    })?;
    // the resolver read the same table, so a refusal here means the URL skipped it
    let format = resolver::module_format(&path).map_err(|kind| {
        let message = format!("cannot load {specifier}: {kind}");
        refused(LoadFailure::Format(kind), message)
    })?;
    let io_error = |error: io::Error| match ReadRefusal::of(&error) {
        Some(ReadRefusal::OutsideBox(_)) => refused(
            LoadFailure::OutsideBox,
            format!("cannot load {specifier}: {error}"),
        ),
        Some(ReadRefusal::ReplacedSinceLookup) => refused(
            LoadFailure::ReplacedSinceLookup,
            format!(
                "cannot load {specifier}: the file at its path is not the one resolved, so it \
                 isn't loaded"
            ),
        ),
        // never a load's: only the record of a run's `package.json` reads refuses it
        Some(ReadRefusal::ReplacedSinceFirstRead) | None => refused(
            LoadFailure::Read(error.kind()),
            format!("failed to load {specifier}: {error}"),
        ),
    };
    check(&path).map_err(io_error)?;
    // read from the file checked, never the path again, which can change in
    // between, and never past the cap, so a huge file or a FIFO isn't read whole
    let code = resolver::read_checked(&path, file_id, Some(MAX_MODULE_SIZE)).map_err(|error| {
        if error.kind() != io::ErrorKind::FileTooLarge {
            return io_error(error);
        }
        refused(
            LoadFailure::TooLarge,
            format!(
                "cannot load {specifier}: the module is larger than {} MiB, the cap on a \
                 module's source",
                MAX_MODULE_SIZE >> 20
            ),
        )
    })?;
    // the bytes read, which V8 runs as is or stripped
    let loaded = recording.is_full().then(|| Loaded {
        format,
        hash: blake3::hash(&code),
    });
    let code = match format {
        ModuleFormat::JavaScript => ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
        ModuleFormat::TypeScript => {
            // invalid UTF-8 reads as U+FFFD, as V8 decodes a JS module's bytes
            let source = String::from_utf8(code)
                .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
            let code = strip_module(&source, specifier).map_err(|refusal| {
                let failure = LoadFailure::Strip {
                    failure: refusal.failure,
                    at: refusal.at,
                };
                refused(failure, refusal.message)
            })?;
            ModuleSourceCode::String(code.into())
        }
    };
    Ok((
        ModuleSource::new(ModuleType::JavaScript, code, specifier, None),
        loaded,
    ))
}
