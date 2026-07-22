import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import { resolve } from 'node:path';

import { task as deploy_task } from '$lib/deploy.task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock
} from './deploy_task_test_helpers.ts';

// Mock dependencies
vi.mock('@fuzdev/fuz_util/git.js', async (import_original) => {
	const actual = await import_original<typeof import('@fuzdev/fuz_util/git.ts')>();
	return {
		...actual, // Preserves GitBranch, GitOrigin, and other exports
		git_check_clean_workspace: vi.fn(),
		git_check_setting_pull_rebase: vi.fn(),
		git_local_branch_exists: vi.fn(),
		git_fetch: vi.fn(),
		git_checkout: vi.fn(),
		git_pull: vi.fn(),
		git_remote_branch_exists: vi.fn(),
		git_clone_locally: vi.fn(),
		git_current_branch_name: vi.fn(),
		git_delete_local_branch: vi.fn(),
		git_push_to_create: vi.fn(),
		git_reset_branch_to_first_commit: vi.fn()
	};
});

vi.mock('@fuzdev/fuz_util/process.js', () => ({
	spawn: vi.fn()
}));

vi.mock('node:fs/promises', () => ({
	cp: vi.fn(),
	mkdir: vi.fn(),
	rm: vi.fn(),
	readdir: vi.fn()
}));

vi.mock('@fuzdev/fuz_util/fs.js', () => ({
	fs_empty_dir: vi.fn(),
	fs_exists: vi.fn()
}));

describe('deploy_task args', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();

		const { fs_empty_dir } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
		vi.mocked(fs_empty_dir).mockResolvedValue(undefined);
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	test('uses default args when none provided', async () => {
		const { git_checkout, git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
		const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
		vi.mocked(fs_exists).mockResolvedValue(true);

		const ctx = create_mock_deploy_task_context({ dry: true }); // dry to skip push

		await deploy_task.run(ctx);

		// Should use default source branch 'main'
		expect(git_checkout).toHaveBeenCalledWith('main');
		expect(git_pull).toHaveBeenNthCalledWith(1, 'origin', 'main');
	});

	test('respects custom source branch', async () => {
		const { git_checkout, git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
		const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
		vi.mocked(fs_exists).mockResolvedValue(true);

		const ctx = create_mock_deploy_task_context({
			source: 'develop',
			dry: true
		});

		await deploy_task.run(ctx);

		expect(git_checkout).toHaveBeenCalledWith('develop');
		expect(git_pull).toHaveBeenNthCalledWith(1, 'origin', 'develop');
	});

	test('respects custom target branch with force flag', async () => {
		const { git_current_branch_name } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
		const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
		vi.mocked(fs_exists).mockResolvedValue(true);
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
		const { git_pull, git_remote_branch_exists } = vi.mocked(
			await import('@fuzdev/fuz_util/git.ts')
		);
		const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
		vi.mocked(fs_exists).mockResolvedValue(true);

		const ctx = create_mock_deploy_task_context({
			origin: 'upstream',
			dry: true
		});

		await deploy_task.run(ctx);

		expect(git_remote_branch_exists).toHaveBeenCalledWith('upstream', 'deploy');
		expect(git_pull).toHaveBeenNthCalledWith(1, 'upstream', 'main');
	});

	test('respects custom deploy_dir', async () => {
		const { git_current_branch_name } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
		const { fs_exists, fs_empty_dir } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
		vi.mocked(fs_exists).mockResolvedValue(true);

		const ctx = create_mock_deploy_task_context({
			deploy_dir: 'custom/deploy',
			dry: true
		});

		await deploy_task.run(ctx);

		// Should use custom deploy_dir in operations
		expect(git_current_branch_name).toHaveBeenCalledWith({ cwd: resolve('custom/deploy') });
		expect(fs_empty_dir).toHaveBeenCalledWith(resolve('custom/deploy'), expect.any(Function));
	});

	test('respects custom build_dir', async () => {
		const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
		const { cp, readdir } = vi.mocked(await import('node:fs/promises'));

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readdir).mockResolvedValue(['index.html'] as any);

		const ctx = create_mock_deploy_task_context({
			build_dir: 'dist',
			dry: true
		});

		await deploy_task.run(ctx);

		// Should read from custom build_dir
		expect(readdir).toHaveBeenCalledWith('dist');
		expect(cp).toHaveBeenCalledWith(
			expect.stringContaining('dist/'),
			expect.anything(),
			expect.anything()
		);
	});

	describe('build/no-build dual args', () => {
		test('calls build task when build=true (default)', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

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
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: false,
				'no-build': true,
				dry: true
			});

			await deploy_task.run(ctx);

			expect(ctx.invoke_task).not.toHaveBeenCalled();
		});

		test('skips build task when build=false', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

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
			const { git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should pull source branch (first call)
			expect(git_pull).toHaveBeenNthCalledWith(1, 'origin', 'main');
		});

		test('skips source pull when no-pull=true', async () => {
			const { git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

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
			const { git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

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
