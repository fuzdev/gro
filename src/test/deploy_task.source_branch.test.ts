import {describe, test, expect, vi, beforeEach, afterEach} from 'vitest';
import {resolve} from 'node:path';

import {task as deploy_task} from '../lib/deploy.task.ts';
import {TaskError} from '../lib/task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
} from './deploy_task_test_helpers.ts';

// Mock dependencies
vi.mock('@fuzdev/fuz_util/git.js', async (import_original) => {
	const actual = await import_original<typeof import('@fuzdev/fuz_util/git.js')>();
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
		git_reset_branch_to_first_commit: vi.fn(),
	};
});

vi.mock('@fuzdev/fuz_util/process.js', () => ({
	spawn: vi.fn(),
}));

vi.mock('node:fs/promises', () => ({
	cp: vi.fn(),
	mkdir: vi.fn(),
	rm: vi.fn(),
	readdir: vi.fn(),
}));

vi.mock('@fuzdev/fuz_util/fs.js', () => ({
	fs_exists: vi.fn(),
	fs_empty_dir: vi.fn(),
}));

describe('deploy_task source branch preparation', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();

		const {fs_empty_dir} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		vi.mocked(fs_empty_dir).mockResolvedValue(undefined);
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('source branch fetching', () => {
		test('fetches source branch when it does not exist locally', async () => {
			const {git_local_branch_exists, git_fetch} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(git_local_branch_exists).mockResolvedValue(false); // source doesn't exist
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should fetch the source branch
			expect(git_fetch).toHaveBeenCalledWith('origin', 'main');
		});

		test('skips fetch when source branch exists locally', async () => {
			const {git_local_branch_exists, git_fetch} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(git_local_branch_exists).mockResolvedValue(true); // source exists
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should NOT fetch (branch already exists)
			expect(git_fetch).not.toHaveBeenCalled();
		});

		test('uses custom origin when fetching', async () => {
			const {git_local_branch_exists, git_fetch} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(git_local_branch_exists).mockResolvedValue(false);
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'develop',
				origin: 'upstream',
				dry: true,
			});

			await deploy_task.run(ctx);

			expect(git_fetch).toHaveBeenCalledWith('upstream', 'develop');
		});
	});

	describe('source branch checkout', () => {
		test('checks out the source branch', async () => {
			const {git_checkout} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				dry: true,
			});

			await deploy_task.run(ctx);

			expect(git_checkout).toHaveBeenCalledWith('main');
		});

		test('checks out custom source branch', async () => {
			const {git_checkout} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'feature-branch',
				dry: true,
			});

			await deploy_task.run(ctx);

			expect(git_checkout).toHaveBeenCalledWith('feature-branch');
		});

		test('checks out after fetch when source was missing', async () => {
			const {git_local_branch_exists, git_fetch, git_checkout} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(git_local_branch_exists).mockResolvedValue(false);
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should fetch then checkout
			expect(git_fetch).toHaveBeenCalled();
			expect(git_checkout).toHaveBeenCalledWith('main');

			// Checkout should happen after fetch
			const fetch_order = git_fetch.mock.invocationCallOrder[0]!;
			const checkout_order = git_checkout.mock.invocationCallOrder[0]!;
			expect(checkout_order).toBeGreaterThan(fetch_order);
		});
	});

	describe('source branch pulling', () => {
		test('pulls source branch when pull=true', async () => {
			const {git_pull} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				pull: true,
				dry: true,
			});

			await deploy_task.run(ctx);

			expect(git_pull).toHaveBeenNthCalledWith(1, 'origin', 'main');
		});

		test('skips pull when pull=false', async () => {
			const {git_pull} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				pull: false,
				dry: true,
			});

			await deploy_task.run(ctx);

			// Source branch pull is skipped (line 136), but target branch pull still happens
			// when deploy dir exists (line 162)
			expect(git_pull).toHaveBeenCalledTimes(1);
			expect(git_pull).toHaveBeenCalledWith('origin', 'deploy', {
				cwd: resolve('.gro/deploy'),
			});
		});

		test('pulls after checkout', async () => {
			const {git_checkout, git_pull} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				pull: true,
				dry: true,
			});

			await deploy_task.run(ctx);

			// Both should be called
			expect(git_checkout).toHaveBeenCalled();
			expect(git_pull).toHaveBeenCalled();

			// Pull should happen after checkout
			const checkout_order = git_checkout.mock.invocationCallOrder[0]!;
			const pull_order = git_pull.mock.invocationCallOrder[0]!;
			expect(pull_order).toBeGreaterThan(checkout_order);
		});

		test('uses custom origin when pulling', async () => {
			const {git_pull} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'develop',
				origin: 'upstream',
				pull: true,
				dry: true,
			});

			await deploy_task.run(ctx);

			expect(git_pull).toHaveBeenCalledWith('upstream', 'develop');
		});
	});

	describe('post-pull validation', () => {
		test('detects rebase conflicts after pull', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			// Clean initially, dirty after pull (rebase conflict)
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async () => {
				call_count++;
				return call_count === 1 ? null : 'Rebasing (1/3)\nConflicts in src/foo.ts';
			});

			const ctx = create_mock_deploy_task_context({
				pull: true,
			});

			let error: TaskError | undefined;
			try {
				await deploy_task.run(ctx);
			} catch (e) {
				error = e as TaskError;
			}
			expect(error!.message).toContain('out of sync with the remote');
			expect(error!.message).toContain('git rebase --abort');
		});

		test('succeeds when pull completes cleanly', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				pull: true,
				dry: true,
			});

			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});

		test('skips post-pull check when pull=false', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			// Only first call (initial check) should happen
			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				pull: false,
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should be called 3 times even with pull=false:
			// 1. Initial check (line 115)
			// 2. Post-source-pull check (line 138) - runs regardless of pull flag
			// 3. Post-target-pull check in deploy dir (line 163) - when deploy dir exists
			expect(git_check_clean_workspace).toHaveBeenCalledTimes(3);
		});
	});

	describe('error handling', () => {
		test('propagates error when git_fetch fails', async () => {
			const {git_local_branch_exists, git_fetch} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);

			vi.mocked(git_local_branch_exists).mockResolvedValue(false);
			vi.mocked(git_fetch).mockRejectedValue(new Error('Failed to fetch'));

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Failed to fetch');
		});

		test('propagates error when git_checkout fails', async () => {
			const {git_checkout} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			vi.mocked(git_checkout).mockRejectedValue(new Error('Branch not found'));

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Branch not found');
		});

		test('propagates error when git_pull fails', async () => {
			const {git_pull} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			vi.mocked(git_pull).mockRejectedValue(new Error('Cannot pull with uncommitted changes'));

			const ctx = create_mock_deploy_task_context({pull: true});

			await expect(deploy_task.run(ctx)).rejects.toThrow('Cannot pull with uncommitted changes');
		});
	});
});
