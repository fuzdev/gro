// The args builtin, `@fuzdev/gro/args.ts` on the runtime: the args host module's runtime
// implementation, the script's arguments. The Node implementation, `src/lib/args.ts`, is the
// oracle, and its `args` is the one declaration of the type: this script declares none, and a
// test in `src/test` checks the namespace's inferred type against it.
//
// A builtin's implementation is a script, one function in two levels, which is in no snapshot:
// the runtime compiles and calls it the first time a run imports the builtin. The outer level
// takes deno_core's bootstrap object and runs once a run: it makes what every importer shares,
// the values that don't depend on a scope, and with them any class, brand, or registry, so
// those are one object whoever imports the builtin. It returns the inner level, a function of
// the scope of an importer's grant that makes only the builtin's namespace, in a
// null-prototype record made in place, since a function of the script's own is one more to
// compile at every first import.
//
// A first import can be after user code has patched the global intrinsics, so the whole
// script, both levels and what they export, names no global and reads no property of an
// intrinsic: every intrinsic it uses is one of `primordials`, taken when the runtime was
// made, each host op comes from `core.ops`, and what it hands to user code is made natively,
// as the op's array is, or a frozen function of its own. `tsconfig.json` beside `@fuzdev`
// type-checks it with no global value and no member of an intrinsic type declared
// (`intrinsics.d.ts`), so either is an error.
//
// Neither level throws, since an import of a builtin whose script threw rejects with the
// host's report of it, not the error: a builtin that can fail fails in what it exports, as
// `cwd.ts` does. The inner level reads no grant but the scope it's given and keeps no state
// per importer, so it can be called once per importing origin. The `args` grant scopes
// nothing, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const { op_gro_args } = core.ops;
	const { ObjectAssign, ObjectCreate, ObjectFreeze } = primordials;

	// the script's arguments as a frozen array of strings, one array for every importer
	const args = ObjectFreeze(op_gro_args());

	// `args`, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => ObjectAssign(ObjectCreate(null), { args, default: undefined });
});
