//! The `deno_core` module loader: a thin adapter over [`crate::resolver`] that
//! reads the resolved modules from disk.
//!
//! Static imports, `import()`, and `import.meta.resolve` all reach
//! [`resolver::resolve`] through `ModuleLoader::resolve`. The entry module is
//! resolved before the runtime starts ([`resolver::resolve_entry`]), so its
//! URL arrives here already final.

use deno_core::error::ModuleLoaderError;
use deno_core::{
    ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader, ModuleSource,
    ModuleSourceCode, ModuleSpecifier, ModuleType, RequestedModuleType, ResolutionKind,
};
use url::Url;

use crate::resolver::{self, DiskTree};

/// Resolves through the strict resolver over the disk, and loads `.js` and
/// `.mjs` files as ES modules.
#[derive(Debug, Default)]
pub struct Loader;

impl ModuleLoader for Loader {
    fn resolve(
        &self,
        specifier: &str,
        referrer: &str,
        kind: ResolutionKind,
    ) -> Result<ModuleSpecifier, ModuleLoaderError> {
        if kind == ResolutionKind::MainModule {
            // the entry, already resolved by `resolver::resolve_entry`
            return Url::parse(specifier)
                .map_err(|error| ModuleLoaderError::type_error(error.to_string()));
        }
        let importer = Url::parse(referrer).map_err(|error| {
            ModuleLoaderError::type_error(format!(
                "cannot resolve '{specifier}': the importer '{referrer}' is not a URL: {error}"
            ))
        })?;
        resolver::resolve(&DiskTree, specifier, &importer)
            .map(|resolution| resolution.url)
            .map_err(|error| ModuleLoaderError::type_error(error.to_string()))
    }

    fn load(
        &self,
        specifier: &ModuleSpecifier,
        _referrer: Option<&ModuleLoadReferrer>,
        options: ModuleLoadOptions,
    ) -> ModuleLoadResponse {
        ModuleLoadResponse::Sync(load(specifier, &options.requested_module_type))
    }
}

/// Reads the module at `specifier`, a URL the resolver produced.
fn load(
    specifier: &ModuleSpecifier,
    requested: &RequestedModuleType,
) -> Result<ModuleSource, ModuleLoaderError> {
    // TODO: JSON modules (`with {type: 'json'}`) join with the module formats past JS
    if *requested != RequestedModuleType::None {
        return Err(ModuleLoaderError::type_error(format!(
            "cannot load {specifier}: import attributes are not supported yet"
        )));
    }
    let path = specifier.to_file_path().map_err(|()| {
        ModuleLoaderError::type_error(format!("cannot load {specifier}: not a local file"))
    })?;
    let code = std::fs::read(&path)
        .map_err(|error| ModuleLoaderError::generic(format!("cannot load {specifier}: {error}")))?;
    Ok(ModuleSource::new(
        ModuleType::JavaScript,
        ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
        specifier,
        None,
    ))
}
