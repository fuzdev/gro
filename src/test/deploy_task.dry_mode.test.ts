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
		...actual,
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
	fs_exists: vi.fn(),
	fs_empty_dir: vi.fn()
}));

describe('deploy_task dry mode', () => {
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

	describe('dry mode behavior', () => {
		test('performs all steps except git commit and push', async () => {
			const { git_checkout, git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { fs_empty_dir } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			const { cp } = await import('node:fs/promises');
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));

			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should perform all preparation steps
			expect(git_checkout).toHaveBeenCalled();
			expect(git_pull).toHaveBeenCalled();
			expect(ctx.invoke_task).toHaveBeenCalledWith('build', {
				sync: true,
				gen: true,
				install: true,
				force_build: false
			});
			expect(fs_empty_dir).toHaveBeenCalled();
			expect(cp).toHaveBeenCalled();

			// Should NOT commit or push
			expect(spawn).not.toHaveBeenCalledWith('git', ['add', '.', '-f'], expect.anything());
			expect(spawn).not.toHaveBeenCalledWith(
				'git',
				['commit', '-m', 'deployment'],
				expect.anything()
			);
			expect(spawn).not.toHaveBeenCalledWith(
				'git',
				expect.arrayContaining(['push']),
				expect.anything()
			);
		});

		test('logs success message with deploy_dir path', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				deploy_dir: '.gro/deploy'
			});

			await deploy_task.run(ctx);

			// Should log dry deploy complete with path
			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.stringContaining('dry deploy complete'),
				expect.anything(),
				expect.stringContaining('.gro/deploy')
			);
		});

		test('uses custom deploy_dir in success message', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				deploy_dir: 'custom/path'
			});

			await deploy_task.run(ctx);

			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.anything(),
				expect.anything(),
				expect.stringContaining('custom/path')
			);
		});

		test('returns early after logging success', async () => {
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			const result = await deploy_task.run(ctx);

			// Should return undefined
			expect(result).toBeUndefined();

			// Should NOT proceed to commit/push
			expect(spawn).not.toHaveBeenCalledWith('git', ['add', '.', '-f'], expect.anything());
		});
	});

	describe('dry mode with build failure', () => {
		test('shows dry deploy failed message when build fails', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				build: true
			});

			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('Build failed'));

			await expect(deploy_task.run(ctx)).rejects.toThrow();

			// Should log dry deploy failed
			expect(ctx.log.info).toHaveBeenCalledWith(expect.stringContaining('dry deploy failed'));
		});

		test('still shows no git changes message on build failure', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				build: true
			});

			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('Build failed'));

			await expect(deploy_task.run(ctx)).rejects.toThrow();

			// Should still mention no git changes
			expect(ctx.log.error).toHaveBeenCalledWith(
				expect.anything(),
				expect.anything(),
				expect.stringContaining('no changes were made to git'),
				expect.anything()
			);
		});
	});

	describe('dry mode with missing build_dir', () => {
		test('throws error when build_dir missing', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));

			// build_dir doesn't exist
			vi.mocked(fs_exists).mockImplementation((path: any) => {
				const path_str = String(path);
				return !path_str.includes('build');
			});

			const ctx = create_mock_deploy_task_context({ dry: true });

			// Should throw error
			await expect(deploy_task.run(ctx)).rejects.toThrow('does not exist after building');
		});
	});

	describe('dry mode integration', () => {
		test('dry mode works with custom branches', async () => {
			const { git_checkout } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				source: 'develop',
				target: 'staging',
				force: true
			});

			await deploy_task.run(ctx);

			// Should still perform all prep steps with custom branches
			expect(git_checkout).toHaveBeenCalledWith('develop');
			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.stringContaining('dry deploy complete'),
				expect.anything(),
				expect.anything()
			);
		});

		test('dry mode works with no-build flag', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				build: false
			});

			await deploy_task.run(ctx);

			// Should not build
			expect(ctx.invoke_task).not.toHaveBeenCalled();
			// Should still log success
			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.stringContaining('dry deploy complete'),
				expect.anything(),
				expect.anything()
			);
		});

		test('dry mode works with reset flag', async () => {
			const { git_reset_branch_to_first_commit } = vi.mocked(
				await import('@fuzdev/fuz_util/git.ts')
			);
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				reset: true
			});

			await deploy_task.run(ctx);

			// Should still reset in dry mode
			expect(git_reset_branch_to_first_commit).toHaveBeenCalled();
			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.stringContaining('dry deploy complete'),
				expect.anything(),
				expect.anything()
			);
		});
	});

	describe('non-dry mode', () => {
		test('performs commit and push when dry=false', async () => {
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should commit and push
			expect(spawn).toHaveBeenCalledWith('git', ['add', '.', '-f'], {
				cwd: resolve('.gro/deploy')
			});
			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'deployment'], {
				cwd: resolve('.gro/deploy')
			});
			expect(spawn).toHaveBeenCalledWith('git', ['push', 'origin', 'deploy', '-f'], {
				cwd: resolve('.gro/deploy')
			});
		});

		test('logs deployed message when dry=false', async () => {
			const { fs_exists } = vi.mocked(await import('@fuzdev/fuz_util/fs.ts'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should log deployed (not dry deploy complete)
			expect(ctx.log.info).toHaveBeenCalledWith(expect.stringContaining('deployed'));
			expect(ctx.log.info).not.toHaveBeenCalledWith(expect.stringContaining('dry deploy complete'));
		});
	});
});
