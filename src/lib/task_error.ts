/**
 * The errors a task throws to fail with a known failure, apart from the task runner (`task.ts`).
 *
 * Making or matching one reaches nothing, so this module grants nothing: library code that
 * fails a task, or classifies a failure, imports it without linking the task runner.
 *
 * @module
 */

import { is_error_class_named } from './internal/error_class.ts';

/**
 * Options for a `TaskError`.
 */
export interface TaskErrorOptions extends ErrorOptions {
	/**
	 * The code the CLI exits with when this error fails the task it invoked,
	 * like `gro run`'s script's own, an integer 1–255; absent or any other value means `1`.
	 */
	exit_code?: number;
}

/**
 * This is used by tasks to signal a known failure.
 * It's useful for cleaning up logging because
 * we usually don't need their stack trace.
 */
export class TaskError extends Error {
	/**
	 * The code the CLI exits with when this error fails the task it invoked,
	 * an integer 1–255; absent or any other value means `1`.
	 */
	readonly exit_code: number | undefined;

	constructor(message?: string, options?: TaskErrorOptions) {
		super(message, options);
		this.exit_code = options?.exit_code;
	}
}

/**
 * This is used to tell Gro to exit silently, usually still with a non-zero exit code.
 * Using it means error logging is handled by the code that threw it.
 */
export class SilentError extends Error {}

/**
 * Whether `error` is a `TaskError` or a subclass of one, from any copy of Gro.
 */
export const is_task_error = (error: unknown): error is TaskError =>
	is_error_class_named(error, 'TaskError');

/**
 * Whether `error` is a `SilentError` or a subclass of one, from any copy of Gro.
 */
export const is_silent_error = (error: unknown): error is SilentError =>
	is_error_class_named(error, 'SilentError');
