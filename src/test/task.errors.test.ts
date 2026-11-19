import { describe, test, assert } from 'vitest';

import { is_reported_error, is_task_error, ReportedError, TaskError } from '#lib/task_error.ts';

// stand-ins for the classes from another copy of Gro, which a task module can load
type ErrorClass = new (message?: string) => Error;

const other_copy = (): { TaskError: ErrorClass; ReportedError: ErrorClass } => {
	class TaskError extends Error {}
	class ReportedError extends Error {}
	return { TaskError, ReportedError };
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
		assert.ok(!is_task_error(new ReportedError()));
		assert.ok(!is_task_error(new (other_copy().ReportedError)()));
		assert.ok(!is_task_error(null));
		assert.ok(!is_task_error(undefined));
		assert.ok(!is_task_error('TaskError'));
		assert.ok(!is_task_error(Object.create(null)));
		assert.ok(!is_task_error({ constructor: TaskError })); // own `constructor`, not its class
	});
});

describe('is_reported_error', () => {
	test('recognizes a `ReportedError`, a subclass, and another copy', () => {
		class SubReportedError extends ReportedError {}
		const other = other_copy();
		class OtherSubReportedError extends other.ReportedError {}
		assert.ok(is_reported_error(new ReportedError()));
		assert.ok(is_reported_error(new SubReportedError()));
		assert.ok(is_reported_error(new other.ReportedError()));
		assert.ok(is_reported_error(new OtherSubReportedError()));
	});

	test('rejects other errors', () => {
		assert.ok(!is_reported_error(new Error('x')));
		assert.ok(!is_reported_error(new TaskError('x')));
	});
});
