import { describe, test, assert, vi, beforeEach } from 'vitest';

import { task as sync_task, type Args } from '#lib/sync.task.ts';
import type { BuiltinTaskContext } from '#lib/builtin_host.ts';
import type { ProcessOutput } from '#lib/stdout.ts';
import type { SpawnProgram } from '#lib/spawn.ts';
import { TaskError } from '#lib/task_error.ts';

import { create_mock_builtin_context } from './test_helpers.ts';

vi.mock('#lib/sveltekit_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/sveltekit_helpers.ts')>()),
	sveltekit_sync: vi.fn(),
	// declared unless a test says otherwise, so the install tests also pin that the sync would run
	project_has_sveltekit_app: vi.fn(async () => true)
}));

vi.mock('#lib/npm_install_helpers.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/npm_install_helpers.ts')>()),
	install_with_cache_healing_or_throw: vi.fn()
}));

const stderr = { write: vi.fn() } as unknown as ProcessOutput;
const program = { name: 'npm' } as SpawnProgram;
const spawn = { program: vi.fn(() => program), find: vi.fn(() => program) };

const create_mock_sync_task_context = (
	args: Partial<Args> = {}
): Promise<BuiltinTaskContext<Args, typeof sync_task.grants>> =>
	create_mock_builtin_context(
		sync_task,
		{
			sveltekit: true,
			'no-sveltekit': false,
			package_json: true,
			'no-package_json': false,
			gen: true,
			'no-gen': false,
			install: false,
			...args
		},
		{ stderr, spawn: spawn as never }
	);

describe('sync_task install', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('installs with the check that Gro and its deps are unchanged', async () => {
		const { install_with_cache_healing_or_throw } = vi.mocked(
			await import('#lib/npm_install_helpers.ts')
		);
		const ctx = await create_mock_sync_task_context({ install: true });

		await sync_task.run(ctx);

		assert.strictEqual(install_with_cache_healing_or_throw.mock.calls.length, 1);
		const [programs, { fs_paths, ...options }] = install_with_cache_healing_or_throw.mock.calls[0]!;
		assert.deepEqual(options, {
			log: ctx.log,
			stderr,
			throw_if_gro_packages_changed: true
		});
		// the host's boundary and npm, with the grant's environment
		assert.strictEqual(fs_paths, ctx.host.fs_paths);
		assert.strictEqual(programs.program(), program);
		assert.deepEqual(spawn.program.mock.calls.at(-1), [
			'npm',
			{
				pass_env: 'all',
				omit_env: ['NODE_ENV', 'npm_config_production', 'npm_config_omit']
			}
		]);
	});

	test('stops before syncing and gen when the install throws', async () => {
		const { install_with_cache_healing_or_throw } = vi.mocked(
			await import('#lib/npm_install_helpers.ts')
		);
		const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
		install_with_cache_healing_or_throw.mockRejectedValueOnce(new TaskError('re-run'));
		const ctx = await create_mock_sync_task_context({ install: true });

		try {
			await sync_task.run(ctx);
			assert.fail('expected the install error');
		} catch (error) {
			assert(error instanceof TaskError);
		}
		assert.strictEqual(sveltekit_sync.mock.calls.length, 0);
		assert.strictEqual(vi.mocked(ctx.invoke_task).mock.calls.length, 0);
	});

	test('does not install by default', async () => {
		const { install_with_cache_healing_or_throw } = vi.mocked(
			await import('#lib/npm_install_helpers.ts')
		);
		await sync_task.run(await create_mock_sync_task_context());
		assert.strictEqual(install_with_cache_healing_or_throw.mock.calls.length, 0);
	});
});

describe('sync_task SvelteKit detection', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('syncs a project with SvelteKit, its log reporting a bad package.json', async () => {
		const { project_has_sveltekit_app, sveltekit_sync } = vi.mocked(
			await import('#lib/sveltekit_helpers.ts')
		);
		const ctx = await create_mock_sync_task_context();

		await sync_task.run(ctx);

		assert.strictEqual(sveltekit_sync.mock.calls.length, 1);
		assert.strictEqual(sveltekit_sync.mock.calls[0]!.length, 1);
		// the host's SvelteKit CLI, the project's first
		assert.strictEqual(sveltekit_sync.mock.calls[0]![0].find('svelte-kit'), program);
		assert.deepEqual(spawn.find.mock.calls.at(-1), [
			'node_modules/.bin/svelte-kit',
			{ pass_env: 'all' }
		]);
		assert.deepEqual(vi.mocked(ctx.log.info).mock.calls, [['synced SvelteKit']]);
		assert.strictEqual(project_has_sveltekit_app.mock.calls[0]![1], ctx.log);
	});

	test('skips a project without SvelteKit and still runs gen', async () => {
		const { project_has_sveltekit_app, sveltekit_sync } = vi.mocked(
			await import('#lib/sveltekit_helpers.ts')
		);
		project_has_sveltekit_app.mockResolvedValueOnce(false);
		const ctx = await create_mock_sync_task_context();

		await sync_task.run(ctx);

		assert.strictEqual(sveltekit_sync.mock.calls.length, 0);
		assert.deepEqual(vi.mocked(ctx.log.info).mock.calls, []);
		assert.deepEqual(vi.mocked(ctx.invoke_task).mock.calls, [['gen']]);
	});

	test('`--no-sveltekit` skips a project with SvelteKit', async () => {
		const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));

		await sync_task.run(await create_mock_sync_task_context({ sveltekit: false }));

		assert.strictEqual(sveltekit_sync.mock.calls.length, 0);
	});
});
