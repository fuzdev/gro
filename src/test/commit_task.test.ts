import { describe, test, assert, vi, beforeEach } from 'vitest';

import { task as commit_task, type Args } from '#lib/commit.task.ts';
import { TaskError } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { git_ops, install_mock_git } from './git_mocks.ts';

vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

const run_commit = () =>
	commit_task.run(create_mock_task_context<Args>({ _: ['the message'], origin: 'origin' }));

describe('gro commit', () => {
	beforeEach(async () => {
		vi.resetAllMocks();
		const mock_spawn = await get_mock_spawn();
		mock_spawn.reset();
		install_mock_git(mock_spawn);
	});

	test('pushes after a commit that succeeded', async () => {
		git_ops.git_command.mockResolvedValue({ kind: 'exited', code: 0 });
		await run_commit();
		assert.strictEqual(git_ops.git_push_to_create.mock.calls.length, 1);
	});

	test('a failed commit throws and pushes nothing', async () => {
		git_ops.git_command.mockResolvedValue({ kind: 'exited', code: 1 });
		let error: unknown;
		try {
			await run_commit();
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, 'nothing was pushed');
		assert.strictEqual(git_ops.git_push_to_create.mock.calls.length, 0);
	});
});
