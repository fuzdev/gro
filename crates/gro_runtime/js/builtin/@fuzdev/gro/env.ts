// The env builtin, `@fuzdev/gro/env.ts` on the runtime: the env host module's runtime
// implementation, the environment variables the run's grant names. The Node implementation,
// `src/lib/env.ts`, is the oracle, and its `env` and `Env` are the one declaration of the types:
// the handle's type here is a claim, since the host makes the handle, and a test in `src/test`
// checks the namespace's inferred type against the Node module's.
//
// A handle is the host's: an object it makes of the handles' prototype, the variables it may
// read under a private symbol of the host's, which no program can read or forge, then frozen.
// The prototype is a frozen null-prototype record of native methods the host makes with the
// namespace (`blake3.ts` has how): each checks its receiver is a handle the host made, checks
// each argument as the value it is, with no conversion, and throws `EnvError`, the host's class,
// for a name outside the handle's set. `narrow` returns a handle the host makes of the
// receiver's prototype. The root handle, `env`, is made with the namespace, from the run's
// grant. The script reads nothing and decides nothing: it hands the host its scope and the
// realm's `Error`, from `primordials`, taken before any user code ran, which `EnvError`
// extends.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). Its scope is
// `undefined`: the root comes from the run's grant, which reaches the project's modules, or, in
// an unconfined run, every module, all of which share the one root.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const { op_gro_namespace } = core.ops;
	const { Error } = primordials;

	// a handle's type, the one claim this script makes, held to the Node module's `Env` by the
	// types test: a handle's methods return its own type, which only an interface can name
	interface EnvHandle {
		get(name: string): string | undefined;
		get_all(): Readonly<{ [name: string]: string }>;
		narrow(names: ReadonlyArray<string>): EnvHandle;
	}

	// `env`, the root handle, the host's, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => op_gro_namespace<'@fuzdev/gro/env.ts', EnvHandle>(_scope, Error);
});
