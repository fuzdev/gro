// The runtime_version builtin, `@fuzdev/gro/runtime_version.ts` on the runtime: the
// runtime_version host module's runtime implementation, the runtime's name and version. The
// Node implementation, `src/lib/runtime_version.ts`, is the oracle, and its `runtime_version` is
// the one declaration of the type: this script declares none, and a test in `src/test` checks
// the namespace's inferred type against it.
//
// The string is the host's, the embedder's name for itself and its version (`fgro/0.1.0`),
// read once, when the builtin is first imported, and exported as data. It differs from the
// Node implementation's (`node/v24.14.0`) by design: it's opaque, compared for equality only.
//
// A script the runtime compiles and calls on the builtin's first import, reaching intrinsics
// only through `primordials` (`args.ts` has the rules). A grant-free module has no grant to
// scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap, _scope: undefined) {
	const { core, primordials } = bootstrap;
	const { op_gro_runtime_version } = core.ops;
	const { ObjectAssign, ObjectCreate } = primordials;

	// a null-prototype record, as the Node modules' `create_record` makes
	const create_record = <T extends object>(fields: T): T =>
		ObjectAssign(ObjectCreate(null), fields);

	// `runtime_version`, the runtime's name and version, and the `default` every builtin has,
	// `undefined`
	return create_record({ runtime_version: op_gro_runtime_version(), default: undefined });
});
