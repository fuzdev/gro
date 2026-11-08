import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';

import { task as deploy_task } from '#lib/deploy.task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
	mock_spawn_results,
	DEPLOY_DIR_ENTRIES,
	FIRST_COMMIT_HASH,
	get_clone_calls,
	expect_cloned,
	is_clone_call
} from './deploy_task_test_helpers.ts';
import { get_mock_project, to_mock_listing } from './fs_mocks.ts';

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

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

describe('deploy_task error handling', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('git operation failures', () => {
		test('propagates git_check_clean_workspace errors', async () => {
			const { git_check_clean_workspace } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));

			vi.mocked(git_check_clean_workspace).mockRejectedValue(new Error('Git not installed'));

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Git not installed');
		});

		test('propagates git_check_setting_pull_rebase errors', async () => {
			const { git_check_setting_pull_rebase } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));

			vi.mocked(git_check_setting_pull_rebase).mockRejectedValue(
				new Error('Failed to read git config')
			);

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Failed to read git config');
		});

		test('propagates git_local_branch_exists errors', async () => {
			const { git_local_branch_exists } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));

			vi.mocked(git_local_branch_exists).mockRejectedValue(new Error('Git command failed'));

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Git command failed');
		});

		test('propagates git_fetch errors', async () => {
			const { git_local_branch_exists, git_fetch } = vi.mocked(
				await import('@fuzdev/fuz_util/git.ts')
			);

			vi.mocked(git_local_branch_exists).mockResolvedValue(false); // trigger fetch
			vi.mocked(git_fetch).mockRejectedValue(new Error('Network error'));

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Network error');
		});

		test('propagates git_checkout errors', async () => {
			const { git_checkout } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));

			vi.mocked(git_checkout).mockRejectedValue(new Error('Branch not found'));

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Branch not found');
		});

		test('propagates git_pull errors', async () => {
			const { git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));

			vi.mocked(git_pull).mockRejectedValue(new Error('Pull failed: merge conflicts'));

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Pull failed: merge conflicts');
		});

		test('propagates git_remote_branch_exists errors', async () => {
			const { git_remote_branch_exists } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));

			vi.mocked(git_remote_branch_exists).mockRejectedValue(new Error('Remote not found'));

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow('Remote not found');
		});

		test('a failed clone or remote step fails the deploy at that step', async () => {
			const { git_remote_branch_exists } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { exists } = await get_mock_project();

			vi.mocked(git_remote_branch_exists).mockResolvedValue(false); // trigger new branch path
			vi.mocked(exists).mockResolvedValue(false);
			const ctx = create_mock_deploy_task_context({ dry: true });

			await mock_spawn_results(is_clone_call);
			await expect(deploy_task.run(ctx)).rejects.toThrow(
				'`git clone -b main --single-branch` failed'
			);
			expect(spawn.mock.calls.some(([, args]) => args?.includes('set-url'))).toBe(false);

			// the project's origin, which the clone's is set to, can't be read
			await mock_spawn_results(
				(cmd, args) => cmd === 'git' && args[0] === 'remote' && args[1] === 'get-url'
			);
			await expect(deploy_task.run(ctx)).rejects.toThrow('`git remote get-url origin` failed');

			await mock_spawn_results((cmd, args) => cmd === 'git' && args[1] === 'set-url');
			await expect(deploy_task.run(ctx)).rejects.toThrow('`git remote set-url origin` failed');

			await mock_spawn_results((cmd, args) => cmd === 'git' && args[0] === 'config');
			await expect(deploy_task.run(ctx)).rejects.toThrow(
				'`git config --local gro.deploy-clone true` failed'
			);
			expect(spawn.mock.calls.some(([, args]) => args?.[1] === '--orphan')).toBe(false);
		});

		test('propagates git_current_branch_name errors', async () => {
			const { git_current_branch_name } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true); // deploy dir exists
			vi.mocked(git_current_branch_name).mockRejectedValue(new Error('Not a git repository'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Not a git repository');
		});

		test('propagates git_delete_local_branch errors', async () => {
			const { git_local_branch_exists, git_delete_local_branch, git_remote_branch_exists } =
				vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { exists } = await get_mock_project();

			// Trigger local branch deletion (line 182 - deploy_dir doesn't exist, remote exists, local branch doesn't exist before fetch)
			vi.mocked(exists).mockImplementation((path: any) => {
				const path_str = String(path);
				if (path_str.includes('build')) return true; // build_dir exists
				return false; // deploy_dir doesn't exist
			});
			vi.mocked(git_remote_branch_exists).mockResolvedValue(true); // remote exists
			vi.mocked(git_local_branch_exists).mockResolvedValue(false); // local branch doesn't exist (before fetch), so we need to clean up after clone
			vi.mocked(git_delete_local_branch).mockRejectedValue(
				new Error('Cannot delete checked out branch')
			);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Cannot delete checked out branch');
		});

		test('a failed reset to the first commit fails the deploy without pushing', async () => {
			const { exists } = await get_mock_project();
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));

			vi.mocked(exists).mockResolvedValue(true);
			await mock_spawn_results(
				(cmd, args) => cmd === 'git' && args[0] === 'reset' && args[2] === FIRST_COMMIT_HASH
			);

			const ctx = create_mock_deploy_task_context({ reset: true, dry: false });

			await expect(deploy_task.run(ctx)).rejects.toThrow(
				`\`git reset --hard ${FIRST_COMMIT_HASH}\` failed resetting the target branch 'deploy'`
			);
			expect(spawn.mock.calls.some((call) => call[1]?.[0] === 'push')).toBe(false);
		});
	});

	describe('filesystem operation failures', () => {
		test('propagates exists errors', async () => {
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockImplementation(() => {
				throw new Error('Permission denied');
			});

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Permission denied');
		});

		test('propagates errors removing the deploy dir', async () => {
			const { git_remote_branch_exists } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { remove, exists } = await get_mock_project();

			vi.mocked(git_remote_branch_exists).mockResolvedValue(false); // trigger deletion
			vi.mocked(exists).mockResolvedValue(true); // deploy dir exists
			vi.mocked(remove).mockRejectedValue(new Error('Cannot remove directory'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Cannot remove directory');
		});

		test('propagates mkdir errors', async () => {
			const { git_remote_branch_exists } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { mkdir, exists } = await get_mock_project();

			vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(mkdir).mockRejectedValue(new Error('Disk full'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Disk full');
		});

		test('propagates errors emptying the deploy dir', async () => {
			const { exists, remove } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(remove).mockRejectedValue(new Error('Failed to empty directory'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Failed to empty directory');
		});

		test('propagates errors listing the build dir', async () => {
			const { exists, list } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(list).mockImplementation((path) => {
				if (path === 'build') throw new Error('Cannot read directory');
				return to_mock_listing(DEPLOY_DIR_ENTRIES);
			});

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Cannot read directory');
		});

		test('propagates copy errors', async () => {
			const { copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(copy).mockRejectedValue(new Error('Failed to copy files'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Failed to copy files');
		});
	});

	describe('spawn operation failures', () => {
		test('a failed git reset recreates the deploy dir', async () => {
			const { remove } = await get_mock_project();
			const { git_pull } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));
			const { exists } = await get_mock_project();

			// the deploy dir exists until it's removed
			let deploy_dir_exists = true;
			vi.mocked(exists).mockImplementation(async (path: any) =>
				path === '.gro/deploy' ? deploy_dir_exists : true
			);
			vi.mocked(remove).mockImplementation(async (path: any) => {
				if (path === '.gro/deploy') deploy_dir_exists = false;
			});
			await mock_spawn_results((cmd, args) => cmd === 'git' && args[0] === 'reset');

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			expect(remove).toHaveBeenCalledWith('.gro/deploy', { recursive: true });
			expect(git_pull).not.toHaveBeenCalledWith('origin', 'deploy', expect.anything());
			await expect_cloned('deploy');
			expect((await get_clone_calls())[0]!.order).toBeGreaterThan(
				remove.mock.invocationCallOrder[0]!
			);
			expect(ctx.log.warn).toHaveBeenCalledWith(
				expect.stringContaining('git reset --hard'),
				expect.anything(),
				expect.anything()
			);
		});

		test('propagates spawn errors for orphan branch creation', async () => {
			const { git_remote_branch_exists, git_push_to_create } = vi.mocked(
				await import('@fuzdev/fuz_util/git.ts')
			);
			const { exists } = await get_mock_project();

			vi.mocked(git_remote_branch_exists).mockResolvedValue(false); // trigger new branch
			vi.mocked(exists).mockResolvedValue(false);

			await mock_spawn_results(
				(cmd, args) => cmd === 'git' && args[0] === 'checkout' && args[1] === '--orphan'
			);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow(
				"`git checkout --orphan deploy` failed creating the target branch 'deploy'"
			);
			expect(git_push_to_create).not.toHaveBeenCalled();
		});

		test('propagates spawn errors for git add in commit phase', async () => {
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockImplementation((path: any) => {
				const path_str = String(path);
				return path_str.includes('build') || path_str.includes('.gro/deploy'); // both exist
			});

			// the commit phase's add, not the orphan setup's
			await mock_spawn_results(
				(cmd, args) => cmd === 'git' && args[0] === 'add' && args[1] === '.'
			);

			const ctx = create_mock_deploy_task_context({ dry: false });

			await expect(deploy_task.run(ctx)).rejects.toThrow('`git add` failed');
		});
	});

	describe('build task failures', () => {
		test('propagates build task errors', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ build: true });

			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('TypeScript compilation failed'));

			await expect(deploy_task.run(ctx)).rejects.toThrow(/Deploy safely canceled/);
		});

		test('does not make git changes when build fails', async () => {
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ build: true, dry: false });

			ctx.invoke_task = vi.fn().mockRejectedValue(new Error('Build error'));

			await expect(deploy_task.run(ctx)).rejects.toThrow();

			// Should NOT have tried to commit or push
			const git_commits = spawn.mock.calls.filter(
				(call) => call[0] === 'git' && call[1]?.[0] === 'commit'
			);
			expect(git_commits.length).toBe(0);
		});
	});

	describe('error message clarity', () => {
		test('safety check errors include helpful context', async () => {
			const ctx = create_mock_deploy_task_context({
				target: 'custom',
				force: false
			});

			await expect(deploy_task.run(ctx)).rejects.toThrow(/--force/);
		});

		test('dirty workspace error includes git status output', async () => {
			const { git_check_clean_workspace } = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));

			vi.mocked(git_check_clean_workspace).mockResolvedValue(
				'Modified files:\n  src/foo.ts\n  src/bar.ts'
			);

			const ctx = create_mock_deploy_task_context();

			try {
				await deploy_task.run(ctx);
				expect.fail('Should have thrown');
			} catch (err: any) {
				expect(err.message).toContain('uncommitted changes');
			}
		});

		test('git operation errors in bad state include actionable message', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			await mock_spawn_results((cmd, args) => cmd === 'git' && args[0] === 'push');

			const ctx = create_mock_deploy_task_context({ dry: false });

			try {
				await deploy_task.run(ctx);
				expect.fail('Should have thrown');
			} catch (err: any) {
				expect(err.message).toContain('bad state');
				expect(err.message).toContain('built but not pushed');
			}
		});
	});
});
