//! Tests of the native members: each row's namespace, functions, values, and hashers.

use super::*;

/// Puts what the blake3 row's namespace and hashers need in `runtime`'s
/// `OpState`, in a run that grants nothing, as a run's start does.
fn put_blake3_state(runtime: &mut JsRuntime) {
    let state = runtime.op_state();
    state.borrow_mut().put(Authority::default());
    state.borrow_mut().put(ErrorClasses::default());
    let namespaces = NativeNamespaces::new(runtime);
    state.borrow_mut().put(namespaces);
    state.borrow_mut().put(Blake3State::default());
}

/// The JS the native members' probe defines first: `thrown`, what a
/// call threw, as `Class: message`, or `null`; `shape`, a value's type,
/// whether it's frozen, its prototype, and its own keys; `function`,
/// a function's shape, name, `length`, whether its source text is native
/// code's, and the class of what `new` of it threw; `functions`, the
/// function shape of each of a record's functions the table names, each
/// kept in `members` under its member's label for the refusals; and
/// `namespaces`, where the probe keeps each row's.
const NATIVE_HELPERS: &str = "globalThis.thrown = (call) => {
	try {
		const value = call();
		if (!(value instanceof Promise)) return null;
		value.catch(() => {});
		const [state, error] = Deno.core.ops.op_get_promise_details(value);
		return state === 2 ? `rejected: ${error.constructor.name}: ${error.message}` : null;
	} catch (error) {
		return `${error.constructor.name}: ${error.message}`;
	}
};
// a key by the name the table gives it: a well-known symbol's as JS names a method of it
globalThis.name_of = (key) => typeof key === 'symbol' ? `[${key.description}]` : key;
globalThis.key_of = (name) => name.startsWith('[Symbol.') ? Symbol[name.slice(8, -1)] : name;
globalThis.shape = (value) => {
	const prototype = Object.getPrototypeOf(value);
	return {
		type: typeof value,
		frozen: Object.isFrozen(value),
		prototype: prototype === null ? null : prototype === Function.prototype ? 'Function.prototype' : 'other',
		keys: Reflect.ownKeys(value).map(name_of),
	};
};
globalThis.function_shape = (fn) => ({
	...shape(fn),
	name: fn.name,
	length: fn.length,
	native_source: Function.prototype.toString.call(fn) === `function ${fn.name}() { [native code] }`,
	constructed: (() => {
		try {
			new fn();
			return null;
		} catch (error) {
			return error.constructor.name;
		}
	})(),
});
globalThis.members = [];
globalThis.functions = (record, module, names) => {
	const found = {};
	for (const key of names) {
		members.push([`${key} of ${module}`, record[key_of(key)]]);
		found[key] = function_shape(record[key_of(key)]);
	}
	return found;
};
globalThis.namespaces = {};";

/// The names of the native functions `module` puts at `place`, in the
/// table's order, as JSON.
fn function_names(module: &str, place: Place) -> String {
    let names: Vec<&str> = NATIVE_MEMBERS
        .iter()
        .filter(|member| member.module == module && member.place == place)
        .filter(|member| matches!(member.member, Member::Function { .. }))
        .map(|member| member.name)
        .collect();
    serde_json::to_string(&names).unwrap()
}

/// The rows with native members, each once, in the table's order.
fn native_rows() -> Vec<&'static str> {
    let mut rows: Vec<&str> = Vec::new();
    for member in NATIVE_MEMBERS {
        if !rows.contains(&member.module) {
            rows.push(member.module);
        }
    }
    rows
}

/// The kinds of handle native methods go on, each once, with the row
/// each is of.
fn native_kinds() -> Vec<(&'static str, &'static crate::host::handle::HandleKind)> {
    let mut kinds: Vec<(&str, &crate::host::handle::HandleKind)> = Vec::new();
    for member in NATIVE_MEMBERS {
        if let Place::Method(kind) = member.place
            && !kinds.iter().any(|(_, held)| *held == kind)
        {
            kinds.push((member.module, kind));
        }
    }
    kinds
}

/// What the probe reads of the values the rows' namespaces hold, made
/// in a run that grants each row, with [`host_data`]: each as JSON,
/// with whether the arguments' array is frozen, and what `cwd` returns.
const NATIVE_VALUES: &str = "{
	const ns = (row) => namespaces[`@fuzdev/gro/${row}.ts`];
	JSON.stringify({
		args: ns('args').args,
		args_shape: shape(ns('args').args),
		args_array: Array.isArray(ns('args').args),
		cwd: ns('cwd').cwd(),
		pid: ns('pid').pid,
		runtime_version: ns('runtime_version').runtime_version,
		env: shape(ns('env').env),
		env_reads: [ns('env').env.get('GRO_TEST_ENV'), Reflect.ownKeys(ns('env').env.get_all())],
	});
}";

/// What the native rows' namespaces are on a runtime started as
/// `start` says: what the namespace op threw in a run that grants
/// nothing, first, called by a script that is no row's (one that names a
/// row's script in a `sourceURL` among them), by the script of a row
/// whose grant the run lacks, and by a row's script with a scope or an
/// `Error` it refuses, and whether a trap ran; in a run that grants
/// every row, what each row's namespace op answered its own script,
/// twice, and the shapes of its record and its functions; the shapes of
/// each kind's prototype the host made with it and its methods; how many
/// namespaces the run held before, after the refusals, and after; what a member
/// of the probe table, which needs `args`, did in a run that grants
/// nothing, one that grants `args`, and a runtime with no authority; and
/// what every member threw called in that runtime.
pub(super) fn native_members(start: Start) -> Value {
    on_runtime("native_members", start, Mode::Run, move |runtime| {
        put_blake3_state(runtime);
        let state = runtime.op_state();
        state.borrow_mut().put(host_data());
        state.borrow_mut().put(ErrorClasses::default());
        drop(state);
        let held = |runtime: &mut JsRuntime| {
            runtime
                .op_state()
                .borrow()
                .borrow::<NativeNamespaces>()
                .len()
        };
        let before = held(runtime);
        runtime.execute_script("<test>", NATIVE_HELPERS).unwrap();
        // what each caller and each argument the op refuses gets, before
        // anything is held
        let call = |arguments: &str| {
            format!("JSON.stringify(thrown(() => Deno.core.ops.op_gro_namespace({arguments})))")
        };
        let trap = "new Proxy({}, { get() { globalThis.trapped = true; } })";
        let blake3 = backing_script("@fuzdev/gro/blake3.ts");
        // code of a caller's a row's script calls, which calls the op
        runtime
            .execute_script(
                "<test>",
                "globalThis.calls_op = () => Deno.core.ops.op_gro_namespace(undefined);",
            )
            .unwrap();
        let mut rows_refused = Vec::new();
        for (name, source) in [
            ("<test>", call("undefined")),
            (
                blake3,
                "JSON.stringify(thrown(() => calls_op()))".to_owned(),
            ),
            (
                "file:///project/node_modules/x/@fuzdev/gro/blake3.ts",
                call("undefined"),
            ),
            (
                "<test>",
                format!("{}\n//# sourceURL={blake3}", call("undefined")),
            ),
            ("ext:gro/builtin/nope.ts", call("undefined")),
            (backing_script("@fuzdev/gro/args.ts"), call("undefined")),
            (blake3, call("null")),
            (blake3, call(trap)),
            (blake3, call("undefined, Error")),
            (blake3, call("undefined, null")),
            (blake3, call("undefined, new Proxy(Error, {})")),
            (blake3, call(&format!("undefined, {trap}"))),
            (
                backing_script("@fuzdev/gro/env.ts"),
                call("undefined, Error"),
            ),
            (
                backing_script("@fuzdev/gro/env_error.ts"),
                call("null, Error"),
            ),
        ] {
            rows_refused.push(named_script_json(runtime, name, source));
        }
        rows_refused.push(script_json(
            runtime,
            "JSON.stringify(globalThis.trapped ?? false)",
        ));
        let errors_held = runtime
            .op_state()
            .borrow()
            .borrow::<NativeNamespaces>()
            .len();
        // a refused row's script hands its `Error` over to nothing
        rows_refused.push(json!(
            runtime
                .op_state()
                .borrow()
                .borrow::<ErrorClasses>()
                .holds_error()
        ));
        let mut namespaces = serde_json::Map::new();
        // every row's namespace is made in a run that grants each row
        runtime
            .op_state()
            .borrow_mut()
            .put(crate::test_support::run_authority());
        for row in native_rows() {
            let names = function_names(row, Place::Export);
            // what the row's script hands over, the realm's `Error` from
            // an error family's row alone
            let arguments = if hands_over_error(row) {
                "undefined, Error"
            } else {
                "undefined"
            };
            let source = format!(
                "{{
	const ops = Deno.core.ops;
	const namespace = ops.op_gro_namespace({arguments});
	namespaces['{row}'] = namespace;
	JSON.stringify({{
		same: namespace === ops.op_gro_namespace({arguments}),
		record: shape(namespace),
		default: namespace.default === undefined,
		functions: functions(namespace, '{row}', {names}),
	}});
}}"
            );
            let made = named_script_json(runtime, backing_script(row), source);
            namespaces.insert(row.to_owned(), made);
        }
        let namespaces = Value::Object(namespaces);
        let after = held(runtime);
        let values = script_json(runtime, NATIVE_VALUES);
        runtime.op_state().borrow_mut().put(Authority::default());
        {
            let state = runtime.op_state();
            let state = state.borrow();
            deno_core::scope!(scope, runtime);
            let global = scope.get_current_context().global(scope);
            let prototypes = v8::Object::new(scope);
            for (_, kind) in native_kinds() {
                // none when the run holds no namespace of its row
                if let Some(prototype) = native::prototype(&state, scope, kind) {
                    let key = v8::String::new(scope, kind.brand).unwrap();
                    prototypes.set(scope, key.into(), prototype.into()).unwrap();
                }
            }
            let key = v8::String::new(scope, "prototypes").unwrap();
            global.set(scope, key.into(), prototypes.into()).unwrap();
            let probe = native::probe::function(scope).unwrap();
            let key = v8::String::new(scope, "probe").unwrap();
            global.set(scope, key.into(), probe.into()).unwrap();
        }
        let prototypes = script_json(
            runtime,
            "{
	const result = {};
	for (const [brand, prototype] of Object.entries(prototypes)) {
		const names = Reflect.ownKeys(prototype).filter((key) => typeof prototype[key] === 'function').map(name_of);
		result[brand] = { record: shape(prototype), functions: functions(prototype, brand, names) };
	}
	JSON.stringify(result);
}",
        );
        // each value declared a handle, by its row, name, kind, and the
        // kind whose record holds it, if it isn't an export
        let handles: Vec<[Option<&str>; 4]> = NATIVE_MEMBERS
            .iter()
            .filter_map(|member| match member.member {
                Member::Value {
                    kind: ValueKind::Handle(kind),
                    ..
                } => Some([
                    Some(member.module),
                    Some(member.name),
                    Some(kind.brand),
                    match member.place {
                        Place::Export => None,
                        Place::Method(on) => Some(on.brand),
                    },
                ]),
                _ => None,
            })
            .collect();
        // a handle is made of its kind's prototype, and a record kind's
        // value is its one record
        let handles_of_held = script_json(
            runtime,
            format!(
                "JSON.stringify({}.map(([module, name, brand, on]) => {{
	const value = on === null ? namespaces[module][name] : prototypes[on][name];
	return Object.getPrototypeOf(value) === prototypes[brand]
		|| (value === prototypes[brand] && Object.getPrototypeOf(value) === null);
}}))",
                json!(handles)
            ),
        );
        let probe = "JSON.stringify(thrown(() => { globalThis.probed = probe(); }) ?? probed)";
        let probe_empty = script_json(runtime, probe);
        runtime
            .op_state()
            .borrow_mut()
            .put(Authority::Confined(Grants {
                args: Some(ArgsGrant {}),
                ..Grants::default()
            }));
        let probe_granted = script_json(runtime, probe);
        runtime.op_state().borrow_mut().take::<Authority>();
        let probe_none = script_json(runtime, probe);
        let hashers = |runtime: &mut JsRuntime| {
            runtime
                .op_state()
                .borrow()
                .borrow::<Blake3State>()
                .hashers_len()
        };
        let hashers_before = hashers(runtime);
        let refused = script_json(
            runtime,
            "JSON.stringify(Object.fromEntries(members.map(([label, fn]) => [label, thrown(() => fn())])))",
        );
        let hashers_after = hashers(runtime);
        // each value made with no authority: whether it was, and what
        // its maker threw
        let mut value_refusals = serde_json::Map::new();
        {
            let state = runtime.op_state();
            let state = state.borrow();
            deno_core::scope!(scope, runtime);
            v8::tc_scope!(let tc, scope);
            for member in NATIVE_MEMBERS {
                let Member::Value { make, .. } = member.member else {
                    continue;
                };
                let made = make(&state, tc, &Prototypes::none()).is_some();
                let thrown = tc.exception().map(|thrown| thrown.to_rust_string_lossy(tc));
                tc.reset();
                let label = format!("{} of {}", member.name, member.module);
                value_refusals.insert(label, json!([made, thrown]));
            }
        }
        let probe_ran = native::probe::RAN.load(std::sync::atomic::Ordering::Relaxed);
        // the probe table's value made in a run that grants nothing, one
        // that grants `args`, and a runtime with no authority: what it
        // gave or threw; then its namespace, made in a run that grants
        // `args`, whole, and with its value failing
        let granted_args = Authority::Confined(Grants {
            args: Some(ArgsGrant {}),
            ..Grants::default()
        });
        let mut value_probe = Vec::new();
        for authority in [Some(Authority::default()), Some(granted_args.clone()), None] {
            match authority {
                Some(authority) => runtime.op_state().borrow_mut().put(authority),
                None => drop(runtime.op_state().borrow_mut().try_take::<Authority>()),
            }
            let state = runtime.op_state();
            let state = state.borrow();
            deno_core::scope!(scope, runtime);
            v8::tc_scope!(let tc, scope);
            let made = native::probe::value(&state, tc).map(|made| made.to_rust_string_lossy(tc));
            let thrown = tc.exception().map(|thrown| thrown.to_rust_string_lossy(tc));
            tc.reset();
            value_probe.push(json!([made, thrown]));
        }
        let value_made = native::probe::MADE.load(std::sync::atomic::Ordering::Relaxed);
        runtime.op_state().borrow_mut().put(granted_args);
        let mut namespace_probe = Vec::new();
        for fail in [false, true] {
            native::probe::FAIL.store(fail, std::sync::atomic::Ordering::Relaxed);
            let state = runtime.op_state();
            let state = state.borrow();
            deno_core::scope!(scope, runtime);
            v8::tc_scope!(let tc, scope);
            let made = native::probe::namespace_made(&state, tc);
            let thrown = tc.exception().map(|thrown| thrown.to_rust_string_lossy(tc));
            tc.reset();
            namespace_probe.push(json!([made, thrown]));
        }
        native::probe::FAIL.store(false, std::sync::atomic::Ordering::Relaxed);
        json!({
            "namespaces": namespaces,
            "values": values,
            "handles_of_held": handles_of_held,
            "prototypes": prototypes,
            "rows_refused": rows_refused,
            "held": [before, errors_held, after],
            "probe": [probe_empty, probe_granted, probe_none],
            "refused": refused,
            "refused_effects": [hashers_before, hashers_after, probe_ran],
            "value_refusals": value_refusals,
            "value_probe": [value_probe, value_made, namespace_probe],
            "pid": std::process::id(),
        })
    })
}

/// The function shape the probe reads of `member`'s function, by its
/// name, `None` for a value: frozen, of the realm's
/// `Function.prototype`, with its own `length` and `name` only, so no
/// `prototype`, its name and `length` the table's, its source text
/// native code's, and not a constructor.
fn native_function_shape(member: &NativeMember) -> Option<(String, Value)> {
    let Member::Function { length, .. } = member.member else {
        return None;
    };
    let shape = json!({
        "type": "function",
        "frozen": true,
        "prototype": "Function.prototype",
        "keys": ["length", "name"],
        "name": member.name,
        "length": length,
        "native_source": true,
        "constructed": "TypeError",
    });
    Some((member.name.to_owned(), shape))
}

/// The native members from either start: each row's namespace is made
/// once and held, the same object every ask, a frozen null-prototype
/// record of exactly the table's exports and the `default` every
/// builtin has, `undefined`; each kind's prototype a frozen
/// null-prototype record of exactly the table's methods; each function
/// frozen, named and of the `length` the table says, with no
/// `prototype`, its source text native code's, and no constructor.
#[test]
fn a_native_row_s_namespace_is_the_table_s_made_once() {
    for start in STARTS {
        let results = probed("native_members", start);
        let rows = native_rows();
        assert_eq!(results["held"], json!([0, 0, rows.len()]), "{start:?}");
        for row in rows {
            let exports: Vec<&NativeMember> = NATIVE_MEMBERS
                .iter()
                .filter(|member| member.module == row && member.place == Place::Export)
                .collect();
            let mut keys: Vec<&str> = exports.iter().map(|member| member.name).collect();
            keys.push("default");
            let functions: serde_json::Map<String, Value> = exports
                .iter()
                .filter_map(|member| native_function_shape(member))
                .collect();
            assert_eq!(
                results["namespaces"][row],
                json!({
                    "same": true,
                    "record": {"type": "object", "frozen": true, "prototype": null, "keys": keys},
                    "default": true,
                    "functions": functions,
                }),
                "{start:?} {row}"
            );
        }
        let kinds = native_kinds();
        assert!(!kinds.is_empty());
        for (_, kind) in &kinds {
            let methods: Vec<&NativeMember> = NATIVE_MEMBERS
                .iter()
                .filter(|member| member.place == Place::Method(kind))
                .collect();
            let keys: Vec<&str> = methods.iter().map(|member| member.name).collect();
            let functions: serde_json::Map<String, Value> = methods
                .iter()
                .filter_map(|member| native_function_shape(member))
                .collect();
            // a kind's prototype extends its parent's
            let prototype = kind.parent.map(|_| "other");
            assert_eq!(
                results["prototypes"][kind.brand],
                json!({
                    "record": {"type": "object", "frozen": true, "prototype": prototype, "keys": keys},
                    "functions": functions,
                }),
                "{start:?} {}",
                kind.brand
            );
        }
        assert_eq!(
            results["prototypes"].as_object().unwrap().len(),
            kinds.len(),
            "{start:?}"
        );
    }
}

/// What `member`, a `what` (a host function or a host value), throws in
/// a runtime that holds no authority: `not_granted` naming it, and its
/// grant when it needs one.
fn member_refusal(member: &NativeMember, what: &str) -> String {
    let label = format!("{} of {}", member.name, member.module);
    let rejected = match member.member {
        Member::Function {
            asynchronous: true, ..
        } => "rejected: ",
        _ => "",
    };
    let refusal = member.grant.map_or_else(
        || {
            format!(
                "Error: not_granted: {label} is a {what}, and this runtime holds no \
                     authority, so it runs none"
            )
        },
        |grant| {
            format!(
                "Error: not_granted: {label} needs the grant '{grant}', which this runtime \
                     doesn't hold"
            )
        },
    );
    format!("{rejected}{refusal}")
}

/// The form's check from either start: every native function, called
/// in a runtime that holds no authority, throws `not_granted` naming
/// itself, and its grant when it needs one, before its body runs, an
/// `async` one rejecting the promise it returns with it,
/// whatever it's passed; and a member that needs
/// a grant (the probe table's) throws it, naming the grant, in a run that
/// grants nothing and in a runtime with no authority, and runs in a run
/// that grants it. A body that skips its check returns or throws what it
/// does instead, and one that runs before its check leaves its effect:
/// a refused factory made no hasher, and the probe's body ran only where
/// it was granted.
#[test]
fn every_native_member_refuses_without_its_grant() {
    for start in STARTS {
        let results = probed("native_members", start);
        let refused = results["refused"].as_object().unwrap();
        let functions = NATIVE_MEMBERS
            .iter()
            .filter(|member| matches!(member.member, Member::Function { .. }));
        assert_eq!(refused.len(), functions.clone().count(), "{start:?}");
        for member in functions {
            let label = match member.place {
                Place::Export => format!("{} of {}", member.name, member.module),
                Place::Method(kind) => format!("{} of {}", member.name, kind.brand),
            };
            assert_eq!(
                refused[&label],
                member_refusal(member, "host function"),
                "{start:?}"
            );
        }
        assert_eq!(
            results["probe"],
            json!([
                "Error: not_granted: probe of @fuzdev/gro/args.ts needs the grant 'args', \
                     which this runtime doesn't hold",
                "ran",
                "Error: not_granted: probe of @fuzdev/gro/args.ts needs the grant 'args', \
                     which this runtime doesn't hold",
            ]),
            "{start:?}"
        );
        // no refused call's body ran: no hasher was made, and the
        // probe's body ran once, in the run that grants it
        assert_eq!(results["refused_effects"], json!([0, 0, 1]), "{start:?}");
    }
}

/// A row's values, from either start: each made once with its
/// namespace in a run that grants its row, of the run's data (the
/// arguments a frozen array of strings, the runtime's name and version,
/// the process's id), with `cwd` returning the run's directory; each
/// value the table declares a handle made of the prototype the run
/// holds for its kind; and each value's maker, run in a runtime that
/// holds no authority, throws
/// `not_granted` naming the value and makes nothing, a granted row's
/// naming its grant.
#[test]
fn a_native_row_s_values_are_the_run_s() {
    for start in STARTS {
        let results = probed("native_members", start);
        assert_eq!(
            results["values"],
            json!({
                "args": ["a"],
                "args_shape": {
                    "type": "object",
                    "frozen": true,
                    "prototype": "other",
                    "keys": ["0", "length"],
                },
                "args_array": true,
                "cwd": "/work",
                "pid": results["pid"],
                "runtime_version": "test/1.2.3",
                "env": {"type": "object", "frozen": true, "prototype": "other", "keys": []},
                "env_reads": [null, []],
            }),
            "{start:?}"
        );
        let values: Vec<&NativeMember> = NATIVE_MEMBERS
            .iter()
            .filter(|member| matches!(member.member, Member::Value { .. }))
            .collect();
        assert!(!values.is_empty());
        // each value declared a handle is one of the prototype the host
        // made for its kind and holds, as the table says (a root)
        let handles = NATIVE_MEMBERS
            .iter()
            .filter(|member| {
                matches!(
                    member.member,
                    Member::Value {
                        kind: ValueKind::Handle(_),
                        ..
                    }
                )
            })
            .count();
        assert!(handles > 0);
        assert_eq!(
            results["handles_of_held"],
            json!(vec![true; handles]),
            "{start:?}"
        );
        let refused = results["value_refusals"].as_object().unwrap();
        assert_eq!(refused.len(), values.len(), "{start:?}");
        for member in values {
            let label = format!("{} of {}", member.name, member.module);
            let thrown = member_refusal(member, "host value");
            assert_eq!(refused[&label], json!([false, thrown]), "{start:?}");
        }
        // a value of a row that needs a grant is made where it's granted
        // alone, its maker's body running past the check only there, and
        // a namespace one of whose values failed is never made
        let refused = "Error: not_granted: probe_value of @fuzdev/gro/args.ts needs the \
                           grant 'args', which this runtime doesn't hold";
        assert_eq!(
            results["value_probe"],
            json!([
                [[null, refused], ["made", null], [null, refused]],
                1,
                [[true, null], [false, "Error: the probe value failed"]],
            ]),
            "{start:?}"
        );
    }
}

/// The namespace op knows the row by the script that calls it, never by
/// what the script says, after its own check of an authority: a caller
/// that is no row's backing script is a `TypeError`, one that names a
/// row's script in a `sourceURL` and one of a name no row has among
/// them; a row whose grant the run lacks is refused naming the grant,
/// before anything is made; and a scope that isn't `undefined`, or
/// anything but `undefined` handed over as the realm's `Error` by a row
/// of no error family's, is a `TypeError`, read with no
/// conversion: no trap runs, and nothing is held.
#[test]
fn the_namespace_op_knows_the_row_by_its_script() {
    let not_a_row = "TypeError: a namespace is asked for by a builtin's own script";
    let not_a_scope = "TypeError: no row takes a scope yet";
    let not_handed = "TypeError: a row whose members declare no error family hands over no Error";
    for start in STARTS {
        assert_eq!(
            probed("native_members", start)["rows_refused"],
            json!([
                not_a_row,
                not_a_row,
                not_a_row,
                not_a_row,
                not_a_row,
                "Error: not_granted: op_gro_namespace needs the grant 'args', which this \
                     runtime doesn't hold",
                not_a_scope,
                not_a_scope,
                not_handed,
                not_handed,
                not_handed,
                not_handed,
                "Error: not_granted: op_gro_namespace needs the grant 'env', which this \
                     runtime doesn't hold",
                not_a_scope,
                false,
                false,
            ]),
            "{start:?}"
        );
    }
}

/// A hasher's native state lives as long as its handle can be reached:
/// a collection drops the state of each hasher no longer reachable, and
/// keeps a reachable one's, which still hashes; and V8 counts each live
/// hasher's native state as external memory, and stops when it's
/// dropped.
#[test]
fn a_hashers_native_state_is_dropped_once_its_handle_is_collected() {
    let (made, collected, digest, external) =
        on_runtime("blake3_collected", Start::Source, Mode::Run, |runtime| {
            put_blake3_state(runtime);
            // the external memory V8 counts, read as a change of nothing
            let external = |runtime: &mut JsRuntime| {
                runtime
                    .v8_isolate()
                    .adjust_amount_of_external_allocated_memory(0)
            };
            // what was garbage before the hashers is collected first
            runtime.v8_isolate().low_memory_notification();
            let before = external(runtime);
            runtime
                .execute_script(
                    backing_script("@fuzdev/gro/blake3.ts"),
                    "const { create_hasher } = Deno.core.ops.op_gro_namespace(undefined);
globalThis.kept = create_hasher();
for (let i = 0; i < 100; i++) create_hasher();",
                )
                .unwrap();
            let len = |runtime: &mut JsRuntime| {
                runtime
                    .op_state()
                    .borrow()
                    .borrow::<Blake3State>()
                    .hashers_len()
            };
            let made = len(runtime);
            let made_external = external(runtime) - before;
            runtime.v8_isolate().low_memory_notification();
            let collected = len(runtime);
            let collected_external = external(runtime) - before;
            let digest = script_json(
                runtime,
                "globalThis.kept.update(new Uint8Array([1]));
JSON.stringify(globalThis.kept.finalize().length)",
            );
            (made, collected, digest, (made_external, collected_external))
        });
    assert_eq!(made, 101);
    assert_eq!(collected, 1);
    assert_eq!(digest, 32);
    let size = i64::try_from(size_of::<blake3::Hasher>()).unwrap();
    assert_eq!(external, (101 * size, size));
}

/// A hasher's number is never one another had: a live hasher keeps its
/// own state across a collection of others and the making of more,
/// which a number reused once its hasher was collected would hand to a
/// new hasher, its entry overwriting the live one's.
#[test]
fn a_live_hasher_keeps_its_state_across_collections() {
    let kept = on_runtime("blake3_ids", Start::Source, Mode::Run, |runtime| {
        put_blake3_state(runtime);
        runtime
            .execute_script(
                backing_script("@fuzdev/gro/blake3.ts"),
                "globalThis.blake3 = Deno.core.ops.op_gro_namespace(undefined);
globalThis.live = [];
for (let i = 0; i < 4; i++) blake3.create_hasher();
for (let i = 0; i < 3; i++) {
\tconst hasher = blake3.create_hasher();
\thasher.update(new Uint8Array([i + 1]));
\tlive.push(hasher);
}
globalThis.before = live.map((hasher) => hasher.finalize().join());",
            )
            .unwrap();
        runtime.v8_isolate().low_memory_notification();
        script_json(
            runtime,
            "{
\tfor (let i = 0; i < 8; i++) {
\t\tconst hasher = blake3.create_hasher();
\t\thasher.update(new Uint8Array([200, i]));
\t\tlive.push(hasher);
\t}
\tJSON.stringify(live.slice(0, 3).map((hasher) => hasher.finalize().join()).join() === before.join());
}",
        )
    });
    assert_eq!(kept, true);
}
