import {describe, test, expect, vi, beforeEach, afterEach} from 'vitest';

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

describe('deploy_task safety checks', () => {
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

	describe('custom target branch validation', () => {
		test('throws TaskError when target is custom and force=false', async () => {
			const ctx = create_mock_deploy_task_context({
				target: 'custom-branch',
				force: false,
			});

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/custom target branch/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/custom-branch/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/--force/);
		});

		test('allows custom target branch with force=true', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				target: 'custom-branch',
				force: true,
				dry: true, // Skip push
			});

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});

		test('allows default target branch without force flag', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				target: 'deploy', // default
				force: false,
				dry: true,
			});

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});

	describe('dangerous branch validation', () => {
		test('throws TaskError when target is main and dangerous=false', async () => {
			const ctx = create_mock_deploy_task_context({
				target: 'main',
				force: true, // Need force for custom target
				dangerous: false,
			});

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/appears very dangerous/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/main/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/--dangerous/);
		});

		test('throws TaskError when target is master and dangerous=false', async () => {
			const ctx = create_mock_deploy_task_context({
				target: 'master',
				force: true, // Need force for custom target
				dangerous: false,
			});

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/appears very dangerous/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/master/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/--dangerous/);
		});

		test('allows dangerous branch with dangerous=true', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				target: 'main',
				force: true, // Also need force for custom target
				dangerous: true,
				dry: true,
			});

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});

		test('requires both force and dangerous for dangerous custom branches', async () => {
			// Missing force
			const ctx1 = create_mock_deploy_task_context({
				target: 'main',
				force: false,
				dangerous: true,
			});

			await expect(deploy_task.run(ctx1)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx1)).rejects.toThrow(/--force/);

			// Missing dangerous
			const ctx2 = create_mock_deploy_task_context({
				target: 'main',
				force: true,
				dangerous: false,
			});

			await expect(deploy_task.run(ctx2)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx2)).rejects.toThrow(/--dangerous/);
		});
	});

	describe('workspace cleanliness check', () => {
		test('throws TaskError when workspace has uncommitted changes', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			vi.mocked(git_check_clean_workspace).mockResolvedValue(
				'Modified files:\n  src/lib/foo.ts\n  src/lib/bar.ts',
			);

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/uncommitted changes/);
		});

		test('throws TaskError when workspace has untracked files', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			vi.mocked(git_check_clean_workspace).mockResolvedValue('Untracked files:\n  temp.ts');

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/uncommitted changes/);
		});

		test('allows deploy when workspace is clean', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: true});

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});

	describe('git pull.rebase setting check', () => {
		test('throws TaskError when pull.rebase is not configured', async () => {
			const {git_check_setting_pull_rebase} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			vi.mocked(git_check_setting_pull_rebase).mockResolvedValue(false);

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/pull\.rebase/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/git config --global pull\.rebase true/);
		});

		test('allows deploy when pull.rebase is configured', async () => {
			const {git_check_setting_pull_rebase} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			vi.mocked(git_check_setting_pull_rebase).mockResolvedValue(true);
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: true});

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});

	describe('post-pull workspace check', () => {
		test('throws TaskError when workspace is dirty after pull', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			// Clean initially, dirty after pull
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async () => {
				call_count++;
				// First call: clean
				// Second call (after pull): dirty (rebase conflict!)
				return call_count === 1 ? null : 'Rebasing... conflicts detected';
			});

			const ctx = create_mock_deploy_task_context();

			let error: TaskError | undefined;
			try {
				await deploy_task.run(ctx);
			} catch (e) {
				error = e as TaskError;
			}
			expect(error).toBeInstanceOf(TaskError);
			expect(error!.message).toContain('out of sync with the remote');
			expect(error!.message).toContain('git rebase --abort');
		});

		test('succeeds when workspace stays clean after pull', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

			// Clean throughout
			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: true});

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});

	describe('combined safety checks', () => {
		test('checks are performed in correct order (custom target checked first)', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			// Set dirty workspace
			vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified: src/foo.ts');

			const ctx = create_mock_deploy_task_context({
				target: 'custom',
				force: false,
			});

			// Should fail on custom target check, not workspace check
			await expect(deploy_task.run(ctx)).rejects.toThrow(/custom target branch/);
		});

		test('checks dangerous branch before workspace check', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			// Set dirty workspace
			vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified: src/foo.ts');

			const ctx = create_mock_deploy_task_context({
				target: 'main',
				force: true,
				dangerous: false,
			});

			// Should fail on dangerous branch check, not workspace check
			await expect(deploy_task.run(ctx)).rejects.toThrow(/appears very dangerous/);
		});

		test('all safety checks pass in normal scenario', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				target: 'deploy', // default, safe
				force: false, // not needed for default
				dangerous: false, // not needed for safe branch
				dry: true,
			});

			// All checks should pass
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});
});
