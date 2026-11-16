//! What holds each row's backing script to its template: the one text a
//! row's needs give, so no script reads, decides, or hands on anything the
//! template doesn't. A row hands the realm's `Error` over when it's a row of
//! an error family's, and no other does ([`hands_over_error`]), so there are
//! two templates, with the `Error` and without. A script that departed from
//! it could return the bootstrap object, `core`, or `core.ops`, hand on a
//! copy of one, read another op, or let its inner level escape, and the
//! checks over the implementations' source (the type program's walk, the
//! reviewed lists, the op reads) hold what a script may name, never what it
//! does with it.
//!
//! The text compared is the backing script, what `deno_core` compiles,
//! so the strip and the wrapping are held with the source: each is
//! formatted by tsv as the body of a function of `__bootstrap`, as
//! `deno_core` compiles it, and every line that is blank or a line comment
//! once formatted is dropped. Formatting gives one text of a program
//! however its source is spaced; a line the formatter starts with `//` is
//! a comment, or inside a string, a template, or a block comment whose
//! start is on a line kept, which the template has none of, so a script
//! whose kept lines are the template's runs what the template runs.

use super::{IMPLEMENTATIONS, Implementation, WRAP_CLOSE, WRAP_OPEN, scope_argument};
use crate::host::error_class::hands_over_error;
use crate::resolver::BUILTINS;

/// The template's backing script for `implementation`'s row: the outer
/// level takes the one op, and the realm's `Error` when `hands_over_error`,
/// and the inner returns what the op gives for the scope, and the `Error`,
/// called with the row's scope argument.
fn template(implementation: &Implementation, hands_over_error: bool) -> String {
    let scope = scope_argument(implementation.grant);
    let body = if hands_over_error {
        "const { core, primordials } = bootstrap;
        const { op_gro_namespace } = core.ops;
        const { Error } = primordials;
        return (_scope) => op_gro_namespace(_scope, Error);"
    } else {
        "const { core } = bootstrap;
        const { op_gro_namespace } = core.ops;
        return (_scope) => op_gro_namespace(_scope);"
    };
    format!("{WRAP_OPEN}(function (bootstrap) {{\n{body}\n}})(__bootstrap)({scope}){WRAP_CLOSE}")
}

/// `script`, a backing script, as it's compared: formatted by tsv as the
/// body of a function of `__bootstrap`, each line blank or a line comment
/// dropped. A comment after code on its line, or a block comment, is kept,
/// and departs from the template.
///
/// # Panics
///
/// Panics when tsv can't parse it, naming `specifier`.
fn normalized(specifier: &str, script: &str) -> Vec<String> {
    let function = format!("function backing(__bootstrap) {{\n{script}\n}}\n");
    let formatted =
        tsv_ts::format_with_goal(&function, tsv_ts::Goal::Script).unwrap_or_else(|error| {
            panic!("{specifier}: tsv can't format its backing script: {error}")
        });
    formatted
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .map(ToOwned::to_owned)
        .collect()
}

/// Where `implementation`'s backing script departs from `template`, the
/// first line that differs once both are normalized, or `None` when it's
/// the template.
fn departure(implementation: &Implementation, template: &str) -> Option<String> {
    let specifier = implementation.script;
    let script = super::to_backing_script(implementation)
        .unwrap_or_else(|refusal| panic!("{specifier} makes no backing script: {refusal}"));
    let found = normalized(specifier, &script);
    let expected = normalized(specifier, template);
    let at = (0..found.len().max(expected.len())).find(|&at| found.get(at) != expected.get(at))?;
    let line = |lines: &[String]| lines.get(at).map_or("(no line)", String::as_str).to_owned();
    Some(format!(
        "{specifier}'s backing script departs from the template at its line {} once \
         normalized:\n  the template: {}\n  the script:   {}",
        at + 1,
        line(&expected),
        line(&found),
    ))
}

/// Every row's backing script is the template its row's needs give, so no
/// row's script hands on anything of the bootstrap object, reads another
/// op, or lets its inner level escape, and only a row of an error family's
/// reads the realm's `Error`.
#[test]
fn every_row_s_backing_script_is_its_template() {
    let departures: Vec<String> = BUILTINS
        .iter()
        .zip(IMPLEMENTATIONS)
        .filter_map(|(row, implementation)| {
            departure(
                implementation,
                &template(implementation, hands_over_error(row.specifier)),
            )
        })
        .collect();
    assert!(departures.is_empty(), "{}", departures.join("\n"));
    assert!(BUILTINS.iter().any(|row| hands_over_error(row.specifier)));
    assert!(BUILTINS.iter().any(|row| !hands_over_error(row.specifier)));
}

/// The normalization passes over what isn't the program (types, a comment
/// on its own line, blank lines, spacing) and over nothing that is: a
/// script that returns `core`, hands on `ObjectAssign`'s copy, reads
/// another op, or hides a statement in a template literal's line that
/// starts `//` departs, named with its line.
#[test]
fn a_script_departs_from_the_template_for_its_program_alone() {
    let probe = |source: &'static str| Implementation {
        script: "ext:gro/builtin/probe.ts",
        source,
        grant: None,
    };
    let same = probe(
        "// a comment\n(function (bootstrap: Bootstrap) {\n\t// another\n\tconst { core, \
         primordials } = bootstrap;\n\n\tconst {\n\t\top_gro_namespace,\n\t} = core.ops;\n\tconst \
         { Error } = primordials;\n\t// the realm's\n\tinterface Unused {\n\t\ta: string;\n\t}\n\treturn \
         (_scope: undefined) =>\n\t\top_gro_namespace<'@fuzdev/gro/pid.ts'>(_scope, Error);\n});\n",
    );
    let departs = |source| departure(&probe(source), &template(&probe(source), true));
    assert_eq!(departs(same.source), None);
    // the template without the `Error` is another text
    assert!(departure(&same, &template(&same, false)).is_some());
    for (source, line) in [
        // returns `core` by value
        (
            "(function (bootstrap: Bootstrap) {\n\tconst { core, primordials } = \
             bootstrap;\n\tconst { op_gro_namespace } = core.ops;\n\tconst { Error } = \
             primordials;\n\treturn (_scope: undefined) => core;\n});\n",
            "return (_scope) => core;",
        ),
        // hands on `ObjectAssign`'s copy of the ops
        (
            "(function (bootstrap: Bootstrap) {\n\tconst { core, primordials } = \
             bootstrap;\n\tconst { op_gro_namespace } = core.ops;\n\tconst { Error, ObjectAssign } \
             = primordials;\n\treturn (_scope: undefined) => ObjectAssign({}, core.ops);\n});\n",
            "const { Error, ObjectAssign } = primordials;",
        ),
        // reads another op
        (
            "(function (bootstrap: Bootstrap) {\n\tconst { core, primordials } = \
             bootstrap;\n\tconst { op_gro_namespace, op_gro_deny } = core.ops;\n\tconst { Error } = \
             primordials;\n\treturn (_scope: undefined) => op_gro_namespace(_scope, Error);\n});\n",
            "const { op_gro_namespace, op_gro_deny } = core.ops;",
        ),
        // a template whose line starts `//`, which the normalization drops
        (
            "(function (bootstrap: Bootstrap) {\n\tconst { core, primordials } = \
             bootstrap;\n\tconst { op_gro_namespace } = core.ops;\n\tconst { Error } = \
             primordials;\n\tconst s = `\n// ${(globalThis.core = core)}`;\n\treturn (_scope: \
             undefined) => op_gro_namespace(_scope, Error);\n});\n",
            "const s = `",
        ),
    ] {
        let departure = departs(source).unwrap_or_else(|| panic!("{source} departs nowhere"));
        assert!(departure.contains(line), "{departure}");
    }
}
