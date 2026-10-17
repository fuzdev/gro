// The deno_core and deno_web internals the bootstrap reads, typed as loosely
// as the bootstrap uses them. Only extension code can import these modules.

declare module 'ext:core/mod.js' {
	export const core: {
		/** Evaluates a lazily loaded extension script once and returns its exports. */
		loadExtScript: (specifier: string) => any;
		/** Writes to stdout, or to stderr when `is_err` is true. */
		print: (message: string, is_err: boolean) => void;
	};
}

declare module 'ext:core/ops' {
	export const op_gro_color: (stderr: boolean) => boolean;
}
