import { describe, test, assert, vi, beforeEach } from 'vitest';

import { task as clean_task, type Args } from '#lib/clean.task.ts';
import { TaskError } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

vi.mock('#lib/clean_fs.ts', () => ({ clean_fs: vi.fn() }));

const to_context = () =>
	create_mock_task_context<Args>({
		build_dist: false,
		sveltekit: false,
		nodemodules: false,
		git: true,
		git_origin: 'upstream'
	});

describe('gro clean --git', () => {
	beforeEach(async () => {
		const mock = await get_mock_spawn();
		mock.reset();
		mock.available = (name) => name === 'git';
	});

	test('prunes the origin', async () => {
		await clean_task.run(to_context());
		assert.deepEqual((await get_mock_spawn()).calls[0]!.args, [
			'remote',
			'prune',
			'--end-of-options',
			'upstream'
		]);
	});

	test('a failed `git remote prune` is a TaskError', async () => {
		(await get_mock_spawn()).result = { kind: 'exited', code: 2 };
		let error: unknown;
		try {
			await clean_task.run(to_context());
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, '`git remote prune upstream` failed: code 2');
	});
});
