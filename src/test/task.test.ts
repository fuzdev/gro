import {describe, test, expect} from 'vitest';
import {resolve} from 'node:path';
import {noop} from '@fuzdev/fuz_util/function.js';

import {
	is_task_path,
	to_task_name,
	validate_task_module,
	find_tasks,
	load_tasks,
} from '../lib/task.ts';
import * as actual_test_task_module from '../lib/test.task.ts';
import {create_empty_gro_config} from '../lib/gro_config.ts';
import {GRO_DIST_DIR} from '../lib/paths.ts';

describe('task', () => {
	test('is_task_path basic behavior', () => {
		expect(is_task_path('foo.task.ts')).toBe(true);
		expect(is_task_path('foo.task.js')).toBe(true);
		expect(is_task_path('foo.ts')).toBe(false);
		expect(is_task_path('foo.js')).toBe(false);
		expect(is_task_path('bar/baz/foo.task.ts')).toBe(true);
		expect(is_task_path('bar/baz/foo.task.js')).toBe(true);
		expect(is_task_path('bar/baz/foo.ts')).toBe(false);
		expect(is_task_path('bar/baz/foo.js')).toBe(false);
	});

	test('to_task_name basic behavior', () => {
		expect(to_task_name('foo.task.ts', process.cwd(), '', '')).toBe('foo');
		expect(to_task_name('foo.task.js', process.cwd(), '', '')).toBe('foo');
		expect(to_task_name('bar/baz/foo.task.ts', process.cwd(), '', '')).toBe('bar/baz/foo');
		expect(to_task_name('a/b/c/foo.task.ts', 'a/b/c', '', '')).toBe('foo');
		expect(to_task_name('a/b/c/foo.task.ts', 'a', '', '')).toBe('b/c/foo');
		expect(to_task_name('a/b/c/foo.task.ts', 'a/b', '', '')).toBe('c/foo');
		expect(to_task_name('/a/b/c/foo.task.ts', '/a/b', '/a/b', '/a/b/d')).toBe('../c/foo');
		expect(to_task_name('/a/b/c/foo.task.ts', '/a/b', '/a/b', '/a/b')).toBe('c/foo');
		expect(to_task_name('/a/b/c/foo.task.ts', '/a/b', '/a/b', '/a/b/c')).toBe('foo');
		expect(to_task_name('/a/b/d/foo.task.js', '/a/b/d', '/a/b/d/foo', '/a/c')).toBe('../b/d/foo');
		expect(
			to_task_name(
				GRO_DIST_DIR + 'foo.task.js',
				GRO_DIST_DIR.slice(0, -1),
				GRO_DIST_DIR + 'foo',
				'/a',
			),
		).toBe('foo');
		expect(
			to_task_name(
				GRO_DIST_DIR + 'foo.task.js',
				GRO_DIST_DIR, // same as above but adds a trailing slash here
				GRO_DIST_DIR + 'foo',
				'/a',
			),
		).toBe('foo');
		expect(to_task_name(resolve('a/b'), resolve('b'), '', '')).toBe(resolve('a/b'));
	});

	test('validate_task_module basic behavior', async () => {
		// TODO if we import directly, svelte-package generates types in `src/test/fixtures`
		const test_task_module_js = await import('./fixtures/' + 'test_task_module.task_fixture.js');
		const test_task_module_ts = await import('./fixtures/' + 'test_task_module.task_fixture.ts');
		const test_invalid_task_module_js = await import('./fixtures/' + 'test_invalid_task_module.js');
		const test_invalid_task_module_ts = await import('./fixtures/' + 'test_invalid_task_module.ts');
		expect(validate_task_module(test_task_module_js)).toBe(true);
		expect(validate_task_module(test_task_module_ts)).toBe(true);
		expect(validate_task_module(test_invalid_task_module_js)).toBe(false);
		expect(validate_task_module(test_invalid_task_module_ts)).toBe(false);
		// demonstrating values:
		expect(validate_task_module({task: {run: noop}})).toBe(true);
		expect(validate_task_module({task: {run: {}}})).toBe(false);
	});

	test('load_tasks basic behavior', async () => {
		const found = await find_tasks(
			[resolve('src/lib/test'), resolve('src/lib/test.task.ts')],
			[resolve('src/lib')],
			create_empty_gro_config(),
		);
		expect(found.ok).toBe(true);
		if (found.ok) {
			const result = await load_tasks(found.value);
			expect(result.ok).toBe(true);
			if (result.ok) {
				expect(result.value.modules.length).toBe(1);
				expect(result.value.modules[0]!.mod).toBe(actual_test_task_module);
			}
		}
	});
});
