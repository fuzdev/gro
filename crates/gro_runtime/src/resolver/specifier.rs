//! Parsing a specifier into a request: a URL against the importer, or a package name.

use std::path::PathBuf;

use url::Url;

use super::error::{ResolveErrorKind, UrlFault};
use super::exports::has_invalid_segment;
use super::package::parse_package_name;

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
            .map_err(|error| ResolveErrorKind::InvalidUrl(UrlFault::Join(error)));
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
        return Err(ResolveErrorKind::InvalidUrl(UrlFault::EncodedSeparator(
            url.path().to_owned(),
        )));
    }
    if encoded.contains("%00") {
        return Err(ResolveErrorKind::InvalidUrl(UrlFault::EncodedNul(
            url.path().to_owned(),
        )));
    }
    url.to_file_path()
        .map_err(|()| ResolveErrorKind::InvalidUrl(UrlFault::NoLocalPath))
}

/// The bare `specifier` as its subpath's path spells it, when that differs
/// from it: the subpath parsed as a URL, as a package's `"exports"` target
/// is with a pattern's match last in it (`"./*"`), which cuts a query or
/// fragment, removes tabs and line breaks, reads `\` as `/`, and strips
/// trailing spaces and control characters, then decoded into a path as
/// [`url_to_path`] decodes every resolved URL. A specifier the resolver
/// refuses either way (an invalid package name, a `.`, `..`, or
/// `node_modules` segment, an encoded separator or NUL) has none.
pub(super) fn to_plain_spelling(specifier: &str) -> Option<String> {
    // bytes both steps keep as they are, dot segments aside, which the lookup refuses
    let kept = |byte: u8| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b'@')
    };
    if specifier.bytes().all(kept) {
        return None;
    }
    let (name, subpath) = parse_package_name(specifier).ok()?;
    if has_invalid_segment(subpath.strip_prefix("./")?) {
        return None;
    }
    let url = Url::parse("file:///").ok()?.join(&subpath).ok()?;
    let path = url_to_path(&url).ok()?;
    let plain = format!("{name}{}", path.to_str()?);
    (plain != specifier).then_some(plain)
}
