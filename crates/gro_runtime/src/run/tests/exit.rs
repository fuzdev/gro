//! The exit builtin, in process: the code `set_exit_code` sets reaches the
//! run's outcome, and a value that is no code is refused natively by both
//! exports, running no code of the caller's. A valid `exit` ends the
//! process, so it's exercised in child processes, by `gro_cli`'s tests.

use super::super::Ended;
use super::builtins::{granted as authority, run_linked_outcome};
use crate::{RunError, RuntimeOptions};

const EXIT: &str = "@fuzdev/gro/exit.ts";

/// A run granted as `fgro run` grants, whose exit step no program here
/// reaches: every call of `exit` is given a value that is no code.
fn granted() -> RuntimeOptions {
    RuntimeOptions {
        authority: authority(),
        ..RuntimeOptions::default()
    }
}

/// Runs `source`, a module of a project granted `exit`.
fn run_granted(name: &str, source: &str) -> Ended {
    run_linked_outcome(name, &[("main.mjs", source)], &[], granted()).0
}

/// The message of the error a run failed with.
fn failure(ended: &Ended) -> &str {
    match &ended.result {
        Err(RunError::Uncaught(report)) => report,
        other => panic!("the run failed with an uncaught error: {other:?}"),
    }
}

/// The code `set_exit_code` set is the outcome's, the last one set, and a
/// run that sets none has none.
#[test]
fn the_code_set_reaches_the_outcome() {
    let none = run_granted("exit_none", &format!("import '{EXIT}';"));
    none.result.unwrap();
    assert_eq!(none.exit_code, None);
    for (name, calls, expected) in [
        ("exit_set_7", "set_exit_code(7);", 7),
        ("exit_set_0", "set_exit_code(3); set_exit_code(0);", 0),
        ("exit_set_255", "set_exit_code(255);", 255),
        ("exit_set_minus_0", "set_exit_code(-0);", 0),
        // a value refused leaves the code as it was
        (
            "exit_set_then_refused",
            "set_exit_code(9); try { set_exit_code(256); } catch {}",
            9,
        ),
        // set after the entry evaluated, before the event loop drained
        (
            "exit_set_late",
            "setTimeout(() => set_exit_code(42), 1);",
            42,
        ),
    ] {
        let source = format!("import {{set_exit_code}} from '{EXIT}';\n{calls}");
        let ended = run_granted(name, &source);
        ended.result.unwrap();
        assert_eq!(ended.exit_code, Some(expected), "{name}");
    }
}

/// A run that fails still carries the code its program set, which the
/// embedder is free to ignore, as `fgro` does.
#[test]
fn a_failed_run_carries_the_code_set() {
    let source = format!(
        "import {{set_exit_code}} from '{EXIT}';\nset_exit_code(5);\nthrow new Error('failed');"
    );
    let ended = run_granted("exit_set_failed", &source);
    assert!(failure(&ended).contains("Error: failed"), "{ended:?}");
    assert_eq!(ended.exit_code, Some(5));
}

/// Both exports refuse a value that is no code with the realm's own
/// `TypeError` and the Node module's message, made natively: no property
/// of the value is read, no conversion called, and a `TypeError` global
/// the program replaced isn't used. `set_exit_code` refuses `undefined`
/// too, which `exit` takes as no code, so `exit` is given every value but
/// it: a valid call would end this process.
#[test]
fn a_value_that_is_no_code_is_refused_untouched() {
    let source = format!(
        "import {{exit, set_exit_code}} from '{EXIT}';
const RealmTypeError = TypeError;
globalThis.TypeError = class extends Error {{}};
let touched = 0;
const touch = () => {{ touched++; return 1; }};
const hostile = [
	['a string', '1'],
	['a fraction', 1.5],
	['a negative', -1],
	['past a byte', 256],
	['NaN', NaN],
	['Infinity', Infinity],
	['null', null],
	['a boolean', true],
	['a bigint', 1n],
	['a boxed number', new Number(1)],
	['valueOf', {{ valueOf: touch }}],
	['toPrimitive', {{ [Symbol.toPrimitive]: touch }}],
	['a thenable', {{ get then() {{ touch(); return undefined; }} }}],
	['a proxy', new Proxy({{}}, {{ get: touch, has: touch, getPrototypeOf: touch, ownKeys: touch }})],
	['an array', [1]],
];
const check = (what, call) => {{
	let error;
	try {{ call(); }} catch (thrown) {{ error = thrown; }}
	if (error === undefined) throw new Error(`${{what}}: returned`);
	if (Object.getPrototypeOf(error) !== RealmTypeError.prototype) throw new Error(`${{what}}: not the realm's TypeError`);
	if (error.message !== 'an exit code is an integer from 0 to 255') throw new Error(`${{what}}: ${{error.message}}`);
}};
for (const [what, value] of hostile) {{
	check(`exit(${{what}})`, () => exit(value));
	check(`set_exit_code(${{what}})`, () => set_exit_code(value));
}}
check('set_exit_code(undefined)', () => set_exit_code(undefined));
check('set_exit_code()', () => set_exit_code());
if (touched !== 0) throw new Error(`a value was touched ${{touched}} times`);
"
    );
    let ended = run_granted("exit_refused_values", &source);
    ended.result.unwrap();
    assert_eq!(ended.exit_code, None);
}

/// A run that isn't granted `exit` is refused the import, naming the
/// grant, and a run granted it imports a namespace of its two exports,
/// each a frozen function.
#[test]
fn the_import_needs_the_grant() {
    let source = format!(
        "const ns = await import('{EXIT}');
const keys = Reflect.ownKeys(ns).filter((key) => typeof key === 'string');
if (JSON.stringify(keys) !== '[\\\"default\\\",\\\"exit\\\",\\\"set_exit_code\\\"]') throw new Error(keys);
for (const fn of [ns.exit, ns.set_exit_code]) {{
	if (typeof fn !== 'function' || !Object.isFrozen(fn)) throw new Error('not a frozen function');
}}"
    );
    run_granted("exit_granted", &source).result.unwrap();
    let options = RuntimeOptions::default();
    let (ended, _) = run_linked_outcome("exit_ungranted", &[("main.mjs", &source)], &[], options);
    let report = failure(&ended);
    assert!(
        report.contains("'@fuzdev/gro/exit.ts'") && report.contains("'exit'"),
        "{report}"
    );
}
