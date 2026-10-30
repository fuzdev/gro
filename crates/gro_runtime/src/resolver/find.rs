//! Finding the real module file a resolved URL names, and the extension suggestions.

use std::path::{Path, PathBuf};

use super::error::{ResolveErrorKind, SUGGESTED_EXTENSIONS};
use super::format::module_format;
use super::tree::{Entry, FileId, SourceTree};
use super::walk::{Consulted, Walk, lookup, package_scope};

/// Checks the extension, finds the real file, checks it with `check` (its
/// containment) before anything about it is read, and checks that it loads: its format
/// by name, and a `.js` file's `"type"`. Returns its real path and its
/// identity. `hint` is the specifier for the messages' suggestions, unset
/// for a package's target, whose messages name the path instead.
pub(super) fn find_module(
    tree: &impl SourceTree,
    path: &Path,
    hint: Option<&str>,
    consulted: &mut Consulted,
    check: impl FnOnce(&Path) -> Result<(), ResolveErrorKind>,
) -> Result<(PathBuf, FileId), ResolveErrorKind> {
    let trailing_slash = path.as_os_str().as_encoded_bytes().ends_with(b"/");

    if path.extension().is_none() || trailing_slash {
        let mut suggestion = None;
        if let Some(hint) = hint
            && !trailing_slash
        {
            for extension in SUGGESTED_EXTENSIONS {
                if let Some(Entry::File { .. }) = lookup(tree, &path.with_extension(extension))? {
                    suggestion = Some(with_extension(hint, extension));
                    break;
                }
            }
        }
        return Err(ResolveErrorKind::NoExtension { suggestion });
    }

    let (real_path, id) = match lookup(tree, path)? {
        Some(Entry::File { path, id }) => (path, id),
        Some(Entry::Dir(_) | Entry::Other) => return Err(ResolveErrorKind::NotAFile),
        None => {
            let ts_extension = match path.extension().and_then(|extension| extension.to_str()) {
                Some("js") => Some("ts"),
                Some("mjs") => Some("mts"),
                _ => None,
            };
            let ts_sibling = if let Some(ts_extension) = ts_extension
                && let Some(Entry::File { .. }) = lookup(tree, &path.with_extension(ts_extension))?
            {
                Some(hint.map_or_else(
                    || path.with_extension(ts_extension).display().to_string(),
                    |hint| replace_extension(hint, ts_extension),
                ))
            } else {
                None
            };
            return Err(ResolveErrorKind::NotFound { ts_sibling });
        }
    };

    check(&real_path)?;
    // format by the real file's name, which a symlink can change, as the loader reads it
    module_format(&real_path)?;
    if real_path
        .extension()
        .is_some_and(|extension| extension == "js")
    {
        let dir = real_path.parent().unwrap_or(&real_path);
        match package_scope(tree, dir, consulted)? {
            Walk::Found(scope) if scope.string("type") == Some("module") => {}
            Walk::Found(scope) => {
                return Err(ResolveErrorKind::NotModuleType {
                    package_json: scope.path,
                });
            }
            Walk::Ended => return Err(ResolveErrorKind::NoTypeScope { stopped_at: None }),
            Walk::Stopped(stopped_at) => {
                return Err(ResolveErrorKind::NoTypeScope {
                    stopped_at: Some(stopped_at),
                });
            }
        }
    }
    Ok((real_path, id))
}

/// Splits a specifier into its path and its query-and-fragment suffix.
fn split_suffix(specifier: &str) -> (&str, &str) {
    specifier
        .find(['?', '#'])
        .map_or((specifier, ""), |index| specifier.split_at(index))
}

/// The specifier with `.{extension}` appended to its path.
fn with_extension(specifier: &str, extension: &str) -> String {
    let (path, suffix) = split_suffix(specifier);
    format!("{path}.{extension}{suffix}")
}

/// The specifier with its path's last extension replaced by `extension`.
fn replace_extension(specifier: &str, extension: &str) -> String {
    let (path, suffix) = split_suffix(specifier);
    let stem = path.rsplit_once('.').map_or(path, |(stem, _)| stem);
    format!("{stem}.{extension}{suffix}")
}
