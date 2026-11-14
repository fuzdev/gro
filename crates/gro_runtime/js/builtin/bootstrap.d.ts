// What a builtin's implementation is called with: deno_core's bootstrap object, typed as far
// as the implementations read it. No user code can reach it: it's the parameter of the
// function deno_core compiles a lazy script as.

/** deno_core's bootstrap object, the first argument of a builtin's implementation. */
interface Bootstrap {
	readonly core: {
		/** Every op, each a function of the runtime the script runs in. */
		readonly ops: {
			readonly op_gro_args: () => Array<string>;
			readonly op_gro_cwd: () => string | null;
			readonly op_gro_pid: () => number;
			readonly op_gro_runtime_version: () => string;
		};
	};
	/** The intrinsics, copied when the runtime is made, before any user code runs. */
	readonly primordials: {
		readonly Error: ErrorConstructor;
		readonly ObjectAssign: <T extends object, U>(target: T, source: U) => T & U;
		readonly ObjectCreate: (prototype: null) => object;
		readonly ObjectFreeze: <T>(value: T) => Readonly<T>;
	};
}
