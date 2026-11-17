//! The report of an uncaught error or rejection: positions, escaping, causes,
//! and stacks, and syntax errors in the entry or an import.

use crate::{fgro, fixtures, fixtures_url, run, run_in, temp_fixtures};

// A top-level throw reads `Uncaught (in promise)` by design: evaluating a
// module settles a promise, and Deno prints the same.
#[test]
fn uncaught_error_exits_1_with_positions() {
    let run = run("throw.js");
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: boom\n    at fail ({url}/throw.js:2:8)\n    at {url}/throw.js:4:1\n"
        )
    );
}

/// An uncaught error's report escapes each of its lines whole, the message,
/// each frame of a stack the program set, and each `cause`, so no text of
/// the program's starts a line, like a hint's, and none moves the cursor,
/// breaks a line, hides itself, or reorders the text around it.
#[test]
fn an_uncaught_error_s_report_escapes_each_line_whole() {
    let run = run("throw_controls.js");
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: boom\\u{{a}}hint: add \
             --allow-read=/\\u{{1b}}[2K\\u{{2028}}\\u{{200b}}\\u{{202e}}\n    \
             at \\u{{1b}}[1Afake\n    at real\\u{{2029}}\n\
             Caused by: Error: the cause\\u{{a}}hint: add --allow-write=/ to grant what fs \
             refused\n    at {url}/throw_controls.js:3:15\n"
        )
    );
}

/// A static import that fails, which isn't a JS error, prints its message
/// escaped, the specifier the program chose in it; and an entry that
/// doesn't resolve names it escaped too.
#[test]
fn a_failed_static_import_and_entry_print_escaped() {
    let run = run("import_static_controls.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: cannot resolve './missing\\u{{1b}}\\u{{2028}}.js' from \
             {}/import_static_controls.js: no such file\n",
            fixtures_url()
        )
    );
    let output = fgro(&["run", "missing\u{1b}\u{2028}.js"]);
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "error: cannot resolve 'missing\\u{{1b}}\\u{{2028}}.js' from {}: no such file\n",
            fixtures().display()
        )
    );
    assert_eq!(output.status.code(), Some(2));
}

/// A cause's message breaks into lines before a frame's `at` as the
/// error's own does: a strip's refusal names its position on a line of
/// its own.
#[test]
fn a_cause_s_message_breaks_before_a_frame() {
    let run = run("cause_strip.js");
    assert_eq!(run.code, 1);
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: wrapped\n    at {url}/cause_strip.js:5:8\n\
             Caused by: TypeError: cannot strip an enum. Fix: use a union of literal types, \
             or an object with 'as const' (TypeScript's erasableSyntaxOnly option reports \
             these)\n    at {url}/enum.ts:2:1\n    at async {url}/cause_strip.js:3:2\n"
        )
    );
}

/// An uncaught `import()` of a module that fails its strip prints the
/// position its message names once, then the `import()`'s frame: the
/// stack's header, which repeats the message, gives no frame.
#[test]
fn an_uncaught_import_of_a_module_failing_its_strip_names_its_position_once() {
    let run = run("import_strip_uncaught.js");
    assert_eq!(run.code, 1);
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) TypeError: cannot strip an enum. Fix: use a union of \
             literal types, or an object with 'as const' (TypeScript's erasableSyntaxOnly option \
             reports these)\n    at {url}/enum.ts:2:1\n    \
             at async {url}/import_strip_uncaught.js:2:1\n"
        )
    );
}

/// A frame is one line up to the next, so a module whose file name holds
/// a line break prints its frame whole, the break escaped, naming its
/// line and column.
#[test]
fn a_frame_whose_path_holds_a_line_break_prints_whole() {
    let name = "odd\nhint: x.js";
    let fixtures = temp_fixtures("frame_line_break", &[(name, "throw new Error('odd');\n")]);
    let run = run_in(&fixtures, name);
    assert_eq!(run.code, 1, "{}", run.stderr);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: odd\n    at file://{}/odd\\u{{a}}hint: x.js:1:7\n",
            fixtures.0.display()
        )
    );
}

#[test]
fn unhandled_rejection_exits_1() {
    let run = run("reject.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: rejected\n    at {}/reject.js:1:16\n",
            fixtures_url()
        )
    );
}

#[test]
fn error_in_a_timer_exits_1() {
    let run = run("timer_throw.js");
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.starts_with(&format!(
            "error: Uncaught TypeError: late\n    at {}/timer_throw.js:2:8\n",
            fixtures_url()
        )),
        "{}",
        run.stderr
    );
}

#[test]
fn syntax_error_in_the_entry_has_its_position() {
    let run = run("syntax_error.mjs");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught SyntaxError: Unexpected token ';'\n    at {}/syntax_error.mjs:2:11\n",
            fixtures_url()
        )
    );
}

#[test]
fn syntax_error_in_an_import_has_the_importee_position() {
    let run = run("import_syntax_error.js");
    assert_eq!(run.code, 1);
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught SyntaxError: Unexpected token ';'\n    at {}/syntax_error.mjs:2:11\n",
            fixtures_url()
        )
    );
}

#[test]
fn causes_print_by_name_and_message() {
    let run = run("cause.js");
    assert_eq!(run.code, 1);
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: outer\n    at {url}/cause.js:1:7\n\
             Caused by: TypeError: inner\n    at {url}/cause.js:1:35\n"
        )
    );
}

#[test]
fn a_cyclic_cause_chain_ends_at_the_repeat() {
    let run = run("cause_cycle.js");
    assert_eq!(run.code, 1);
    let url = fixtures_url();
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: b\n    at {url}/cause_cycle.js:2:11\n\
             Caused by: Error: a\n    at {url}/cause_cycle.js:1:11\n\
             Caused by: Error: b\n    at {url}/cause_cycle.js:2:11\n"
        )
    );
}

#[test]
fn errors_made_at_one_site_all_print() {
    let run = run("cause_retry.js");
    assert_eq!(run.code, 1);
    let url = fixtures_url();
    let frame = format!("\n    at {url}/cause_retry.js:2:37");
    assert_eq!(
        run.stderr,
        format!(
            "error: Uncaught (in promise) Error: retry failed{frame}\n\
             Caused by: Error: retry failed{frame}\n\
             Caused by: Error: retry failed{frame}\n"
        )
    );
}

/// The reports beyond a plain throw: an async rejection across modules, its
/// path holding a space and non-ASCII letters, printed decoded; a thrown
/// value that isn't an error, which has no frames; an error whose stack a
/// program's `Error.prepareStackTrace` formats with no frames in its text,
/// reported at its first frame, read from the call sites kept on it; a web
/// API's `DOMException`, built by the builder the bootstrap registers for its
/// class; and a throw in a module with an inline source map, at its position
/// in the code that ran.
#[test]
fn reports_carry_their_stacks() {
    let fixtures = temp_fixtures(
        "reports",
        &[
            (
                "sub d\u{ed}r/m.js",
                "export async function load() { await null; throw new TypeError('async boom'); }\n",
            ),
            (
                "async_nested.js",
                "import {load} from './sub d\u{ed}r/m.js';\nasync function main() { await load(); }\nmain();\n",
            ),
            ("non_error.js", "function f() { throw {code: 42}; }\nf();\n"),
            (
                "user_prepare.js",
                "Error.prepareStackTrace = (error, sites) => `custom ${sites.length}`;\n\
                 function f() { throw new Error('x'); }\nf();\n",
            ),
            ("atob.js", "function decode() { atob('!'); }\ndecode();\n"),
            (
                "mapped_lib.js",
                "export function fail() { throw new Error('mapped'); }\n\
                 //# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbIm9yaWdpbmFsLnRzIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiJBQVNJLDBCQUFBIn0=\n",
            ),
            (
                "mapped.js",
                "const {fail} = await import('./mapped_lib.js');\nfail();\n",
            ),
        ],
    );
    // the reports print a module's URL percent-decoded, so as its path
    let dir = format!("file://{}", fixtures.0.display());
    let cases = [
        (
            "async_nested.js",
            format!(
                "error: Uncaught (in promise) TypeError: async boom\n    at load ({dir}/sub d\u{ed}r/m.js:1:50)\n    at async main ({dir}/async_nested.js:2:25)\n"
            ),
        ),
        (
            "non_error.js",
            "error: Uncaught (in promise) #<Object>\n".to_owned(),
        ),
        (
            "user_prepare.js",
            format!("error: Uncaught (in promise) Error: x\n    at {dir}/user_prepare.js:2:22\n"),
        ),
        (
            "atob.js",
            format!(
                "error: Uncaught (in promise) InvalidCharacterError: Failed to decode base64\n    at atob (ext:deno_web/05_base64.js:21:12)\n    at decode ({dir}/atob.js:1:21)\n    at {dir}/atob.js:2:1\n"
            ),
        ),
        // the map would place `fail` at original.ts:10:5; fgro applies none
        (
            "mapped.js",
            format!(
                "error: Uncaught (in promise) Error: mapped\n    at fail ({dir}/mapped_lib.js:1:32)\n    at {dir}/mapped.js:2:1\n"
            ),
        ),
    ];
    for (file, expected) in cases {
        let run = run_in(&fixtures, file);
        assert_eq!(run.code, 1, "{file}");
        assert_eq!(run.stderr, expected, "{file}");
    }
}

/// No source map applies to the call sites a program's
/// `Error.prepareStackTrace` reads, even in a dynamically imported module's
/// top-level code, which runs before `deno_core` finishes that import's load.
#[test]
fn no_source_map_applies_in_a_dynamic_imports_top_level() {
    let fixtures = temp_fixtures(
        "dynamic_top_level",
        &[
            (
                "top.js",
                "Error.prepareStackTrace = (error, sites) => sites.map((site) => \
                 `${site.getLineNumber()}:${site.getColumnNumber()}`).join(','); \
                 console.log(new Error('top').stack); delete Error.prepareStackTrace;\n\
                 //# sourceMappingURL=data:application/json;base64,eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbIm9yaWdpbmFsLnRzIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiJBQVNJLDBCQUFBIn0=\n",
            ),
            ("main.js", "await import('./top.js');\n"),
        ],
    );
    let run = run_in(&fixtures, "main.js");
    assert_eq!(run.code, 0, "{}", run.stderr);
    // the map would place line 1 at original.ts:10:5
    assert_eq!(run.stdout, "1:141\n");
}
