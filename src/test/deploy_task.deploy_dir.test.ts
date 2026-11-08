import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import { join, resolve } from 'node:path';

import { task as deploy_task } from '#lib/deploy.task.ts';
import { GIT_DIRNAME } from '#lib/constants.ts';

import {
	create_mock_deploy_task_context,
	DEPLOY_DIR_ENTRIES,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock
} from './deploy_task_test_helpers.ts';
import { get_mock_project } from './fs_mocks.ts';

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
		git_current_branch_first_commit_hash: vi.fn()
	};
});

vi.mock('@fuzdev/fuz_util/process.js', () => ({
	spawn: vi.fn(),
	spawn_out: vi.fn()
}));

vi.mock('node:fs/promises', async (import_original) => {
	// only the real `realpath` and `readlink`, which `fs_open_path` resolves paths with
	const { realpath, readlink } = await import_original<typeof import('node:fs/promises')>();
	return {
		realpath,
		readlink,
		cp: vi.fn(),
		mkdir: vi.fn(),
		rm: vi.fn(),
		readdir: vi.fn()
	};
});

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

// spied, to see when the deploy dir is opened for emptying
vi.mock('#lib/internal/fs_path.ts', async (import_original) => {
	const actual = await import_original<typeof import('#lib/internal/fs_path.ts')>();
	return { ...actual, fs_open_path: vi.fn(actual.fs_open_path) };
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
			const { git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
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
			const { git_remote_branch_exists, git_clone_locally } = vi.mocked(
				await import('@fuzdev/fuz_util/git.ts')
			);
			const { rm, mkdir } = vi.mocked(await import('node:fs/promises'));
			const { fs_open_path } = vi.mocked(await import('#lib/internal/fs_path.ts'));
			const { exists, list } = await get_mock_project();

			// no remote target, so the existing deploy dir is removed, recreated, and cloned into
			vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			const deploy_dir = resolve('.gro/deploy');
			const open_orders = fs_open_path.mock.calls
				.map(([path], i) => (path === deploy_dir ? fs_open_path.mock.invocationCallOrder[i]! : 0))
				.filter((order) => order > 0);
			const last_open_order = Math.max(...open_orders);
			const list_order = list.mock.invocationCallOrder[0]!;

			expect(rm).toHaveBeenCalledWith(deploy_dir, { recursive: true });
			expect(last_open_order).toBeGreaterThan(rm.mock.invocationCallOrder[0]!);
			expect(last_open_order).toBeGreaterThan(mkdir.mock.invocationCallOrder[0]!);
			expect(last_open_order).toBeGreaterThan(git_clone_locally.mock.invocationCallOrder[0]!);
			expect(last_open_order).toBeLessThan(list_order);
			// every open but the one for emptying is an existence check before the clone
			for (const order of open_orders) {
				if (order !== last_open_order) {
					expect(order).toBeLessThan(git_clone_locally.mock.invocationCallOrder[0]!);
				}
			}
		});

		test('empty happens before copying files', async () => {
			const { cp } = await import('node:fs/promises');
			const { exists, remove } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// emptying should happen before cp
			const empty_order = remove.mock.invocationCallOrder.at(-1);
			const cp_order = (cp as any).mock.invocationCallOrder[0];

			expect(empty_order).toBeLessThan(cp_order);
		});
	});

	describe('file copying', () => {
		test('copies all files from build_dir to deploy_dir', async () => {
			const { readdir } = vi.mocked(await import('node:fs/promises'));
			const { cp } = await import('node:fs/promises');
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(readdir).mockResolvedValue(['index.html', 'assets', 'favicon.ico'] as any);

			const ctx = create_mock_deploy_task_context({
				build_dir: 'build',
				deploy_dir: '.gro/deploy',
				dry: true
			});

			await deploy_task.run(ctx);

			// Should copy each file/directory
			expect(cp).toHaveBeenCalledWith(
				join('build', 'index.html'),
				join(resolve('.gro/deploy'), 'index.html'),
				{ recursive: true }
			);
			expect(cp).toHaveBeenCalledWith(
				join('build', 'assets'),
				join(resolve('.gro/deploy'), 'assets'),
				{
					recursive: true
				}
			);
			expect(cp).toHaveBeenCalledWith(
				join('build', 'favicon.ico'),
				join(resolve('.gro/deploy'), 'favicon.ico'),
				{ recursive: true }
			);
		});

		test('uses custom build_dir and deploy_dir paths', async () => {
			const { readdir } = vi.mocked(await import('node:fs/promises'));
			const { cp } = await import('node:fs/promises');
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(readdir).mockResolvedValue(['app.js'] as any);

			const ctx = create_mock_deploy_task_context({
				build_dir: 'dist',
				deploy_dir: 'custom/deploy',
				dry: true
			});

			await deploy_task.run(ctx);

			expect(cp).toHaveBeenCalledWith(
				join('dist', 'app.js'),
				join(resolve('custom/deploy'), 'app.js'),
				{
					recursive: true
				}
			);
		});

		test('copies directories recursively', async () => {
			const { readdir } = vi.mocked(await import('node:fs/promises'));
			const { cp } = await import('node:fs/promises');
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(readdir).mockResolvedValue(['assets'] as any);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should copy with recursive flag
			expect(cp).toHaveBeenCalledWith(expect.anything(), expect.anything(), { recursive: true });
		});

		test('copies all files in parallel', async () => {
			const { readdir } = vi.mocked(await import('node:fs/promises'));
			const { cp } = await import('node:fs/promises');
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(readdir).mockResolvedValue(['file1.html', 'file2.html', 'file3.html'] as any);

			// Track when cp is called
			let concurrent_calls = 0;
			let max_concurrent = 0;
			vi.mocked(cp).mockImplementation(async () => {
				concurrent_calls++;
				max_concurrent = Math.max(max_concurrent, concurrent_calls);
				await new Promise((resolve) => setTimeout(resolve, 10));
				concurrent_calls--;
			});

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// All 3 files should have been copied with some concurrency
			expect(cp).toHaveBeenCalledTimes(3);
			expect(max_concurrent).toBeGreaterThan(1);
		});

		test('handles empty build directory', async () => {
			const { readdir } = vi.mocked(await import('node:fs/promises'));
			const { cp } = await import('node:fs/promises');
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(readdir).mockResolvedValue([]); // empty

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should not copy anything
			expect(cp).not.toHaveBeenCalled();
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

		test('propagates error when readdir fails', async () => {
			const { readdir } = vi.mocked(await import('node:fs/promises'));
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(readdir).mockImplementation(() => {
				throw new Error('Cannot read directory');
			});

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Cannot read directory');
		});

		test('propagates error when cp fails', async () => {
			const { readdir } = vi.mocked(await import('node:fs/promises'));
			const { cp } = await import('node:fs/promises');
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(readdir).mockResolvedValue(['index.html'] as any);
			vi.mocked(cp).mockRejectedValue(new Error('Disk full'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Disk full');
		});

		test('fails if any parallel copy fails', async () => {
			const { readdir } = vi.mocked(await import('node:fs/promises'));
			const { cp } = await import('node:fs/promises');
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(readdir).mockResolvedValue(['file1.html', 'file2.html', 'file3.html'] as any);

			// Second copy fails
			let call_count = 0;
			vi.mocked(cp).mockImplementation(async () => {
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
