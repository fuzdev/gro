//! A package's `"exports"` and `"imports"`, Node's `PACKAGE_EXPORTS_RESOLVE`
//! and `PACKAGE_IMPORTS_RESOLVE`, over parsed JSON.

use std::path::Path;

use serde_json::{Map, Value};
use url::Url;

use super::CONDITIONS;
use super::error::{MapField, ResolveErrorKind, TargetFault, UrlFault};
use super::package::parse_package_name;

/// What a target in `"exports"` resolved to.
pub(super) enum Target {
    /// A URL, not yet checked to exist.
    Url(Url),
    /// `null`, or an empty array: not exported, and no fallback.
    Null,
    /// A conditional object none of whose conditions matched, so its parent
    /// tries its next key.
    Unmatched,
}

/// The package and the key a target is resolved for.
struct TargetContext<'a> {
    /// Which field the key is in.
    field: MapField,
    /// The directory of the `package.json`, as a URL.
    root: &'a Url,
    /// The key that matched.
    key: &'a str,
    /// What a pattern key's `*` matched.
    matched: Option<&'a str>,
    /// The package's `package.json`, for errors.
    package_json: &'a Path,
}

/// Resolves `subpath` through a package's `"exports"`, Node's
/// `PACKAGE_EXPORTS_RESOLVE`.
pub(super) fn resolve_exports(
    root: &Path,
    package_json: &Path,
    exports: &Value,
    subpath: &str,
) -> Result<Url, ResolveErrorKind> {
    // a string, an array, or an object of conditions alone is the main entry
    let entries: Vec<(&str, &Value)> = match exports {
        Value::String(_) | Value::Array(_) => vec![(".", exports)],
        Value::Object(map) => {
            let mut conditional = None;
            for key in map.keys() {
                let is_condition = key.is_empty() || !key.starts_with('.');
                if *conditional.get_or_insert(is_condition) != is_condition {
                    return Err(ResolveErrorKind::InvalidPackageJson {
                        path: package_json.to_owned(),
                        reason: "\"exports\" cannot mix keys starting with '.' with keys that \
                                 don't"
                            .to_owned(),
                    });
                }
            }
            if conditional == Some(true) {
                vec![(".", exports)]
            } else {
                map.iter()
                    .map(|(key, value)| (key.as_str(), value))
                    .collect()
            }
        }
        _ => Vec::new(),
    };
    let not_exported = || ResolveErrorKind::NotExported {
        subpath: subpath.to_owned(),
        package_json: package_json.to_owned(),
        exported: entries.iter().map(|(key, _)| (*key).to_owned()).collect(),
    };
    let Some(selected) = select_entry(&entries, subpath) else {
        return Err(not_exported());
    };
    resolve_selected(MapField::Exports, root, package_json, selected, subpath)?
        .ok_or_else(not_exported)
}

/// What a `#` specifier's `"imports"` mapping names.
pub(super) struct Import {
    /// The target's URL, not yet checked to exist.
    pub(super) url: Url,
    /// Whether the specifier's text after the key's `*` ends the target's
    /// path, as in `"#lib/*": "./src/lib/*"`, so a message can suggest a
    /// specifier for a file that exists.
    pub(super) ends_in_match: bool,
}

/// Resolves the `#` specifier `specifier` through the `"imports"` of the
/// `package.json` at `package_json`, in `root`, Node's
/// `PACKAGE_IMPORTS_RESOLVE` once the package scope is found. Only a
/// target that is a path starting with `./` resolves, with no empty
/// segment; one naming a package or a builtin is refused.
pub(super) fn resolve_imports(
    root: &Path,
    package_json: &Path,
    imports: &Map<String, Value>,
    specifier: &str,
) -> Result<Import, ResolveErrorKind> {
    let entries: Vec<(&str, &Value)> = imports
        .iter()
        .map(|(key, value)| (key.as_str(), value))
        .collect();
    let not_defined = || ResolveErrorKind::ImportNotDefined {
        package_json: package_json.to_owned(),
        defined: Some(entries.iter().map(|(key, _)| (*key).to_owned()).collect()),
    };
    let Some(selected @ (key, target, _)) = select_entry(&entries, specifier) else {
        return Err(not_defined());
    };
    let ends_in_match =
        key.ends_with('*') && target.as_str().is_some_and(|target| target.ends_with('*'));
    let url = resolve_selected(MapField::Imports, root, package_json, selected, specifier)?
        .ok_or_else(not_defined)?;
    Ok(Import { url, ends_in_match })
}

/// Resolves `subpath` through the entry of `field` it selected, its key, its
/// target, and what a pattern key's `*` matched: the URL the target names,
/// or `None` when the target is `null` or holds no conditions.
fn resolve_selected(
    field: MapField,
    root: &Path,
    package_json: &Path,
    (key, target, matched): (&str, &Value, Option<&str>),
    subpath: &str,
) -> Result<Option<Url>, ResolveErrorKind> {
    let root = Url::from_directory_path(root)
        .map_err(|()| ResolveErrorKind::InvalidUrl(UrlFault::NoLocalPath))?;
    let context = TargetContext {
        field,
        root: &root,
        key,
        matched,
        package_json,
    };
    match resolve_target(&context, target)? {
        Target::Url(url) => Ok(Some(url)),
        Target::Null => Ok(None),
        Target::Unmatched => {
            let mut unmatched = Vec::new();
            unmatched_conditions(target, "", &mut unmatched);
            // no conditions at all (`{}`) is no mapping, as Node words it
            if unmatched.is_empty() {
                return Ok(None);
            }
            Err(ResolveErrorKind::NoMatchingCondition {
                field,
                subpath: subpath.to_owned(),
                unmatched,
            })
        }
    }
}

/// Collects the conditions in `target` that didn't match, each once, in the
/// package's order: a key outside [`CONDITIONS`], or, under a key in it, its
/// own unmatched conditions, written after `prefix` and the key and a dot.
fn unmatched_conditions(target: &Value, prefix: &str, unmatched: &mut Vec<String>) {
    match target {
        Value::Array(targets) => {
            for target in targets {
                unmatched_conditions(target, prefix, unmatched);
            }
        }
        Value::Object(conditions) => {
            for (condition, target) in conditions {
                if CONDITIONS.contains(&condition.as_str()) {
                    unmatched_conditions(target, &format!("{prefix}{condition}."), unmatched);
                } else {
                    let condition = format!("{prefix}{condition}");
                    if !unmatched.contains(&condition) {
                        unmatched.push(condition);
                    }
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// The entry `subpath` selects, with what a pattern key's `*` matched: the
/// exact key, unless the subpath has a `*` or ends in `/`, else the most
/// specific single-`*` pattern by [`pattern_key_compare`]. An exact key is
/// never passed over for a pattern, even when its target is `null`.
fn select_entry<'a>(
    entries: &[(&'a str, &'a Value)],
    subpath: &'a str,
) -> Option<(&'a str, &'a Value, Option<&'a str>)> {
    if !subpath.contains('*')
        && !subpath.ends_with('/')
        && let Some(&(key, target)) = entries.iter().find(|(key, _)| *key == subpath)
    {
        return Some((key, target, None));
    }
    let mut best: Option<(&str, &Value, &str)> = None;
    for &(key, target) in entries {
        let Some(star) = key.find('*') else {
            continue;
        };
        let (prefix, trailer) = (&key[..star], &key[star + 1..]);
        if key.rfind('*') == Some(star)
            && subpath.len() >= key.len()
            && subpath.starts_with(prefix)
            && subpath.ends_with(trailer)
            && best.is_none_or(|(best, _, _)| pattern_key_compare(best, key).is_gt())
        {
            best = Some((key, target, &subpath[star..subpath.len() - trailer.len()]));
        }
    }
    best.map(|(key, target, matched)| (key, target, Some(matched)))
}

/// Node's `PATTERN_KEY_COMPARE`: `Greater` when `b` is the more specific
/// pattern, by the length of its prefix up to and including `*`, then by its
/// whole length.
pub(super) fn pattern_key_compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let base_length = |key: &str| key.find('*').map_or(key.len(), |index| index + 1);
    match base_length(b).cmp(&base_length(a)) {
        Ordering::Equal => {}
        ordering => return ordering,
    }
    if !a.contains('*') {
        return Ordering::Greater;
    }
    if !b.contains('*') {
        return Ordering::Less;
    }
    b.len().cmp(&a.len())
}

/// Resolves one target, Node's `PACKAGE_TARGET_RESOLVE`. File existence
/// isn't checked, so an array falls back past an invalid target, never past
/// a missing file, nor past an `"imports"` target naming a package or
/// nothing, which Node resolves or refuses as a bare specifier. An
/// `"imports"` target with an empty segment is refused wherever an array
/// reaches it, never fallen back past.
fn resolve_target(context: &TargetContext<'_>, target: &Value) -> Result<Target, ResolveErrorKind> {
    match target {
        Value::String(target) => resolve_target_string(context, target).map(Target::Url),
        Value::Array(targets) => {
            // as Node: the last invalid target is the error, unless a `null` came after it
            let mut last = None;
            for target in targets {
                match resolve_target(context, target) {
                    Ok(Target::Url(url)) => return Ok(Target::Url(url)),
                    Ok(Target::Unmatched) => {}
                    Ok(Target::Null) => last = Some(Ok(())),
                    Err(
                        error @ ResolveErrorKind::InvalidTarget {
                            reason:
                                TargetFault::NotRelative
                                | TargetFault::Segment
                                | TargetFault::Outside
                                | TargetFault::NotATarget,
                            ..
                        },
                    ) => last = Some(Err(error)),
                    Err(error) => return Err(error),
                }
            }
            match last {
                None if !targets.is_empty() => Ok(Target::Unmatched),
                None | Some(Ok(())) => Ok(Target::Null),
                Some(Err(error)) => Err(error),
            }
        }
        Value::Object(conditions) => {
            if conditions.keys().any(|key| is_array_index(key)) {
                return Err(ResolveErrorKind::InvalidPackageJson {
                    path: context.package_json.to_owned(),
                    reason: format!("\"{}\" cannot have numeric keys", context.field),
                });
            }
            for (condition, target) in conditions {
                if CONDITIONS.contains(&condition.as_str()) {
                    match resolve_target(context, target)? {
                        Target::Unmatched => {}
                        resolved => return Ok(resolved),
                    }
                }
            }
            Ok(Target::Unmatched)
        }
        Value::Null => Ok(Target::Null),
        Value::Bool(_) | Value::Number(_) => Err(ResolveErrorKind::InvalidTarget {
            field: context.field,
            key: context.key.to_owned(),
            target: target.to_string(),
            reason: TargetFault::NotATarget,
        }),
    }
}

/// Whether `key` is an array index as Node's resolver counts one, which a
/// conditional object can't have: the canonical string of a number (what
/// `String(Number(key))` gives back unchanged) from 0 up to, not including,
/// `0xFFFF_FFFF`. Node counts a non-integer (`1.5`, `1e-7`) too, past the
/// spec's integer array indices, so such a key is refused as invalid
/// package config, as in Node.
pub(super) fn is_array_index(key: &str) -> bool {
    // a canonical number string parses the same in Rust as in JS; any other
    // string's canonical form differs from it, whatever each parses it to
    let Ok(number) = key.parse::<f64>() else {
        return false;
    };
    (0.0..f64::from(0xFFFF_FFFF_u32)).contains(&number) && js_number_string(number) == key
}

/// JS's `Number.prototype.toString` of `number`, a finite number from 0 up
/// to `0xFFFF_FFFF`: `0` for either zero, positional digits from `1e-6` up,
/// and below that the shortest digits in exponent form (`1.5e-7`), which
/// Rust's `{:e}` writes as JS does.
fn js_number_string(number: f64) -> String {
    if number == 0.0 {
        "0".to_owned()
    } else if number < 1e-6 {
        format!("{number:e}")
    } else {
        format!("{number}")
    }
}

/// Resolves a string target, refusing one that could leave the package:
/// it must start with `./`, and neither it nor what a pattern's `*` matched
/// may have a `.`, `..`, or `node_modules` segment, nor, in `"imports"`,
/// the two together. An `"imports"` target may have no empty segment
/// either, and one Node would resolve as a bare specifier (not `../`, `/`,
/// or a URL) is refused as the package or builtin module it names, a
/// pattern's match substituted, or as naming nothing when that isn't a
/// package name or starts with `#`.
fn resolve_target_string(
    context: &TargetContext<'_>,
    target: &str,
) -> Result<Url, ResolveErrorKind> {
    let invalid = |reason| ResolveErrorKind::InvalidTarget {
        field: context.field,
        key: context.key.to_owned(),
        target: Value::from(target).to_string(),
        reason,
    };
    let Some(rest) = target.strip_prefix("./") else {
        if context.field == MapField::Imports
            && !target.starts_with("../")
            && !target.starts_with('/')
            && Url::parse(target).is_err()
        {
            let specifier = context
                .matched
                .map_or_else(|| target.to_owned(), |matched| target.replace('*', matched));
            if specifier.starts_with('#') || parse_package_name(&specifier).is_err() {
                return Err(invalid(TargetFault::NamesNothing));
            }
            return Err(ResolveErrorKind::ImportsPackage {
                key: context.key.to_owned(),
                target: Value::from(target).to_string().into(),
                specifier: specifier.into(),
            });
        }
        return Err(invalid(TargetFault::NotRelative));
    };
    if has_invalid_segment(rest) {
        return Err(invalid(TargetFault::Segment));
    }
    if context.field == MapField::Imports && has_empty_segment(rest) {
        return Err(invalid(TargetFault::EmptySegment));
    }
    let inside = |url: &Url| url.path().starts_with(context.root.path());
    let unsubstituted = context
        .root
        .join(target)
        .map_err(|error| ResolveErrorKind::InvalidUrl(UrlFault::Join(error)))?;
    if !inside(&unsubstituted) {
        return Err(invalid(TargetFault::Outside));
    }
    let Some(matched) = context.matched else {
        return Ok(unsubstituted);
    };
    let invalid_subpath = || ResolveErrorKind::InvalidSubpath {
        field: context.field,
        key: context.key.to_owned(),
        matched: matched.to_owned(),
    };
    // a match spells no `.`, `..`, or `node_modules` segment, alone or, in
    // `"imports"`, with the target around it (`modules` into `./node_*`)
    if has_invalid_segment(matched)
        || (context.field == MapField::Imports && has_invalid_segment(&rest.replace('*', matched)))
    {
        return Err(invalid_subpath());
    }
    // every `*` in the target, substituted before parsing, so a `?` or `#` in
    // the match is the URL's query or fragment, as in Node
    let url = context
        .root
        .join(&target.replace('*', matched))
        .map_err(|error| ResolveErrorKind::InvalidUrl(UrlFault::Join(error)))?;
    // the target alone stayed inside, so the match is what left (`.?` after a `.*`);
    // and an empty segment the target doesn't have is the match's, a `/` at
    // its edge beside one in the target (`x/` into `*/index.js`, as a subpath
    // with a trailing `/` makes, which Node deprecates) or a `//` in it
    if !inside(&url) || empty_segments(&url) > empty_segments(&unsubstituted) {
        return Err(invalid_subpath());
    }
    Ok(url)
}

/// How many empty segments `url`'s path has before its last, as URL parsing
/// leaves them (a `\` read as `/`, tabs and line breaks removed). An empty
/// last segment, a trailing `/`, is left to the file lookup, which refuses
/// it as a directory.
fn empty_segments(url: &Url) -> usize {
    let mut segments: Vec<&str> = url.path().split('/').skip(1).collect();
    segments.pop();
    segments.iter().filter(|segment| segment.is_empty()).count()
}

/// Whether `path` has a `.`, `..`, or `node_modules` segment (any case),
/// split at `/` and `\`, its percent-encoded forms included, the segments
/// Node's `invalidSegmentRegEx` refuses. Tabs and line breaks are removed
/// first, as URL parsing removes them, so `.\t.` is `..`; Node's check
/// misses that form, and only its containment check catches it, for a
/// target and not for a pattern's match. Empty segments pass, as in Node.
pub(super) fn has_invalid_segment(path: &str) -> bool {
    let path: String = path
        .chars()
        .filter(|char| !matches!(char, '\t' | '\n' | '\r'))
        .collect();
    path.split(['/', '\\']).any(|segment| {
        let decoded = percent_decode(segment);
        decoded == b"." || decoded == b".." || decoded.eq_ignore_ascii_case(b"node_modules")
    })
}

/// Whether `path` has an empty segment, a trailing `/` included, split at
/// `/` and `\\` after removing tabs and line breaks, as URL parsing reads
/// it.
fn has_empty_segment(path: &str) -> bool {
    let path: String = path
        .chars()
        .filter(|char| !matches!(char, '\t' | '\n' | '\r'))
        .collect();
    path.split(['/', '\\']).any(str::is_empty)
}

/// `segment` with each `%XX` escape decoded to its byte.
fn percent_decode(segment: &str) -> Vec<u8> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = bytes
            .get(index + 1..index + 3)
            .filter(|_| bytes[index] == b'%')
            .and_then(|hex| u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok());
        if let Some(byte) = escape {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    decoded
}
