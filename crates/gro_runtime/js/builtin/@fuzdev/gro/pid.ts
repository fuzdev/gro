// The pid builtin, `@fuzdev/gro/pid.ts` on the runtime: the pid host module's runtime
// implementation, the id of the running process. The Node implementation, `src/lib/pid.ts`, is
// the oracle, and its `pid` is the one declaration of the type: this script declares none, and
// a test in `src/test` checks the namespace's inferred type against it.
//
// The id is asked of the host once, when the builtin is first imported, and exported as data.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). A grant-free
// module has no grant to scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const { op_gro_pid } = core.ops;
	const { ObjectAssign, ObjectCreate } = primordials;

	const pid = op_gro_pid();

	// `pid`, the process's id, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => ObjectAssign(ObjectCreate(null), { pid, default: undefined });
});
