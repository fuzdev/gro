import {describe, test, expect, vi, beforeEach, afterEach} from 'vitest';

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

describe('deploy_task target branch creation (remote does not exist)', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();

		const {fs_empty_dir} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		vi.mocked(fs_empty_dir).mockResolvedValue(undefined);

		// Default: remote target does NOT exist
		const {git_remote_branch_exists} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('deploy directory cleanup', () => {
		test('deletes existing deploy directory', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			const {rm, mkdir} = await import('node:fs/promises');

			vi.mocked(fs_exists).mockResolvedValue(true); // deploy dir exists

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should delete deploy dir
			expect(rm).toHaveBeenCalledWith(expect.stringContaining('.gro/deploy'), {recursive: true});
			// Should recreate it
			expect(mkdir).toHaveBeenCalledWith(expect.stringContaining('.gro/deploy'), {recursive: true});
		});

		test('skips deletion when deploy directory does not exist', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			const {rm, mkdir} = await import('node:fs/promises');

			// deploy dir doesn't exist, but build_dir does
			vi.mocked(fs_exists).mockImplementation((path: any) => String(path).includes('build'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should NOT delete (doesn't exist)
			expect(rm).not.toHaveBeenCalled();
			// Should NOT recreate (only does that if it existed before)
			expect(mkdir).not.toHaveBeenCalled();
		});

		test('uses custom deploy_dir path', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			const {rm, mkdir} = await import('node:fs/promises');

			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				deploy_dir: 'custom/path',
				dry: true,
			});

			await deploy_task.run(ctx);

			expect(rm).toHaveBeenCalledWith(expect.stringContaining('custom/path'), {recursive: true});
			expect(mkdir).toHaveBeenCalledWith(expect.stringContaining('custom/path'), {recursive: true});
		});
	});

	describe('local target branch cleanup', () => {
		test('deletes local target branch if it exists', async () => {
			const {git_local_branch_exists, git_delete_local_branch} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);

			// Mock target branch exists locally
			vi.mocked(git_local_branch_exists).mockImplementation(async (branch) => {
				return branch === 'deploy'; // target branch exists
			});

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should delete local target branch
			expect(git_delete_local_branch).toHaveBeenCalledWith('deploy');
		});

		test('skips deletion when local target branch does not exist', async () => {
			const {git_local_branch_exists, git_delete_local_branch} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);

			// Mock target branch doesn't exist locally
			vi.mocked(git_local_branch_exists).mockResolvedValue(false);

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should NOT delete (doesn't exist)
			expect(git_delete_local_branch).not.toHaveBeenCalledWith('deploy');
		});
	});

	describe('orphan branch creation', () => {
		test('clones source branch locally to deploy dir', async () => {
			const {git_clone_locally} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				deploy_dir: '.gro/deploy',
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should clone from origin/source to deploy dir
			expect(git_clone_locally).toHaveBeenCalledWith(
				'origin',
				'main',
				process.cwd(),
				expect.stringContaining('.gro/deploy'),
			);
		});

		test('creates orphan target branch in deploy dir', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));

			const ctx = create_mock_deploy_task_context({
				target: 'deploy',
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should create orphan branch
			expect(spawn).toHaveBeenCalledWith('git', ['checkout', '--orphan', 'deploy'], {
				cwd: expect.stringContaining('.gro/deploy'),
			});
		});

		test('removes all files in orphan branch', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should remove all files
			expect(spawn).toHaveBeenCalledWith('git', ['rm', '-rf', '.'], {
				cwd: expect.stringContaining('.gro/deploy'),
			});
		});

		test('creates initial .gitkeep file', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should create .gitkeep
			expect(spawn).toHaveBeenCalledWith('touch', ['.gitkeep'], {
				cwd: expect.stringContaining('.gro/deploy'),
			});
		});

		test('adds .gitkeep to git', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should add .gitkeep
			expect(spawn).toHaveBeenCalledWith('git', ['add', '.gitkeep'], {
				cwd: expect.stringContaining('.gro/deploy'),
			});
		});

		test('commits initial commit', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should commit with 'init' message
			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'init'], {
				cwd: expect.stringContaining('.gro/deploy'),
			});
		});

		test('operations happen in correct order', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Get all spawn calls for the deploy dir
			const deploy_calls = spawn.mock.calls.filter((call) =>
				(call[2]?.cwd as string | undefined)?.includes('.gro/deploy'),
			);

			// Extract commands for verification
			// For git commands: use args[0], for non-git commands: use cmd
			const commands = deploy_calls.map((call) => {
				if (call[0] === 'git') {
					return call[1]?.[0]; // git subcommand
				}
				return call[0]; // command name (e.g., 'touch')
			});

			// Should happen in order: checkout --orphan, rm, touch, add, commit, push (skipped in dry)
			expect(commands.indexOf('checkout')).toBeLessThan(commands.indexOf('rm'));
			expect(commands.indexOf('rm')).toBeLessThan(commands.indexOf('touch'));
			expect(commands.indexOf('touch')).toBeLessThan(commands.indexOf('add'));
			expect(commands.indexOf('add')).toBeLessThan(commands.indexOf('commit'));
		});
	});

	describe('remote branch creation', () => {
		test('pushes to create remote target branch', async () => {
			const {git_push_to_create} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await deploy_task.run(ctx);

			// Should push to create remote branch
			expect(git_push_to_create).toHaveBeenCalledWith('origin', 'deploy', {
				cwd: expect.stringContaining('.gro/deploy'),
			});
		});

		test('uses custom origin and target', async () => {
			const {git_push_to_create} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			const ctx = create_mock_deploy_task_context({
				origin: 'upstream',
				target: 'gh-pages',
				force: true,
				dry: true,
			});

			await deploy_task.run(ctx);

			expect(git_push_to_create).toHaveBeenCalledWith('upstream', 'gh-pages', {
				cwd: expect.stringContaining('.gro/deploy'),
			});
		});
	});

	describe('cleanup after creation', () => {
		test('deletes source branch from deploy dir', async () => {
			const {git_delete_local_branch} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				dry: true,
			});

			await deploy_task.run(ctx);

			// Should delete source branch from deploy dir (last delete call)
			const delete_calls = git_delete_local_branch.mock.calls;
			const last_call = delete_calls[delete_calls.length - 1];
			expect(last_call).toEqual(['main', {cwd: expect.stringContaining('.gro/deploy')}]);
		});

		test('deletes custom source branch from deploy dir', async () => {
			const {git_delete_local_branch} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			const ctx = create_mock_deploy_task_context({
				source: 'develop',
				dry: true,
			});

			await deploy_task.run(ctx);

			const delete_calls = git_delete_local_branch.mock.calls;
			const last_call = delete_calls[delete_calls.length - 1];
			expect(last_call).toEqual(['develop', {cwd: expect.stringContaining('.gro/deploy')}]);
		});
	});

	describe('error handling', () => {
		test('propagates error when git_clone_locally fails', async () => {
			const {git_clone_locally} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			vi.mocked(git_clone_locally).mockRejectedValue(new Error('Clone failed'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await expect(deploy_task.run(ctx)).rejects.toThrow('Clone failed');
		});

		test('propagates error when orphan branch creation fails', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));

			// Make checkout --orphan fail
			vi.mocked(spawn).mockImplementation(async (cmd, args) => {
				if (cmd === 'git' && args?.[0] === 'checkout' && args[1] === '--orphan') {
					throw new Error('Failed to create orphan branch');
				}
				return {code: 0} as any;
			});

			const ctx = create_mock_deploy_task_context({dry: true});

			await expect(deploy_task.run(ctx)).rejects.toThrow('Failed to create orphan branch');
		});

		test('propagates error when push fails', async () => {
			const {git_push_to_create} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

			vi.mocked(git_push_to_create).mockRejectedValue(new Error('Push failed'));

			const ctx = create_mock_deploy_task_context({dry: true});

			await expect(deploy_task.run(ctx)).rejects.toThrow('Push failed');
		});
	});
});
