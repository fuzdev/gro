//! Tests for the loader's reading of the stack left: held to V8's own limit.

use deno_core::{JsRuntime, RuntimeOptions, v8};

use super::Headroom;
use crate::STACK_SIZE;
use crate::confine::Confinement;
use crate::engine::{EngineOptions, init_engine};

/// Dives until V8 refuses a call, then reads what's left from a native
/// function called in the deepest frame that can call one, once.
const DIVE: &str = "(() => {
	let reading;
	const dive = () => {
		try {
			dive();
		} catch (error) {
			if (reading === undefined) {
				try {
					reading = left();
				} catch {
					// too deep to call it, so the next frame up does
				}
			}
			throw error;
		}
	};
	try {
		dive();
	} catch {
		// the overflow that found the limit
	}
	return reading;
})()";

/// How far below V8's limit, in bytes, the reading at the deepest call may
/// lie: the native frames of the call, which start just above it, about
/// 1 KiB in a debug build.
const BELOW: isize = 4 * 1024;

/// How far above V8's limit, in bytes, the reading at the deepest call may
/// lie: tighter than [`BELOW`], since a limit read too low lets an import
/// start with less stack than it needs.
const ABOVE: isize = 2 * 1024;

/// The stack left as the loader reads it, as a number.
fn left<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    let left = Headroom::default().left().unwrap();
    #[expect(
        clippy::cast_precision_loss,
        reason = "a stack's size is far below 2^53"
    )]
    rv.set(v8::Number::new(scope, left as f64).into());
}

/// The loader reads the stack left from the limit V8 holds JS to: in the
/// deepest frame a dive reaches, on a thread sized as a run's, what it reads
/// is within a few frames of nothing, so a stack start read off by a few
/// KiB, or a stack size other than V8's, fails here, refusing too many
/// imports or too few.
#[test]
fn the_stack_left_is_measured_from_v8_s_limit() {
    init_engine(Confinement::Unconfined, &EngineOptions::default()).unwrap();
    let reading = std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(|| {
            // V8 posts delayed tasks, which `deno_core` hands to tokio
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let _context = tokio.enter();
            let mut runtime = JsRuntime::new(RuntimeOptions::default());
            {
                deno_core::scope!(scope, &mut runtime);
                let function = v8::Function::new(scope, left).unwrap();
                let key = v8::String::new(scope, "left").unwrap();
                let global = scope.get_current_context().global(scope);
                global.set(scope, key.into(), function.into()).unwrap();
            }
            let value = runtime.execute_script("<test>", DIVE).unwrap();
            deno_core::scope!(scope, &mut runtime);
            v8::Local::new(scope, value).number_value(scope).unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
    #[expect(clippy::cast_precision_loss, reason = "a few KiB, exact as a float")]
    let (below, above) = (-BELOW as f64, ABOVE as f64);
    assert!(below < reading && reading < above, "{reading}");
}
