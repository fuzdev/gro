/**
 * The errors of the fs host module (`fs.ts`), apart from it.
 *
 * Making or matching an error reaches no filesystem, so this module grants nothing:
 * code that only classifies a failure, or builds one, imports it without linking the
 * host module, and a mock of the host module throws the same errors it does.
 * The class and its prototype are frozen.
 *
 * @module
 */

import { is_error_class_named } from './internal/error_class.ts';

/**
 * Why an fs op failed. `outside_root` is a path that would leave its handle;
 * `io_error` is every failure without a kind of its own.
 */
export type FsErrorKind =
	| 'not_found'
	| 'already_exists'
	| 'permission_denied'
	| 'not_a_directory'
	| 'is_a_directory'
	| 'not_empty'
	| 'outside_root'
	| 'io_error';

// set in the class's static block, where the private brand is in scope
let has_brand: (value: unknown) => value is FsError;

/**
 * The error every fs op throws, typed by `kind`.
 * Match it with `is_fs_error`, which also recognizes one from another copy of Gro.
 * The class and its prototype are frozen.
 */
export class FsError extends Error {
	override name = 'FsError';
	readonly kind: FsErrorKind;
	/**
	 * The absolute path the op targeted, or for `outside_root` the path as requested.
	 */
	readonly path: string;
	readonly #brand = true;

	static {
		has_brand = (value): value is FsError =>
			typeof value === 'object' && value !== null && #brand in value;
	}

	constructor(kind: FsErrorKind, path: string, message?: string, options?: ErrorOptions) {
		super(message ?? `${kind}: ${path}`, options);
		this.kind = kind;
		this.path = path;
	}
}

Object.freeze(FsError.prototype);
Object.freeze(FsError);

/**
 * Whether `error` is an `FsError` from any copy of Gro, optionally of one `kind`.
 * @param error - any thrown value
 * @param kind - when given, the kind the error must have
 */
export const is_fs_error = (error: unknown, kind?: FsErrorKind): error is FsError =>
	is_error_class_named(error, 'FsError') &&
	(kind === undefined || (error as FsError).kind === kind);

/**
 * Whether `value` is an `FsError` made by this copy of the module, by its private brand,
 * so classifying an error mid-op runs no caller's code, as `instanceof` would through a
 * `Symbol.hasInstance` or a proxy's `getPrototypeOf`. For the host module's own ops;
 * other code matches errors with `is_fs_error`.
 * @param value - any value
 * @internal
 */
export const is_own_fs_error = (value: unknown): value is FsError => has_brand(value);
