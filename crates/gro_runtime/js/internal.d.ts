// The deno_core and deno_web internals the bootstrap reads, typed as loosely
// as the bootstrap uses them. Only extension code can import these modules.

declare module 'ext:core/mod.js' {
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
}
