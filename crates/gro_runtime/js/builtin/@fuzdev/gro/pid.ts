// The pid builtin, `@fuzdev/gro/pid.ts` on the runtime: the pid host module's runtime
// implementation, the id of the running process. The Node implementation, `src/lib/pid.ts`, is
// the oracle, and its `pid` is the one declaration of the type: `bootstrap.d.ts` declares what
// the op returns, and a test in `src/test` checks the namespace's inferred type against it.
//
// The id is the host's, asked of the OS once, when the run first asks for the namespace, and
// exported as data (`blake3.ts` has how the host makes a namespace).
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). A grant-free
// module has no grant to scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core } = bootstrap;
	const { op_gro_namespace } = core.ops;

	// `pid`, the host's, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => op_gro_namespace<'@fuzdev/gro/pid.ts'>(_scope);
});
