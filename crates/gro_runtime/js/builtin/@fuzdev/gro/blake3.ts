// The blake3 builtin, `@fuzdev/gro/blake3.ts` on the runtime: the blake3 host module's runtime
// implementation, BLAKE3 hashing over bytes, by the native `blake3` crate. The Node
// implementation, `src/lib/blake3.ts`, is the oracle, and its exports and `Blake3Hasher` are the
// one declaration of the types: `bootstrap.d.ts` declares what the op returns, and a test in
// `src/test` checks the namespace's inferred type against the Node module's.
//
// The namespace is the host's: each export, and each method of a hasher, is a native function
// the host makes the first time the run asks for the namespace and holds, so a script cut short
// and run again finds the same functions, and so would an instance per origin. The host knows
// the row by the script that asks, never by an argument; the script's type argument names it
// only for the types. Each function reads its arguments natively, with no conversion, so no
// code of a caller's runs: bytes are a `Uint8Array` and a context a string, and anything else is
// a `TypeError`. A hasher is the host's too: a frozen object of a frozen null-prototype record of
// the methods, whose state is the host's own `blake3::Hasher`, which the host drops once the
// hasher is collected. The script reads nothing and decides nothing: it hands the host its
// scope, and no `Error`, since no member throws a host error.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). A grant-free module
// has no grant to scope, so its scope is `undefined`.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core } = bootstrap;
	const { op_gro_namespace } = core.ops;

	// `hash` and the hashers' factories, the host's, and the `default` every builtin has,
	// `undefined`
	return (_scope: undefined) => op_gro_namespace<'@fuzdev/gro/blake3.ts'>(_scope);
});
