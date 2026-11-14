//! Tests for `import()` in code compiled from a string: refused whoever
//! compiled it and in whichever order, while every `import()` a module's own
//! function makes is allowed, wherever it's called from.

use super::builtins::{ARGS, granted, run_files};
use crate::RuntimeOptions;
use crate::loader::graph::{FailureReason, Reach, Reached};

/// The project's helpers, which compile strings as the package's do.
const PROJECT_LIB: &str = "export const ind = (source) => (0, eval)(source);
export const fn = (...args) => new Function(...args);
export const load = () => import('./who.mjs');
";

/// The package: helpers that compile a string, call a function now or in a
/// timer, run a string timer, and import its own module.
const PKG_LIB: &str = "export const ind = (source) => (0, eval)(source);
export const fn = (...args) => new Function(...args);
export const call = (f) => f();
export const later = (f) => new Promise((resolve) => setTimeout(() => resolve(f()), 0));
export const timer = (source) =>
\tnew Promise((resolve) => {
\t\tglobalThis.__resolve = resolve;
\t\tsetTimeout(source, 0);
\t});
export const own = () => import('./who.js');
";

/// The entry: each form of string-compiled `import()` refused, the
/// project's and the package's, both compile orders, then the controls, a
/// module function's `import()` however it's reached, allowed.
const MAIN: &str = "import * as pkg from 'pkg/lib.js';
import * as lib from './lib.mjs';
const B = \"import('@fuzdev/gro/args.ts')\";
const W = \"import('./who.mjs')\";
const failures = [];
const refused = async (name, f) => {
\ttry {
\t\tawait f();
\t\tfailures.push(`${name}: imported`);
\t} catch (error) {
\t\tconst message = String(error?.message);
\t\tif (
\t\t\t!(error instanceof TypeError) ||
\t\t\t!message.includes('from code compiled from a string') ||
\t\t\t!message.includes('Fix: call import() in a module')
\t\t) {
\t\t\tfailures.push(`${name}: ${error}`);
\t\t}
\t}
};
const allowed = async (name, f) => {
\ttry {
\t\tconst ns = await f();
\t\tif (ns === undefined || ns === null) failures.push(`${name}: no namespace`);
\t} catch (error) {
\t\tfailures.push(`${name}: ${error}`);
\t}
};
const AsyncFunction = (async () => {}).constructor;
const GeneratorFunction = function* () {}.constructor;
const AsyncGeneratorFunction = async function* () {}.constructor;

// the project compiles first, then the package compiles the same string
await refused('project indirect eval', () => lib.ind(B));
await refused('package indirect eval after the project', () => pkg.ind(B));
await refused('project new Function', () => lib.fn('return ' + B)());
await refused('package new Function after the project', () => pkg.fn('return ' + B)());
await refused('package new Function, other parameters', () => pkg.fn('a', 'return ' + B)());
// the package compiles first, then the project
const R = B + ' // reversed';
await refused('package indirect eval first', () => pkg.ind(R));
await refused('project indirect eval after the package', () => lib.ind(R));
await refused('package new Function first', () => pkg.fn('return ' + R)());
await refused('project new Function after the package', () => lib.fn('return ' + R)());
// every way to compile code from a string
await refused('direct eval', () => eval(W));
await refused('async Function', () => new AsyncFunction('return ' + W)());
await refused('generator Function', () => new GeneratorFunction('yield ' + W)().next().value);
await refused('async generator Function', async () => (await new AsyncGeneratorFunction('yield await ' + W)().next()).value);
await refused('Reflect.apply of eval', () => Reflect.apply(eval, undefined, [W]));
await refused('nested', () => lib.ind('new Function(' + JSON.stringify('return ' + W) + ')()'));
await refused('a string timer', () => pkg.timer('globalThis.__resolve(' + W + ')'));
await refused('a sourceURL comment', () => lib.ind(W + '\\n//# sourceURL=file:///elsewhere.js'));
await refused('a computed specifier', () => lib.ind(\"import('./' + 'who.mjs')\"));
// a function defined in string-compiled code, wherever it's called from
const closure = lib.ind('() => ' + B);
await refused('a closure called by the package', () => pkg.call(closure));
await refused('a closure called in a timer', () => pkg.later(closure));
await refused('a closure in a microtask', () => Promise.resolve().then(lib.ind('() => ' + W)));
await refused('a tagged template', () => lib.ind('((strings) => import(strings[0]))`./who.mjs`'));
await refused('a class static block', () => lib.ind('(class { static p; static { this.p = ' + W + '; } }).p'));
await refused('a getter', () => lib.ind('({ get p() { return ' + W + '; } })').p);
await refused('a Proxy trap', () => new Proxy({}, lib.ind('({ get: () => ' + W + ' })')).p);
await refused('an async function after awaits', () =>
\tnew AsyncFunction('await null; await new Promise((resolve) => setTimeout(resolve, 0)); return ' + W)(),
);

// a module's own function, however string-compiled code or a package reaches it
await allowed('a literal', () => import('./who.mjs'));
await allowed('a computed specifier', () => import('./' + 'who.mjs'));
await allowed('a builtin', () => import('@fuzdev/gro/args.ts'));
await allowed('a module function new Function calls', () => new Function('f', 'return f()')(() => import('./who.mjs')));
await allowed('a module function eval calls', () => lib.ind('(load) => load()')(lib.load));
await allowed('a callback the package calls', () => pkg.call(() => import('@fuzdev/gro/args.ts')));
await allowed('a callback the package calls in a timer', () => pkg.later(() => import('./who.mjs')));
await allowed('a microtask', () => Promise.resolve().then(() => import('./who.mjs')));
await allowed('a timer', () => new Promise((resolve) => setTimeout(() => resolve(import('./who.mjs')), 0)));
await allowed('the package importing its own module', () => pkg.own());
// the imports of the module it loads resolve with no JS frame on the stack
await allowed('a module with static imports', () => import('./dep.mjs'));

if (failures.length > 0) throw new Error(failures.join('\\n'));
";

/// The project's files and the package's, beside [`MAIN`].
const FILES: [(&str, &str); 7] = [
    ("main.mjs", MAIN),
    ("lib.mjs", PROJECT_LIB),
    ("who.mjs", "export default 'project';\n"),
    (
        "dep.mjs",
        "import who from './who.mjs';\nimport { args } from '@fuzdev/gro/args.ts';\n\
         export default [who, args];\n",
    ),
    (
        "node_modules/pkg/package.json",
        r#"{"name": "pkg", "type": "module", "exports": {"./lib.js": "./lib.js"}}"#,
    ),
    ("node_modules/pkg/lib.js", PKG_LIB),
    ("node_modules/pkg/who.js", "export default 'pkg';\n"),
];

/// Every `import()` in code compiled from a string is refused with a
/// `TypeError` naming the fix, recorded as its own failure, the project's
/// and the package's alike in either compile order, while a module's own
/// `import()`, the imports of what it loads included, is allowed.
#[test]
fn an_import_in_code_compiled_from_a_string_is_refused() {
    let options = RuntimeOptions {
        authority: granted(),
        ..RuntimeOptions::default()
    };
    let (result, graph) = run_files("string_code", &FILES, options);
    assert!(result.is_ok(), "{result:?}");
    let refused: Vec<_> = graph
        .failures()
        .iter()
        .filter(|(_, failures)| {
            failures
                .iter()
                .any(|failure| failure.reason == FailureReason::StringCompiledImport)
        })
        .map(|(site, _)| site)
        .collect();
    assert!(
        refused
            .iter()
            .all(|site| site.reached == Reached::Import(Reach::Dynamic)),
        "{refused:?}"
    );
    for specifier in [ARGS, "./who.mjs"] {
        assert!(
            refused.iter().any(|site| site.specifier == specifier),
            "{specifier}: {:?}",
            graph.failures()
        );
    }
    // nothing else failed: every control resolved and loaded
    assert!(
        graph
            .failures()
            .values()
            .flatten()
            .all(|failure| failure.reason == FailureReason::StringCompiledImport),
        "{:?}",
        graph.failures()
    );
}
