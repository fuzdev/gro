// What the outer level of a builtin's implementation is called with: deno_core's bootstrap
// object, typed as far as the implementations read it. No user code can reach it: it's the
// parameter of the function deno_core compiles a lazy script as.
//
// This is the list of what an implementation may use: an op or a primordial joins it here and
// in the reviewed list of `src/test/builtin_intrinsics.test.ts`, which says of each primordial
// why a call of it runs no user code. `core` gives `ops` alone, and the object nothing else:
// no `loadExtScript`, which would run another script and make the ops again, and no
// `internals`, an ordinary object every extension script shares.

/** deno_core's bootstrap object, the argument of a builtin's implementation's outer level. */
interface Bootstrap {
	readonly core: {
		/** Every op, each a function of the runtime the script runs in. */
		readonly ops: {
			readonly op_gro_args: () => Array<string>;
			readonly op_gro_cwd: () => string;
			/**
			 * The env error row's namespace, the host's: the class extending the realm's `Error`,
			 * which the implementation hands over, and its guard. The types are claims, held to
			 * the Node module's by the types test.
			 */
			readonly op_gro_env_error: (error: ErrorConstructor) => {
				readonly EnvError: new (
					kind: 'not_granted',
					target: string,
					options?: { cause?: unknown }
				) => {
					name: string;
					message: string;
					stack?: string;
					cause?: unknown;
					readonly kind: 'not_granted';
					readonly target: string;
				};
				readonly is_env_error: (
					error: unknown,
					kind?: 'not_granted'
				) => error is {
					name: string;
					message: string;
					stack?: string;
					cause?: unknown;
					readonly kind: 'not_granted';
					readonly target: string;
				};
				readonly default: undefined;
			};
			/**
			 * The env host module's root handle, made of the prototype the implementation gives,
			 * so of its type, the realm's `Error` handed over for the class the host throws.
			 */
			readonly op_gro_env_root: <T extends object>(prototype: T, error: ErrorConstructor) => T;
			/** `get`, the receiver handed over: a value, or `undefined` when it's unset. */
			readonly op_gro_env_get: (handle: unknown, name: unknown) => string | undefined;
			/** `get_all`, the receiver handed over: the handle's frozen null-prototype record. */
			readonly op_gro_env_get_all: (handle: unknown) => Readonly<{ [name: string]: string }>;
			/** `narrow`, the receiver handed over: a handle of the receiver's prototype, its type. */
			readonly op_gro_env_narrow: <T>(handle: T, names: unknown) => T;
			/**
			 * The namespace of the row whose script calls it, the host's: each member a native
			 * function the host makes the first time the run asks, and holds. The host knows the row
			 * by the calling script; the type argument names it for the types alone, which are
			 * claims, held to the Node module's by the types test. The script hands over its scope
			 * and the realm's `Error`.
			 */
			readonly op_gro_namespace: <Row extends '@fuzdev/gro/blake3.ts'>(
				scope: undefined,
				error: ErrorConstructor
			) => {
				readonly '@fuzdev/gro/blake3.ts': {
					readonly hash: (data: Uint8Array) => Uint8Array;
					readonly create_hasher: () => {
						update(data: Uint8Array): void;
						finalize(): Uint8Array;
					};
					readonly create_keyed_hasher: (key: Uint8Array) => {
						update(data: Uint8Array): void;
						finalize(): Uint8Array;
					};
					readonly create_derive_key_hasher: (context: string) => {
						update(data: Uint8Array): void;
						finalize(): Uint8Array;
					};
					readonly default: undefined;
				};
			}[Row];
			readonly op_gro_exit: (code: unknown) => never;
			readonly op_gro_pid: () => number;
			readonly op_gro_runtime_version: () => string;
			readonly op_gro_set_exit_code: (code: unknown) => void;
		};
	};
	/** The intrinsics, each taken by name when the runtime is made, before any user code runs. */
	readonly primordials: {
		readonly Error: ErrorConstructor;
		readonly ObjectAssign: <T extends object, U>(target: T, source: U) => T & U;
		readonly ObjectCreate: (prototype: null) => object;
		readonly ObjectFreeze: <T>(value: T) => Readonly<T>;
	};
}
