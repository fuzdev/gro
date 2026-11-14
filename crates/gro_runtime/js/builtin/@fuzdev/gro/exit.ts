// The exit builtin, `@fuzdev/gro/exit.ts` on the runtime: the exit host module's runtime
// implementation, control over how the run ends. The Node implementation, `src/lib/exit.ts`,
// is the oracle, and its `exit` and `set_exit_code` are the one declaration of the types: this
// script declares none, and a test in `src/test` checks the namespace's inferred type against
// it.
//
// Each export is a shell: a frozen function that takes no receiver and hands what it was
// called with to its host op, whole. The script reads nothing and decides nothing. The host
// checks the code as the value it is, a number that is an integer from 0 to 255, with no
// conversion, so a thenable, a `Symbol.toPrimitive` object, a boxed number, and a string are
// each a `TypeError` and no code of the caller's runs; it keeps the code `set_exit_code` set,
// which is the run's and not this script's, so a script cut short and run again loses none;
// and it ends the run inside `exit`'s op, so no `finally` block and no queued job runs after
// the call.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). The `exit` grant
// scopes nothing, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const { op_gro_exit, op_gro_set_exit_code } = core.ops;
	const { ObjectAssign, ObjectCreate, ObjectFreeze } = primordials;

	const exit = (code?: number): never => op_gro_exit(code);
	// a statement, since an op that returns nothing returns `null`, where the function returns
	// `undefined`
	const set_exit_code = (code: number): void => {
		op_gro_set_exit_code(code);
	};
	ObjectFreeze(exit);
	ObjectFreeze(set_exit_code);

	// `exit` and `set_exit_code`, one function of each for every importer, and the `default`
	// every builtin has, `undefined`
	return (_scope: undefined) =>
		ObjectAssign(ObjectCreate(null), { exit, set_exit_code, default: undefined });
});
