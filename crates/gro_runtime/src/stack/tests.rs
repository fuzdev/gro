//! Tests for the stack's formatting: gro's callback against `deno_core`'s on
//! the same errors, the text pinned for a few, and the call sites kept where
//! `deno_core`'s `JsError` reads them.

use std::path::Path;
use std::rc::Rc;

use deno_core::error::{JsError, prepare_stack_trace_callback};
use deno_core::{JsRuntime, PollEventLoopOptions, v8};

use super::prepare_stack_trace;
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};
use crate::loader::Loader;
use crate::loader::graph::Recording;
use crate::resolver::{DiskTree, resolve_entry};
use crate::runtime::{Mode, start_runtime};
use crate::test_support::TestDir;
use crate::{ConsoleOptions, STACK_SIZE};

/// The directory the cases' module sits in: a space, non-ASCII letters, and
/// a `%`, which the module's URL percent-encodes and the stack prints decoded.
const CASES_DIR: &str = "d\u{ed}r w\u{ef}th sp\u{e5}ce%41 \u{65e5}\u{672c}";

/// The cases' module: `make` returns a fresh error of each shape, the async
/// ones made after an `await`, so each call's errors are formatted anew.
const CASES: &str = r#"import {mapped} from './mapped.js';
import {mapped_mid, after_map} from './mapped_mid.js';
import {mapped_at} from './mapped_at.js';
import {mapped_block} from './mapped_block.js';
import {external} from './external.js';

export async function make() {
	const errors = [];
	function plain() { return new Error('plain'); }
	errors.push(plain());
	const o = {};
	o.foo = function xfoo() { return new Error('method named apart'); };
	o.baz = function o_baz() { return new Error('method name a suffix'); };
	o.bar = function () { return new Error('anonymous method'); };
	errors.push(o.foo(), o.baz(), o.bar());
	class K {
		constructor() { this.error = new Error('constructor'); }
		static make() { return new Error('static'); }
		get getter() { return new Error('getter'); }
		[Symbol.iterator]() { return new Error('symbol method'); }
	}
	errors.push(new K().error, K.make(), new K().getter, new K()[Symbol.iterator]());
	errors.push(new (class { constructor() { this.error = new Error('anonymous class'); } })().error);
	errors.push([1].map(() => new Error('arrow in map'))[0]);
	function* generate() { yield new Error('generator'); }
	errors.push(generate().next().value);
	async function inner() { await null; return new Error('async'); }
	async function outer() { return await inner(); }
	errors.push(await outer());
	for (const combinator of ['all', 'any', 'allSettled']) {
		const made = [];
		await Promise[combinator]([(async () => { await null; made.push(new Error(combinator)); })()]);
		errors.push(made[0]);
	}
	errors.push(eval('new Error("eval")'));
	errors.push(eval('eval("new Error(\'nested eval\')")'));
	errors.push(new Function('return new Error("function")')());
	const unnamed = new Error('unnamed'); unnamed.name = ''; errors.push(unnamed);
	errors.push(new Error(''));
	const numbered = new Error('m'); numbered.message = 42; numbered.name = undefined; errors.push(numbered);
	const getter = new Error('getter name');
	Object.defineProperty(getter, 'name', {get() { return 'Getter'; }});
	errors.push(getter);
	class Custom extends Error { name = 'Custom'; }
	errors.push(new Custom('subclass'));
	errors.push(new DOMException('dom', 'AbortError'));
	errors.push(new AggregateError([new Error('a')], 'aggregate'));
	const captured = {}; Error.captureStackTrace(captured); errors.push(captured);
	Error.stackTraceLimit = 0; errors.push(new Error('no frames')); Error.stackTraceLimit = 10;
	errors.push(mapped());
	errors.push(mapped_mid(), after_map(), mapped_at(), mapped_block(), external());
	for (const make_error of [() => new TextDecoder('nope'), () => new URL('::')]) {
		try { make_error(); } catch (error) { errors.push(error); }
	}
	errors.push(eval('new Error("source url")\n//# sourceURL=http://example.com/a%20b%zz.js'));
	errors.push(eval('(() => new Error("named eval"))()\n//# sourceURL=named%41.js'));
	errors.push(new Proxy({}, {get() { return new Error('proxy trap'); }}).x);
	const setter = {set value(v) { errors.push(new Error('setter')); }};
	setter.value = 1;
	class Static { static { errors.push(new Error('static block')); } }
	const wasm = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 7, 5, 1, 1, 102, 0, 0, 10, 6, 1, 4, 0, 0, 0, 11]);
	const {instance} = await WebAssembly.instantiate(wasm);
	try { instance.exports.f(); } catch (error) { errors.push(error); }
	return errors;
}
"#;

/// The JSON of an inline source map placing a module's first line at line
/// 10, column 5 of `original.ts`, base64:
/// `{"version":3,"sources":["original.ts"],"names":[],"mappings":"AASI,0BAAA"}`.
const MAP: &str = "eyJ2ZXJzaW9uIjozLCJzb3VyY2VzIjpbIm9yaWdpbmFsLnRzIl0sIm5hbWVzIjpbXSwibWFwcGluZ3MiOiJBQVNJLDBCQUFBIn0=";

/// The modules beside the cases': inline source maps in each comment form
/// (`//#`, `//@`, and `/*# */`, one mid-file with code after it), and one
/// naming an external map, none of which fgro applies.
fn mapped_modules() -> [(&'static str, String); 5] {
    let url = format!("data:application/json;base64,{MAP}");
    [
        (
            "mapped.js",
            format!(
                "export function mapped() {{ return new Error('mapped'); }}\n\
                 //# sourceMappingURL={url}\n"
            ),
        ),
        (
            "mapped_mid.js",
            format!(
                "export function mapped_mid() {{ return new Error('mapped mid'); }}\n\
                 //# sourceMappingURL={url}\n\
                 export function after_map() {{ return new Error('after map'); }}\n"
            ),
        ),
        (
            "mapped_at.js",
            format!(
                "export function mapped_at() {{ return new Error('mapped at'); }}\n\
                 //@ sourceMappingURL={url}\n"
            ),
        ),
        (
            "mapped_block.js",
            format!(
                "export function mapped_block() {{ return new Error('mapped block'); }}\n\
                 /*# sourceMappingURL={url} */\n"
            ),
        ),
        (
            "external.js",
            "export function external() { return new Error('external map'); }\n\
             //# sourceMappingURL=external.js.map\n"
                .to_owned(),
        ),
    ]
}

/// Runs `test` on a started runtime that has evaluated a `main.mjs` putting
/// the cases' `make` on the global object, in a fresh directory, on a thread
/// reserving [`STACK_SIZE`].
fn on_cases<T: Send + 'static>(
    name: &str,
    test: impl AsyncFnOnce(&mut JsRuntime, &Path) -> T + Send + 'static,
) -> T {
    init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let dir = TestDir::on_disk(name);
    let cases = dir.path().join(CASES_DIR);
    std::fs::create_dir(&cases).unwrap();
    std::fs::write(cases.join("cases.js"), CASES).unwrap();
    for (name, source) in mapped_modules() {
        std::fs::write(cases.join(name), source).unwrap();
    }
    std::fs::write(cases.join("external.js.map"), "{}").unwrap();
    // the specifier percent-encodes the `%`, which a URL decodes
    let specifier = format!("./{}/cases.js", CASES_DIR.replace('%', "%25"));
    let mapped = format!("./{}/mapped.js", CASES_DIR.replace('%', "%25"));
    std::fs::write(
        dir.path().join("main.mjs"),
        format!(
            "import {{make}} from {specifier:?};\nimport {{mapped}} from {mapped:?};\n\
             globalThis.make = make;\nglobalThis.mapped = mapped;\n"
        ),
    )
    .unwrap();
    let cwd = dir.path().to_owned();
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let (entry, roots) = resolve_entry(&DiskTree, Path::new("main.mjs"), &cwd).unwrap();
            let url = entry.url.clone();
            let loader = Rc::new(Loader::new(&entry, roots, Recording::Off));
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            tokio.block_on(async {
                let mut runtime =
                    start_runtime(None, ConsoleOptions::default(), loader, Mode::Run).unwrap();
                let id = runtime.load_main_es_module(&url).await.unwrap();
                let evaluation = runtime.mod_evaluate(id);
                runtime
                    .run_event_loop(PollEventLoopOptions::default())
                    .await
                    .unwrap();
                evaluation.await.unwrap();
                test(&mut runtime, &cwd).await
            })
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Evaluates `source`, a script whose value is a promise of a string or a
/// string, and returns the string.
#[expect(
    clippy::future_not_send,
    reason = "a runtime stays on its thread, driven by a current-thread tokio runtime"
)]
async fn eval_string(runtime: &mut JsRuntime, source: &'static str) -> String {
    let value = runtime.execute_script("<test>", source).unwrap();
    let resolved = runtime.resolve(value);
    let value = runtime
        .with_event_loop_promise(resolved, PollEventLoopOptions::default())
        .await
        .unwrap();
    deno_core::scope!(scope, runtime);
    v8::Local::new(scope, value).to_rust_string_lossy(scope)
}

/// Each case's stack, as a JSON array, formatted by the callback in place.
const STACKS: &str = "make().then((errors) => JSON.stringify(errors.map((error) => error.stack)))";

/// Each case's stack read with `Error.prepareStackTrace` set, as a JSON array.
const USER_STACKS: &str = "make().then((errors) => {
	Error.prepareStackTrace = (error, sites) =>
		`${error.message}|${sites.map((site) => `${site.getFunctionName()}@${site.getFileName()}:${site.getLineNumber()}`).join(',')}`;
	try {
		return JSON.stringify(errors.map((error) => error.stack));
	} finally {
		delete Error.prepareStackTrace;
	}
})";

/// The stacks `source` reads under gro's callback, then under `deno_core`'s.
fn stacks_both_ways(name: &str, source: &'static str) -> (Vec<String>, Vec<String>) {
    on_cases(name, async move |runtime: &mut JsRuntime, _: &Path| {
        let gro = eval_string(runtime, source).await;
        runtime
            .v8_isolate()
            .set_prepare_stack_trace_callback(prepare_stack_trace_callback);
        let deno = eval_string(runtime, source).await;
        runtime
            .v8_isolate()
            .set_prepare_stack_trace_callback(prepare_stack_trace);
        (
            serde_json::from_str(&gro).unwrap(),
            serde_json::from_str(&deno).unwrap(),
        )
    })
}

/// Every case's stack is `deno_core`'s, character for character: a method
/// called by another name, `Promise.any`, eval origins, a decoded path,
/// `ext:` frames, an eval's `sourceURL` with a `%` and a non-file URL, Proxy
/// trap, setter, and static block frames, modules with a source map in each
/// comment form (unmapped by both, since the vendored `deno_core` applies
/// none), a wasm frame, and the rest. A `deno_core` upgrade that changes
/// its format fails here.
#[test]
fn every_stack_is_deno_cores() {
    let (gro, deno) = stacks_both_ways("stack_every", STACKS);
    assert!(!gro.is_empty());
    for (gro, deno) in gro.iter().zip(&deno) {
        assert_eq!(gro, deno);
    }
    assert_eq!(gro.len(), deno.len());
}

/// With `Error.prepareStackTrace` set, the program's function formats each
/// stack, from the call sites `deno_core` hands it.
#[test]
fn a_user_prepare_stack_trace_formats_as_with_deno_core() {
    let (gro, deno) = stacks_both_ways("stack_user", USER_STACKS);
    assert_eq!(gro, deno);
    // `deno_core`'s call sites name a file by its path, not its URL
    assert!(gro[0].starts_with("plain|plain@/"), "{}", gro[0]);
    // nor do they apply a source map
    let mapped = gro
        .iter()
        .find(|stack| stack.starts_with("mapped|"))
        .unwrap();
    assert!(mapped.contains("/mapped.js:1,"), "{mapped}");
}

/// The text for a few cases, pinned: the module's URL printed decoded, a
/// method called by another name, `Promise.any` as a function, an eval origin,
/// and each module with a source map at its generated positions.
#[test]
fn stacks_are_formatted_as_pinned() {
    let (stacks, url) = on_cases("stack_pinned", async move |runtime, cwd| {
        let stacks = eval_string(runtime, STACKS).await;
        let stacks: Vec<String> = serde_json::from_str(&stacks).unwrap();
        (stacks, format!("file://{}/{CASES_DIR}", cwd.display()))
    });
    let find = |header: &str| -> &str {
        stacks
            .iter()
            .find(|stack| stack.starts_with(header))
            .unwrap_or_else(|| panic!("no stack starts {header:?}: {stacks:?}"))
    };
    assert_eq!(
        find("Error: plain\n"),
        format!(
            "Error: plain\n    at plain ({url}/cases.js:9:28)\n    at make ({url}/cases.js:10:14)\n    at <test>:1:1"
        )
    );
    assert!(find("Error: method named apart\n").starts_with(&format!(
        "Error: method named apart\n    at Object.xfoo ({url}/cases.js:12:"
    )));
    assert!(find("Error: any\n").contains("\n    at async Promise.any (<anonymous>)\n"));
    assert!(find("Error: eval\n").starts_with(&format!(
        "Error: eval\n    at eval (eval at make ({url}/cases.js:"
    )));
    assert_eq!(find("Error: no frames"), "Error: no frames");
    // fgro applies no source map, whatever the comment's form or place
    for (header, at) in [
        ("Error: mapped\n", "mapped.js:1:"),
        ("Error: mapped mid\n", "mapped_mid.js:1:"),
        ("Error: after map\n", "mapped_mid.js:3:"),
        ("Error: mapped at\n", "mapped_at.js:1:"),
        ("Error: mapped block\n", "mapped_block.js:1:"),
    ] {
        let stack = find(header);
        assert!(stack.contains(&format!("({url}/{at}")), "{stack}");
        assert!(!stack.contains("10:5"), "{stack}");
    }
    assert!(find("Error: external map\n").contains(&format!("({url}/external.js:1:")));
}

/// The callback keeps an error's call sites where `deno_core`'s `JsError`
/// reads its frames from, so a callback that stopped storing them fails here.
#[test]
fn a_js_error_still_has_its_frames() {
    let frames = on_cases("stack_frames", async move |runtime, _| {
        let value = runtime
            .execute_script("<test>", "function f() { return new Error('x'); } f()")
            .unwrap();
        deno_core::scope!(scope, runtime);
        let value = v8::Local::new(scope, value);
        let error = JsError::from_v8_exception(scope, value);
        error
            .frames
            .iter()
            .map(|frame| frame.function_name.clone().unwrap_or_default())
            .collect::<Vec<_>>()
    });
    assert_eq!(frames, ["f", ""]);
}

/// A `JsError` made from an error in a module with an inline source map has
/// its frames at the generated positions, as the stack does: the report reads
/// its location from them when the stack has no frames of its own.
#[test]
fn a_js_errors_frames_are_unmapped() {
    let frames = on_cases("stack_frames_unmapped", async move |runtime, _| {
        let value = runtime.execute_script("<test>", "mapped()").unwrap();
        deno_core::scope!(scope, runtime);
        let value = v8::Local::new(scope, value);
        let error = JsError::from_v8_exception(scope, value);
        error
            .frames
            .iter()
            .map(|frame| (frame.line_number, frame.column_number))
            .collect::<Vec<_>>()
    });
    assert_eq!(frames.first(), Some(&(Some(1), Some(35))));
}
