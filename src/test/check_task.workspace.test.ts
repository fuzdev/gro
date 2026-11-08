import { describe, test, assert, vi, beforeEach } from 'vitest';

import { task as check_task, type Args } from '#lib/check.task.ts';
import { TaskError } from '#lib/task.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_spawn } from './spawn_mocks.ts';
import { git_ops, install_mock_git } from './git_mocks.ts';

vi.mock('#lib/spawn.ts', async () => (await import('./spawn_mocks.ts')).create_mock_spawn_module());

const create_ctx = () =>
	create_mock_task_context<Args>({ workspace: true }, {}, {
		typecheck: false,
		test: false,
		gen: false,
		format: false,
		package_json: false,
		lint: false,
		build: false,
		sync: false,
		install: false,
		workspace: true
	} as unknown as Args);

describe('gro check --workspace', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		(await get_mock_spawn()).reset();
	});

	test('warns and passes with no git on PATH, the check not having run', async () => {
		// `available` by default finds only a project's local bins, so no `git`
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.include(vi.mocked(ctx.log.warn).mock.calls[0]?.[0] as string, 'git not found');
	});

	test('passes a clean workspace without a warning', async () => {
		install_mock_git(await get_mock_spawn());
		git_ops.git_check_clean_workspace.mockResolvedValue(null);
		const ctx = create_ctx();
		await check_task.run(ctx);
		assert.strictEqual(vi.mocked(ctx.log.warn).mock.calls.length, 0);
	});

	test('fails a dirty workspace', async () => {
		const mock_spawn = await get_mock_spawn();
		install_mock_git(mock_spawn);
		git_ops.git_check_clean_workspace.mockResolvedValue('git has unstaged changes');
		git_ops.git_command.mockResolvedValue({ kind: 'exited', code: 0 });
		let error: unknown;
		try {
			await check_task.run(create_ctx());
		} catch (e) {
			error = e;
		}
		assert.instanceOf(error, TaskError);
		assert.include(error.message, 'git_check_clean_workspace');
	});
});
