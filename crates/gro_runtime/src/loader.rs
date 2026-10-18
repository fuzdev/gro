//! The `deno_core` module loader: a thin adapter over [`crate::resolver`] that
//! reads the resolved modules from disk, stripping TypeScript's types in
//! process.
//!
//! Static imports, `import()`, and `import.meta.resolve` all reach
//! [`resolver::resolve`] through `ModuleLoader::resolve`. The entry module is
//! resolved before the runtime starts ([`resolver::resolve_entry`]), so its
//! URL arrives here already final. The loader keeps the [`Origin`] each URL
//! first resolved with, since the resolver takes the importer's origin as an
//! input rather than reading it from the path.
//!
//! A TypeScript module is stripped by `tsv_ts`, which blanks every type-only
//! range rather than rewriting the code, so every surviving character keeps
//! its line and column and V8's positions are the source's without a
//! sourcemap. TypeScript with runtime semantics (an `enum`, a parameter
//! property) is refused, not transformed: the load fails naming the construct,
//! a fix, and its position, in the `at <url>:<line>:<column>` form V8's frames
//! print. Like every load failure, it reaches JS as a `TypeError`, which
//! `import()` can catch.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use deno_core::error::ModuleLoaderError;
use deno_core::{
    ModuleLoadOptions, ModuleLoadReferrer, ModuleLoadResponse, ModuleLoader, ModuleSource,
    ModuleSourceCode, ModuleSpecifier, ModuleType, RequestedModuleType, ResolutionKind,
};
use tsv_ts::{ErrorReport, Goal, StripError, UnsupportedKind};
use url::Url;

use crate::resolver::{self, DiskTree, ModuleFormat, Origin, Resolution};

/// Resolves through the strict resolver over the disk, and loads `.js` and
/// `.mjs` files as ES modules, and `.ts` and `.mts` files as ES modules
/// stripped of their types.
#[derive(Debug)]
pub struct Loader {
    /// The origin each URL first resolved with, the entry's included: what
    /// the module's own imports resolve with. Not necessarily the origin its
    /// module instance loaded under, since a URL can resolve again, from
    /// another importer, with another origin.
    origins: RefCell<HashMap<Url, Origin>>,
}

impl Loader {
    /// A loader for the graph of `entry`, already resolved.
    pub fn new(entry: &Resolution) -> Self {
        Self {
            origins: RefCell::new(HashMap::from([(entry.url.clone(), entry.origin.clone())])),
        }
    }
}

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
        // every importer was resolved here first, the entry by `Loader::new`
        let importer_origin = self
            .origins
            .borrow()
            .get(&importer)
            .cloned()
            .ok_or_else(|| {
                ModuleLoaderError::type_error(format!(
                    "cannot resolve '{specifier}': the importer '{referrer}' was never resolved"
                ))
            })?;
        let resolution = resolver::resolve(&DiskTree, specifier, &importer, &importer_origin)
            .map_err(|error| ModuleLoaderError::type_error(error.to_string()))?;
        // the first origin a URL resolves with stays, whatever resolves it later: an
        // `import.meta.resolve` records one before any module instance exists, and a
        // project's relative import of a file inside node_modules records `Project`
        // TODO: settle the rule before sandboxed eval grants by origin: on a conflict keep the
        // less-trusted origin, or record origins only when a module loads
        self.origins
            .borrow_mut()
            .entry(resolution.url.clone())
            .or_insert(resolution.origin);
        Ok(resolution.url)
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

/// Reads the module at `specifier`, a URL the resolver produced, stripping a
/// TypeScript module's types.
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
    // the resolver read the same table, so a refusal here means the URL skipped it
    let format = resolver::module_format(&path).map_err(|kind| {
        ModuleLoaderError::type_error(format!("cannot load {specifier}: {kind}"))
    })?;
    let code = std::fs::read(&path)
        .map_err(|error| ModuleLoaderError::generic(format!("cannot load {specifier}: {error}")))?;
    let code = match format {
        ModuleFormat::JavaScript => ModuleSourceCode::Bytes(code.into_boxed_slice().into()),
        ModuleFormat::TypeScript => {
            // invalid UTF-8 reads as U+FFFD, as V8 decodes a JS module's bytes
            let source = String::from_utf8(code)
                .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned());
            // the class `deno_core` drops: it rethrows every load failure to JS as a
            // `TypeError` (`js_class_and_message_to_exception`)
            let code = strip_types(source, specifier)
                .map_err(|message| ModuleLoaderError::new("SyntaxError", message))?;
            ModuleSourceCode::String(code.into())
        }
    };
    Ok(ModuleSource::new(
        ModuleType::JavaScript,
        code,
        specifier,
        None,
    ))
}

/// Strips the types from `source`, the TypeScript module at `url`, returning
/// the JS, or the message the load fails with: what was wrong, a fix for a
/// refused construct, and the position V8's frames would print,
/// `\n    at <url>:<line>:<column>`.
///
/// Source with nothing type-only comes back as the same `String`, uncopied.
///
/// The strip recurses with the source's nesting, on the caller's stack, so
/// the thread `run_main` runs on sets the depth it reaches (see its docs).
fn strip_types(source: String, url: &Url) -> Result<String, String> {
    // one arena per module, freed when the strip returns
    let arena = bumpalo::Bump::new();
    // TODO: nesting past the thread's stack overflows it and aborts the process,
    // since `tsv_ts` has no depth guard; one in tsv that returns a parse error
    // closes it, as V8's checked limit does for JS
    // a module goal only: V8 evaluates every module as one, so no script retry
    let stripped = match tsv_ts::strip_with_goal(&source, Some(Goal::Module), &arena) {
        Ok(stripped) => match stripped.code {
            Cow::Borrowed(_) => None,
            Cow::Owned(code) => Some(code),
        },
        Err(error) => return Err(strip_error_message(&error, &source, url)),
    };
    Ok(stripped.unwrap_or(source))
}

/// The message of a failed strip: the summary (a parse error's says it's
/// one), a fix for a refusal, and the position as a V8 frame prints it
/// (1-based line and UTF-16 column).
fn strip_error_message(error: &StripError, source: &str, url: &Url) -> String {
    const PARSE_PREFIX: &str = "cannot parse the TypeScript: ";
    let (report, fix) = match error {
        StripError::Parse(error) => (error.report(), None),
        StripError::Unsupported { kind, span } => {
            (Some(kind.report(*span, source)), Some(fix(*kind)))
        }
    };
    // a parse error has no report only past the 4 GiB source cap, so no position
    let Some(ErrorReport { summary, start, .. }) = report else {
        return format!("{PARSE_PREFIX}{error}");
    };
    let message = match fix {
        None => format!("{PARSE_PREFIX}{summary}"),
        Some(Fix { how, erasable }) => {
            let mut message = summary;
            message.push_str(". Fix: ");
            message.push_str(how);
            if erasable {
                message.push_str(" (TypeScript's erasableSyntaxOnly option reports these)");
            }
            message
        }
    };
    format!(
        "{message}\n    at {url}:{}:{}",
        start.line,
        start.column + 1
    )
}

/// How to write a refused construct as TypeScript whose types strip away.
#[derive(Debug, Clone, Copy)]
struct Fix {
    /// The rewrite.
    how: &'static str,
    /// Whether TypeScript's `erasableSyntaxOnly` option reports the construct,
    /// so a project can catch the rest of them before running.
    erasable: bool,
}

/// The fix for a refused construct.
const fn fix(kind: UnsupportedKind) -> Fix {
    let (how, erasable) = match kind {
        UnsupportedKind::Enum => (
            "use a union of literal types, or an object with 'as const'",
            true,
        ),
        UnsupportedKind::ValueNamespace => ("use the module itself, or a plain object", true),
        UnsupportedKind::ParameterProperty => (
            "declare the field in the class body and assign it in the constructor",
            true,
        ),
        UnsupportedKind::ImportEquals => ("use an import declaration", true),
        UnsupportedKind::ExportEquals => ("use 'export default'", true),
        UnsupportedKind::TypeAssertion => ("write the assertion as 'x as T'", false),
        UnsupportedKind::AssertionGrouping => {
            ("wrap the assertion in parentheses: '(x as T)'", false)
        }
    };
    Fix { how, erasable }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url() -> Url {
        Url::parse("file:///p/a.ts").unwrap()
    }

    fn strip(source: &str) -> Result<String, String> {
        strip_types(source.to_owned(), &url())
    }

    #[test]
    fn types_blank_in_place() {
        assert_eq!(
            strip("const a: string = 'x';\nexport type T = 1;\nf<T>(a as T);").unwrap(),
            "const a         = 'x';\n                  \nf   (a     );"
        );
    }

    #[test]
    fn source_with_nothing_to_strip_is_not_copied() {
        let source = "export const a = 1;\n".to_owned();
        let pointer = source.as_ptr();
        let code = strip_types(source, &url()).unwrap();
        assert_eq!(code.as_ptr(), pointer);
    }

    #[test]
    fn a_refusal_names_the_construct_its_fix_and_its_position() {
        assert_eq!(
            strip("const a = 1;\n  enum E { A }\n").unwrap_err(),
            "cannot strip an enum. Fix: use a union of literal types, or an object with 'as \
             const' (TypeScript's erasableSyntaxOnly option reports these)\n    at \
             file:///p/a.ts:2:3"
        );
        assert!(
            strip("class C { constructor(private a: number) {} }")
                .unwrap_err()
                .starts_with("cannot strip a parameter property. Fix: declare the field")
        );
    }

    #[test]
    fn a_parse_error_says_so_and_has_its_position() {
        let message = strip("const a: = 1;").unwrap_err();
        assert!(
            message.starts_with("cannot parse the TypeScript: "),
            "{message}"
        );
        assert!(
            message.ends_with("\n    at file:///p/a.ts:1:10"),
            "{message}"
        );
        assert_eq!(message.lines().count(), 2, "{message}");
    }

    #[test]
    fn only_erasable_refusals_name_the_option() {
        let message = strip("const a = -x as number ** 2;").unwrap_err();
        assert!(
            message.starts_with("cannot strip ")
                && message.contains("Fix: wrap the assertion in parentheses: '(x as T)'\n"),
            "{message}"
        );
        assert!(!message.contains("erasableSyntaxOnly"), "{message}");
        let message = strip("namespace N { export const a = 1; }").unwrap_err();
        assert_eq!(
            message.matches("erasableSyntaxOnly").count(),
            1,
            "{message}"
        );
    }

    #[test]
    fn columns_count_utf16_units() {
        // the astral character is two UTF-16 units, as V8 counts columns
        assert!(
            strip("const s = '\u{1F600}'; enum E {}")
                .unwrap_err()
                .ends_with(":1:17"),
        );
    }

    #[test]
    fn the_goal_is_a_module() {
        // `with` is a sloppy script's; a module refuses it
        assert!(strip("with (a) {}").is_err());
    }
}
