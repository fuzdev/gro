import { describe, test, assert, vi, beforeEach } from 'vitest';

import { task as sync_task, type Args } from '#lib/sync.task.ts';
import { TaskError, type TaskContext } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';

vi.mock('#lib/sveltekit_helpers.ts', () => ({
	sveltekit_sync: vi.fn()
}));

vi.mock('#lib/npm_install_helpers.ts', () => ({
	install_with_cache_healing_or_throw: vi.fn()
}));

const create_mock_sync_task_context = (args: Partial<Args> = {}): TaskContext<Args> =>
	create_mock_task_context(
		args,
		{},
		{
			sveltekit: true,
			'no-sveltekit': false,
			package_json: true,
			'no-package_json': false,
			gen: true,
			'no-gen': false,
			install: false
		}
	);

describe('sync_task install', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('installs with the check that Gro and its deps are unchanged', async () => {
		const { install_with_cache_healing_or_throw } = vi.mocked(
			await import('#lib/npm_install_helpers.ts')
		);
		const ctx = create_mock_sync_task_context({ install: true });

		await sync_task.run(ctx);

		assert.strictEqual(install_with_cache_healing_or_throw.mock.calls.length, 1);
		assert.deepEqual(install_with_cache_healing_or_throw.mock.calls[0]![1], {
			log: ctx.log,
			throw_if_gro_packages_changed: true
		});
	});

	test('stops before syncing and gen when the install throws', async () => {
		const { install_with_cache_healing_or_throw } = vi.mocked(
			await import('#lib/npm_install_helpers.ts')
		);
		const { sveltekit_sync } = vi.mocked(await import('#lib/sveltekit_helpers.ts'));
		install_with_cache_healing_or_throw.mockRejectedValueOnce(new TaskError('re-run'));
		const ctx = create_mock_sync_task_context({ install: true });

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
		await sync_task.run(create_mock_sync_task_context());
		assert.strictEqual(install_with_cache_healing_or_throw.mock.calls.length, 0);
	});
});
