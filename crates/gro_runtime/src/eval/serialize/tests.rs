//! The serializer over values a script makes, on a bare `deno_core`
//! runtime: what it accepts, checked against `JSON.stringify` where the two
//! agree, and what it refuses, with the path and the reason.

use deno_core::{JsRuntime, v8};

use super::{MAX_DEPTH, MAX_KEY_SHOWN, Reason, SerializeError, to_json};
use crate::STACK_SIZE;
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};

/// Runs `test` on a thread reserving [`STACK_SIZE`], as eval runs.
fn on_eval_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}

/// A bare runtime: no extensions, so nothing but the language's globals.
struct Realm {
    runtime: JsRuntime,
    /// Kept for the runtime's lifetime: V8 posts its delayed tasks on it.
    _tokio: tokio::runtime::Runtime,
}

impl Realm {
    fn new() -> Self {
        init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
        let tokio = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let runtime = {
            let _entered = tokio.enter();
            JsRuntime::new(deno_core::RuntimeOptions::default())
        };
        Self {
            runtime,
            _tokio: tokio,
        }
    }

    /// Runs `source` as a script, returning its completion value.
    fn run(&mut self, source: &str) -> v8::Global<v8::Value> {
        self.runtime
            .execute_script("<test>", source.to_owned())
            .unwrap()
    }

    /// The completion value of `source`, serialized.
    fn serialize(&mut self, source: &str) -> Result<String, SerializeError> {
        self.serialize_capped(source, usize::MAX)
    }

    /// The completion value of `source`, serialized to at most `max` bytes.
    fn serialize_capped(&mut self, source: &str, max: usize) -> Result<String, SerializeError> {
        let value = self.run(source);
        let runtime = &mut self.runtime;
        deno_core::scope!(scope, runtime);
        let value = v8::Local::new(scope, value);
        to_json(scope, value, &|| false, max)
    }

    /// The completion value of `source`, a string, as Rust's.
    fn string(&mut self, source: &str) -> String {
        let value = self.run(source);
        let runtime = &mut self.runtime;
        deno_core::scope!(scope, runtime);
        v8::Local::new(scope, value).to_rust_string_lossy(scope)
    }

    /// Asserts the serializer writes `expression` as `JSON.stringify` does.
    fn assert_as_stringify(&mut self, expression: &str) {
        let expected = self.string(&format!("JSON.stringify({expression})"));
        let written = self.serialize(&format!("({expression})"));
        assert_eq!(written.as_deref(), Ok(expected.as_str()), "{expression}");
    }

    /// Asserts the serializer refuses `expression` at `at` for `reason`.
    fn assert_refused(&mut self, expression: &str, at: &str, reason: &Reason) {
        let error = self.serialize(&format!("({expression})")).unwrap_err();
        assert_eq!(
            (error.at.as_str(), &error.reason),
            (at, reason),
            "{expression}"
        );
    }
}

#[test]
fn data_is_written_as_json_stringify_writes_it() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        for expression in [
            "null",
            "true",
            "false",
            "0",
            "-0",
            "1.5",
            "-2e-7",
            "1e21",
            "123456789012345680000",
            "0.1 + 0.2",
            "Number.MAX_VALUE",
            "Number.MIN_VALUE",
            "''",
            r#""q\"\\\n\t\b\f\r\u0001\u001f\u007f  é😀""#,
            "[]",
            "{}",
            "[1, 'two', null, true, [[]], {}]",
            "{b: 1, a: 2, 2: 'two', 1: 'one', '-1': 'minus', '01': 'zero one', 4294967295: 'big'}",
            "{'a b': {c: [1]}, '': 0, 'é': 1}",
            "{u: undefined, keep: 1, also: undefined}",
            "Object.assign(Object.create(null), {a: 1})",
            "(() => { const shared = {x: 1}; return {a: shared, b: [shared, shared]}; })()",
            "(() => { const o = {}; Object.defineProperty(o, 'hidden', {value: 1}); o.shown = 2; return o; })()",
            "(() => { const a = [1, 2]; a.extra = 3; return a; })()",
            "(() => { const o = {a: 1, b: 2, c: 3}; delete o.b; o.b = 4; return o; })()",
            // wider than a batch of the walk's steps, at every level
            "Array.from({length: 5000}, (_, i) => ({i, s: [i, {u: undefined, k: String(i)}]}))",
            "Object.fromEntries(Array.from({length: 3000}, (_, i) => ['k' + i, i % 3 ? i : {i}]))",
            "(() => { let v = Array.from({length: 1500}, (_, i) => i); for (let i = 0; i < 900; i++) v = [i, v, {i}]; return v; })()",
        ] {
            realm.assert_as_stringify(expression);
        }
    });
}

#[test]
fn what_is_not_data_is_refused_with_its_path() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        let non_plain = |kind: &str| Reason::NonPlain(kind.to_owned());
        let cases: Vec<(&str, &str, Reason)> = vec![
            ("undefined", "$", Reason::Undefined),
            ("[1, undefined]", "$[1]", Reason::Undefined),
            ("[1, , 3]", "$[1]", Reason::Hole),
            ("new Array(3)", "$[0]", Reason::Hole),
            ("{f() {}}", "$.f", Reason::Function),
            ("{a: [{f: class {}}]}", "$.a[0].f", Reason::Function),
            ("{n: 1n}", "$.n", Reason::BigInt),
            ("{s: Symbol('s')}", "$.s", Reason::Symbol),
            ("{n: NaN}", "$.n", Reason::NonFinite("NaN".to_owned())),
            (
                "[Infinity]",
                "$[0]",
                Reason::NonFinite("Infinity".to_owned()),
            ),
            (
                "{'a b': [-Infinity]}",
                "$[\"a b\"][0]",
                Reason::NonFinite("-Infinity".to_owned()),
            ),
            ("{m: new Map()}", "$.m", non_plain("a `Map`")),
            ("{s: new Set()}", "$.s", non_plain("a `Set`")),
            ("{d: new Date(0)}", "$.d", non_plain("a `Date`")),
            ("{r: /x/}", "$.r", non_plain("a `RegExp`")),
            ("{p: Promise.resolve()}", "$.p", non_plain("a `Promise`")),
            ("{e: new Error('x')}", "$.e", non_plain("an error")),
            (
                "{s: new String('x')}",
                "$.s",
                non_plain("a `String` object"),
            ),
            ("{n: new Number(1)}", "$.n", non_plain("a `Number` object")),
            (
                "{b: new Boolean(true)}",
                "$.b",
                non_plain("a `Boolean` object"),
            ),
            ("{t: new Uint8Array(2)}", "$.t", non_plain("a typed array")),
            (
                "{b: new ArrayBuffer(2)}",
                "$.b",
                non_plain("an `ArrayBuffer`"),
            ),
            (
                "{v: new DataView(new ArrayBuffer(2))}",
                "$.v",
                non_plain("a `DataView`"),
            ),
            ("{w: new WeakMap()}", "$.w", non_plain("a `WeakMap`")),
            (
                "{a: (function () { return arguments; })(1)}",
                "$.a",
                non_plain("an `arguments` object"),
            ),
            ("{p: new Proxy({}, {})}", "$.p", non_plain("a `Proxy`")),
            ("{p: new Proxy([], {})}", "$.p", non_plain("a `Proxy`")),
            // exotic whatever its prototype
            (
                "{t: Object.setPrototypeOf(new Uint8Array(2), Object.prototype)}",
                "$.t",
                non_plain("a typed array"),
            ),
            (
                "{m: Object.setPrototypeOf(new Map(), null)}",
                "$.m",
                non_plain("a `Map`"),
            ),
            (
                "{c: new (class Point { x = 1; })()}",
                "$.c",
                non_plain(
                    "an object whose prototype is neither `Object.prototype` nor null (its constructor is `Point`)",
                ),
            ),
            (
                "{c: Object.create({inherited: 1})}",
                "$.c",
                non_plain("an object whose prototype is neither `Object.prototype` nor null"),
            ),
            (
                "{c: Object.create(Object.create(Object.prototype))}",
                "$.c",
                non_plain("an object whose prototype is neither `Object.prototype` nor null"),
            ),
            (
                "{a: Object.setPrototypeOf([1], null)}",
                "$.a",
                non_plain("an array whose prototype is not `Array.prototype`"),
            ),
            (
                "{a: new (class List extends Array {})()}",
                "$.a",
                non_plain("an array whose prototype is not `Array.prototype`"),
            ),
            (
                "(() => { const a = {}; a.self = a; return a; })()",
                "$.self",
                Reason::Cycle,
            ),
            (
                "(() => { const a = [1]; a.push({back: [a]}); return a; })()",
                "$[1].back[0]",
                Reason::Cycle,
            ),
            (
                "{s: 'a\\uD800b'}",
                "$.s",
                Reason::LoneSurrogate {
                    unit: 0xD800,
                    in_key: false,
                },
            ),
            (
                "{s: 'tail\\uD83D'}",
                "$.s",
                Reason::LoneSurrogate {
                    unit: 0xD83D,
                    in_key: false,
                },
            ),
            (
                "{o: {'\\uDC00': 1}}",
                "$.o[\"\u{FFFD}\"]",
                Reason::LoneSurrogate {
                    unit: 0xDC00,
                    in_key: true,
                },
            ),
            ("{$a_1: {b: [undefined]}}", "$.$a_1.b[0]", Reason::Undefined),
            // the path after nested siblings closed
            (
                "{a: {b: [1, {c: []}]}, d: [{}, [2, [3]], undefined]}",
                "$.d[2]",
                Reason::Undefined,
            ),
            (
                "[[[{a: {}}]], {b: {c: {}}}, () => 1]",
                "$[2]",
                Reason::Function,
            ),
            (
                "(() => { const shared = {}; return {a: shared, b: {shared}, c: [shared, 1n]}; })()",
                "$.c[1]",
                Reason::BigInt,
            ),
            ("{'1x': [undefined]}", "$[\"1x\"][0]", Reason::Undefined),
            ("{7: [undefined]}", "$[\"7\"][0]", Reason::Undefined),
        ];
        for (expression, at, reason) in &cases {
            realm.assert_refused(expression, at, reason);
        }
    });
}

/// No getter, setter, proxy trap, or `toJSON` runs, and no patched builtin
/// is consulted: the walk reads V8's state, never the realm's methods.
#[test]
fn nothing_the_realm_can_patch_runs() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        realm.run(PATCHES);
        let written = realm
            .serialize("({a: [1, {b: 2.5, 2: 'x'}], s: 'text', n: null, o: {}, e: [], 1: true})");
        assert_eq!(
            written.as_deref(),
            Ok(r#"{"1":true,"a":[1,{"2":"x","b":2.5}],"s":"text","n":null,"o":{},"e":[]}"#)
        );
        // a hole falls to the prototype's index getter in JS, never here
        assert_eq!(
            realm.serialize("[0, , 2]").unwrap_err().reason,
            Reason::Hole
        );
        assert_eq!(realm.string("calls.join()"), "");
    });
}

/// Every method a JS serializer would reach for, replaced by one that
/// records its call, and getters for the keys a descriptor read through the
/// prototype chain would find.
const PATCHES: &str = "globalThis.calls = [];
const record = (name) => function () { calls.push(name); return 'patched'; };
JSON.stringify = record('JSON.stringify');
Array.prototype.toJSON = record('Array.prototype.toJSON');
Object.prototype.toJSON = record('Object.prototype.toJSON');
Object.keys = record('Object.keys');
Object.entries = record('Object.entries');
Object.getOwnPropertyDescriptor = record('Object.getOwnPropertyDescriptor');
Object.getPrototypeOf = record('Object.getPrototypeOf');
Array.isArray = record('Array.isArray');
Number.prototype.toString = record('Number.prototype.toString');
String.prototype.toString = record('String.prototype.toString');
for (const key of ['value', 'get', 'set', 'writable', 'enumerable']) {
\tObject.defineProperty(Object.prototype, key, {__proto__: null, get: record(`Object.prototype.${key}`), configurable: true});
}
Object.defineProperty(Array.prototype, '1', {__proto__: null, get: record('Array.prototype[1]'), configurable: true});
globalThis.Object = record('Object');
globalThis.Array = record('Array');
";

#[test]
fn no_getter_or_setter_runs() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        realm.run("globalThis.called = [];");
        let accessor = Reason::Accessor;
        realm.assert_refused(
            "{a: {get b() { called.push('get b'); return 1; }}}",
            "$.a.b",
            &accessor,
        );
        realm.assert_refused("{set c(value) { called.push('set c'); }}", "$.c", &accessor);
        realm.assert_refused(
            "(() => { const a = [1]; Object.defineProperty(a, 0, {get() { called.push('get 0'); return 1; }, enumerable: true}); return a; })()",
            "$[0]",
            &accessor,
        );
        // a non-enumerable accessor is skipped, as JSON.stringify skips it, and not run
        assert_eq!(
            realm
                .serialize(
                    "Object.defineProperty({a: 1}, 'x', {get() { called.push('get x'); return 1; }})"
                )
                .as_deref(),
            Ok(r#"{"a":1}"#)
        );
        // nor is a Proxy's trap
        realm.assert_refused(
            "{p: new Proxy({}, {ownKeys() { called.push('ownKeys'); return []; }, getPrototypeOf() { called.push('getPrototypeOf'); return null; }})}",
            "$.p",
            &Reason::NonPlain("a `Proxy`".to_owned()),
        );
        assert_eq!(realm.string("called.join()"), "");
    });
}

/// Nested to the limit serializes; one more is refused at its path; and a
/// value far deeper is refused the same way, never overflowing the stack.
#[test]
fn nesting_is_bounded() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        let nested = |depth: usize| {
            format!(
                "(() => {{ let v = 1; for (let i = 0; i < {depth}; i++) v = [v]; return v; }})()"
            )
        };
        let at_limit = realm.serialize(&nested(MAX_DEPTH)).unwrap();
        assert_eq!(at_limit.len(), 2 * MAX_DEPTH + 1);
        let error = realm.serialize(&nested(MAX_DEPTH + 1)).unwrap_err();
        assert_eq!(error.reason, Reason::TooDeep);
        assert_eq!(error.at, format!("${}", "[0]".repeat(MAX_DEPTH)));
        let error = realm.serialize(&nested(1_000_000)).unwrap_err();
        assert_eq!(error.reason, Reason::TooDeep);
        // objects count as arrays do
        let objects = format!(
            "(() => {{ let v = {{}}; for (let i = 0; i < {MAX_DEPTH}; i++) v = {{v}}; return v; }})()"
        );
        assert_eq!(
            realm.serialize(&objects).unwrap_err().reason,
            Reason::TooDeep
        );
    });
}

#[test]
fn the_messages_name_the_path() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        let error = realm.serialize("({a: [() => 1]})").unwrap_err();
        assert_eq!(
            error.to_string(),
            "$.a[0] is a function; the value is data, so call it or write what it returns"
        );
        assert!(!error.is_internal());
    });
}

/// A key a path shows is cut past [`MAX_KEY_SHOWN`] UTF-16 units and
/// named by its length, so a path stays small whatever the keys: a key at
/// the limit is shown whole, as before, an identifier past it is cut like
/// any other key, and a cut that would split a surrogate pair takes one
/// unit fewer.
#[test]
fn a_long_key_is_cut_in_the_path() {
    assert_eq!(MAX_KEY_SHOWN, 64);
    on_eval_stack(|| {
        let mut realm = Realm::new();
        let at = |realm: &mut Realm, key: &str| {
            let error = realm
                .serialize(&format!("({{a: {{[{key}]: 1n}}}})"))
                .unwrap_err();
            assert_eq!(error.reason, Reason::BigInt, "{key}");
            error.at
        };
        let a64 = "a".repeat(64);
        assert_eq!(at(&mut realm, "'a'.repeat(64)"), format!("$.a.{a64}"));
        assert_eq!(
            at(&mut realm, "' '.repeat(64)"),
            format!("$.a[\"{}\"]", " ".repeat(64))
        );
        assert_eq!(
            at(&mut realm, "'a'.repeat(65)"),
            format!("$.a[\"{a64}…\" (65 units)]")
        );
        assert_eq!(
            at(&mut realm, "'\"'.repeat(1000)"),
            format!("$.a[\"{}…\" (1000 units)]", "\\\"".repeat(64))
        );
        // U+1F600 is two units, the 64th and 65th
        assert_eq!(
            at(&mut realm, "'a'.repeat(63) + '\u{1F600}b'"),
            format!("$.a[\"{}…\" (66 units)]", "a".repeat(63))
        );
        // a cut key leads into the members below it too
        let error = realm
            .serialize("({['k'.repeat(100)]: {b: [1, () => 1]}})")
            .unwrap_err();
        assert_eq!(
            error.at,
            format!("$[\"{}…\" (100 units)].b[1]", "k".repeat(64))
        );
    });
}

/// A key whose JSON passes the cap partway, each unit written as a
/// six-byte escape, is refused with a short path and message, not the key
/// escaped again in each.
#[test]
fn a_key_past_the_cap_is_named_short() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        let max = 1 << 16;
        let error = realm
            .serialize_capped("({['\\u0001'.repeat((1 << 16) - 16)]: 1})", max)
            .unwrap_err();
        assert_eq!(error.reason, Reason::TooLarge { max });
        assert_eq!(
            error.at,
            format!("$[\"{}…\" (65520 units)]", "\\u0001".repeat(64))
        );
        assert!(error.to_string().len() < 512, "{error}");
    });
}

/// An object with internal fields is refused whatever its prototype: the
/// global object, made to look plain.
#[test]
fn a_host_object_is_refused_whatever_its_prototype() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        realm.assert_refused(
            "(Object.setPrototypeOf(globalThis, Object.prototype), {g: globalThis})",
            "$.g",
            &Reason::NonPlain("a host object".to_owned()),
        );
    });
}

/// Two objects sharing an identity hash, one inside the other, are a shared
/// value, not a cycle: a hash hit is confirmed by identity.
#[test]
fn an_identity_hash_collision_is_not_a_cycle() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        let pool = realm.run("globalThis.pool = Array.from({length: 1 << 16}, () => ({})); pool");
        let (i, j) = {
            let runtime = &mut realm.runtime;
            deno_core::scope!(scope, runtime);
            let pool: v8::Local<'_, v8::Object> = v8::Local::new(scope, pool).cast();
            let mut seen = std::collections::HashMap::new();
            let mut found = None;
            for index in 0..(1u32 << 16) {
                let object: v8::Local<'_, v8::Object> =
                    pool.get_index(scope, index).unwrap().cast();
                if let Some(first) = seen.insert(object.get_identity_hash(), index) {
                    found = Some((first, index));
                    break;
                }
            }
            found.expect("no two of the pool's objects share an identity hash")
        };
        assert_eq!(
            realm
                .serialize(&format!(
                    "pool[{i}].child = pool[{j}]; ({{x: pool[{i}], y: [pool[{j}]]}})"
                ))
                .as_deref(),
            Ok(r#"{"x":{"child":{}},"y":[{}]}"#)
        );
        realm.assert_refused(
            &format!("pool[{j}].back = pool[{i}], {{x: pool[{i}]}}"),
            "$.x.child.back",
            &Reason::Cycle,
        );
    });
}

/// The walk asks before each batch whether the run was stopped, and ends
/// there: a value of a few batches stops after the first once asked to, at
/// the member the walk had reached.
#[test]
fn a_stop_ends_the_walk_at_a_batch() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        let value = realm.run("Array.from({length: 4096}, (_, i) => [i])");
        let runtime = &mut realm.runtime;
        deno_core::scope!(scope, runtime);
        let value = v8::Local::new(scope, value);
        let asked = std::cell::Cell::new(0);
        let stopped = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        let error = to_json(scope, value, &stopped, usize::MAX).unwrap_err();
        assert_eq!(error.reason, Reason::Stopped);
        assert_eq!(asked.get(), 2);
        assert!(!error.is_internal());
        // the first batch's steps read into the array's members
        assert!(error.at.starts_with("$["), "{}", error.at);
        // never asked, a walk of the same value runs to its end
        assert!(to_json(scope, value, &|| false, usize::MAX).is_ok());
    });
}

/// The JSON is capped: a value whose JSON is exactly the cap is written,
/// and one byte more is refused, whether the last byte is a string's read
/// whole (its length checked before it's read), a string's escape, a key's,
/// or a closing bracket; and a walk that passes the cap partway is refused
/// where it was.
#[test]
fn the_json_is_capped_at_its_last_byte() {
    on_eval_stack(|| {
        let mut realm = Realm::new();
        for expression in [
            // the length check before the read
            "'x'.repeat(100)",
            // escapes, past the length check: each `\n` writes two bytes
            "'\\n'.repeat(50)",
            "({a: 1, ['k'.repeat(40)]: 2})",
            "Array.from({length: 2000}, (_, i) => i)",
            "[[[1, 2], [3]], {b: [true, null]}]",
        ] {
            let source = format!("({expression})");
            let json = realm.serialize(&source).unwrap();
            assert_eq!(
                realm.serialize_capped(&source, json.len()).as_deref(),
                Ok(json.as_str()),
                "{expression}"
            );
            let max = json.len() - 1;
            let error = realm.serialize_capped(&source, max).unwrap_err();
            assert_eq!(
                (error.at.as_str(), &error.reason),
                ("$", &Reason::TooLarge { max }),
                "{expression}"
            );
            assert!(!error.is_internal());
        }
        for (expression, max, at) in [
            ("({a: ['x'.repeat(1 << 20)]})", 1 << 20, "$.a[0]"),
            ("({a: 1, ['k'.repeat(1 << 20)]: 2})", 1 << 20, "$"),
            ("({a: '\\n'.repeat(1 << 19)})", 1 << 20, "$.a"),
            ("Array.from({length: 1 << 20}, (_, i) => i)", 1 << 20, "$["),
        ] {
            let error = realm
                .serialize_capped(&format!("({expression})"), max)
                .unwrap_err();
            assert_eq!(error.reason, Reason::TooLarge { max }, "{expression}");
            assert!(error.at.starts_with(at), "{expression}: {}", error.at);
        }
        let error = realm
            .serialize_capped("({a: ['x'.repeat(1 << 20)]})", 1 << 20)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "$.a[0] takes the value's JSON past 1 MiB, the envelope's cap; write less data"
        );
    });
}
