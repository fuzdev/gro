import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import { resolve } from 'node:path';

import { task as deploy_task } from '#lib/deploy.task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
	FIRST_COMMIT_HASH
} from './deploy_task_test_helpers.ts';
import { get_mock_project } from './fs_mocks.ts';
import { git_ops } from './git_mocks.ts';

// Mock dependencies
vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('@fuzdev/fuz_util/process.js', () => ({
	spawn: vi.fn(),
	spawn_out: vi.fn()
}));

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

describe('deploy_task dry mode', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('dry mode behavior', () => {
		test('performs all steps except git commit and push', async () => {
			const { git_checkout, git_pull } = git_ops;
			const { copy } = await get_mock_project();
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { exists, remove } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

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
			expect(remove).toHaveBeenCalled();
			expect(copy).toHaveBeenCalled();

			// Should NOT commit or push
			expect(spawn).not.toHaveBeenCalledWith('git', ['add', '.'], expect.anything());
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
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

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
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				deploy_dir: '.gro/custom/path'
			});

			await deploy_task.run(ctx);

			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.anything(),
				expect.anything(),
				expect.stringContaining('.gro/custom/path')
			);
		});

		test('returns early after logging success', async () => {
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			const result = await deploy_task.run(ctx);

			// Should return undefined
			expect(result).toBeUndefined();

			// Should NOT proceed to commit/push
			expect(spawn).not.toHaveBeenCalledWith('git', ['add', '.'], expect.anything());
		});
	});

	describe('dry mode with build failure', () => {
		test('shows dry deploy failed message when build fails', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

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
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

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
			const { exists } = await get_mock_project();

			// build_dir doesn't exist
			vi.mocked(exists).mockImplementation((path: any) => {
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
			const { git_checkout } = git_ops;
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

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
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

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
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				reset: true
			});

			await deploy_task.run(ctx);

			// resets locally in dry mode, pushing nothing
			expect(spawn).toHaveBeenCalledWith('git', ['reset', '--hard', FIRST_COMMIT_HASH], {
				cwd: resolve('.gro/deploy')
			});
			expect(spawn.mock.calls.some((call) => call[1]?.[0] === 'push')).toBe(false);
			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.stringContaining('dry deploy complete'),
				expect.anything(),
				expect.anything()
			);
		});

		test('a first dry deploy creates the target branch locally, pushing nothing', async () => {
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { git_remote_branch_exists, git_push_to_create } = git_ops;
			const { exists } = await get_mock_project();
			vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
			vi.mocked(exists).mockImplementation(async (path: any) => String(path).includes('build'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			expect(spawn).toHaveBeenCalledWith('git', ['checkout', '--orphan', 'deploy'], {
				cwd: resolve('.gro/deploy')
			});
			expect(git_push_to_create).not.toHaveBeenCalled();
			expect(spawn.mock.calls.some((call) => call[1]?.[0] === 'push')).toBe(false);
		});
	});

	describe('non-dry mode', () => {
		test('performs commit and push when dry=false', async () => {
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should commit and push
			expect(spawn).toHaveBeenCalledWith('git', ['add', '.'], {
				cwd: resolve('.gro/deploy')
			});
			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'deployment'], {
				cwd: resolve('.gro/deploy')
			});
			expect(spawn).toHaveBeenCalledWith(
				'git',
				['push', '-f', '--end-of-options', 'origin', 'deploy'],
				{
					cwd: resolve('.gro/deploy')
				}
			);
		});

		test('logs deployed message when dry=false', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should log deployed (not dry deploy complete)
			expect(ctx.log.info).toHaveBeenCalledWith(expect.stringContaining('deployed'));
			expect(ctx.log.info).not.toHaveBeenCalledWith(expect.stringContaining('dry deploy complete'));
		});
	});
});
