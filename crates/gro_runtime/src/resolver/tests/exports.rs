use super::package;
use crate::resolver::exports::{has_invalid_segment, is_array_index, pattern_key_compare};

/// The cases checked against Node's resolver: the URL a specifier resolves
/// to, or the parts of its error's `Debug` form.
#[test]
fn exports_resolve_as_node_does() {
    let t = "file:///w/proj/node_modules/t";
    let cases: &[(&str, Result<String, &[&str]>)] = &[
        ("t", Ok(format!("{t}/main.js"))),
        ("t/up", Err(&["InvalidTarget", "./up", "NotRelative"])),
        ("t/bare", Err(&["InvalidTarget", "NotRelative"])),
        ("t/abs", Err(&["InvalidTarget", "NotRelative"])),
        ("t/https", Err(&["InvalidTarget", "NotRelative"])),
        ("t/dotdot", Err(&["InvalidTarget", "Segment"])),
        ("t/nm", Err(&["InvalidTarget", "Segment"])),
        ("t/encoded", Err(&["InvalidTarget", "Segment"])),
        ("t/encoded_nm", Err(&["InvalidTarget", "Segment"])),
        // Node: a directory import
        ("t/dir", Err(&["Target", "NoExtension"])),
        ("t/dir_slash", Err(&["Target", "NoExtension"])),
        // Node loads it: the extension is mandatory here
        ("t/no_extension", Err(&["Target", "NoExtension"])),
        (
            "t/missing",
            Err(&["Target", "NotFound { ts_sibling: None }"]),
        ),
        ("t/ts", Err(&["Target", "only_ts.ts"])),
        // Node allows empty segments, with a deprecation warning
        ("t/empty_segment", Ok(format!("{t}/lib/a.js"))),
        ("t/fall", Ok(format!("{t}/d.js"))),
        ("t/blocked", Err(&["NotExported", "./blocked"])),
        // Node matches its `node` condition
        (
            "t/unmatched",
            Err(&["NoMatchingCondition", "\"node\", \"require\""]),
        ),
        ("t/numeric", Err(&["InvalidPackageJson", "numeric"])),
        // a two-star key never matches
        ("t/two/a/b", Err(&["NotExported"])),
        ("t/array_invalid_first", Ok(format!("{t}/d.js"))),
        (
            "t/array_missing_first",
            Err(&["Target", "missing.js", "NotFound"]),
        ),
        ("t/array_null_first", Ok(format!("{t}/d.js"))),
        (
            "t/array_invalid_after_null",
            Err(&["InvalidTarget", "NotRelative"]),
        ),
        ("t/array_null_after_invalid", Err(&["NotExported"])),
        (
            "t/array_unmatched",
            Err(&["NoMatchingCondition", "unmatched: [\"node\"]"]),
        ),
        // Node matches `node` and fails on the missing file
        ("t/array_object", Ok(format!("{t}/d.js"))),
        ("t/array_empty", Err(&["NotExported"])),
        ("t/number", Err(&["InvalidTarget", "NotATarget"])),
        ("t/order", Ok(format!("{t}/d.js"))),
        ("t/import", Ok(format!("{t}/b.js"))),
        ("t/types", Ok(format!("{t}/b.js"))),
        ("t/lib/a", Ok(format!("{t}/lib/a.js"))),
        // the longer prefix wins
        ("t/lib/a/b", Ok(format!("{t}/lib/a/b.js"))),
        ("t/star/a.js", Ok(format!("{t}/lib/a.js"))),
        ("t/star/a", Err(&["NotExported"])),
        ("t/everywhere/x", Ok(format!("{t}/lib/x/x.js"))),
        // Node loads it as CommonJS
        ("t/cjs", Err(&["Target", "UnsupportedExtension(\"cjs\")"])),
        ("t/lib/%2F..%2Fa", Err(&["InvalidUrl", "%2F"])),
        ("t/lib/..%2Fa", Err(&["InvalidUrl", "%2F"])),
        ("t/lib/x/../a", Err(&["InvalidSubpath", "x/../a"])),
        ("t/lib/node_modules/q", Err(&["InvalidSubpath"])),
        ("t/lib/%6eode_modules/q", Err(&["InvalidSubpath"])),
        ("t/lib/%2e%2e/a", Err(&["InvalidSubpath"])),
        ("t/pattern/x.js?v=1", Ok(format!("{t}/lib/x.js?v=1"))),
        ("t/pattern/x.js#frag", Ok(format!("{t}/lib/x.js#frag"))),
        ("t/pattern/dir.js", Err(&["Target", "NotAFile"])),
        // Node refuses to strip under node_modules
        ("t/src/s.ts", Ok(format!("{t}/src/s.ts"))),
        ("t/null_pattern/x", Err(&["NotExported"])),
        ("t/trailing/a.js", Err(&["NotExported"])),
        ("t/", Err(&["NotExported"])),
        ("t/lib/", Err(&["NotExported"])),
        ("t/pattern/", Err(&["NotExported"])),
        ("t/unknown", Err(&["NotExported", "./unknown"])),
        // Node: tabs are dropped, and the containment check refuses it
        ("t/tab", Err(&["InvalidTarget", "Segment"])),
        ("t/query_up", Err(&["InvalidTarget", "Outside"])),
        ("t/fragment_up", Err(&["InvalidTarget", "Outside"])),
        // Node loads node_modules/escape_tab.js
        ("t/tabbed/.\t./.\t./escape_tab", Err(&["InvalidSubpath"])),
        ("t/pattern/.\t./.\t./x", Err(&["InvalidSubpath"])),
        // Node: a directory import, of the package's parent
        ("t/dot_star/.?", Err(&["InvalidSubpath", ".?"])),
        (
            "t/empty_conditions",
            Err(&["NotExported", "./empty_conditions"]),
        ),
        ("t/array_empty_conditions", Err(&["NotExported"])),
        (
            "t/nested_unmatched",
            Err(&["NoMatchingCondition", "[\"gro.browser\", \"node\"]"]),
        ),
        (
            "t/nested_array_unmatched",
            Err(&["NoMatchingCondition", "[\"import.node\", \"node\"]"]),
        ),
        (
            "looped",
            Err(&["SymlinkLoop", "/w/proj/node_modules/looped"]),
        ),
        // Node loads it, guessing its format: a relative specifier never reaches node_modules
        (
            "./node_modules/loose.js",
            Err(&["IntoNodeModules", "/w/proj/node_modules/loose.js"]),
        ),
        (
            "sugar_string",
            Ok("file:///w/proj/node_modules/sugar_string/i.mjs".to_owned()),
        ),
        (
            "sugar_array",
            Ok("file:///w/proj/node_modules/sugar_array/i.mjs".to_owned()),
        ),
        (
            "sugar_conditions",
            Ok("file:///w/proj/node_modules/sugar_conditions/i.mjs".to_owned()),
        ),
        ("mixed", Err(&["InvalidPackageJson", "mix"])),
        // Node falls back to `main`
        ("exports_null", Err(&["NoExports"])),
        ("no_exports", Err(&["NoExports"])),
        ("empty_exports", Err(&["NotExported", "exported: []"])),
        ("not_json", Err(&["InvalidPackageJson", "EOF"])),
        (
            "not_object",
            Err(&["InvalidPackageJson", "not a JSON object"]),
        ),
        // Node loads its index.js, and neither looks further up
        (
            "no_package",
            Err(&["NoPackageJson", "/w/proj/node_modules/no_package"]),
        ),
        // Node loads it as CommonJS
        (
            "cjs",
            Err(&["Target", "NotModuleType", "/cjs/package.json"]),
        ),
        (
            "@scope/pkg",
            Ok("file:///w/proj/node_modules/@scope/pkg/i.mjs".to_owned()),
        ),
        (
            "@scope/pkg/x.mjs",
            Ok("file:///w/proj/node_modules/@scope/pkg/x.mjs".to_owned()),
        ),
        ("@scope/pkg/y.mjs", Err(&["NotExported", "./y.mjs"])),
        ("linked", Ok("file:///w/linked/i.js".to_owned())),
        (
            "nested",
            Ok(
                "file:///w/proj/node_modules/.deno/nested@1.0.0/node_modules/nested/i.mjs"
                    .to_owned(),
            ),
        ),
        // the importer's own package, not the stale copy in node_modules
        ("proj/self.js", Ok("file:///w/proj/self.js".to_owned())),
        // Node reaches the workspace root's node_modules
        (
            "parent_only",
            Err(&[
                "PackageNotFound",
                "package_json: \"/w/proj/package.json\"",
                "above: Some(\"/w/package.json\")",
            ]),
        ),
        ("fs", Err(&["PackageNotFound", "\"fs\""])),
        (".x", Err(&["InvalidPackageName"])),
        ("@scope", Err(&["InvalidPackageName"])),
        ("@scope/", Err(&["InvalidPackageName"])),
        ("@scope/..", Err(&["InvalidPackageName"])),
        ("a%20b", Err(&["InvalidPackageName"])),
        ("a\\b", Err(&["InvalidPackageName"])),
        // Node guesses the format from the code
        (
            "./no_type/a.js",
            Err(&["NotModuleType", "/w/proj/no_type/package.json"]),
        ),
        // Node loads them: they're outside the project
        (
            "../../loose/a.js",
            Err(&["OutsideRoot", "/loose/a.js", "root: \"/w/proj\""]),
        ),
        (
            "../../loose/b.mjs",
            Err(&["OutsideRoot", "/loose/b.mjs", "root: \"/w/proj\""]),
        ),
        (
            "./name_number/a.js",
            Err(&["InvalidPackageJson", "\\\"name\\\" is not a string"]),
        ),
        (
            "./type_number/a.js",
            Err(&["InvalidPackageJson", "\\\"type\\\" is not a string"]),
        ),
        (
            "name_null",
            Err(&[
                "InvalidPackageJson",
                "name_null/package.json",
                "\\\"name\\\"",
            ]),
        ),
        ("name_array", Err(&["InvalidPackageJson", "\\\"name\\\""])),
        ("type_array", Err(&["InvalidPackageJson", "\\\"type\\\""])),
        (
            "./name_empty/a.js",
            Ok("file:///w/proj/name_empty/a.js".to_owned()),
        ),
        (
            "loose_fields",
            Ok("file:///w/proj/node_modules/loose_fields/i.mjs".to_owned()),
        ),
    ];
    for (specifier, expected) in cases {
        match (package(specifier), expected) {
            (Ok(resolution), Ok(url)) => {
                assert_eq!(resolution.url.as_str(), url, "{specifier}");
            }
            (Err(kind), Err(parts)) => {
                let debug = format!("{kind:?}");
                for part in *parts {
                    assert!(debug.contains(part), "{specifier}: {debug}");
                }
            }
            (actual, expected) => panic!("{specifier}: {actual:?}, expected {expected:?}"),
        }
    }
}

#[test]
fn pattern_keys_compare_as_node_does() {
    use std::cmp::Ordering;
    assert_eq!(pattern_key_compare("", "./*"), Ordering::Greater);
    assert_eq!(pattern_key_compare("./*", "./a/*"), Ordering::Greater);
    assert_eq!(pattern_key_compare("./a/*", "./*"), Ordering::Less);
    // the same prefix: the longer trailer wins
    assert_eq!(pattern_key_compare("./*", "./*.js"), Ordering::Greater);
    assert_eq!(pattern_key_compare("./*.js", "./*.js"), Ordering::Equal);
}

#[test]
fn segments() {
    for path in [
        ".",
        "..",
        "a/./b",
        "a/../b",
        "%2e",
        "%2E%2e",
        "node_modules",
        "NODE_MODULES",
    ] {
        assert!(has_invalid_segment(path), "{path}");
    }
    for path in [
        "a/b",
        "",
        "a//b",
        "...",
        ".a",
        "%2e%2e%2e",
        "node_modules_x",
        "%2",
    ] {
        assert!(!has_invalid_segment(path), "{path}");
    }
    assert!(has_invalid_segment("a\\..\\b"));
    // tabs and line breaks are removed first, as URL parsing removes them
    for path in [".\t.", "a/.\n./b", "\r.", "%2e\t%2e", "node\t_modules"] {
        assert!(has_invalid_segment(path), "{path:?}");
    }
    assert!(is_array_index("0") && is_array_index("12"));
    assert!(!is_array_index("01") && !is_array_index("-1") && !is_array_index("4294967295"));
    assert!(!is_array_index("") && !is_array_index("a1"));
}
