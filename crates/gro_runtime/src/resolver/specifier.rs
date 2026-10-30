//! Parsing a specifier into a request: a URL against the importer, or a package name.

use std::path::PathBuf;

use url::Url;

use super::error::ResolveErrorKind;

/// A parsed specifier.
pub(super) enum Request<'a> {
    /// A relative or absolute specifier, parsed against the importer.
    Url(Url),
    /// A bare specifier, which names a package.
    Package(&'a str),
}

/// Parses a specifier, refusing every form but relative and absolute `file:`
/// ones and bare ones.
pub(super) fn parse_specifier<'a>(
    specifier: &'a str,
    importer: &Url,
) -> Result<Request<'a>, ResolveErrorKind> {
    // `.` and `..` alone are relative too, as in Node, so they fail as directories
    if specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with('/')
        || specifier == "."
        || specifier == ".."
    {
        return importer
            .join(specifier)
            .map(Request::Url)
            .map_err(|error| ResolveErrorKind::InvalidUrl(error.to_string()));
    }
    if let Ok(url) = Url::parse(specifier) {
        return match url.scheme() {
            "file" => Ok(Request::Url(url)),
            "node" => Err(ResolveErrorKind::NodeBuiltin),
            "npm" | "jsr" => Err(ResolveErrorKind::Registry),
            "http" | "https" | "data" => Err(ResolveErrorKind::Remote),
            scheme => Err(ResolveErrorKind::UnsupportedScheme(scheme.to_owned())),
        };
    }
    if specifier.is_empty() {
        return Err(ResolveErrorKind::Empty);
    }
    if specifier.starts_with('#') {
        return Err(ResolveErrorKind::PackageImport);
    }
    Ok(Request::Package(specifier))
}

/// The local path a resolved URL names, refusing what Node refuses in one: an
/// encoded `/` or `\`, which would decode to a separator the URL never had,
/// and an encoded NUL, which no path holds.
pub(super) fn url_to_path(url: &Url) -> Result<PathBuf, ResolveErrorKind> {
    let encoded = url.path().to_ascii_lowercase();
    if encoded.contains("%2f") || encoded.contains("%5c") {
        return Err(ResolveErrorKind::InvalidUrl(format!(
            "the path {} has an encoded '/' or '\\' (%2F or %5C), which never names a separator",
            url.path()
        )));
    }
    if encoded.contains("%00") {
        return Err(ResolveErrorKind::InvalidUrl(format!(
            "the path {} has an encoded NUL (%00)",
            url.path()
        )));
    }
    url.to_file_path()
        .map_err(|()| ResolveErrorKind::InvalidUrl("names no local path".to_owned()))
}
