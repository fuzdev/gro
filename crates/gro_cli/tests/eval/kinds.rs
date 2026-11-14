//! The failure kinds of an envelope: `link`, `module_threw`, `export_missing`,
//! `serialize`, late work, and the hints that name a fix.

use serde_json::json;

use crate::support::Scratch;
use crate::{driven, eval, fixtures_url, spawn_eval};

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_static_import_that_does_not_resolve_is_a_resolve_exiting_1() {
    let run = eval(&["import_missing.js"]);
    run.assert_failed("resolve", 1);
    assert_eq!(run.envelope["error"]["specifier"], "./missing.js");
    assert_eq!(
        run.envelope["error"]["importer"],
        format!("{}/import_missing.js", fixtures_url())
    );
}

/// Eval imports no builtin module, so a host module's import is refused at link
/// time, naming the grant, as a `resolve`; with an import attribute it is
/// refused first, as a `link`, as a file's attribute is.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_host_module_import_is_a_resolve_naming_the_grant_or_a_link_with_attributes() {
    let run = eval(&["import_args.js"]);
    run.assert_failed("resolve", 1);
    let importer = format!("{}/import_args.js", fixtures_url());
    assert_eq!(
        run.message(),
        format!(
            "cannot import '@fuzdev/gro/args.ts' from {importer}: it needs the grant 'args', \
             and eval imports no builtin module"
        )
    );
    assert_eq!(run.envelope["error"]["specifier"], "@fuzdev/gro/args.ts");
    assert_eq!(run.envelope["error"]["importer"], importer);
    let run = eval(&["import_args_attribute.js"]);
    run.assert_failed("link", 1);
    let importer = format!("{}/import_args_attribute.js", fixtures_url());
    assert_eq!(
        run.message(),
        format!(
            "cannot import '@fuzdev/gro/args.ts' from {importer}: a builtin module takes no \
             import attributes"
        )
    );
    assert_eq!(run.envelope["error"]["specifier"], "@fuzdev/gro/args.ts");
    assert_eq!(run.envelope["error"]["importer"], importer);
}

/// An import attribute under any key but `type` is refused before the
/// specifier resolves, as a `link` naming the key, a file's and a builtin
/// module's alike.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn an_attribute_under_another_key_is_a_link_naming_it() {
    for (fixture, specifier) in [
        ("import_attribute_key.js", "./value.js"),
        ("import_args_attribute_key.js", "@fuzdev/gro/args.ts"),
    ] {
        let run = eval(&[fixture]);
        run.assert_failed("link", 1);
        let importer = format!("{}/{fixture}", fixtures_url());
        assert_eq!(
            run.message(),
            format!(
                "cannot import '{specifier}' from {importer}: the import attribute 'x' is not \
                 supported; only 'type' is"
            )
        );
        assert_eq!(run.envelope["error"]["specifier"], specifier);
        assert_eq!(run.envelope["error"]["importer"], importer);
    }
}

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_graph_that_does_not_link_is_a_link() {
    let run = eval(&["syntax_error.mjs"]);
    run.assert_failed("link", 1);
    assert!(
        run.message().starts_with("Uncaught SyntaxError: "),
        "{}",
        run.message()
    );
    let run = eval(&["enum.ts"]);
    run.assert_failed("link", 1);
    assert!(
        run.message().starts_with("cannot strip an enum."),
        "{}",
        run.message()
    );
    let url = format!("{}/enum.ts", fixtures_url());
    assert_eq!(run.envelope["error"]["specifier"], url);
    assert_eq!(run.envelope["error"]["at"], format!("{url}:1:1"));
}

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_throw_is_module_threw() {
    let run = eval(&["throw.js"]);
    run.assert_failed("module_threw", 1);
    assert_eq!(
        run.message(),
        format!(
            "Uncaught (in promise) Error: boom\n    at {}/throw.js:3:7",
            fixtures_url()
        )
    );
    // the driver's call throwing
    let run = driven("value.js", "throws.js");
    run.assert_failed("module_threw", 1);
    assert!(
        run.message()
            .starts_with("Uncaught Error: driver boom\n    at default (gro:driver:2:8)"),
        "{}",
        run.message()
    );
}

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_missing_export_is_export_missing() {
    let run = eval(&["value.js", "--export", "nope"]);
    run.assert_failed("export_missing", 1);
    assert_eq!(
        run.message(),
        "the module has no export named `nope` to evaluate"
    );
    let run = driven("value.js", "no_default.js");
    run.assert_failed("export_missing", 1);
    // an export that holds `undefined` is present, and doesn't serialize
    let run = eval(&["value.js", "--export", "missing_value"]);
    run.assert_failed("serialize", 1);
    assert_eq!(run.envelope["error"]["at"], "$");
}

#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_value_that_is_not_data_is_serialize() {
    let run = eval(&["function.js"]);
    run.assert_failed("serialize", 1);
    assert_eq!(run.envelope["error"]["at"], "$.a[0].f");
    assert_eq!(
        run.message(),
        "$.a[0].f is a function; the value is data, so call it or write what it returns"
    );
}

/// With no timers, work left after the evaluation is promise jobs, which all
/// run before the value is taken: an error in one, however long the chain
/// before it, fails the evaluation.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn late_work_runs_before_the_value_is_taken() {
    let run = eval(&["late.js"]);
    run.assert_failed("module_threw", 1);
    assert_eq!(
        run.message(),
        format!(
            "Uncaught (in promise) Error: late\n    at {}/late.js:7:8",
            fixtures_url()
        )
    );
}

/// A value inside the depth cap serializes even when the walk allocates
/// enough near its deepest level to start a garbage collection.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_deep_value_serializes_through_a_garbage_collection() {
    // nested past what `serde_json` parses, so checked as text
    let run = spawn_eval(&["deep_gc.js"], None, &[]);
    assert_eq!(run.code, 0, "{}\n{}", run.stdout, run.stderr);
    assert!(run.stdout.starts_with(r#"{"eval_protocol":2,"ok":true,"#));
    let value = format!(
        "{}[{}]{}",
        "[".repeat(995),
        vec!["{}"; 100_000].join(","),
        "]".repeat(995)
    );
    assert!(
        run.stdout
            .contains(&format!(r#","value":{value},"access":"#))
    );
}

/// `WebAssembly` undefined in a jitless eval hints at `--jit`, and an
/// uncaught `EvalError` that code generation from strings is off.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn the_hints_name_the_fix() {
    let run = eval(&["wasm_global.js"]);
    run.assert_failed("module_threw", 1);
    assert_eq!(
        run.envelope["error"]["hint"],
        "eval runs jitless unless asked, and WebAssembly comes only with V8's JIT: pass --jit to \
         turn it on"
    );
    eval(&["wasm_global.js", "--jit"]).assert_value(&json!("function"));
    let run = eval(&["codegen_uncaught.js"]);
    run.assert_failed("module_threw", 1);
    assert!(
        run.message()
            .starts_with("Uncaught (in promise) EvalError: Code generation from strings"),
        "{}",
        run.message()
    );
    assert_eq!(
        run.envelope["error"]["hint"],
        "eval turns code generation from strings off, with or without --jit: eval(), new \
         Function(), and the other function constructors throw an EvalError; write the code in \
         the module instead"
    );
    // keyed on the class, never the message: an error that only reads like
    // the refusal gets none
    let run = eval(&["codegen_text.js"]);
    run.assert_failed("module_threw", 1);
    assert!(
        run.envelope["error"].get("hint").is_none(),
        "{}",
        run.stdout
    );
    // no hint where there's nothing to add
    let run = eval(&["throw.js"]);
    assert!(
        run.envelope["error"].get("hint").is_none(),
        "{}",
        run.stdout
    );
}

/// fgro applies no source map: a throw in a module with an inline map, and a
/// failed import's position in one, are at the positions in the code that
/// ran, never the map's (`original.ts:10:5`).
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn no_source_map_applies() {
    let url = fixtures_url();
    let run = eval(&["mapped.js"]);
    run.assert_failed("module_threw", 1);
    assert_eq!(
        run.message(),
        format!(
            "Uncaught (in promise) Error: mapped\n    at fail ({url}/mapped_lib.js:2:8)\n    at {url}/mapped.js:3:1"
        )
    );
    let run = eval(&["import_attribute_mapped.js"]);
    run.assert_failed("link", 1);
    assert_eq!(
        run.envelope["error"]["at"],
        format!("{url}/import_attribute_mapped_dep.js:1:19")
    );
}

/// A package's bare import of the project's own module, through the
/// project linked into `node_modules`, is a `resolve` naming the package,
/// and `import.meta.resolve` of it from the package throws the same
/// refusal; the project imports the module by that link and by
/// self-reference.
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_package_importing_the_project_by_name_is_a_resolve() {
    let scratch = Scratch::new(
        "project_by_name",
        &[
            (
                "package.json",
                r#"{"name": "app", "type": "module", "exports": {"./config.js": "./config.js"}}"#,
            ),
            ("config.js", "export default {name: 'app'};\n"),
            (
                "own.js",
                "import a from 'self/config.js';\nimport b from 'app/config.js';\n\
                 export default {same: a === b, name: a.name};\n",
            ),
            ("hop.js", "import x from 'pkg/hop.js';\nexport default x;\n"),
            (
                "where.js",
                "import {where} from 'pkg/where.js';\nlet message = null;\n\
                 try { where(); } catch (error) { message = error.message; }\n\
                 export default {message};\n",
            ),
            (
                "node_modules/pkg/package.json",
                r#"{"name": "pkg", "type": "module", "exports": {"./hop.js": "./hop.js", "./where.js": "./where.js"}}"#,
            ),
            (
                "node_modules/pkg/hop.js",
                "import config from 'self/config.js';\nexport default config;\n",
            ),
            (
                "node_modules/pkg/where.js",
                "export const where = () => import.meta.resolve('self/config.js');\n",
            ),
        ],
    );
    std::os::unix::fs::symlink("..", scratch.0.join("node_modules/self")).unwrap();
    let root = scratch.0.display();
    eval(&[&scratch.path("own.js")]).assert_value(&json!({"same": true, "name": "app"}));
    let refusal = format!(
        "cannot resolve 'self/config.js' from file://{root}/node_modules/pkg/hop.js: package \
         'pkg' imports it by name, and it resolves to {root}/config.js, a module of the project \
         at {root}: a package can't import the project's own modules by name, since through one \
         it would reach the grants the project's modules hold"
    );
    let run = eval(&[&scratch.path("hop.js")]);
    run.assert_failed("resolve", 1);
    assert_eq!(run.message(), refusal);
    assert_eq!(run.envelope["error"]["specifier"], "self/config.js");
    let run = eval(&[&scratch.path("where.js")]);
    run.assert_value(&json!({"message": refusal.replace("pkg/hop.js", "pkg/where.js")}));
}

/// A specifier the builtin table reserves, or a respelling of one or of a
/// builtin's, is a `resolve`, with the message `fgro run` gives, ahead of
/// eval's refusal of every builtin module, and `import.meta.resolve` of one
/// throws it. (Eval refuses every `import()` before it resolves.)
#[test]
#[cfg_attr(
    not(feature = "snapshot"),
    ignore = "a confined eval needs a snapshot build"
)]
fn a_reserved_specifier_is_a_resolve() {
    let scratch = Scratch::new(
        "reserved",
        &[
            ("package.json", r#"{"name": "p", "type": "module"}"#),
            (
                "js_spelling.js",
                "import {args} from '@fuzdev/gro/args.js';\nexport default args;\n",
            ),
            (
                "where.js",
                "let message = null;\n\
                 try { import.meta.resolve('@fuzdev/gro/fs.ts'); } catch (error) { message = error.message; }\n\
                 export default {message};\n",
            ),
            (
                "respelled.js",
                "import {args} from '@fuzdev/gro/arg%73.ts';\nexport default args;\n",
            ),
            (
                "split.js",
                "import {process} from '@fuzdev/gro/process.ts';\nexport default process;\n",
            ),
            (
                "where_respelled.js",
                "let message = null;\n\
                 try { import.meta.resolve('@fuzdev/gro/fs.js?.ts'); } catch (error) { message = error.message; }\n\
                 export default {message};\n",
            ),
        ],
    );
    let root = scratch.0.display();
    let run = eval(&[&scratch.path("js_spelling.js")]);
    run.assert_failed("resolve", 1);
    assert_eq!(
        run.message(),
        format!(
            "cannot resolve '@fuzdev/gro/args.js' from file://{root}/js_spelling.js: it is the \
             .js spelling of the builtin module '@fuzdev/gro/args.ts', reserved so it never \
             resolves to a package. Fix: import '@fuzdev/gro/args.ts'"
        )
    );
    assert_eq!(run.envelope["error"]["specifier"], "@fuzdev/gro/args.js");
    eval(&[&scratch.path("where.js")]).assert_value(&json!({
        "message": format!(
            "cannot resolve '@fuzdev/gro/fs.ts' from file://{root}/where.js: it names Gro's host \
             module '@fuzdev/gro/fs.ts', which isn't on fgro yet, and a host module's specifier \
             never resolves to a package. Fix: run this under Node with gro"
        )
    }));
    let run = eval(&[&scratch.path("respelled.js")]);
    run.assert_failed("resolve", 1);
    assert_eq!(
        run.message(),
        format!(
            "cannot resolve '@fuzdev/gro/arg%73.ts' from file://{root}/respelled.js: it \
             respells the builtin module '@fuzdev/gro/args.ts', reserved so no spelling of it \
             resolves to a package. Fix: import '@fuzdev/gro/args.ts'"
        )
    );
    assert_eq!(run.envelope["error"]["specifier"], "@fuzdev/gro/arg%73.ts");
    let run = eval(&[&scratch.path("split.js")]);
    run.assert_failed("resolve", 1);
    assert!(
        run.message().starts_with(&format!(
            "cannot resolve '@fuzdev/gro/process.ts' from file://{root}/split.js: it names \
             '@fuzdev/gro/process.ts', which isn't one of Gro's modules: its members are modules \
             of their own, and its specifier never resolves to a package. Fix: import what you \
             use from '@fuzdev/gro/env.ts', '@fuzdev/gro/args.ts',"
        )),
        "{}",
        run.message()
    );
    eval(&[&scratch.path("where_respelled.js")]).assert_value(&json!({
        "message": format!(
            "cannot resolve '@fuzdev/gro/fs.js?.ts' from file://{root}/where_respelled.js: it \
             names Gro's host module '@fuzdev/gro/fs.ts', which isn't on fgro yet, and a host \
             module's specifier never resolves to a package. Fix: run this under Node with gro"
        )
    }));
}
