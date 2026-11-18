//! Tests of the spawn roots' scopes: each method decides from the scope its
//! root names, never from the run's as a whole.

use super::*;
use crate::host::spawn;

/// A root decides from the scope its state names: in a run whose table
/// holds the project's scope, granting `b` and `c`, and a second, granting
/// `a`, and `c` with `V` to forward, the run's root makes what the
/// project's grants, and a root naming the second makes what that one
/// grants, refusing the rest, a variable to forward included; a root
/// narrowed from either keeps its parent's scope, through a chain of
/// narrows. Every program is found nowhere, so one a scope grants is
/// `null` from `find`, and one it doesn't is refused, here with no record
/// of refusals to land in.
#[test]
fn a_spawn_root_decides_from_the_scope_it_names() {
    let results = on_new_runtime("spawn_root_scopes", Mode::Run, |runtime| {
        let state = runtime.op_state();
        state.borrow_mut().put(crate::test_support::run_authority());
        let data = HostData {
            spawn: Some(Rc::new(spawn::run_of_scopes(&[
                &[("b", &[]), ("c", &[])],
                &[("a", &[]), ("c", &["V"])],
            ]))),
            ..host_data()
        };
        state.borrow_mut().put(data);
        state.borrow_mut().put(ErrorClasses::default());
        let namespaces = NativeNamespaces::new(runtime);
        state.borrow_mut().put(namespaces);
        drop(state);
        named_script_json(
            runtime,
            backing_script("@fuzdev/gro/spawn.ts"),
            "globalThis.project = Deno.core.ops.op_gro_namespace(undefined, Error).spawn;
'null'"
                .to_owned(),
        );
        {
            deno_core::scope!(scope, runtime);
            let global = scope.get_current_context().global(scope);
            let key = v8::String::new(scope, "project").unwrap();
            let project = global.get(scope, key.into()).unwrap();
            let project = v8::Local::<v8::Object>::try_from(project).unwrap();
            let other = spawn::root_of_scope(scope, project, 1).unwrap();
            let key = v8::String::new(scope, "other").unwrap();
            global.set(scope, key.into(), other.into()).unwrap();
        }
        script_json(
            runtime,
            "const outcome = (call) => {
	try {
		return call() === null ? 'found nothing' : 'made';
	} catch (error) {
		return error.message.startsWith('spawn refused')
			? 'refused'
			: `${error.constructor.name}: ${error.message}`;
	}
};
const ask = (root) => ({
	a: outcome(() => root.find('a')),
	b: outcome(() => root.find('b')),
	c_forwarded: outcome(() => root.find('c', { pass_env: ['V'] })),
	narrowed_a: outcome(() => root.narrow(['a']).find('a')),
	narrowed_b: outcome(() => root.narrow(['b']).find('b')),
	narrowed_c_forwarded: outcome(() => root.narrow(['c']).find('c', { pass_env: ['V'] })),
	narrowed_twice_a: outcome(() => root.narrow(['a', 'c']).narrow(['a']).find('a')),
	narrowed_twice_c_forwarded: outcome(() =>
		root.narrow(['c']).narrow(['c']).find('c', { pass_env: ['V'] })
	),
});
JSON.stringify({ project: ask(globalThis.project), other: ask(globalThis.other) })",
        )
    });
    assert_eq!(
        results,
        json!({
            "project": {
                "a": "refused",
                "b": "found nothing",
                "c_forwarded": "refused",
                "narrowed_a": "refused",
                "narrowed_b": "found nothing",
                "narrowed_c_forwarded": "refused",
                "narrowed_twice_a": "refused",
                "narrowed_twice_c_forwarded": "refused",
            },
            "other": {
                "a": "found nothing",
                "b": "refused",
                "c_forwarded": "found nothing",
                "narrowed_a": "found nothing",
                "narrowed_b": "refused",
                "narrowed_c_forwarded": "found nothing",
                "narrowed_twice_a": "found nothing",
                "narrowed_twice_c_forwarded": "found nothing",
            },
        })
    );
}
