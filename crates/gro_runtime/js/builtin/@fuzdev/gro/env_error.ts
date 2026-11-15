// The env_error builtin, `@fuzdev/gro/env_error.ts` on the runtime: the env host module's
// errors, apart from it, which grant nothing. The Node implementation, `src/lib/env_error.ts`,
// is the oracle, and its `EnvError` and `is_env_error` are the one declaration of the types:
// `bootstrap.d.ts` declares what the op returns, and a test in `src/test` checks the
// namespace's inferred type against the Node module's, the class by its public shape.
//
// The class and its guard are the host's: the class made natively the first time the run needs
// it and held by the host, so any member can throw it, a script cut short and run again finds
// the same class, and so would an instance per origin; the guard a native function the host
// makes the first time the run asks for the namespace (`blake3.ts` has how). The class extends
// the realm's `Error`, which this script hands over from `primordials`, taken before any user
// code ran. The script reads nothing and decides nothing.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). A grant-free module
// has no grant to scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const { op_gro_namespace } = core.ops;
	const { Error } = primordials;

	// `EnvError` and `is_env_error`, the host's, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => op_gro_namespace<'@fuzdev/gro/env_error.ts'>(_scope, Error);
});
