//! What a module file's name says about how it loads.

use std::path::Path;

use super::error::ResolveErrorKind;

/// How a module's source becomes the JS V8 evaluates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleFormat {
    /// `.js` and `.mjs`: evaluated as written.
    JavaScript,
    /// `.ts` and `.mts`: stripped of their types first, every surviving
    /// character kept at its line and column.
    TypeScript,
}

/// The format of the module file at `path`, by its name: the one table both
/// resolution and loading read.
///
/// The compound names (`.d.ts`, `.svelte.ts`, `.svelte.test.ts`) are checked
/// before the last extension. A `.js` file's `"type"` is checked at
/// resolution, not here.
///
/// # Errors
///
/// Returns the reason the file doesn't load as a module.
pub fn module_format(path: &Path) -> Result<ModuleFormat, ResolveErrorKind> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    if [".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        return Err(ResolveErrorKind::Declaration);
    }
    if is_svelte_module(&name) {
        return Err(ResolveErrorKind::SvelteModule);
    }
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy())
        .unwrap_or_default();
    match &*extension {
        "js" | "mjs" => Ok(ModuleFormat::JavaScript),
        "ts" | "mts" => Ok(ModuleFormat::TypeScript),
        "cts" => Err(ResolveErrorKind::CommonJs),
        "tsx" => Err(ResolveErrorKind::Jsx),
        _ => Err(ResolveErrorKind::UnsupportedExtension(
            extension.into_owned(),
        )),
    }
}

/// Whether a file named `name` is a Svelte module by vite-plugin-svelte's
/// default rule, applied to the file name (the query already stripped):
/// `.svelte.`, then any non-empty segments, then `js` or `ts`. vite's regex
/// also stops at `?`, `#`, and `\`, which are query and separator characters
/// in an id but plain file-name characters here, so such names are refused.
fn is_svelte_module(name: &str) -> bool {
    const INFIX: &str = ".svelte.";
    name.match_indices(INFIX).any(|(index, _)| {
        let mut segments = name[index + INFIX.len()..].split('.');
        matches!(segments.next_back(), Some("js" | "ts"))
            && segments.all(|segment| !segment.is_empty())
    })
}
