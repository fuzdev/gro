//! Tests of the run's error classes: made once, at first need, by the host.

use super::*;

/// What the run's error classes are across calls of the namespace op by
/// the error row's script on a runtime started as `start` says, in a run
/// that grants nothing: how many classes the start made, before any
/// call, and after the calls, what each value handed over as the realm's
/// `Error` before any was held threw, whether each call got the one
/// namespace, of the class the host made once, frozen, extending the
/// `Error` first held, which a later value doesn't replace, and of the
/// guard.
pub(super) fn error_classes(start: Start) -> Value {
    let source = "const op = (error) => Deno.core.ops.op_gro_namespace(undefined, error);
const not_errors = [
	undefined,
	{},
	function NotError() {},
	TypeError,
	new Proxy(Error, {}),
	Object.setPrototypeOf(function Fake() {}, Error),
];
const refused = not_errors.map((value) => {
	try {
		op(value);
		return 'returned';
	} catch (error) {
		return `${error.constructor.name}: ${error.message}`;
	}
});
const a = op(Error);
const b = op(Error);
const c = op(TypeError);
JSON.stringify({
	refused,
	held: c.EnvError === a.EnvError && Object.getPrototypeOf(a.EnvError) === Error,
	namespaces: a === b && b === c,
	frozen: Object.isFrozen(a.EnvError) && Object.isFrozen(a.EnvError.prototype) && Object.isFrozen(a.is_env_error),
	named: a.EnvError.name === 'EnvError' && a.is_env_error.name === 'is_env_error',
	keys: Reflect.ownKeys(a),
	prototype: Object.getPrototypeOf(a),
	error: new a.EnvError('not_granted', 'HOME') instanceof Error,
})";
    on_runtime("error_classes", start, Mode::Run, move |runtime| {
        let state = runtime.op_state();
        state.borrow_mut().put(Authority::default());
        state.borrow_mut().put(ErrorClasses::default());
        let namespaces = NativeNamespaces::new(runtime);
        state.borrow_mut().put(namespaces);
        let before = state.borrow().borrow::<ErrorClasses>().len();
        drop(state);
        let row = backing_script("@fuzdev/gro/env_error.ts");
        let calls = named_script_json(runtime, row, source.to_owned());
        let after = runtime.op_state().borrow().borrow::<ErrorClasses>().len();
        json!({"before": before, "calls": calls, "after": after})
    })
}

/// A run's error class is made the first time it needs it, by the
/// host, once: each call of the namespace op by the row's script gets
/// the one namespace of the frozen class and the guard, from either
/// start; a start makes none.
#[test]
fn an_error_class_is_made_once_a_run_at_first_need() {
    for start in STARTS {
        assert_eq!(
            probed("error_classes", start),
            json!({
                "before": 0,
                "calls": {
                    "refused": vec!["TypeError: an error class extends the realm's Error"; 6],
                    "held": true,
                    "namespaces": true,
                    "frozen": true,
                    "named": true,
                    "keys": ["EnvError", "is_env_error", "default"],
                    "prototype": null,
                    "error": true,
                },
                "after": 1,
            }),
            "{start:?}"
        );
    }
}

/// A family missing from the host's table is never made a class of:
/// asked for its class, the host throws an `Error` naming it and holds
/// no class, where its class's constructor would otherwise return a
/// plain object.
#[test]
fn a_family_the_host_s_table_lacks_throws_when_its_class_is_made() {
    use crate::host::error_class::ErrorFamily;

    static MISSING: ErrorFamily = ErrorFamily {
        name: "MissingError",
        target: "target",
        brand: "MissingError",
    };
    let (message, made) = on_new_runtime("missing_family", Mode::Run, |runtime| {
        let classes = ErrorClasses::default();
        deno_core::scope!(scope, runtime);
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "Error").unwrap();
        let error = global.get(scope, key.into()).unwrap();
        v8::tc_scope!(let tc, scope);
        classes.hand_over(tc, error);
        assert!(
            classes
                .class(tc, DeclaredFamily::for_test(&MISSING))
                .is_none()
        );
        let thrown = tc.exception().unwrap();
        let thrown = v8::Local::<v8::Object>::try_from(thrown).unwrap();
        let message = v8::String::new(tc, "message").unwrap();
        let message = thrown.get(tc, message.into()).unwrap();
        (message.to_rust_string_lossy(tc), classes.len())
    });
    assert_eq!(
        message,
        "the host has no error family MissingError: a family's class is one of the host's \
             table"
    );
    assert_eq!(made, 0);
}

/// The realm's `Error` a row hands over is held unchecked until a class
/// is first made of it: `env`'s first import, handed the wrong
/// intrinsic, makes the namespace and no class; making the class then
/// refuses it, as a `TypeError`, and drops it, so the next import's
/// `Error` is the one checked and the class is made of it.
#[test]
fn the_realm_s_error_is_checked_when_a_class_is_first_made() {
    use crate::host::error_class::ENV_ERROR;

    let results = on_new_runtime("error_checked_at_class", Mode::Run, |runtime| {
        let state = runtime.op_state();
        state.borrow_mut().put(crate::test_support::run_authority());
        state.borrow_mut().put(host_data());
        state.borrow_mut().put(ErrorClasses::default());
        let namespaces = NativeNamespaces::new(runtime);
        state.borrow_mut().put(namespaces);
        drop(state);
        let env = backing_script("@fuzdev/gro/env.ts");
        let import = |runtime: &mut JsRuntime, error: &str| {
            named_script_json(
                runtime,
                env,
                format!(
                    "JSON.stringify(Object.keys(Deno.core.ops.op_gro_namespace(undefined, \
                         {error})))"
                ),
            )
        };
        let make_class = |runtime: &mut JsRuntime| {
            let state = runtime.op_state();
            let state = state.borrow();
            let classes = state.borrow::<ErrorClasses>();
            deno_core::scope!(scope, runtime);
            v8::tc_scope!(let tc, scope);
            let made = classes
                .class(tc, DeclaredFamily::for_test(&ENV_ERROR))
                .is_some();
            let thrown = tc.exception().map(|thrown| {
                let thrown = v8::Local::<v8::Object>::try_from(thrown).unwrap();
                let message = v8::String::new(tc, "message").unwrap();
                thrown
                    .get(tc, message.into())
                    .unwrap()
                    .to_rust_string_lossy(tc)
            });
            (made, thrown, classes.len())
        };
        let wrong = import(runtime, "TypeError");
        let refused = make_class(runtime);
        let right = import(runtime, "Error");
        let made = make_class(runtime);
        // the first held is the one checked: a later value is ignored
        runtime.op_state().borrow_mut().put(ErrorClasses::default());
        import(runtime, "Error");
        import(runtime, "TypeError");
        let first = make_class(runtime);
        json!({"wrong": wrong, "refused": refused, "right": right, "made": made, "first": first})
    });
    assert_eq!(
        results,
        json!({
            "wrong": ["env", "default"],
            "refused": [false, "an error class extends the realm's Error", 0],
            "right": ["env", "default"],
            "made": [true, null, 1],
            "first": [true, null, 1],
        })
    );
}

/// A refused env name in a runtime with no record of refusals, which no
/// run's start leaves out, throws an `Error` saying the refusal can't
/// fail the run, never an `EnvError` that would pass unrecorded; a
/// granted name still reads.
#[test]
fn a_refusal_with_no_record_to_land_in_fails_loudly() {
    let results = on_new_runtime("no_refusal_log", Mode::Run, |runtime| {
        let state = runtime.op_state();
        state.borrow_mut().put(Authority::Confined(Grants {
            env: Some(EnvGrant {
                names: ["A".to_owned()].into(),
            }),
            ..Grants::default()
        }));
        state.borrow_mut().put(host_data());
        state.borrow_mut().put(ErrorClasses::default());
        let namespaces = NativeNamespaces::new(runtime);
        state.borrow_mut().put(namespaces);
        drop(state);
        named_script_json(
            runtime,
            backing_script("@fuzdev/gro/env.ts"),
            "const { env } = Deno.core.ops.op_gro_namespace(undefined, Error);
let thrown;
try {
	env.get('B');
} catch (error) {
	thrown = `${error.constructor.name}: ${error.message}`;
}
JSON.stringify({ thrown, read: env.get('A') })"
                .to_owned(),
        )
    });
    assert_eq!(
        results,
        json!({
            "thrown": "Error: env refused a name, but this runtime was started without the \
                           run's record of refusals, so the refusal can't fail the run",
            "read": "1",
        })
    );
}
