import { describe, test, assert, vi } from 'vitest';

import { create_builtin_host, type BuiltinHostRoot } from '#lib/builtin_host.ts';
import { is_env_error } from '#lib/env_error.ts';
import { is_spawn_error } from '#lib/spawn_error.ts';
import { env } from '#lib/env.ts';
import type { Spawn, SpawnProgram } from '#lib/spawn.ts';
import { task as check_task } from '#lib/check.task.ts';

import { create_mock_fs_paths } from './fs_mocks.ts';
import { assert_throws_matching } from './test_helpers.ts';

const program = { name: 'git' } as SpawnProgram;

const create_root = (): BuiltinHostRoot & {
	program: ReturnType<typeof vi.fn>;
	find: ReturnType<typeof vi.fn>;
} => {
	const program_fn = vi.fn(() => program);
	const find = vi.fn(() => program);
	return {
		env,
		spawn: { program: program_fn, find } as unknown as Spawn,
		program: program_fn,
		find
	};
};

describe('create_builtin_host', () => {
	test('a builtin with no grants gets only its own frozen copy of the forwarded args', () => {
		const raw_rest_args = ['--', 'x'];
		const host = create_builtin_host(create_root(), {}, create_mock_fs_paths(), raw_rest_args);
		assert.deepEqual(Object.keys(host), ['raw_rest_args']);
		assert.deepEqual(host.raw_rest_args, raw_rest_args);
		assert.notStrictEqual(host.raw_rest_args, raw_rest_args);
		assert.ok(Object.isFrozen(host.raw_rest_args));
		raw_rest_args.push('y');
		assert.deepEqual(host.raw_rest_args, ['--', 'x']);
		assert.ok(Object.isFrozen(host));
		assert.strictEqual(Object.getPrototypeOf(host), null);
	});

	test('each handle is present only when the grants name it', () => {
		const fs_paths = create_mock_fs_paths();
		const root = create_root();
		assert.deepEqual(Object.keys(create_builtin_host(root, { env: [] }, fs_paths, [])), [
			'raw_rest_args',
			'env'
		]);
		const host = create_builtin_host(root, { fs: { read: ['.'] } }, fs_paths, []);
		assert.deepEqual(Object.keys(host), ['raw_rest_args', 'fs_paths']);
		assert.strictEqual(host.fs_paths, fs_paths);
		assert.deepEqual(Object.keys(create_builtin_host(root, { programs: {} }, fs_paths, [])), [
			'raw_rest_args',
			'programs'
		]);
		assert.deepEqual(Object.keys(create_builtin_host(root, { invokes: ['gen'] }, fs_paths, [])), [
			'raw_rest_args'
		]);
	});

	test("`env` holds only the grants' names", () => {
		const host = create_builtin_host(create_root(), { env: ['CI'] }, create_mock_fs_paths(), []);
		assert.strictEqual(host.env.get('CI'), env.get('CI'));
		assert_throws_matching(
			() => host.env.get('HOME'),
			(error) => is_env_error(error, 'not_granted')
		);
	});

	test('`programs` builds a granted program with its environment, refusing any other', () => {
		const root = create_root();
		const host = create_builtin_host(
			root,
			{ programs: { git: { pass_env: ['GIT_*'] }, node: {} } },
			create_mock_fs_paths(),
			[]
		);
		assert.ok(Object.isFrozen(host.programs));
		assert.strictEqual(host.programs.program('git'), program);
		assert.strictEqual(host.programs.find('node'), program);
		assert.deepEqual(root.program.mock.calls, [['git', { pass_env: ['GIT_*'] }]]);
		assert.deepEqual(root.find.mock.calls, [['node', undefined]]);
		const { programs } = host as unknown as {
			programs: Record<'program' | 'find', (name: string) => unknown>;
		};
		for (const name of ['vitest', 'toString', '__proto__']) {
			for (const method of ['program', 'find'] as const) {
				assert_throws_matching(
					() => programs[method](name),
					(error) => is_spawn_error(error, 'not_granted')
				);
			}
		}
		assert.strictEqual(root.program.mock.calls.length, 1);
		assert.strictEqual(root.find.mock.calls.length, 1);
	});

	test('a later patch of `Object.hasOwn` grants no other program', () => {
		const root = create_root();
		const host = create_builtin_host(root, { programs: { git: {} } }, create_mock_fs_paths(), []);
		const { programs } = host as unknown as { programs: { find: (name: string) => unknown } };
		const original = Object.hasOwn;
		Object.hasOwn = () => true;
		try {
			assert_throws_matching(
				() => programs.find('toString'),
				(error) => is_spawn_error(error, 'not_granted')
			);
		} finally {
			Object.hasOwn = original;
		}
		assert.strictEqual(root.find.mock.calls.length, 0);
	});

	test("`check`'s host holds what its grants name", () => {
		const host = create_builtin_host(create_root(), check_task.grants, create_mock_fs_paths(), []);
		assert.deepEqual(Object.keys(host).sort(), ['env', 'fs_paths', 'programs', 'raw_rest_args']);
		assert_throws_matching(
			() => host.env.get('HOME'),
			(error) => is_env_error(error, 'not_granted')
		);
	});
});

test('`programs` is a frozen null-prototype record', () => {
	const host = create_builtin_host(
		create_root(),
		{ programs: { git: {} } },
		create_mock_fs_paths(),
		[]
	);
	assert.ok(Object.isFrozen(host.programs));
	assert.strictEqual(Object.getPrototypeOf(host.programs), null);
});
