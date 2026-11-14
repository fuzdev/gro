// The cwd builtin, `@fuzdev/gro/cwd.ts` on the runtime: the cwd host module's runtime
// implementation, the working directory the run was started in. The Node implementation,
// `src/lib/cwd.ts`, is the oracle, and its `cwd` is the one declaration of the type: this script
// declares none, and a test in `src/test` checks the namespace's inferred type against it.
//
// The directory is the host's, read once, when the builtin is first imported: `cwd` returns
// that string whenever it's called, and a run has no way to change its directory. As in the
// Node implementation, `cwd` is a frozen function that takes no receiver. When the directory's
// path isn't UTF-8, which the host checks, `cwd` throws when it's called, where importing the
// Node module throws: a builtin's script never throws, since the import of a builtin whose
// script threw would reject with the host's report of it, not the error.
//
// A script the runtime compiles and calls on the builtin's first import, reaching intrinsics
// only through `primordials` (`args.ts` has the rules). A grant-free module has no grant to
// scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap, _scope: undefined) {
	const { core, primordials } = bootstrap;
	const { op_gro_cwd } = core.ops;
	const { Error, ObjectAssign, ObjectCreate, ObjectFreeze } = primordials;

	// a null-prototype record, as the Node modules' `create_record` makes
	const create_record = <T extends object>(fields: T): T =>
		ObjectAssign(ObjectCreate(null), fields);

	const checked_cwd = op_gro_cwd();
	const cwd = (): string => {
		if (checked_cwd === null) {
			throw new Error("the working directory's path isn't UTF-8, so Gro can't name it");
		}
		return checked_cwd;
	};
	ObjectFreeze(cwd);

	// `cwd`, a frozen function returning the working directory the run was started in, or
	// throwing an `Error` when its path isn't UTF-8, and the `default` every builtin has,
	// `undefined`
	return create_record({ cwd, default: undefined });
});
