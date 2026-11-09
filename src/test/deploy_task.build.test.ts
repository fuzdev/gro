import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';

import { task as deploy_task } from '#lib/deploy.task.ts';
import { TaskError } from '#lib/task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
	mock_listings
} from './deploy_task_test_helpers.ts';
import { get_mock_project } from './fs_mocks.ts';
import { git_ops } from './git_mocks.ts';

// Mock dependencies
vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

describe('deploy_task build integration', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('build task invocation', () => {
		test('invokes build task when build=true', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: true,
				dry: true
			});

			await deploy_task.run(ctx);

			expect(ctx.invoke_task).toHaveBeenCalledWith('build', {
				sync: true,
				gen: true,
				install: true,
				force_build: false
			});
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

		test('build happens after source and target branch preparation', async () => {
			const { git_checkout } = git_ops;
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: true,
				dry: true
			});

			await deploy_task.run(ctx);

			// git_checkout should be called before invoke_task
			expect(git_checkout).toHaveBeenCalled();
			expect(ctx.invoke_task).toHaveBeenCalled();

			const checkout_order = git_checkout.mock.invocationCallOrder[0]!;
			const build_order = (ctx.invoke_task as any).mock.invocationCallOrder[0]!;

			expect(build_order).toBeGreaterThan(checkout_order);
		});
	});

	describe('build directory validation', () => {
		test('checks that build_dir exists after building', async () => {
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: true,
				build_dir: 'build',
				dry: true
			});

			await deploy_task.run(ctx);

			// Should check build_dir exists (after multiple other exists calls)
			const exists_calls = (exists as any).mock.calls;
			const build_check = exists_calls.find((call: any) => call[0] === 'build');
			expect(build_check).toBeDefined();
		});

		test('throws error when build_dir missing', async () => {
			const { exists } = await get_mock_project();

			// Everything exists except build_dir
			vi.mocked(exists).mockImplementation((path: any) => {
				const path_str = String(path);
				return !path_str.includes('build'); // build dir doesn't exist
			});

			const ctx = create_mock_deploy_task_context({
				build: true,
				build_dir: 'build',
				dry: true
			});

			// Should throw TaskError
			await expect(deploy_task.run(ctx)).rejects.toThrow('does not exist after building');
		});

		test('validates custom build_dir path', async () => {
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockImplementation((path: any) => {
				const path_str = String(path);
				return !path_str.includes('dist'); // custom dist dir doesn't exist
			});

			const ctx = create_mock_deploy_task_context({
				build: true,
				build_dir: 'dist',
				dry: true
			});

			// Should throw TaskError mentioning dist
			await expect(deploy_task.run(ctx)).rejects.toThrow(/does not exist after building.*dist/);
		});
	});

	describe('build task failure handling', () => {
		test('catches build error and throws TaskError', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ build: true });

			// Mock build task failure
			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('Build failed: TypeScript errors'));

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/Deploy safely canceled/);
		});

		test('logs build failure with git safety message', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ build: true, dry: false });

			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('Build failed'));

			await expect(deploy_task.run(ctx)).rejects.toThrow();

			expect(ctx.log.error).toHaveBeenCalledWith(
				expect.stringContaining('build failed'),
				'but',
				expect.stringContaining('nothing was pushed'),
				expect.anything()
			);
		});

		test('says the new remote branch was created when the build fails after creating it', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(git_ops.git_remote_branch_exists).mockResolvedValue(false);

			const ctx = create_mock_deploy_task_context({ build: true, dry: false });
			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('Build failed'));

			await expect(deploy_task.run(ctx)).rejects.toThrow();

			expect(git_ops.git_push_to_create).toHaveBeenCalled();
			expect(ctx.log.error).toHaveBeenCalledWith(
				expect.stringContaining('build failed'),
				'and',
				expect.stringContaining("the new remote branch 'deploy' has only its init commit"),
				'- nothing was deployed',
				expect.anything()
			);
		});

		test('does not modify git when build fails', async () => {
			const { git_command: spawn } = git_ops;
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ build: true, dry: false });

			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('Build failed'));

			await expect(deploy_task.run(ctx)).rejects.toThrow();

			// Should NOT have committed or pushed
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

		test('shows different message in dry mode when build fails', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ build: true, dry: true });

			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('Build failed'));

			await expect(deploy_task.run(ctx)).rejects.toThrow();

			// Should log dry deploy failed message
			expect(ctx.log.info).toHaveBeenCalledWith(expect.stringContaining('dry deploy failed'));
		});
	});

	describe('build task success scenarios', () => {
		test('continues to file copying after successful build', async () => {
			const { exists, copy } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			await mock_listings(['index.html', 'assets']);

			const ctx = create_mock_deploy_task_context({
				build: true,
				dry: true
			});

			ctx.invoke_task = vi.fn().mockResolvedValue(undefined);

			await deploy_task.run(ctx);

			// Should copy files after successful build
			expect(copy).toHaveBeenCalled();
		});

		test('creates build_dir before checking if it exists', async () => {
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: true,
				dry: true
			});

			ctx.invoke_task = vi.fn().mockResolvedValue(undefined);

			await deploy_task.run(ctx);

			// invoke_task should be called before exists checks build dir
			expect(ctx.invoke_task).toHaveBeenCalledWith('build', {
				sync: true,
				gen: true,
				install: true,
				force_build: false
			});
		});
	});

	describe('build-less deploy', () => {
		test('skips build and uses existing build_dir when no-build=true', async () => {
			const { exists, copy } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: false,
				dry: true
			});

			await deploy_task.run(ctx);

			// Should NOT invoke build
			expect(ctx.invoke_task).not.toHaveBeenCalled();

			// Should still check build_dir and copy files
			expect(copy).toHaveBeenCalled();
		});

		test('fails gracefully when build_dir missing and no-build=true', async () => {
			const { exists } = await get_mock_project();

			// build_dir doesn't exist, deploy_dir does
			vi.mocked(exists).mockImplementation((path: any) => {
				const path_str = String(path);
				return !path_str.includes('build'); // build_dir: false, deploy_dir: true
			});

			const ctx = create_mock_deploy_task_context({
				build: false,
				dry: true
			});

			// Should throw TaskError when build_dir doesn't exist
			await expect(deploy_task.run(ctx)).rejects.toThrow('does not exist after building');
		});
	});
});
