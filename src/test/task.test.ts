import { describe, test, expect, assert } from 'vitest';
import { join, resolve } from 'node:path';
import { noop } from '@fuzdev/fuz_util/function.ts';

import {
	is_builtin_task_name,
	is_task_path,
	is_task_path_input,
	load_task,
	to_builtin_task_id,
	to_task_command,
	to_task_path,
	validate_task_module
} from '#lib/task.ts';
import * as actual_test_task_module from '#lib/test.task.ts';
import { paths } from '#lib/paths.ts';

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

	test('is_task_path_input: a `/` or a task suffix makes a path, anything else is a name', () => {
		expect(is_task_path_input('check')).toBe(false);
		expect(is_task_path_input('foo')).toBe(false);
		expect(is_task_path_input('foo.ts')).toBe(false);
		expect(is_task_path_input('')).toBe(false);
		expect(is_task_path_input('foo.task.ts')).toBe(true);
		expect(is_task_path_input('foo.task.js')).toBe(true);
		expect(is_task_path_input('src/lib/foo')).toBe(true);
		expect(is_task_path_input('src/lib/')).toBe(true);
		expect(is_task_path_input('gro/check')).toBe(true);
		expect(is_task_path_input('./foo')).toBe(true);
		expect(is_task_path_input('/a/foo.task.ts')).toBe(true);
	});

	test('to_task_path normalizes to the root-relative path, `./` optional, absolute accepted', () => {
		expect(to_task_path('src/lib/a.task.ts', '/p')).toBe('src/lib/a.task.ts');
		expect(to_task_path('./src/lib/a.task.ts', '/p')).toBe('src/lib/a.task.ts');
		expect(to_task_path('src//lib/../lib/a.task.ts', '/p')).toBe('src/lib/a.task.ts');
		expect(to_task_path('/p/src/lib/a.task.ts', '/p')).toBe('src/lib/a.task.ts');
		expect(to_task_path('/p/src/lib/a.task.ts', '/p/')).toBe('src/lib/a.task.ts');
		expect(to_task_path('a.task.ts', '/p')).toBe('a.task.ts');
		expect(to_task_path('/q/a.task.ts', '/p')).toBe('../q/a.task.ts');
		expect(to_task_path('src/lib/a.task.ts')).toBe('src/lib/a.task.ts');
		expect(to_task_path(resolve('src/lib/a.task.ts'))).toBe('src/lib/a.task.ts');
	});

	test('to_task_command', () => {
		expect(to_task_command('check')).toBe('gro check');
		expect(to_task_command('src/lib/a.task.ts')).toBe('gro task src/lib/a.task.ts');
		expect(to_task_command('a.task.ts')).toBe('gro task a.task.ts');
	});

	test('is_builtin_task_name', () => {
		expect(is_builtin_task_name('check')).toBe(true);
		expect(is_builtin_task_name('test')).toBe(true);
		expect(is_builtin_task_name('run')).toBe(true);
		expect(is_builtin_task_name('task')).toBe(false);
		expect(is_builtin_task_name('resolve')).toBe(false);
		expect(is_builtin_task_name('gro/check')).toBe(false);
		expect(is_builtin_task_name('check.task.ts')).toBe(false);
		expect(is_builtin_task_name('')).toBe(false);
		expect(is_builtin_task_name('__proto__')).toBe(false);
		expect(is_builtin_task_name('constructor')).toBe(false);
		expect(is_builtin_task_name('toString')).toBe(false);
	});

	test('to_builtin_task_id is beside the task module, in source here', () => {
		expect(to_builtin_task_id('check')).toBe(join(paths.lib, 'check.task.ts'));
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
		expect(validate_task_module({ task: { run: noop } })).toBe(true);
		expect(validate_task_module({ task: { run: {} } })).toBe(false);
	});

	test('load_task imports the module with the name it displays', async () => {
		const id = to_builtin_task_id('test');
		const result = await load_task(id, 'test');
		assert.ok(result.ok);
		assert.strictEqual(result.value.id, id);
		assert.strictEqual(result.value.name, 'test');
		assert.ok(Object.is(result.value.mod, actual_test_task_module));
	});

	test('load_task fails on a module without a task', async () => {
		const id = resolve('src/test/fixtures/test_invalid_task_module.ts');
		const result = await load_task(id, 'x');
		assert.ok(!result.ok);
		assert.include(result.reason, 'has no `task` export with a `run` function');
	});

	test('load_task fails on a missing module', async () => {
		const result = await load_task(resolve('src/test/fixtures/missing.task.ts'), 'x');
		assert.ok(!result.ok);
		assert.include(result.reason, 'failed to import');
	});
});
