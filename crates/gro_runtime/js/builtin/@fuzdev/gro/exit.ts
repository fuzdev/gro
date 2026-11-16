// The exit builtin, `@fuzdev/gro/exit.ts` on the runtime: the exit host module's runtime
// implementation, control over how the run ends. The Node implementation, `src/lib/exit.ts`,
// is the oracle, and its `exit` and `set_exit_code` are the one declaration of the types:
// `bootstrap.d.ts` declares what the op returns, and a test in `src/test` checks the namespace's
// inferred type against it.
//
// Each export is a native function the host makes the first time the run asks for the
// namespace and holds (`blake3.ts` has how), and takes no receiver. The host checks the code as
// the value it is, a number that is an integer from 0 to 255, with no conversion, so a thenable,
// a `Symbol.toPrimitive` object, a boxed number, and a string are each a `TypeError` and no code
// of the caller's runs; it keeps the code `set_exit_code` set, which is the run's and not this
// script's, so a script cut short and run again loses none; and it ends the run inside `exit`,
// so no `finally` block and no queued job runs after the call. The script reads nothing and
// decides nothing.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). The `exit` grant
// scopes nothing, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core } = bootstrap;
	const { op_gro_namespace } = core.ops;

	// `exit` and `set_exit_code`, the host's, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => op_gro_namespace<'@fuzdev/gro/exit.ts'>(_scope);
});
