// The cwd builtin, `@fuzdev/gro/cwd.ts` on the runtime: the cwd host module's runtime
// implementation, the working directory the run was started in. The Node implementation,
// `src/lib/cwd.ts`, is the oracle, and its `cwd` is the one declaration of the type:
// `bootstrap.d.ts` declares what the op returns, and a test in `src/test` checks the namespace's
// inferred type against it.
//
// The directory is the host's, read once, when the run starts: `cwd` returns that string
// whenever it's called, and a run has no way to change its directory. `cwd` is a native function
// the host makes the first time the run asks for the namespace and holds (`blake3.ts` has how),
// and takes no receiver. When the directory's path isn't UTF-8, it throws an `Error` when it's
// called, where importing the Node module throws, since an import whose script threw would
// reject with the host's report, not the error. The script reads nothing and decides nothing.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). A grant-free
// module has no grant to scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core } = bootstrap;
	const { op_gro_namespace } = core.ops;

	// `cwd`, the host's, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => op_gro_namespace<'@fuzdev/gro/cwd.ts'>(_scope);
});
