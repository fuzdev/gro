/**
 * The errors of the spawn host module (`spawn.ts`), apart from it.
 *
 * Making or matching an error starts no process, so this module grants nothing:
 * code that only classifies a failure, or builds one, imports it without linking the
 * host module, and a mock of the host module throws the same errors it does.
 * The class and its prototype are frozen.
 *
 * @module
 */

import { is_error_class_named } from './internal/error_class.ts';

/**
 * Why a program couldn't be reached or started.
 * `not_found` is a name that resolves to no regular executable; `not_executable` is a regular file
 * without execute permission, or one the module refuses (`.bat`, `.cmd`, a format that's neither
 * `#!` nor ELF); `not_granted` is a name outside what the handle allows, or an environment
 * a narrowed root can't give; `launch_failed` is a pinned
 * program that couldn't start, its cause on `cause`, including one that wasn't started because
 * the call's `signal` had already aborted.
 */
export type SpawnErrorKind = 'not_found' | 'not_executable' | 'not_granted' | 'launch_failed';

/**
 * The error `spawn.ts` throws when a program can't be reached or started, typed by `kind`.
 * A program that started and exited non-zero is a result, not this error.
 * Match it with `is_spawn_error`, which also recognizes one from another copy of Gro.
 * The class and its prototype are frozen.
 */
export class SpawnError extends Error {
	override name = 'SpawnError';
	readonly kind: SpawnErrorKind;
	/**
	 * The program's name as it was requested.
	 */
	readonly program: string;

	constructor(kind: SpawnErrorKind, program: string, message?: string, options?: ErrorOptions) {
		super(message ?? `${kind}: ${program}`, options);
		this.kind = kind;
		this.program = program;
	}
}

Object.freeze(SpawnError.prototype);
Object.freeze(SpawnError);

/**
 * Whether `error` is a `SpawnError` from any copy of Gro, optionally of one `kind`.
 * @param error - any thrown value
 * @param kind - when given, the kind the error must have
 */
export const is_spawn_error = (error: unknown, kind?: SpawnErrorKind): error is SpawnError =>
	is_error_class_named(error, 'SpawnError') &&
	(kind === undefined || (error as SpawnError).kind === kind);
