// The deno_core and deno_web internals the bootstrap reads, typed as loosely
// as the bootstrap uses them. Only extension code can import these modules.

declare module 'ext:core/mod.js' {
	/** deno_core's internal object, shared by every extension module and script. */
	export const internals: object;
	/** The intrinsics, copied when the runtime is made, before any user code runs. */
	export const primordials: {
		ObjectFreeze: <T>(value: T) => Readonly<T>;
	};
	export const core: {
		/** Evaluates a lazily loaded extension script once and returns its exports. */
		loadExtScript: (specifier: string) => any;
		/** Writes to stdout, or to stderr when `is_err` is true. */
		print: (message: string, is_err: boolean) => void;
		/** Registers how to build the error an op raises with the class name `class_name`. */
		registerErrorBuilder: (class_name: string, build: (message: string) => Error) => void;
	};
}

declare module 'ext:core/ops' {
	export const op_gro_color: (stderr: boolean) => boolean;
	export const op_gro_console_stderr: () => boolean;
	export const op_gro_deny: () => boolean;
	export const op_gro_process_args: () => Array<string>;
}
