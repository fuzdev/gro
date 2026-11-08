import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import { resolve } from 'node:path';

import { task as deploy_task } from '#lib/deploy.task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
	mock_listings
} from './deploy_task_test_helpers.ts';
import { get_mock_project } from './fs_mocks.ts';
import { git_ops } from './git_mocks.ts';
import { get_mock_spawn } from './spawn_mocks.ts';

// Mock dependencies
vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('@fuzdev/fuz_util/process.js', () => ({
	spawn: vi.fn(),
	spawn_out: vi.fn()
}));

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

describe('deploy_task args', () => {
	test('builds its git with the whole environment, a named unconfined phase', async () => {
		const ctx = create_mock_deploy_task_context({ dry: true });
		await deploy_task.run(ctx);
		const mock = await get_mock_spawn();
		const git = mock.calls.find((call) => call.name === 'git');
		expect(git?.program_options).toEqual({ pass_env: 'all' });
	});

	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	test('uses default args when none provided', async () => {
		const { git_checkout, git_pull } = git_ops;
		const { exists } = await get_mock_project();
		vi.mocked(exists).mockResolvedValue(true);

		const ctx = create_mock_deploy_task_context({ dry: true }); // dry to skip push

		await deploy_task.run(ctx);

		// Should use default source branch 'main'
		expect(git_checkout).toHaveBeenCalledWith('main');
		expect(git_pull).toHaveBeenNthCalledWith(1, 'origin', 'main');
	});

	test('respects custom source branch', async () => {
		const { git_checkout, git_pull } = git_ops;
		const { exists } = await get_mock_project();
		vi.mocked(exists).mockResolvedValue(true);

		const ctx = create_mock_deploy_task_context({
			source: 'develop',
			dry: true
		});

		await deploy_task.run(ctx);

		expect(git_checkout).toHaveBeenCalledWith('develop');
		expect(git_pull).toHaveBeenNthCalledWith(1, 'origin', 'develop');
	});

	test('respects custom target branch with force flag', async () => {
		const { git_current_branch_name } = git_ops;
		const { exists } = await get_mock_project();
		vi.mocked(exists).mockResolvedValue(true);
		vi.mocked(git_current_branch_name).mockResolvedValue('custom-deploy');

		const ctx = create_mock_deploy_task_context({
			target: 'custom-deploy',
			force: true,
			dry: true
		});

		await deploy_task.run(ctx);

		// Should check if deploy dir has correct branch
		expect(git_current_branch_name).toHaveBeenCalledWith({
			cwd: resolve('.gro/deploy')
		});
	});

	test('respects custom origin', async () => {
		const { git_pull, git_remote_branch_exists } = git_ops;
		const { exists } = await get_mock_project();
		vi.mocked(exists).mockResolvedValue(true);

		const ctx = create_mock_deploy_task_context({
			origin: 'upstream',
			dry: true
		});

		await deploy_task.run(ctx);

		expect(git_remote_branch_exists).toHaveBeenCalledWith('upstream', 'deploy');
		expect(git_pull).toHaveBeenNthCalledWith(1, 'upstream', 'main');
	});

	test('respects custom deploy_dir', async () => {
		const { git_current_branch_name } = git_ops;
		const { exists, list } = await get_mock_project();
		vi.mocked(exists).mockResolvedValue(true);

		const ctx = create_mock_deploy_task_context({
			deploy_dir: '.gro/custom/deploy',
			dry: true
		});

		await deploy_task.run(ctx);

		// Should use custom deploy_dir in operations
		expect(git_current_branch_name).toHaveBeenCalledWith({ cwd: resolve('.gro/custom/deploy') });
		expect(list).toHaveBeenCalledWith('.gro/custom/deploy');
	});

	test('respects custom build_dir', async () => {
		const { exists, list, copy } = await get_mock_project();

		vi.mocked(exists).mockResolvedValue(true);
		await mock_listings(['index.html'], 'dist');

		const ctx = create_mock_deploy_task_context({
			build_dir: 'dist',
			dry: true
		});

		await deploy_task.run(ctx);

		// Should read from custom build_dir
		expect(list).toHaveBeenCalledWith('dist');
		expect(copy).toHaveBeenCalledWith('dist/index.html', '.gro/deploy/index.html', {
			recursive: true
		});
	});

	describe('build/no-build dual args', () => {
		test('calls build task when build=true (default)', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			expect(ctx.invoke_task).toHaveBeenCalledWith('build', {
				sync: true,
				gen: true,
				install: true,
				force_build: false
			});
		});

		test('skips build task when no-build=true', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: false,
				'no-build': true,
				dry: true
			});

			await deploy_task.run(ctx);

			expect(ctx.invoke_task).not.toHaveBeenCalled();
		});

		test('skips build task when build=false', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: false,
				dry: true
			});

			await deploy_task.run(ctx);

			expect(ctx.invoke_task).not.toHaveBeenCalled();
		});
	});

	describe('pull/no-pull dual args', () => {
		test('pulls source branch when pull=true (default)', async () => {
			const { git_pull } = git_ops;
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should pull source branch (first call)
			expect(git_pull).toHaveBeenNthCalledWith(1, 'origin', 'main');
		});

		test('skips source pull when no-pull=true', async () => {
			const { git_pull } = git_ops;
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				pull: false,
				'no-pull': true,
				dry: true
			});

			await deploy_task.run(ctx);

			// Should still pull target branch (to sync deploy dir), but not source
			expect(git_pull).toHaveBeenCalledTimes(1);
			expect(git_pull).toHaveBeenCalledWith('origin', 'deploy', { cwd: resolve('.gro/deploy') });
		});

		test('skips source pull when pull=false', async () => {
			const { git_pull } = git_ops;
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				pull: false,
				dry: true
			});

			await deploy_task.run(ctx);

			// Should still pull target branch (to sync deploy dir), but not source
			expect(git_pull).toHaveBeenCalledTimes(1);
			expect(git_pull).toHaveBeenCalledWith('origin', 'deploy', { cwd: resolve('.gro/deploy') });
		});
	});
});
