/**
 * The errors of the stdio host modules (`stdin.ts`, `stdout.ts`, and `stderr.ts`), apart from them.
 *
 * Making or matching an error touches no stream, so this module grants nothing:
 * code that only classifies a failure, or builds one, imports it without linking a
 * host module, and a mock of one throws the same errors it does.
 * The class and its prototype are frozen.
 *
 * @module
 */

import { is_error_class_named } from './internal/error_class.ts';

/**
 * Why a stdio op failed. `io_error` is a read or write failure other than the reader going away,
 * which is a `false` result from a write.
 */
export type StdioErrorKind = 'io_error';

/**
 * The error the stdio host modules throw, typed by `kind`.
 * Match it with `is_stdio_error`, which also recognizes one from another copy of Gro.
 * The class and its prototype are frozen.
 */
export class StdioError extends Error {
	override name = 'StdioError';
	readonly kind: StdioErrorKind;
	/**
	 * The stream: `stdin`, `stdout`, or `stderr`.
	 */
	readonly target: string;

	constructor(kind: StdioErrorKind, target: string, message?: string, options?: ErrorOptions) {
		super(message ?? `${kind}: ${target}`, options);
		this.kind = kind;
		this.target = target;
	}
}

Object.freeze(StdioError.prototype);
Object.freeze(StdioError);

/**
 * Whether `error` is a `StdioError` from any copy of Gro, optionally of one `kind`.
 * @param error - any thrown value
 * @param kind - when given, the kind the error must have
 */
export const is_stdio_error = (error: unknown, kind?: StdioErrorKind): error is StdioError =>
	is_error_class_named(error, 'StdioError') &&
	(kind === undefined || (error as StdioError).kind === kind);
