import { describe, test, assert } from 'vitest';

import { is_silent_error, is_task_error, SilentError, TaskError } from '#lib/task.ts';

// stand-ins for the classes from another copy of Gro, which a task module can load
type ErrorClass = new (message?: string) => Error;

const other_copy = (): { TaskError: ErrorClass; SilentError: ErrorClass } => {
	class TaskError extends Error {}
	class SilentError extends Error {}
	return { TaskError, SilentError };
};

describe('is_task_error', () => {
	test('recognizes a `TaskError`', () => {
		assert.ok(is_task_error(new TaskError('x')));
	});

	test('recognizes a subclass', () => {
		class SubTaskError extends TaskError {}
		class SubSubTaskError extends SubTaskError {}
		assert.ok(is_task_error(new SubTaskError('x')));
		assert.ok(is_task_error(new SubSubTaskError('x')));
	});

	test('recognizes another copy and its subclasses', () => {
		const other = other_copy();
		assert.notStrictEqual<ErrorClass>(other.TaskError, TaskError);
		class OtherSubTaskError extends other.TaskError {}
		assert.ok(is_task_error(new other.TaskError('x')));
		assert.ok(is_task_error(new OtherSubTaskError('x')));
	});

	test('rejects other errors and non-errors', () => {
		assert.ok(!is_task_error(new Error('x')));
		assert.ok(!is_task_error(new SilentError()));
		assert.ok(!is_task_error(new (other_copy().SilentError)()));
		assert.ok(!is_task_error(null));
		assert.ok(!is_task_error(undefined));
		assert.ok(!is_task_error('TaskError'));
		assert.ok(!is_task_error(Object.create(null)));
		assert.ok(!is_task_error({ constructor: TaskError })); // own `constructor`, not its class
	});
});

describe('is_silent_error', () => {
	test('recognizes a `SilentError`, a subclass, and another copy', () => {
		class SubSilentError extends SilentError {}
		const other = other_copy();
		class OtherSubSilentError extends other.SilentError {}
		assert.ok(is_silent_error(new SilentError()));
		assert.ok(is_silent_error(new SubSilentError()));
		assert.ok(is_silent_error(new other.SilentError()));
		assert.ok(is_silent_error(new OtherSubSilentError()));
	});

	test('rejects other errors', () => {
		assert.ok(!is_silent_error(new Error('x')));
		assert.ok(!is_silent_error(new TaskError('x')));
	});
});
