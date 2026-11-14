//! Tests for import attributes: an attribute under any key but `type` is
//! refused before anything resolves, for a static import and `import()`,
//! a file and a builtin module alike, ahead of every refusal the specifier
//! would get, after only an `import()`'s in code compiled from a string;
//! `type` on a file is refused when it loads, whatever its value, and no
//! value makes a file load another way.

use super::builtins::{HOP, PROCESS, SELF_LINK, granted, run_files, run_linked};
use crate::loader::graph::{FailureReason, Graph, LoadFailure, Reach, Reached};
use crate::{RunError, RuntimeOptions};

/// The refusal of an attribute under `key`, of `specifier` imported by the
/// project's `importer`, as its message ends.
fn key_refusal(graph: &Graph, specifier: &str, importer: &str, key: &str) -> String {
    format!(
        "cannot import '{specifier}' from file://{}/{importer}: the import attribute '{key}' is \
         not supported; only 'type' is",
        graph.roots().project().dir.display()
    )
}

/// Asserts `graph` recorded a refused attribute, and nothing else, for
/// `specifier` imported by the project's `importer` as `reach`.
fn assert_attribute_failure(graph: &Graph, specifier: &str, importer: &str, reach: Reach) {
    let importer = url::Url::from_file_path(graph.roots().project().dir.join(importer)).unwrap();
    let failures: Vec<_> = graph
        .failures()
        .iter()
        .filter(|(site, _)| {
            site.specifier == specifier
                && site.importer.as_deref() == Some(importer.as_str())
                && site.reached == Reached::Import(reach)
        })
        .flat_map(|(_, failures)| failures)
        .map(|failure| failure.reason.clone())
        .collect();
    assert_eq!(
        failures,
        [FailureReason::Load(LoadFailure::ImportAttributes)],
        "{specifier}: {:?}",
        graph.failures()
    );
}

/// A static import with an attribute under another key fails the run naming
/// the key, the least of several, before the specifier resolves: a file, a
/// missing file, a builtin granted or not, a reserved specifier, and a
/// package's import of the project by name all get the attribute's refusal.
#[test]
fn a_static_import_with_another_key_is_refused_before_it_resolves() {
    let ungranted = RuntimeOptions::default;
    let cases: [(&str, &str, &str, RuntimeOptions); 7] = [
        ("file", "./who.mjs", "{ x: 'y' }", granted_options()),
        ("missing", "./missing.mjs", "{ x: 'y' }", granted_options()),
        ("builtin", PROCESS, "{ x: 'y' }", granted_options()),
        ("ungranted", PROCESS, "{ x: 'y' }", ungranted()),
        (
            "reserved",
            "@fuzdev/gro/process.js",
            "{ x: 'y' }",
            granted_options(),
        ),
        (
            "several",
            "./who.mjs",
            "{ type: 'json', b: '1', a: '2' }",
            granted_options(),
        ),
        (
            "type_first",
            "./who.mjs",
            "{ type: 'json', x: 'y' }",
            granted_options(),
        ),
    ];
    for (case, specifier, attributes, options) in cases {
        let main = format!("import '{specifier}' with {attributes};\n");
        let files = [
            ("main.mjs", main.as_str()),
            ("who.mjs", "export default 'who';\n"),
        ];
        let (result, graph) = run_files(&format!("attributes_key_{case}"), &files, options);
        let Err(RunError::Uncaught(report)) = result else {
            panic!("{case}: {result:?}");
        };
        let key = if case == "several" { "a" } else { "x" };
        assert!(
            report.contains(&key_refusal(&graph, specifier, "main.mjs", key)),
            "{case}: {report}"
        );
        assert_attribute_failure(&graph, specifier, "main.mjs", Reach::Static);
    }
    // a package's import of the project by name, refused for the attribute
    // ahead of the import itself
    let hopstatic = "import { args } from 'self/args.js' with { x: 'y' };\nexport default args;\n";
    let files: Vec<_> = HOP
        .iter()
        .map(|&(path, content)| {
            (
                path,
                if path == "node_modules/pkg/hopstatic.js" {
                    hopstatic
                } else {
                    content
                },
            )
        })
        .chain([("main.mjs", "import 'pkg/hopstatic.js';\n")])
        .collect();
    let (result, graph) = run_linked(
        "attributes_key_project",
        &files,
        &SELF_LINK,
        granted_options(),
    );
    let Err(RunError::Uncaught(report)) = result else {
        panic!("{result:?}");
    };
    let root = &graph.roots().project().dir;
    assert!(
        report.contains(&format!(
            "cannot import 'self/args.js' from file://{}/node_modules/pkg/hopstatic.js: the \
             import attribute 'x' is not supported; only 'type' is",
            root.display()
        )),
        "{report}"
    );
}

/// `fgro run`'s options: `process` granted.
fn granted_options() -> RuntimeOptions {
    RuntimeOptions {
        authority: granted(),
        ..RuntimeOptions::default()
    }
}

/// A static import of a file with `type` is refused when it loads,
/// whatever the value, and one with no attributes loads.
#[test]
fn a_static_import_of_a_file_with_type_is_refused_when_it_loads() {
    for value in ["json", "javascript", "text"] {
        let main = format!("import './who.mjs' with {{ type: '{value}' }};\n");
        let files = [
            ("main.mjs", main.as_str()),
            ("who.mjs", "export default 'who';\n"),
        ];
        let (result, graph) = run_files(
            &format!("attributes_type_{value}"),
            &files,
            granted_options(),
        );
        let Err(RunError::Uncaught(report)) = result else {
            panic!("{value}: {result:?}");
        };
        assert!(
            report.contains(&format!(
                "cannot load file://{}/who.mjs: import attributes are not supported yet",
                graph.roots().project().dir.display()
            )),
            "{value}: {report}"
        );
    }
    let files = [
        (
            "main.mjs",
            "import who from './who.mjs' with {};\nif (who !== 'who') throw new Error(who);\n",
        ),
        ("who.mjs", "export default 'who';\n"),
    ];
    let (result, _) = run_files("attributes_empty", &files, granted_options());
    assert!(result.is_ok(), "{result:?}");
}

/// The same rules for `import()`, each refusal a `TypeError` the program
/// catches: another key before anything resolves, ahead of a missing file,
/// an ungranted builtin, a reserved specifier, and a package's import of
/// the project, but after the refusal of an `import()` in code compiled
/// from a string, which comes first; `type` on a builtin before its grant,
/// on a file when it loads; `type: 'text'` loads no file its extension
/// doesn't; and no attributes, an empty `with`, or `undefined` options
/// import.
#[test]
fn an_import_call_with_attributes_follows_the_same_rules() {
    let main = format!(
        "import {{ hopx }} from 'pkg/hopx.js';
const failures = [];
const refused = async (name, f, expected) => {{
\ttry {{
\t\tawait f();
\t\tfailures.push(`${{name}}: imported`);
\t}} catch (error) {{
\t\tif (!(error instanceof TypeError) || !String(error.message).includes(expected)) failures.push(`${{name}}: ${{error}}`);
\t}}
}};
const key = (k) => `the import attribute '${{k}}' is not supported; only 'type' is`;
await refused('file', () => import('./who.mjs', {{ with: {{ x: 'y' }} }}), key('x'));
await refused('missing file', () => import('./missing.mjs', {{ with: {{ x: 'y' }} }}), key('x'));
await refused('builtin', () => import('{PROCESS}', {{ with: {{ x: 'y' }} }}), key('x'));
await refused('reserved', () => import('@fuzdev/gro/process.js', {{ with: {{ x: 'y' }} }}), key('x'));
await refused('several', () => import('./who.mjs', {{ with: {{ type: 'json', b: '1', a: '2' }} }}), key('a'));
await refused('a package importing the project', () => hopx(), key('x'));
// code compiled from a string is refused first, so its refusal names no importer
await refused('from string code', () => (0, eval)(\"import('./who.mjs', {{ with: {{ x: 'y' }} }})\"), 'from code compiled from a string');
await refused('a builtin from string code', () => (0, eval)(\"import('{PROCESS}', {{ with: {{ type: 'json' }} }})\"), 'from code compiled from a string');
await refused('builtin with type', () => import('{PROCESS}', {{ with: {{ type: 'json' }} }}), 'a builtin module takes no import attributes');
await refused('file with type', () => import('./who.mjs', {{ with: {{ type: 'json' }} }}), 'import attributes are not supported yet');
await refused('text by extension', () => import('./secret.txt', {{ with: {{ type: 'text' }} }}), \"the extension '.txt' does not load as a module\");
await refused('text by name', () => import('./.env', {{ with: {{ type: 'text' }} }}), 'the path has no extension');
await refused('bytes by extension', () => import('./secret.txt', {{ with: {{ type: 'bytes' }} }}), \"the extension '.txt' does not load as a module\");
for (const [name, f] of [
\t['no attributes', () => import('./who.mjs')],
\t['an empty with', () => import('./who.mjs', {{ with: {{}} }})],
\t['undefined options', () => import('./who.mjs', undefined)],
\t['an empty with on a builtin', () => import('{PROCESS}', {{ with: {{}} }})],
]) {{
\ttry {{
\t\tawait f();
\t}} catch (error) {{
\t\tfailures.push(`${{name}}: ${{error}}`);
\t}}
}}
if (failures.length > 0) throw new Error(failures.join('\\n'));
"
    );
    let extra = [
        ("main.mjs", main.as_str()),
        ("who.mjs", "export default 'who';\n"),
        ("secret.txt", "SECRET\n"),
        (".env", "SECRET=1\n"),
        (
            "node_modules/pkg/package.json",
            r#"{"name": "pkg", "type": "module", "exports": {"./hopx.js": "./hopx.js"}}"#,
        ),
        (
            "node_modules/pkg/hopx.js",
            "export const hopx = () => import('self/args.js', { with: { x: 'y' } });\n",
        ),
    ];
    let files: Vec<_> = HOP
        .iter()
        .copied()
        .filter(|(path, _)| !path.starts_with("node_modules/pkg/"))
        .chain(extra)
        .collect();
    let (result, graph) = run_linked("attributes_dynamic", &files, &SELF_LINK, granted_options());
    assert!(result.is_ok(), "{result:?}");
    for specifier in [
        "./who.mjs",
        "./missing.mjs",
        PROCESS,
        "@fuzdev/gro/process.js",
    ] {
        assert_attribute_failure(&graph, specifier, "main.mjs", Reach::Dynamic);
    }
    assert_attribute_failure(
        &graph,
        "self/args.js",
        "node_modules/pkg/hopx.js",
        Reach::Dynamic,
    );
}
