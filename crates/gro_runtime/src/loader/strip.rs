//! Stripping TypeScript's types in process, and the message a refused strip
//! fails the load with. Free of `deno_core` types.

use std::borrow::Cow;

use tsv_ts::{ErrorKind, ErrorReport, Goal, StripError, UnsupportedKind};
use url::Url;

/// Strips the types from `source`, the TypeScript module at `url`, returning
/// the JS, or the message the load fails with: what was wrong, a fix for a
/// refused construct, and the position V8's frames would print,
/// `\n    at <url>:<line>:<column>`.
///
/// Source with nothing type-only comes back as the same `String`, uncopied.
///
/// The strip recurses with the source's nesting, on the caller's stack, which
/// must hold tsv's contract (`tsv_ts::STACK_SIZE`; see `run_main`). Input
/// past tsv's depth limits fails the load rather than overflowing it.
pub(super) fn strip_types(source: String, url: &Url) -> Result<String, String> {
    // a module goal only: V8 evaluates every module as one, so no script retry
    let stripped = match tsv_ts::strip_with_goal(&source, Goal::Module) {
        Ok(stripped) => match stripped.code {
            Cow::Borrowed(_) => None,
            Cow::Owned(code) => Some(code),
        },
        Err(error) => return Err(strip_error_message(&error, &source, url)),
    };
    Ok(stripped.unwrap_or(source))
}

/// The message of a failed strip: the summary (a parse error's says it's
/// one, and a depth limit's that it's a limit), a fix for a refusal, and the
/// position as a V8 frame prints it (1-based line and UTF-16 column).
fn strip_error_message(error: &StripError, source: &str, url: &Url) -> String {
    const PARSE_PREFIX: &str = "cannot parse the TypeScript: ";
    const STRIP_PREFIX: &str = "cannot strip the TypeScript: ";
    let (prefix, fix) = match error {
        // a depth limit is a bound on the input, not a verdict on its grammar
        StripError::Parse(parse) if parse.kind() == ErrorKind::TooDeep => (STRIP_PREFIX, None),
        StripError::Parse(_) => (PARSE_PREFIX, None),
        // the summary is the construct's own `cannot strip ...`
        StripError::Unsupported { kind, .. } => ("", fix(*kind)),
        // a failure a later tsv adds, until it gets an arm: tsv's own report
        // of it, never `StripError::message`'s caret frame, which no other load
        // error prints
        _ => (STRIP_PREFIX, None),
    };
    // no report, so no `at` line: a source past the 4 GiB cap, or a later
    // failure tsv reports nothing for, printed by its one-line `Display`
    let Some(ErrorReport { summary, start, .. }) = error.report(source) else {
        return format!("{prefix}{error}");
    };
    let mut message = format!("{prefix}{summary}");
    if let Some(Fix { how, erasable }) = fix {
        message.push_str(". Fix: ");
        message.push_str(how);
        if erasable {
            message.push_str(" (TypeScript's erasableSyntaxOnly option reports these)");
        }
    }
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

/// The fix for a refused construct, if gro knows one: a construct a later tsv
/// refuses has none until it gets an arm.
const fn fix(kind: UnsupportedKind) -> Option<Fix> {
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
        _ => return None,
    };
    Some(Fix { how, erasable })
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::strip_types;

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
    fn a_depth_limit_says_it_is_one_and_has_its_position() {
        let nested = format!("const a = {}1{};", "(".repeat(255), ")".repeat(255));
        let chained = format!("const a = 1{};", " + 1".repeat(7000));
        // input up to the limits needs tsv's stack in a debug build, past the
        // harness's 2 MiB threads
        let (nested, message) =
            tsv_ts::run_with_stack(|| (strip(&nested), strip(&chained))).unwrap();
        assert_eq!(
            nested.unwrap_err(),
            "cannot strip the TypeScript: Nesting exceeds tsv's depth limit of 256 levels\n    \
             at file:///p/a.ts:1:266"
        );
        let message = message.unwrap_err();
        assert!(
            message.starts_with(
                "cannot strip the TypeScript: Chain of operators, member accesses, calls, \
                 assertions or type suffixes exceeds tsv's length limit\n    at file:///p/a.ts:1:"
            ),
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
