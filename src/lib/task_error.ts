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
 * A failure its thrower has already reported, so Gro reports it no further and exits non-zero.
 * Throw one after logging the failure; `invoke_task` throws one, with the failure as its `cause`,
 * when the task it invoked fails.
 */
export class ReportedError extends Error {}

/**
 * Whether `error` is a `TaskError` or a subclass of one, from any copy of Gro.
 */
export const is_task_error = (error: unknown): error is TaskError =>
	is_error_class_named(error, 'TaskError');

/**
 * Whether `error` is a `ReportedError` or a subclass of one, from any copy of Gro.
 */
export const is_reported_error = (error: unknown): error is ReportedError =>
	is_error_class_named(error, 'ReportedError');
