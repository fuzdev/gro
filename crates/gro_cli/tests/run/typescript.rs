//! TypeScript runs stripped: positions, type-only imports, tsv's depth limits and
//! the byte cap, and unloadable sources.

use crate::{fixtures_url, run, run_in, temp_fixtures};

#[test]
fn typescript_import_runs_stripped() {
    let run = run("import_ts.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "typed\n");
}

#[test]
fn typescript_entry_runs() {
    let run = run("smoke.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(run.stdout.ends_with("\nstripped\n"), "{}", run.stdout);
}

/// Interfaces, generics, `as`, `satisfies`, and `import type` (of a module
/// that doesn't exist, which is never resolved) all blank away, and a `.mts`
/// import loads.
#[test]
fn typescript_entry_runs_its_type_only_constructs() {
    let run = run("entry.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "9 typed undefined\n");
}

/// Blanking keeps every character where it was, so V8's positions are the
/// source's: `new` follows a stripped annotation on its line.
#[test]
fn typescript_positions_are_the_source_positions() {
    let run = run("throw.ts");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: boom\n    at {}/throw.ts:2:22\n",
            fixtures_url()
        )
    );
}

/// Columns count UTF-16 units, V8's and the strip's alike.
#[test]
fn typescript_columns_count_utf16_units() {
    let run = run("throw_astral.ts");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: astral\n    at {}/throw_astral.ts:2:38\n",
            fixtures_url()
        )
    );
}

/// TypeScript with runtime semantics is refused, not transformed: a program
/// error (exit 1) naming the construct, the fix, and its position.
#[test]
fn an_enum_is_refused_with_its_position() {
    let run = run("enum.ts");
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot strip an enum. Fix: use a union of literal types, or an object with \
             'as const' (TypeScript's erasableSyntaxOnly option reports these)\n    at \
             {}/enum.ts:2:1\n",
            fixtures_url()
        )
    );
}

#[test]
fn a_typescript_parse_error_has_its_position() {
    let fixtures = temp_fixtures(
        "parse_error",
        &[
            ("parse_error.ts", "const a = 1;\nconst b: = 2;\n"),
            ("import_parse_error.js", "import './parse_error.ts';\n"),
        ],
    );
    let url = format!("file://{}", fixtures.0.display());
    for entry in ["parse_error.ts", "import_parse_error.js"] {
        let run = run_in(&fixtures, entry);
        assert_eq!(run.code, 1, "{entry}");
        assert!(
            run.stderr
                .starts_with("error: cannot parse the TypeScript: "),
            "{}",
            run.stderr
        );
        assert!(
            run.stderr
                .ends_with(&format!("\n    at {url}/parse_error.ts:2:10\n")),
            "{}",
            run.stderr
        );
        assert_eq!(run.stderr.lines().count(), 2, "{}", run.stderr);
    }
}

/// `import {type T}` keeps its import, so the module's side effects run and
/// a missing one is an error, while `import type`, `export type {…} from`, and
/// `export type * from` drop entirely, as TypeScript's `verbatimModuleSyntax`
/// has it.
#[test]
fn type_only_imports_drop_and_inline_type_specifiers_keep_the_import() {
    let fixtures = temp_fixtures(
        "type_imports",
        &[
            (
                "side.ts",
                "export type T = number;\nconsole.log('side effect');\n",
            ),
            (
                "keeps.ts",
                "import { type T } from './side.ts';\nconst a: T = 1;\nconsole.log('kept', a);\n",
            ),
            (
                "keeps_missing.ts",
                "import { type T } from './missing.ts';\nlet a: T;\nconsole.log('ran');\n",
            ),
            (
                "drops.ts",
                "import type { T } from './missing.ts';\n\
                 export type { U } from './missing.ts';\n\
                 export type * from './missing.ts';\n\
                 export type * as types from './missing.ts';\n\
                 let a: T;\nconsole.log('dropped');\n",
            ),
        ],
    );
    let run = run_in(&fixtures, "keeps.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "side effect\nkept 1\n");

    let run = run_in(&fixtures, "keeps_missing.ts");
    assert_eq!(run.code, 1, "{}", run.stdout);
    assert!(run.stdout.is_empty(), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("error: cannot resolve './missing.ts' from ")
            && run.stderr.contains("no such file"),
        "{}",
        run.stderr
    );

    let run = run_in(&fixtures, "drops.ts");
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(run.stdout, "dropped\n");
}

/// `n` levels of nested parens around a `1`, logged.
fn nested_parens(n: usize) -> String {
    format!(
        "const x = {}1{};\nconsole.log('ok', x);\n",
        "(".repeat(n),
        ")".repeat(n)
    )
}

/// `n` binary operators chained after a `1`, logged.
fn chained_operators(n: usize) -> String {
    format!("const x = 1{};\nconsole.log('ok', x);\n", " + 1".repeat(n))
}

/// A `.ts` module nested or chained past tsv's depth limits fails to load
/// with a message naming the limit, an error exit rather than a stack
/// overflow, though V8, whose limit is far deeper, runs the same `.js`.
#[test]
fn typescript_past_tsv_depth_limits_fails_to_load() {
    let nested = nested_parens(255);
    let chained = chained_operators(7000);
    let fixtures = temp_fixtures(
        "deep_limit",
        &[
            ("deep.ts", &nested),
            ("deep.js", &nested),
            ("chain.ts", &chained),
            ("chain.js", &chained),
        ],
    );
    let url = format!("file://{}", fixtures.0.display());
    let run = run_in(&fixtures, "deep.ts");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot strip the TypeScript: Nesting exceeds tsv's depth limit of 256 \
             wire levels\n    at {url}/deep.ts:1:266\n"
        )
    );
    let run = run_in(&fixtures, "chain.ts");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    // the column is where tsv's weighted count trips, so only the file is pinned
    assert!(
        run.stderr.starts_with(&format!(
            "error: cannot strip the TypeScript: Chain of operators, member accesses, calls, \
             assertions or type suffixes exceeds tsv's length limit\n    at {url}/chain.ts:1:"
        )),
        "{}",
        run.stderr
    );
    assert_eq!(run.stderr.lines().count(), 2, "{}", run.stderr);
    for (file, out) in [("deep.js", "ok 1\n"), ("chain.js", "ok 7001\n")] {
        let run = run_in(&fixtures, file);
        assert_eq!(run.code, 0, "{file}: {}", run.stderr);
        assert_eq!(run.stdout, out, "{file}");
    }
}

/// A module's source is capped as eval's is, since the loader is shared: one
/// at 16 MiB runs, and one a byte over fails to load naming the cap and the
/// file, before its syntax error is parsed.
#[test]
fn a_module_past_its_byte_cap_fails_to_load() {
    // `gro_runtime::resolver::MAX_MODULE_SIZE`, which gro_runtime pins; a
    // literal keeps V8 out of this test binary
    let cap: usize = 16 << 20;
    let padded = |first: &str, size: usize| {
        let mut source = format!("{first}\n//");
        source.push_str(&"x".repeat(size - source.len()));
        source
    };
    let at = padded("console.log('at the cap');", cap);
    let over = padded("console.log() 'over';", cap + 1);
    let fixtures = temp_fixtures("module_cap", &[("at.js", &at), ("over.js", &over)]);
    let run = run_in(&fixtures, "at.js");
    assert_eq!(
        (run.code, run.stdout.as_str()),
        (0, "at the cap\n"),
        "{}",
        run.stderr
    );
    let run = run_in(&fixtures, "over.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert!(run.stdout.is_empty());
    let url = format!("file://{}/over.js", fixtures.0.display());
    assert!(
        run.stderr.starts_with(&format!(
            "error: cannot load {url}: the module is larger than 16 MiB, the cap on a module's \
             source"
        )),
        "{}",
        run.stderr
    );
}

/// Within tsv's depth limits, deep TypeScript strips and runs as the same JS
/// does, on `fgro`'s sized thread whatever the build profile.
#[test]
fn deep_typescript_within_the_limits_runs_as_the_same_js_does() {
    // the deepest parens the nesting limit admits, the statement taking a level
    let nested = nested_parens(254);
    let chained = chained_operators(5000);
    let fixtures = temp_fixtures(
        "deep",
        &[
            ("deep.ts", &nested),
            ("deep.js", &nested),
            ("chain.ts", &chained),
            ("chain.js", &chained),
        ],
    );
    for (name, out) in [("deep", "ok 1\n"), ("chain", "ok 5001\n")] {
        let ts = run_in(&fixtures, &format!("{name}.ts"));
        let js = run_in(&fixtures, &format!("{name}.js"));
        assert_eq!(ts.code, 0, "{name}: {}", ts.stderr);
        assert_eq!(js.code, 0, "{name}: {}", js.stderr);
        assert_eq!(ts.stdout, out, "{name}");
        assert_eq!(js.stdout, out, "{name}");
        assert!(ts.stderr.is_empty() && js.stderr.is_empty(), "{name}");
    }
}

/// V8 keeps its own checked limit on the sized thread, so unbounded JS
/// recursion is a `RangeError` the program can catch, and an uncaught one an
/// error exit, not a crash.
#[test]
fn deep_js_recursion_is_a_catchable_range_error() {
    let run = run("deep_recursion.js");
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(run.stdout, "true Maximum call stack size exceeded\n");
    assert!(
        run.stderr.starts_with(
            "error: Uncaught (in promise) RangeError: Maximum call stack size exceeded\n"
        ),
        "{}",
        run.stderr
    );
}

/// The TypeScript that can't load by stripping is refused at resolution, each
/// kind with its own reason: a usage error as an entry, a catchable one as an
/// import.
#[test]
fn unloadable_typescript_is_refused_by_kind() {
    let refused = [
        (
            "common.cts",
            "a .cts file is CommonJS, and only ES modules load",
        ),
        ("view.tsx", "JSX is not supported"),
        ("types.d.ts", "a declaration file holds only types"),
        ("types.d.mts", "a declaration file holds only types"),
        ("types.d.cts", "a declaration file holds only types"),
        ("state.svelte.ts", "needs the Svelte compiler"),
        ("state.svelte.js", "needs the Svelte compiler"),
        ("state.svelte.test.ts", "needs the Svelte compiler"),
    ];
    let main = format!(
        "for (const specifier of {:?}) {{\n\
         \ttry {{ await import('./' + specifier); }}\n\
         \tcatch (error) {{ console.log(String(error)); }}\n}}\n",
        refused.map(|(file, _)| file)
    );
    let mut files: Vec<(&str, &str)> = refused
        .iter()
        .map(|&(file, _)| (file, "export {};\n"))
        .collect();
    files.push(("main.js", &main));
    let fixtures = temp_fixtures("refused", &files);

    for (file, reason) in refused {
        let run = run_in(&fixtures, file);
        assert_eq!(run.code, 2, "{file}: {}", run.stderr);
        assert!(
            run.stderr
                .starts_with(&format!("error: cannot resolve '{file}' from ")),
            "{}",
            run.stderr
        );
        assert!(run.stderr.contains(reason), "{file}: {}", run.stderr);
    }
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), refused.len(), "{}", run.stdout);
    for (line, (file, reason)) in lines.iter().zip(refused) {
        assert!(
            line.starts_with(&format!("TypeError: cannot resolve './{file}' from ")),
            "{line}"
        );
        assert!(line.contains(reason), "{line}");
    }
}
