//! Resolving one specifier into the graph, the builtins' grant check, and
//! the failure each records.

use deno_core::ModuleSpecifier;
use url::Url;

use super::Refused;
use super::graph::{FailureReason, Graph, Reached};
use crate::Authority;
use crate::resolver::{
    Builtin, Origin, ResolveError, ResolveErrorKind, Resolved, SourceTree, UrlFault,
};

/// Resolves `specifier`, imported by the module at `referrer`, into `graph`,
/// as [`Loader::resolve_reached`](super::Loader::resolve_reached) describes,
/// an import of a builtin checked against `authority`.
pub(super) fn resolve_into(
    graph: &mut Graph,
    authority: Option<&Authority>,
    tree: &impl SourceTree,
    specifier: &str,
    referrer: &str,
    reached: Reached,
) -> Result<ModuleSpecifier, Refused> {
    let importer = Url::parse(referrer).map_err(|error| {
        Refused::new(
            FailureReason::UnknownImporter,
            format!(
                "cannot resolve '{specifier}': the importer '{referrer}' is not a URL: {error}"
            ),
        )
    })?;
    let refused = |error: ResolveError| {
        let message = error.to_string();
        Refused::new(FailureReason::Resolve(error.kind), message)
    };
    // every importer was resolved here first, the entry by `Loader::new`
    let Some(resolved) = graph.resolve(tree, specifier, &importer) else {
        return Err(Refused::new(
            FailureReason::UnknownImporter,
            format!("cannot resolve '{specifier}': the importer '{referrer}' was never resolved"),
        ));
    };
    let mut resolution = match resolved.map_err(refused)? {
        Resolved::Module(resolution) => resolution,
        Resolved::Builtin(builtin) => {
            return resolve_builtin(graph, authority, builtin, specifier, &importer, reached);
        }
    };
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

/// The URL of `builtin`, which `specifier` named, imported by `importer`:
/// for an import, once its grant is checked against `authority` and the
/// importer's origin, recorded with the edge to it; for
/// `import.meta.resolve`, unchecked, since it imports nothing.
fn resolve_builtin(
    graph: &mut Graph,
    authority: Option<&Authority>,
    builtin: &'static Builtin,
    specifier: &str,
    importer: &Url,
    reached: Reached,
) -> Result<ModuleSpecifier, Refused> {
    // the table's URLs parse, which its tests check
    let url = Url::parse(builtin.url).map_err(|error| {
        Refused::new(
            FailureReason::Resolve(ResolveErrorKind::InvalidUrl(UrlFault::Join(error))),
            format!(
                "cannot resolve '{specifier}': its URL {} is invalid: {error}",
                builtin.url
            ),
        )
    })?;
    let Reached::Import(reach) = reached else {
        return Ok(url);
    };
    // `graph.resolve` resolved from it, so the importer is a recorded module
    let Some(origin) = graph.origin(importer).cloned() else {
        return Err(Refused::new(
            FailureReason::UnknownImporter,
            format!("cannot import '{specifier}': the importer '{importer}' was never resolved"),
        ));
    };
    check_grant(authority, builtin, &origin).map_err(|reason| {
        Refused::new(
            FailureReason::NotGranted { builtin, origin },
            format!("cannot import '{specifier}' from {importer}: {reason}"),
        )
    })?;
    graph.admit_builtin(&url, builtin);
    graph.link(importer, specifier, reach, &url);
    Ok(url)
}

/// Whether an importer of `origin` may import `builtin` under `authority`,
/// or why not. With no authority (eval's) nothing is imported, a grant-free
/// builtin included, which is checked before the grant.
fn check_grant(
    authority: Option<&Authority>,
    builtin: &Builtin,
    origin: &Origin,
) -> Result<(), String> {
    let Some(authority) = authority else {
        return Err(builtin.grant.map_or_else(
            || "eval imports no builtin module".to_owned(),
            |capability| {
                format!("it needs the grant '{capability}', and eval imports no builtin module")
            },
        ));
    };
    let Some(capability) = builtin.grant else {
        return Ok(());
    };
    let needs = format!("it needs the grant '{capability}'");
    match (authority, origin) {
        (Authority::Confined(_), Origin::Package { name, .. }) => Err(format!(
            "{needs}, which package '{name}' isn't granted: only the project's own modules are \
             granted host modules"
        )),
        (Authority::Confined(grants), Origin::Project) => {
            if grants.holds(capability) {
                Ok(())
            } else {
                Err(format!("{needs}, which this run doesn't grant"))
            }
        }
    }
}
