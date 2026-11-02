//! Resolving one specifier into the graph, and the failure it records.

use deno_core::ModuleSpecifier;
use deno_core::error::ModuleLoaderError;
use url::Url;

use super::graph::{FailureError, Graph, Reached};
use super::reads::RecordingTree;
use super::{Refused, reason};
use crate::resolver::{self, ResolveError, SourceTree};

/// Resolves `specifier`, imported by the module at `referrer`, into `graph`,
/// as [`Loader::resolve_reached`](super::Loader::resolve_reached) describes.
pub(super) fn resolve_into(
    graph: &mut Graph,
    tree: &impl SourceTree,
    specifier: &str,
    referrer: &str,
    reached: Reached,
) -> Result<ModuleSpecifier, Refused> {
    let recording = graph.recording();
    let importer = Url::parse(referrer).map_err(|error| {
        (
            reason(recording, || FailureError::UnknownImporter),
            ModuleLoaderError::type_error(format!(
                "cannot resolve '{specifier}': the importer '{referrer}' is not a URL: {error}"
            )),
        )
    })?;
    // every importer was resolved here first, the entry by `Loader::new`
    let Some(importer_record) = graph.records.get(&importer) else {
        return Err((
            reason(recording, || FailureError::UnknownImporter),
            ModuleLoaderError::type_error(format!(
                "cannot resolve '{specifier}': the importer '{referrer}' was never resolved"
            )),
        ));
    };
    // the record's root is the one its path is inside: admitting a root
    // around a recorded module is a conflict, so none moves it
    let importer_root = &importer_record.root;
    let refused = |error: ResolveError| {
        let message = error.to_string();
        (
            reason(recording, || FailureError::Resolve(error.kind)),
            ModuleLoaderError::type_error(message),
        )
    };
    let resolved = if graph.is_full() {
        let tree = RecordingTree::new(tree, &mut graph.reads);
        resolver::resolve_from(&tree, specifier, &importer, importer_root, &graph.roots)
    } else {
        resolver::resolve_from(tree, specifier, &importer, importer_root, &graph.roots)
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
