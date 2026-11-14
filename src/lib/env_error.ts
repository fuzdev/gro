/**
 * The errors of the env host module (`env.ts`), apart from it.
 *
 * Making or matching an error reads no environment, so this module grants nothing:
 * code that only classifies a failure, or builds one, imports it without linking the
 * host module, and a mock of the host module throws the same errors it does.
 * The class and its prototype are frozen.
 *
 * @module
 */

import { is_error_class_named } from './internal/error_class.ts';

/**
 * Why an env op failed. `not_granted` is a variable name outside what the handle allows.
 */
export type EnvErrorKind = 'not_granted';

/**
 * The error `env.ts` throws, typed by `kind`.
 * Match it with `is_env_error`, which also recognizes one from another copy of Gro.
 * The class and its prototype are frozen.
 */
export class EnvError extends Error {
	override name = 'EnvError';
	readonly kind: EnvErrorKind;
	/**
	 * The variable's name, never its value.
	 */
	readonly target: string;

	constructor(kind: EnvErrorKind, target: string, message?: string, options?: ErrorOptions) {
		super(message ?? `${kind}: ${target}`, options);
		this.kind = kind;
		this.target = target;
	}
}

Object.freeze(EnvError.prototype);
Object.freeze(EnvError);

/**
 * Whether `error` is an `EnvError` from any copy of Gro, optionally of one `kind`.
 * @param error - any thrown value
 * @param kind - when given, the kind the error must have
 */
export const is_env_error = (error: unknown, kind?: EnvErrorKind): error is EnvError =>
	is_error_class_named(error, 'EnvError') &&
	(kind === undefined || (error as EnvError).kind === kind);
