import {describe, test, expect, vi, beforeEach, afterEach} from 'vitest';
import {resolve} from 'node:path';

import {task as deploy_task} from '../lib/deploy.task.ts';

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

describe('deploy_task target branch sync (remote exists)', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();

		const {fs_empty_dir} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		vi.mocked(fs_empty_dir).mockResolvedValue(undefined);

		// Default: remote target EXISTS
		const {git_remote_branch_exists} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		vi.mocked(git_remote_branch_exists).mockResolvedValue(true);
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('deploy dir exists with correct branch', () => {
		test('resets and pulls when deploy dir has correct branch', async () => {
			const {git_current_branch_name, git_pull} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true); // deploy dir exists
			vi.mocked(git_current_branch_name).mockResolvedValue('deploy'); // correct branch

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should reset --hard
			expect(spawn).toHaveBeenCalledWith('git', ['reset', '--hard'], {
				cwd: resolve('.gro/deploy'),
			});
			// Should pull
			expect(git_pull).toHaveBeenCalledWith('origin', 'deploy', {
				cwd: resolve('.gro/deploy'),
			});
		});

		test('uses custom target branch name', async () => {
			const {git_current_branch_name} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);
			vi.mocked(git_current_branch_name).mockResolvedValue('custom-deploy');

			const ctx = create_mock_deploy_task_context({
				target: 'custom-deploy',
				force: true,
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should check branch name
			expect(git_current_branch_name).toHaveBeenCalledWith({
				cwd: resolve('.gro/deploy'),
			});
		});

		test('reset happens before pull', async () => {
			const {git_pull} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Find the reset and pull calls
			const reset_call_order = spawn.mock.invocationCallOrder.find((_, idx) => {
				const call = spawn.mock.calls[idx]!;
				return call[0] === 'git' && call[1]?.[0] === 'reset';
			});
			// Check second git_pull call (target branch in deploy dir), not first (source branch)
			const pull_call_order = git_pull.mock.invocationCallOrder[1]!;

			expect(reset_call_order).toBeLessThan(pull_call_order);
		});
	});

	describe('deploy dir exists with wrong branch', () => {
		test('deletes deploy dir when branch name differs', async () => {
			const {git_current_branch_name} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {rm} = await import('node:fs/promises');
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true); // deploy dir exists
			vi.mocked(git_current_branch_name).mockResolvedValue('wrong-branch'); // wrong branch!

			const ctx = create_mock_deploy_task_context({
				target: 'deploy',
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should delete deploy dir
			expect(rm).toHaveBeenCalledWith(resolve('.gro/deploy'), {recursive: true});
		});

		test('reinitializes deploy dir after deletion', async () => {
			const {git_current_branch_name, git_fetch, git_clone_locally} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			// Deploy dir exists first check, doesn't exist after deletion
			let exists_call_count = 0;
			vi.mocked(fs_exists).mockImplementation((path: any) => {
				const path_str = String(path);
				if (path_str.includes('build')) return true; // build_dir always exists
				exists_call_count++;
				return exists_call_count === 1; // Only first deploy_dir check returns true
			});
			vi.mocked(git_current_branch_name).mockResolvedValue('wrong-branch');

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should fetch and clone
			expect(git_fetch).toHaveBeenCalledWith('origin', '+deploy:deploy');
			expect(git_clone_locally).toHaveBeenCalledWith(
				'origin',
				'deploy',
				process.cwd(),
				resolve('.gro/deploy'),
			);
		});
	});

	describe('deploy dir exists but is out of sync', () => {
		test('deletes deploy dir when pull results in dirty workspace', async () => {
			const {git_check_clean_workspace, git_current_branch_name} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {rm} = await import('node:fs/promises');
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);
			vi.mocked(git_current_branch_name).mockResolvedValue('deploy');

			// Clean for initial checks, then dirty after pull in deploy dir
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async (options) => {
				call_count++;
				// Only the check in deploy dir after pull should be dirty
				if (String(options?.cwd).includes('.gro/deploy') && call_count > 2) {
					return 'Unmerged paths:\n  index.html';
				}
				return null;
			});

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should delete deploy dir due to sync failure
			expect(rm).toHaveBeenCalledWith(resolve('.gro/deploy'), {recursive: true});
		});

		test('reinitializes after sync failure', async () => {
			const {git_check_clean_workspace, git_fetch, git_clone_locally} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			// Deploy dir exists first check, doesn't exist after deletion
			let exists_call_count = 0;
			vi.mocked(fs_exists).mockImplementation((path: any) => {
				const path_str = String(path);
				if (path_str.includes('build')) return true; // build_dir always exists
				exists_call_count++;
				return exists_call_count === 1;
			});

			// Dirty after pull
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async (options) => {
				call_count++;
				if (String(options?.cwd).includes('.gro/deploy') && call_count > 2) {
					return 'Conflicts!';
				}
				return null;
			});

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should reinitialize
			expect(git_fetch).toHaveBeenCalled();
			expect(git_clone_locally).toHaveBeenCalled();
		});
	});

	describe('deploy dir does not exist', () => {
		test('fetches and clones when deploy dir missing', async () => {
			const {git_fetch, git_clone_locally, git_local_branch_exists} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			// deploy dir doesn't exist, but build_dir does
			vi.mocked(fs_exists).mockImplementation((path: any) => String(path).includes('build'));
			vi.mocked(git_local_branch_exists).mockResolvedValue(false); // target doesn't exist locally

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should fetch with + for non-fastforward
			expect(git_fetch).toHaveBeenCalledWith('origin', '+deploy:deploy');
			// Should clone
			expect(git_clone_locally).toHaveBeenCalledWith(
				'origin',
				'deploy',
				process.cwd(),
				resolve('.gro/deploy'),
			);
		});

		test('cleans up local target branch if created during fetch', async () => {
			const {git_local_branch_exists, git_delete_local_branch} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			// deploy dir doesn't exist, but build_dir does
			vi.mocked(fs_exists).mockImplementation((path: any) => String(path).includes('build'));
			vi.mocked(git_local_branch_exists).mockResolvedValue(false); // didn't exist before

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should delete target branch from cwd (cleanup after fetch created it)
			expect(git_delete_local_branch).toHaveBeenCalledWith('deploy');
		});

		test('skips cleanup if local target branch existed before', async () => {
			const {git_local_branch_exists, git_delete_local_branch} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			// deploy dir doesn't exist, but build_dir does
			vi.mocked(fs_exists).mockImplementation((path: any) => String(path).includes('build'));

			// Target branch DID exist locally before fetch
			vi.mocked(git_local_branch_exists).mockImplementation(async (branch) => {
				return branch === 'deploy'; // target branch already existed
			});

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should NOT delete target branch (it existed before)
			expect(git_delete_local_branch).not.toHaveBeenCalledWith('deploy');
		});
	});

	describe('reset flag handling', () => {
		test('resets to first commit when reset=true', async () => {
			const {git_reset_branch_to_first_commit} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				reset: true,
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should reset to first commit
			expect(git_reset_branch_to_first_commit).toHaveBeenCalledWith('origin', 'deploy', {
				cwd: resolve('.gro/deploy'),
			});
		});

		test('skips reset when reset=false', async () => {
			const {git_reset_branch_to_first_commit} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				reset: false,
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should NOT reset
			expect(git_reset_branch_to_first_commit).not.toHaveBeenCalled();
		});

		test('reset happens after sync', async () => {
			const {git_reset_branch_to_first_commit, git_pull} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				reset: true,
				dry: true,
			});

			await deploy_task.run(ctx);

			// Reset should happen after pull
			const pull_order = git_pull.mock.invocationCallOrder[0]!;
			const reset_order = git_reset_branch_to_first_commit.mock.invocationCallOrder[0]!;

			expect(reset_order).toBeGreaterThan(pull_order);
		});
	});

	describe('error handling', () => {
		test('propagates error when git_current_branch_name fails', async () => {
			const {git_current_branch_name} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);
			vi.mocked(git_current_branch_name).mockRejectedValue(new Error('Git command failed'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await expect(deploy_task.run(ctx)).rejects.toThrow('Git command failed');
		});

		test('propagates error when reset --hard fails', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);

			// Make reset fail
			vi.mocked(spawn).mockImplementation(async (cmd, args) => {
				if (cmd === 'git' && args?.[0] === 'reset') {
					throw new Error('Reset failed');
				}
				return {code: 0} as any;
			});

			const ctx = create_mock_deploy_task_context({dry: true});

			await expect(deploy_task.run(ctx)).rejects.toThrow('Reset failed');
		});

		test('propagates error when git_pull fails', async () => {
			const {git_pull} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(true);
			vi.mocked(git_pull).mockRejectedValue(new Error('Pull failed'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await expect(deploy_task.run(ctx)).rejects.toThrow('Pull failed');
		});

		test('propagates error when git_fetch fails during reinitialization', async () => {
			const {git_fetch} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(fs_exists).mockResolvedValue(false); // deploy dir missing
			vi.mocked(git_fetch).mockRejectedValue(new Error('Fetch failed'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await expect(deploy_task.run(ctx)).rejects.toThrow('Fetch failed');
		});
	});
});
