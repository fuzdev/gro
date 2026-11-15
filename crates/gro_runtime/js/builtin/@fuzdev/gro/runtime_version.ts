// The runtime_version builtin, `@fuzdev/gro/runtime_version.ts` on the runtime: the
// runtime_version host module's runtime implementation, the runtime's name and version. The
// Node implementation, `src/lib/runtime_version.ts`, is the oracle, and its `runtime_version` is
// the one declaration of the type: `bootstrap.d.ts` declares what the op returns, and a test in
// `src/test` checks the namespace's inferred type against it.
//
// The string is the host's, the embedder's name for itself and its version (`fgro/0.1.0`),
// made once, when the run first asks for the namespace, and exported as data (`blake3.ts` has
// how the host makes a namespace). It differs from the Node implementation's (`node/v24.14.0`)
// by design: it's opaque, compared for equality only.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). A grant-free
// module has no grant to scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const { op_gro_namespace } = core.ops;
	const { Error } = primordials;

	// `runtime_version`, the host's, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => op_gro_namespace<'@fuzdev/gro/runtime_version.ts'>(_scope, Error);
});
