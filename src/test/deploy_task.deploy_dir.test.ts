import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import { resolve } from 'node:path';

import { task as deploy_task } from '#lib/deploy.task.ts';
import { GIT_DIRNAME } from '#lib/constants.ts';
import type { FsPaths } from '#lib/fs_paths.ts';

import {
	create_mock_deploy_task_context,
	DEPLOY_DIR_ENTRIES,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
	mock_listings,
	get_clone_calls
} from './deploy_task_test_helpers.ts';
import { get_mock_project, to_mock_listing } from './fs_mocks.ts';
import { git_ops } from './git_mocks.ts';

// Mock dependencies
vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

// the task's `open_path` spied, to see when the deploy dir is opened for emptying
vi.mock('#lib/fs_paths.ts', async (import_original) => {
	const actual = await import_original<typeof import('#lib/fs_paths.ts')>();
	return {
		...actual,
		create_fs_paths: vi.fn((fs: Parameters<typeof actual.create_fs_paths>[0]) => {
			const fs_paths = actual.create_fs_paths(fs);
			return { ...fs_paths, open_path: vi.fn(fs_paths.open_path) };
		})
	};
});

describe('deploy_task deploy directory operations', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('deploy directory cleanup', () => {
		test('empties deploy_dir before copying build output', async () => {
			const { exists, list, remove } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				deploy_dir: '.gro/deploy',
				dry: true
			});

			await deploy_task.run(ctx);

			// Should empty the deploy dir
			expect(list).toHaveBeenCalledWith('.gro/deploy');
			expect(remove).toHaveBeenCalledWith('.gro/deploy/index.html', { recursive: true });
			expect(remove).toHaveBeenCalledWith('.gro/deploy/assets', { recursive: true });
		});

		test('preserves .git directory when emptying deploy_dir', async () => {
			const { exists, remove } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// removes every entry but the git directory
			expect(DEPLOY_DIR_ENTRIES).toContain(GIT_DIRNAME);
			expect(remove.mock.calls.map(([path]) => path).sort()).toEqual(
				DEPLOY_DIR_ENTRIES.filter((name) => name !== GIT_DIRNAME)
					.map((name) => '.gro/deploy/' + name)
					.sort()
			);
		});

		test('empty happens after target branch preparation', async () => {
			const { git_pull } = git_ops;
			const { exists, remove } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// emptying should happen after git operations
			const pull_order = git_pull.mock.invocationCallOrder[0]!;
			const empty_order = remove.mock.invocationCallOrder[0]!;

			expect(empty_order).toBeGreaterThan(pull_order);
		});

		test('the deploy dir is opened for emptying after it is recreated', async () => {
			const { git_remote_branch_exists } = git_ops;
			const { remove, mkdir } = await get_mock_project();
			const { create_fs_paths } = vi.mocked(await import('#lib/fs_paths.ts'));
			const { exists, list } = await get_mock_project();

			// no remote target, so the existing deploy dir is removed, recreated, and cloned into
			vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			const fs_open_path = vi.mocked<FsPaths['open_path']>(
				create_fs_paths.mock.results.at(-1)!.value.open_path
			);
			const deploy_dir = resolve('.gro/deploy');
			const open_orders = fs_open_path.mock.calls
				.map(([path], i) => (path === deploy_dir ? fs_open_path.mock.invocationCallOrder[i]! : 0))
				.filter((order) => order > 0);
			const last_open_order = Math.max(...open_orders);
			const list_order = list.mock.invocationCallOrder[0]!;

			expect(remove).toHaveBeenCalledWith('.gro/deploy', { recursive: true });
			expect(last_open_order).toBeGreaterThan(remove.mock.invocationCallOrder[0]!);
			expect(last_open_order).toBeGreaterThan(mkdir.mock.invocationCallOrder[0]!);
			const clone_order = (await get_clone_calls())[0]!.order;
			expect(last_open_order).toBeGreaterThan(clone_order);
			expect(last_open_order).toBeLessThan(list_order);
		});

		test('empty happens before copying files', async () => {
			const { copy, exists, remove } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// emptying should happen before copying
			const empty_order = remove.mock.invocationCallOrder.at(-1)!;
			const copy_order = copy.mock.invocationCallOrder[0]!;

			expect(empty_order).toBeLessThan(copy_order);
		});
	});

	describe('file copying', () => {
		test('copies all files from build_dir to deploy_dir', async () => {
			const { copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			await mock_listings(['index.html', 'assets', 'favicon.ico']);

			const ctx = create_mock_deploy_task_context({
				build_dir: 'build',
				deploy_dir: '.gro/deploy',
				dry: true
			});

			await deploy_task.run(ctx);

			// Should copy each file/directory
			expect(copy).toHaveBeenCalledWith('build/index.html', '.gro/deploy/index.html', {
				recursive: true
			});
			expect(copy).toHaveBeenCalledWith('build/assets', '.gro/deploy/assets', { recursive: true });
			expect(copy).toHaveBeenCalledWith('build/favicon.ico', '.gro/deploy/favicon.ico', {
				recursive: true
			});
		});

		test('uses custom build_dir and deploy_dir paths', async () => {
			const { copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			await mock_listings(['app.js'], 'dist');

			const ctx = create_mock_deploy_task_context({
				build_dir: 'dist',
				deploy_dir: '.gro/custom/deploy',
				dry: true
			});

			await deploy_task.run(ctx);

			expect(copy).toHaveBeenCalledWith('dist/app.js', '.gro/custom/deploy/app.js', {
				recursive: true
			});
		});

		test('copies directories recursively', async () => {
			const { copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			await mock_listings(['assets']);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should copy with recursive flag
			expect(copy).toHaveBeenCalledWith(expect.anything(), expect.anything(), { recursive: true });
		});

		test('copies all files in parallel', async () => {
			const { copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			await mock_listings(['file1.html', 'file2.html', 'file3.html']);

			// Track when copy is called
			let concurrent_calls = 0;
			let max_concurrent = 0;
			vi.mocked(copy).mockImplementation(async () => {
				concurrent_calls++;
				max_concurrent = Math.max(max_concurrent, concurrent_calls);
				await new Promise((resolve) => setTimeout(resolve, 10));
				concurrent_calls--;
			});

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// All 3 files should have been copied with some concurrency
			expect(copy).toHaveBeenCalledTimes(3);
			expect(max_concurrent).toBeGreaterThan(1);
		});

		test('handles empty build directory', async () => {
			const { copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			await mock_listings([]); // empty

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should not copy anything
			expect(copy).not.toHaveBeenCalled();
		});
	});

	describe('error handling', () => {
		test('propagates error when emptying fails', async () => {
			const { exists, remove } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(remove).mockRejectedValue(new Error('Permission denied'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Permission denied');
		});

		test('propagates error when listing the build dir fails', async () => {
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			const { list } = await get_mock_project();
			vi.mocked(list).mockImplementation((path) => {
				if (path === 'build') throw new Error('Cannot read directory');
				return to_mock_listing(DEPLOY_DIR_ENTRIES);
			});

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Cannot read directory');
		});

		test('propagates error when copying fails', async () => {
			const { copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			await mock_listings(['index.html']);
			vi.mocked(copy).mockRejectedValue(new Error('Disk full'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Disk full');
		});

		test('fails if any parallel copy fails', async () => {
			const { copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			await mock_listings(['file1.html', 'file2.html', 'file3.html']);

			// Second copy fails
			let call_count = 0;
			vi.mocked(copy).mockImplementation(async () => {
				call_count++;
				if (call_count === 2) {
					throw new Error('Copy failed');
				}
			});

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Copy failed');
		});
	});
});
