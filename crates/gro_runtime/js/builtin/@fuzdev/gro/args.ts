// The args builtin, `@fuzdev/gro/args.ts` on the runtime: the args host module's runtime
// implementation, the script's arguments. The Node implementation, `src/lib/args.ts`, is the
// oracle, and its `args` is the one declaration of the type: this script declares none, and a
// test in `src/test` checks the namespace's inferred type against it.
//
// A builtin's implementation is a script, one function, which is in no snapshot: the runtime
// compiles and calls it the first time a run imports the builtin, with deno_core's bootstrap
// object and the scope of the importer's grant, and its result is the builtin's namespace. That
// can be after user code has patched the global intrinsics, so the whole script, its top level
// included, names no global and reads no property of an intrinsic: every intrinsic it uses is
// one of `primordials`, copied when the runtime was made, each host op comes from `core.ops`,
// and what it hands to user code is a null-prototype record or made natively, as the op's
// array is. `tsconfig.json` beside `@fuzdev` type-checks it with no global value and no member
// of an intrinsic type declared (`intrinsics.d.ts`), so either is an error.
//
// The function never throws, since an import of a builtin whose script threw rejects with the
// host's report of it, not the error: a builtin that can fail fails in what it exports, as
// `cwd.ts` does. It reads no grant but the scope it's given and keeps no state per importer,
// so it can be called once per importing origin. The `args` grant scopes nothing, so its
// scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap, _scope: undefined) {
	const { core, primordials } = bootstrap;
	const { op_gro_args } = core.ops;
	const { ObjectAssign, ObjectCreate, ObjectFreeze } = primordials;

	// a null-prototype record, as the Node modules' `create_record` makes
	const create_record = <T extends object>(fields: T): T =>
		ObjectAssign(ObjectCreate(null), fields);

	// `args`, the script's arguments as a frozen array of strings, and the `default` every
	// builtin has, `undefined`
	return create_record({ args: ObjectFreeze(op_gro_args()), default: undefined });
});
