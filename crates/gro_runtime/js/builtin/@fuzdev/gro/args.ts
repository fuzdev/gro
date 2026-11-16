// The args builtin, `@fuzdev/gro/args.ts` on the runtime: the args host module's runtime
// implementation, the script's arguments. The Node implementation, `src/lib/args.ts`, is the
// oracle, and its `args` is the one declaration of the type: `bootstrap.d.ts` declares what the
// op returns, and a test in `src/test` checks the namespace's inferred type against it.
//
// The namespace is the host's: `args` is a frozen array of strings the host makes the first time
// the run asks for the namespace and holds, one array for every importer (`blake3.ts` has how
// the host makes a namespace).
//
// A builtin's implementation is a script, one function in two levels, which is in no snapshot:
// the runtime compiles and calls it the first time a run imports the builtin. The outer level
// takes deno_core's bootstrap object and runs once a run: it takes the one op, the namespace
// op, from `core.ops`, and, only in a row of an error family's (its error row, `env_error.ts`,
// and a row whose members throw its error, `env.ts`), the realm's `Error` from `primordials`, taken when the runtime was made,
// before any user code ran, for the host to make the error's class of. The outer level returns
// the inner, a function of the scope of an importer's grant that asks the host for the
// builtin's namespace, handing over the scope, and `Error` where the outer level took it. The
// script reads nothing and decides nothing: the namespace, each
// function and value it holds, and whatever has identity or state, a class, a brand, a
// registry, are the host's, made natively.
//
// A first import can be after user code has patched the global intrinsics, so the whole
// script names no global and reads no property of an intrinsic. `tsconfig.json` beside
// `@fuzdev` type-checks it with no global value and no member of an intrinsic type declared
// (`intrinsics.d.ts`), so either is an error.
//
// An `import()` runs the script on its caller's stack, so a caller that left it none cuts
// either level short at any call, and the next import runs both again. So a level stores
// nothing outside its own bindings, and calls only an op that can be called again, as the
// namespace op, a get-or-make, can; and a builtin that can fail fails in what it exports, as
// `cwd` does, since an import whose script threw rejects with the host's report of it, not the
// error. The inner level reads no grant but the scope it's given, which it hands to the host
// whole, and keeps no state per importer. The `args` grant scopes nothing, so its scope is
// `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core } = bootstrap;
	const { op_gro_namespace } = core.ops;

	// `args`, the host's, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => op_gro_namespace<'@fuzdev/gro/args.ts'>(_scope);
});
