import { describe, test, assert } from 'vitest';
import { args_parse } from '@fuzdev/fuz_util/args.ts';
import { format_issue } from '@fuzdev/fuz_schema/schema.ts';

import { Args } from '#lib/run.task.ts';

// every issue `args_parse` reports for `args`, in order, as `run_task` prints them
const to_issue_texts = (args: Record<string, unknown>): Array<string> => {
	const parsed = args_parse(args as any, Args);
	assert.ok(!parsed.ok, 'expected the args to fail');
	return parsed.issues.map(format_issue);
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
		assert.deepEqual(to_issue_texts({ _: [1] }), ['_.0: expected string, received number']);
		assert.deepEqual(to_issue_texts({ _: 'a' }), ['_: expected array, received string']);
	});

	test('an object value fails, naming its key', () => {
		assert.deepEqual(to_issue_texts({ _: [], x: { a: 1 } }), [
			'x: expected string | number | boolean | array, received object'
		]);
	});

	test('a null value fails, naming its key', () => {
		assert.deepEqual(to_issue_texts({ _: [], x: null }), [
			'x: expected string | number | boolean | array, received null'
		]);
	});

	test('a key that is not only word characters is quoted as JSON', () => {
		assert.deepEqual(to_issue_texts({ _: [], 'some-key': { a: 1 } }), [
			'"some-key": expected string | number | boolean | array, received object'
		]);
	});

	test('a nested array fails', () => {
		assert.deepEqual(to_issue_texts({ _: [], x: ['a', ['b']] }), [
			'x: expected string | number | boolean | array, received array'
		]);
	});

	test('an array holding an object fails', () => {
		assert.deepEqual(to_issue_texts({ _: [], x: [{}] }), [
			'x: expected string | number | boolean | array, received array'
		]);
	});

	test('reports every key that fails, in the input order', () => {
		assert.deepEqual(to_issue_texts({ _: [], ok: 1, y: {}, z: [[]] }), [
			'y: expected string | number | boolean | array, received object',
			'z: expected string | number | boolean | array, received array'
		]);
	});

	test('reports a bad `_` item before every key that fails', () => {
		assert.deepEqual(to_issue_texts({ z: [[]], _: [1, 'ok', 2], ok: 1, y: {}, w: null }), [
			'_.0: expected string, received number',
			'_.2: expected string, received number',
			'z: expected string | number | boolean | array, received array',
			'y: expected string | number | boolean | array, received object',
			'w: expected string | number | boolean | array, received null'
		]);
	});
});
