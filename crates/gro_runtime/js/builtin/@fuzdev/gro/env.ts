// The env builtin, `@fuzdev/gro/env.ts` on the runtime: the env host module's runtime
// implementation, the environment variables the run's grant names. The Node implementation,
// `src/lib/env.ts`, is the oracle, and its `env` and `Env` are the one declaration of the types:
// the handle's type here is a claim, since the host makes the handle, and a test in `src/test`
// checks the namespace's inferred type against the Node module's.
//
// A handle is the host's: an object it makes of the prototype this script gives, the variables
// it may read under a private symbol of the host's, which no program can read or forge, then
// frozen. The prototype is a frozen null-prototype record of shells, each handing its receiver
// and its arguments to an op of this row, whole: the host checks the receiver is a handle it
// made, checks each argument as the value it is, with no conversion, and throws `EnvError`, the
// host's class, for a name outside the handle's set. `narrow` returns a handle the host makes
// of the receiver's prototype. The script reads nothing and decides nothing.
//
// A script the runtime compiles and calls on the builtin's first import, in two levels,
// reaching intrinsics only through `primordials` (`args.ts` has the rules). The outer level
// makes the prototype, one for every importer, so a handle one importer holds is a handle in
// another's hands; the inner level asks the host for the root handle, which the host makes the
// first time and holds, from the run's grant, handing over the realm's `Error` for the class
// it throws. Its scope is `undefined`: the root comes from the run's grant, which reaches the
// project's modules, or, in an unconfined run, every module, all of which share the one root.
//
// This file must stay 7-bit ASCII: the build fails otherwise.

(function (bootstrap: Bootstrap) {
	const { core, primordials } = bootstrap;
	const { op_gro_env_get, op_gro_env_get_all, op_gro_env_narrow, op_gro_env_root } = core.ops;
	const { Error, ObjectAssign, ObjectCreate, ObjectFreeze } = primordials;

	// a handle's type, the one claim this script makes, held to the Node module's `Env` by the
	// types test
	interface EnvHandle {
		get(name: string): string | undefined;
		get_all(): Readonly<{ [name: string]: string }>;
		narrow(names: ReadonlyArray<string>): EnvHandle;
	}

	// the handles' prototype, with no `constructor`: each method a shell of its op, which the
	// host freezes, as the prototype is, since every holder of a handle shares them
	const prototype: EnvHandle = ObjectFreeze(
		ObjectAssign(ObjectCreate(null), {
			get(name: string): string | undefined {
				return op_gro_env_get(this, name);
			},
			get_all(): Readonly<{ [name: string]: string }> {
				return op_gro_env_get_all(this);
			},
			narrow(names: ReadonlyArray<string>): EnvHandle {
				return op_gro_env_narrow<EnvHandle>(this, names);
			}
		})
	);

	// `env`, the root handle, the host's, and the `default` every builtin has, `undefined`
	return (_scope: undefined) => {
		const env = op_gro_env_root(prototype, Error);
		return ObjectAssign(ObjectCreate(null), { env, default: undefined });
	};
});
