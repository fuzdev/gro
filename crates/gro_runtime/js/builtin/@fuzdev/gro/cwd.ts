// The cwd builtin, `@fuzdev/gro/cwd.ts` on the runtime: the cwd host module's runtime
// implementation, the working directory the run was started in. The Node implementation,
// `src/lib/cwd.ts`, is the oracle, and its `cwd` is the one declaration of the type: this script
// declares none, and a test in `src/test` checks the namespace's inferred type against it.
//
// The directory is the host's, read once, when the run starts: `cwd` returns that string
// whenever it's called, and a run has no way to change its directory. As in the Node
// implementation, `cwd` is a frozen function that takes no receiver. It's a shell: when the
// directory's path isn't UTF-8, the host's op throws an `Error` when `cwd` is called, where
// importing the Node module throws, since an import whose script threw would reject with the
// host's report, not the error. The script reads nothing and decides nothing.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). A grant-free
// module has no grant to scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const { op_gro_cwd } = core.ops;
	const { ObjectAssign, ObjectCreate, ObjectFreeze } = primordials;

	const cwd = (): string => op_gro_cwd();
	ObjectFreeze(cwd);

	// `cwd`, a frozen function returning the working directory the run was started in, or
	// throwing an `Error` when its path isn't UTF-8, one function for every importer, and the
	// `default` every builtin has, `undefined`
	return (_scope: undefined) => ObjectAssign(ObjectCreate(null), { cwd, default: undefined });
});
