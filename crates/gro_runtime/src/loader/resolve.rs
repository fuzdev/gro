//! Resolving one specifier into the graph, and the failure it records.

use deno_core::ModuleSpecifier;
use url::Url;

use super::Refused;
use super::graph::{FailureError, Graph, Reached};
use crate::resolver::{ResolveError, SourceTree};

/// Resolves `specifier`, imported by the module at `referrer`, into `graph`,
/// as [`Loader::resolve_reached`](super::Loader::resolve_reached) describes.
pub(super) fn resolve_into(
    graph: &mut Graph,
    tree: &impl SourceTree,
    specifier: &str,
    referrer: &str,
    reached: Reached,
) -> Result<ModuleSpecifier, Refused> {
    let importer = Url::parse(referrer).map_err(|error| {
        Refused::new(
            FailureError::UnknownImporter,
            format!(
                "cannot resolve '{specifier}': the importer '{referrer}' is not a URL: {error}"
            ),
        )
    })?;
    let refused = |error: ResolveError| {
        let message = error.to_string();
        Refused::new(FailureError::Resolve(error.kind), message)
    };
    // every importer was resolved here first, the entry by `Loader::new`
    let Some(resolved) = graph.resolve(tree, specifier, &importer) else {
        return Err(Refused::new(
            FailureError::UnknownImporter,
            format!("cannot resolve '{specifier}': the importer '{referrer}' was never resolved"),
        ));
    };
    let mut resolution = resolved.map_err(refused)?;
    // read whether or not the root is admitted
    graph.consult(std::mem::take(&mut resolution.files_consulted));
    let Reached::Import(reach) = reached else {
        return Ok(resolution.url);
    };
    graph.admit(&resolution).map_err(|kind| {
        refused(ResolveError {
            specifier: specifier.to_owned(),
            importer: referrer.to_owned(),
            kind,
        })
    })?;
    graph.link(&importer, specifier, reach, &resolution.url);
    Ok(resolution.url)
}
