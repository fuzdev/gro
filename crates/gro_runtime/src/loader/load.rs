//! Reading a resolved module: the bytes, its format, and its types stripped.

use std::io;
use std::path::Path;

use deno_core::{
    FastString, ModuleSource, ModuleSourceCode, ModuleSpecifier, ModuleType, RequestedModuleType,
};

use super::Refused;
use super::graph::{FailureReason, LoadFailure, Loaded, Recording};
use super::strip::{StripRefusal, strip_module};
use crate::file_id::FileId;
use crate::resolver::{self, EmbeddedModule, MAX_MODULE_SIZE, ModuleFormat, ReadRefusal};

/// A module's code, as V8 runs it: a JS module's bytes as read, or a
/// TypeScript module's JS, its types stripped.
#[derive(Debug, Clone)]
pub(super) enum Code {
    /// A JS module's bytes, which V8 decodes.
    Bytes(Vec<u8>),
    /// A TypeScript module, its types stripped.
    Stripped(String),
}

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
    check_requested(specifier, requested)?;
    let (format, bytes) = read_module(specifier, file_id, check)?;
    // the bytes read, which V8 runs as is or stripped
    let loaded = recording.is_full().then(|| Loaded {
        format,
        hash: blake3::hash(&bytes),
    });
    let code = match format {
        ModuleFormat::JavaScript => Code::Bytes(bytes),
        ModuleFormat::TypeScript => {
            Code::Stripped(strip_module(&decoded(bytes), specifier).map_err(strip_refused)?)
        }
    };
    Ok((module_source(specifier, code), loaded))
}

/// Refuses a module imported with a `type`: no module type past JS loads
/// yet.
pub(super) fn check_requested(
    specifier: &ModuleSpecifier,
    requested: &RequestedModuleType,
) -> Result<(), Refused> {
    // TODO: JSON modules (`with {type: 'json'}`) join with the module formats past JS
    if *requested == RequestedModuleType::None {
        return Ok(());
    }
    Err(Refused::new(
        FailureReason::Load(LoadFailure::ModuleType),
        format!("cannot load {specifier}: import attributes are not supported yet"),
    ))
}

/// Reads the module at `specifier`, the file `file_id` names, once `check`
/// allows its path, returning its format and the bytes read, never past
/// [`MAX_MODULE_SIZE`].
pub(super) fn read_module(
    specifier: &ModuleSpecifier,
    file_id: FileId,
    check: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(ModuleFormat, Vec<u8>), Refused> {
    let refused = |failure, message| Refused::new(FailureReason::Load(failure), message);
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
        // never a load's: only the record of a run's `package.json` reads
        // refuses the first, and only a read graph's tree the second
        Some(ReadRefusal::ReplacedSinceFirstRead | ReadRefusal::OutsideGraph(_)) | None => refused(
            LoadFailure::Read(error.kind()),
            format!("failed to load {specifier}: {error}"),
        ),
    };
    check(&path).map_err(io_error)?;
    // read from the file checked, never the path again, which can change in
    // between, and never past the cap, so a huge file or a FIFO isn't read whole
    let bytes = resolver::read_checked(&path, file_id, Some(MAX_MODULE_SIZE)).map_err(|error| {
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
    Ok((format, bytes))
}

/// `bytes` as text, invalid UTF-8 read as U+FFFD, as V8 decodes a JS
/// module's bytes.
pub(super) fn decoded(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

/// The refusal a strip's `refusal` fails the load with.
pub(super) fn strip_refused(refusal: StripRefusal) -> Refused {
    let failure = LoadFailure::Strip {
        failure: refusal.failure,
        at: refusal.at,
    };
    Refused::new(FailureReason::Load(failure), refusal.message)
}

/// The module V8 runs at `specifier` from `code`.
pub(super) fn module_source(specifier: &ModuleSpecifier, code: Code) -> ModuleSource {
    let code = match code {
        Code::Bytes(bytes) => ModuleSourceCode::Bytes(bytes.into_boxed_slice().into()),
        Code::Stripped(code) => ModuleSourceCode::String(code.into()),
    };
    ModuleSource::new(ModuleType::JavaScript, code, specifier, None)
}

/// Serves the entry the embedder generated at `specifier` from `source` as
/// it is, JavaScript, never stripped. Imported with a `type`, it's refused
/// as a file is, though nothing imports it: its URL's scheme is refused
/// from source.
pub(super) fn load_generated(
    specifier: &ModuleSpecifier,
    requested: &RequestedModuleType,
    source: String,
) -> Result<(ModuleSource, Option<Loaded>), Refused> {
    if *requested != RequestedModuleType::None {
        return Err(Refused::new(
            FailureReason::Load(LoadFailure::ModuleType),
            format!("cannot load {specifier}: import attributes are not supported yet"),
        ));
    }
    let code = ModuleSourceCode::String(source.into());
    Ok((
        ModuleSource::new(ModuleType::JavaScript, code, specifier, None),
        None,
    ))
}

/// Serves the embedded `module` at `specifier` from its code as it is: the
/// copy was stripped of its types when it was made, so it's never stripped
/// again, and nothing is read. Imported with a `type`, it's refused as a
/// file is.
pub(super) fn load_embedded(
    specifier: &ModuleSpecifier,
    requested: &RequestedModuleType,
    module: EmbeddedModule,
) -> Result<(ModuleSource, Option<Loaded>), Refused> {
    if *requested != RequestedModuleType::None {
        return Err(Refused::new(
            FailureReason::Load(LoadFailure::ModuleType),
            format!("cannot load {specifier}: import attributes are not supported yet"),
        ));
    }
    let code = ModuleSourceCode::String(FastString::from_static(module.code));
    Ok((
        ModuleSource::new(ModuleType::JavaScript, code, specifier, None),
        None,
    ))
}
