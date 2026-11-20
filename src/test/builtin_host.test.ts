import { describe, test, assert, vi } from 'vitest';

import {
	create_builtin_host,
	type BuiltinHostInvocation,
	type BuiltinHostRoot
} from '#lib/builtin_host.ts';
import { is_env_error } from '#lib/env_error.ts';
import { is_spawn_error, SpawnError } from '#lib/spawn_error.ts';
import { env } from '#lib/env.ts';
import type { Spawn, SpawnProgram } from '#lib/spawn.ts';
import type { ProcessOutput } from '#lib/stdout.ts';
import { TaskError } from '#lib/task_error.ts';
import { task as check_task } from '#lib/check.task.ts';

import { create_mock_fs_paths } from './fs_mocks.ts';
import { assert_throws_matching } from './test_helpers.ts';

const program = { name: 'git' } as SpawnProgram;

const to_invocation = (overrides: Partial<BuiltinHostInvocation> = {}): BuiltinHostInvocation => ({
	fs_paths: create_mock_fs_paths(),
	import_module: vi.fn(),
	...overrides
});

const stdout = { is_terminal: false } as ProcessOutput;
const stderr = { is_terminal: false } as ProcessOutput;

const create_root = (): BuiltinHostRoot & {
	program: ReturnType<typeof vi.fn>;
	find: ReturnType<typeof vi.fn>;
} => {
	const program_fn = vi.fn(() => program);
	const find = vi.fn(() => program);
	return {
		env,
		spawn: { program: program_fn, find } as unknown as Spawn,
		stdout,
		stderr,
		program: program_fn,
		find
	};
};

describe('create_builtin_host', () => {
	test('a builtin with no grants gets an empty host', () => {
		const host = create_builtin_host(create_root(), {}, to_invocation());
		assert.deepEqual(Reflect.ownKeys(host), []);
		assert.ok(Object.isFrozen(host));
		assert.strictEqual(Object.getPrototypeOf(host), null);
	});

	test('each handle is present only when the grants name it', () => {
		const fs_paths = create_mock_fs_paths();
		const root = create_root();
		assert.deepEqual(
			Object.keys(create_builtin_host(root, { env: [] }, to_invocation({ fs_paths }))),
			['env']
		);
		const host = create_builtin_host(root, { fs: { read: ['.'] } }, to_invocation({ fs_paths }));
		assert.deepEqual(Object.keys(host), ['fs_paths']);
		assert.strictEqual(host.fs_paths, fs_paths);
		assert.deepEqual(
			Object.keys(create_builtin_host(root, { programs: {} }, to_invocation({ fs_paths }))),
			['programs']
		);
		assert.deepEqual(
			Object.keys(create_builtin_host(root, { invokes: ['gen'] }, to_invocation({ fs_paths }))),
			[]
		);
	});

	test("`project_modules` holds the invocation's project loader and nothing that invokes", () => {
		const invocation = to_invocation();
		const host = create_builtin_host(create_root(), { project_modules: true }, invocation);
		assert.deepEqual(Object.keys(host), ['import_module']);
		assert.strictEqual(host.import_module, invocation.import_module);
		assert.deepEqual(
			Object.keys(create_builtin_host(create_root(), { fs: { read: ['.'] } }, invocation)),
			['fs_paths']
		);
	});

	test("`env` holds only the grants' names", () => {
		const host = create_builtin_host(create_root(), { env: ['CI'] }, to_invocation());
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
			{ programs: { git_cli: { name: 'git', pass_env: ['GIT_*'] }, node_cli: { name: 'node' } } },
			to_invocation()
		);
		assert.ok(Object.isFrozen(host.programs));
		assert.strictEqual(host.programs.program('git_cli'), program);
		assert.strictEqual(host.programs.find('node_cli'), program);
		assert.deepEqual(root.program.mock.calls, [['git', { pass_env: ['GIT_*'] }]]);
		assert.deepEqual(root.find.mock.calls, [['node', {}]]);
		// the program is always the grant's, whatever else a caller passes
		const loose = host.programs as unknown as Record<
			'program' | 'find',
			(role: string, name: string) => unknown
		>;
		assert.strictEqual(loose.program('git_cli', 'evil'), program);
		assert.strictEqual(loose.find('node_cli', 'evil'), program);
		assert.deepEqual(root.program.mock.calls[1], ['git', { pass_env: ['GIT_*'] }]);
		assert.deepEqual(root.find.mock.calls[1], ['node', {}]);
		const { programs } = host as unknown as {
			programs: Record<'program' | 'find', (name: string) => unknown>;
		};
		for (const name of ['git', 'vitest_cli', 'toString', '__proto__']) {
			for (const method of ['program', 'find'] as const) {
				assert_throws_matching(
					() => programs[method](name),
					(error) => is_spawn_error(error, 'not_granted')
				);
			}
		}
		assert.strictEqual(root.program.mock.calls.length, 2);
		assert.strictEqual(root.find.mock.calls.length, 2);
	});

	test('a later patch of `Object.hasOwn` grants no other program', () => {
		const root = create_root();
		const host = create_builtin_host(
			root,
			{ programs: { git_cli: { name: 'git' } } },
			to_invocation()
		);
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
		const host = create_builtin_host(create_root(), check_task.grants, to_invocation());
		assert.deepEqual(Object.keys(host).sort(), ['env', 'fs_paths', 'programs']);
		assert_throws_matching(
			() => host.env.get('HOME'),
			(error) => is_env_error(error, 'not_granted')
		);
	});
});

test('`programs` is a frozen null-prototype record', () => {
	const host = create_builtin_host(
		create_root(),
		{ programs: { git_cli: { name: 'git' } } },
		to_invocation()
	);
	assert.ok(Object.isFrozen(host.programs));
	assert.strictEqual(Object.getPrototypeOf(host.programs), null);
});

describe('a program grant', () => {
	test("`local` finds only the project's `node_modules/.bin`, never `PATH`", () => {
		const root = create_root();
		// only a copy on `PATH`, which never runs in place of the project's
		root.find.mockImplementation(((name: string) => (name === 'vitest' ? program : null)) as never);
		const host = create_builtin_host(
			root,
			{ programs: { vitest_cli: { name: 'vitest', local: true } } },
			to_invocation()
		);
		assert.strictEqual(host.programs.find('vitest_cli'), null);
		assert.deepEqual(
			root.find.mock.calls.map((c) => c[0]),
			['node_modules/.bin/vitest']
		);
		root.find.mockImplementation(((name: string) =>
			name === 'node_modules/.bin/vitest' ? program : null) as never
		);
		assert.strictEqual(host.programs.find('vitest_cli'), program);
	});

	test("`program` on a `local` grant resolves only the project's install", () => {
		const root = create_root();
		const host = create_builtin_host(
			root,
			{ programs: { vitest_cli: { name: 'vitest', local: true, pass_env: 'all' } } },
			to_invocation()
		);
		assert.strictEqual(host.programs.program('vitest_cli'), program);
		assert.deepEqual(root.program.mock.calls, [['node_modules/.bin/vitest', { pass_env: 'all' }]]);
		assert.deepEqual(root.find.mock.calls, []);
	});

	test('`local` turns a lookup failure into a `TaskError`, as `find_local_cli` does', () => {
		const root = create_root();
		root.find.mockImplementation(() => {
			throw new SpawnError('not_executable', 'node_modules/.bin/vitest');
		});
		const host = create_builtin_host(
			root,
			{ programs: { vitest_cli: { name: 'vitest', local: true } } },
			to_invocation()
		);
		assert_throws_matching(
			() => host.programs.find('vitest_cli'),
			(error) => error instanceof TaskError
		);
	});

	test("`program` on a `local` grant the project doesn't have throws `spawn.program`'s `SpawnError`", () => {
		const root = create_root();
		root.program.mockImplementation(() => {
			throw new SpawnError('not_found', 'node_modules/.bin/vitest');
		});
		const host = create_builtin_host(
			root,
			{ programs: { vitest_cli: { name: 'vitest', local: true } } },
			to_invocation()
		);
		assert_throws_matching(
			() => host.programs.program('vitest_cli'),
			(error) => is_spawn_error(error, 'not_found')
		);
	});

	test('a grant without `local` resolves its bare name, as `PATH` does', () => {
		const root = create_root();
		const host = create_builtin_host(
			root,
			{ programs: { git_cli: { name: 'git' } } },
			to_invocation()
		);
		assert.strictEqual(host.programs.find('git_cli'), program);
		assert.strictEqual(host.programs.program('git_cli'), program);
		assert.deepEqual(
			root.find.mock.calls.map((c) => c[0]),
			['git']
		);
		assert.deepEqual(
			root.program.mock.calls.map((c) => c[0]),
			['git']
		);
	});

	test('`omit_env` and `env` reach the program with `pass_env`', () => {
		const root = create_root();
		const host = create_builtin_host(
			root,
			{
				programs: {
					svelte_check_cli: {
						name: 'svelte-check',
						pass_env: 'all',
						omit_env: ['NODE_ENV'],
						env: { FORCE_COLOR: '1' }
					}
				}
			},
			to_invocation()
		);
		host.programs.find('svelte_check_cli');
		assert.deepEqual(root.find.mock.calls, [
			['svelte-check', { pass_env: 'all', omit_env: ['NODE_ENV'], env: { FORCE_COLOR: '1' } }]
		]);
	});
});

test("`stdout` and `stderr` are the root's, each held only when granted", () => {
	const root = create_root();
	const both = create_builtin_host(root, { stdout: true, stderr: true }, to_invocation());
	assert.strictEqual(both.stdout, stdout);
	assert.strictEqual(both.stderr, stderr);
	assert.deepEqual(Object.keys(create_builtin_host(root, { stderr: true }, to_invocation())), [
		'stderr'
	]);
});
