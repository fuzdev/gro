import { describe, test, assert } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';
import { format_issue } from '@fuzdev/fuz_schema/schema.ts';

import { Args } from '#lib/run.task.ts';

// the one issue `args_parse` reports for `args`, as `run_task` prints it
const to_issue_text = (args: Record<string, unknown>): string => {
	const parsed = args_parse(args as any, Args);
	assert.ok(!parsed.ok, 'expected the args to fail');
	assert.strictEqual(parsed.issues.length, 1);
	return format_issue(parsed.issues[0]!);
};

describe('run.task Args', () => {
	test('passes forwarded args of every arg type', () => {
		const args = {
			_: ['script.ts', 'more'],
			name: 'a',
			port: 3000,
			watch: true,
			'no-open': false,
			list: ['a', 1, false]
		};
		const parsed = args_parse(args, Args);
		assert.ok(parsed.ok);
		assert.deepEqual(parsed.value, args);
	});

	test('`_` defaults to a fresh array per parse', () => {
		const a = args_parse({ x: 1 }, Args);
		const b = args_parse({ x: 1 }, Args);
		assert.ok(a.ok && b.ok);
		assert.deepEqual(a.value._, []);
		assert.deepEqual(b.value._, []);
		assert.notStrictEqual(a.value._, b.value._);
	});

	test('`_` must be an array of strings', () => {
		const parsed = args_parse({ _: [1] } as any, Args);
		assert.ok(!parsed.ok);
	});

	test('an object value fails, naming its key', () => {
		assert.strictEqual(
			to_issue_text({ _: [], x: { a: 1 } }),
			'x: expected string | number | boolean | array, received object'
		);
	});

	test('a null value fails, naming its key', () => {
		assert.strictEqual(
			to_issue_text({ _: [], x: null }),
			'x: expected string | number | boolean | array, received null'
		);
	});

	test('a key that is not only word characters is quoted as JSON', () => {
		assert.strictEqual(
			to_issue_text({ _: [], 'some-key': { a: 1 } }),
			'"some-key": expected string | number | boolean | array, received object'
		);
	});

	test('a nested array fails', () => {
		assert.strictEqual(
			to_issue_text({ _: [], x: ['a', ['b']] }),
			'x: expected string | number | boolean | array, received array'
		);
	});

	test('an array holding an object fails', () => {
		assert.strictEqual(
			to_issue_text({ _: [], x: [{}] }),
			'x: expected string | number | boolean | array, received array'
		);
	});

	test('names the first key that fails', () => {
		assert.strictEqual(
			to_issue_text({ _: [], ok: 1, y: {}, z: [[]] }),
			'y: expected string | number | boolean | array, received object'
		);
	});
});
