//! Tests of the host ops: each refuses without an authority.

use super::*;

/// Calls every host op through `Deno.core.ops`, before the start
/// removes it, on a runtime started as `start` says and built as `mode`
/// says whose `OpState` holds `authority` (none at all when unset, as
/// eval's) and [`host_data`], returning each op's result as JSON or
/// what it threw. Each op is called with no argument: a host op that
/// takes one takes it as the value passed, converting nothing
/// (`host_ops!`), so its check comes first here too.
fn call_host_ops(name: &str, start: Start, mode: Mode, authority: Option<Authority>) -> Value {
    let names: Vec<&str> = HOST_OPS.to_vec();
    let source = format!(
        "const results = {{}};
for (const name of {}) {{
\ttry {{
\t\tresults[name] = JSON.stringify(Deno.core.ops[name]());
\t}} catch (error) {{
\t\tresults[name] = `${{error.constructor.name}}: ${{error.message}}`;
\t}}
}}
JSON.stringify(results)",
        serde_json::to_string(&names).unwrap()
    );
    on_runtime(name, start, mode, move |runtime| {
        let state = runtime.op_state();
        if let Some(authority) = authority {
            state.borrow_mut().put(authority);
        }
        state.borrow_mut().put(host_data());
        state.borrow_mut().put(ErrorClasses::default());
        drop(state);
        script_json(runtime, source)
    })
}

/// Every host op's result in a runtime that holds no authority, in each
/// mode, in a run that grants nothing, and in one that grants the
/// script's arguments alone. No run here grants `exit`, whose function would
/// end this process.
pub(super) fn host_ops_results(start: Start) -> Value {
    let granted = Authority::Confined(Grants {
        args: Some(ArgsGrant {}),
        ..Grants::default()
    });
    json!({
        "none_eval": call_host_ops("host_none_eval", start, Mode::Eval, None),
        "none_run": call_host_ops("host_none_run", start, Mode::Run, None),
        "empty_run": call_host_ops("host_empty_run", start, Mode::Run, Some(Authority::default())),
        "granted_run": call_host_ops("host_granted_run", start, Mode::Run, Some(granted)),
    })
}

/// The choke point by behavior: every host op, called by a runtime that
/// holds no authority (eval's), in either mode and from either start,
/// throws `not_granted` naming itself before its body runs; and runs in
/// a run whatever it grants, nothing included, so its refusals are the
/// missing authority's: the namespace op, called by a script that is no
/// row's, gets past its check to refuse the caller. From either start:
/// an op reads the `OpState` of the runtime that calls it, and the
/// snapshot was made by a process that held none of this.
#[test]
fn every_host_op_refuses_without_an_authority() {
    for start in STARTS {
        let results = probed("host_ops", start);
        for op in HOST_OPS {
            for case in ["none_eval", "none_run"] {
                assert_eq!(
                    results[case][op].as_str().unwrap_or_default(),
                    format!(
                        "Error: not_granted: {op} is a host op, and this runtime holds no \
                             authority, so it runs none"
                    ),
                    "{case} {start:?}"
                );
            }
            for case in ["granted_run", "empty_run"] {
                assert_eq!(
                    results[case][op].as_str().unwrap_or_default(),
                    "TypeError: a namespace is asked for by a builtin's own script",
                    "{case} {start:?}"
                );
            }
        }
    }
}
