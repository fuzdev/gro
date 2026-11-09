/**
 * The errors of the process host module (`process.ts`), apart from it.
 *
 * Making or matching an error reads no environment and touches no stream, so this module
 * grants nothing: code that only classifies a failure, or builds one, imports it without
 * linking the host module, and a mock of the host module throws the same errors it does.
 * The class and its prototype are frozen.
 *
 * @module
 */

import { is_error_class_named } from './internal/error_class.ts';

/**
 * Why a process op failed. `not_granted` is an environment name outside what the handle allows;
 * `io_error` is a stdio failure other than the reader going away, which is a `false` result.
 */
export type ProcessErrorKind = 'not_granted' | 'io_error';

/**
 * The error `process.ts` throws, typed by `kind`.
 * Match it with `is_process_error`, which also recognizes one from another copy of Gro.
 * The class and its prototype are frozen.
 */
export class ProcessError extends Error {
	override name = 'ProcessError';
	readonly kind: ProcessErrorKind;
	/**
	 * What the op targeted, an environment name or `stdin`, `stdout`, or `stderr`.
	 */
	readonly target: string;

	constructor(kind: ProcessErrorKind, target: string, message?: string, options?: ErrorOptions) {
		super(message ?? `${kind}: ${target}`, options);
		this.kind = kind;
		this.target = target;
	}
}

Object.freeze(ProcessError.prototype);
Object.freeze(ProcessError);

/**
 * Whether `error` is a `ProcessError` from any copy of Gro, optionally of one `kind`.
 * @param error - any thrown value
 * @param kind - when given, the kind the error must have
 */
export const is_process_error = (error: unknown, kind?: ProcessErrorKind): error is ProcessError =>
	is_error_class_named(error, 'ProcessError') &&
	(kind === undefined || (error as ProcessError).kind === kind);
