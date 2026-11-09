import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import { resolve } from 'node:path';

import { task as deploy_task, DEPLOY_CLONE_CONFIG_KEY } from '#lib/deploy.task.ts';
import { FsError } from '#lib/fs_error.ts';
import { TaskError } from '#lib/task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
	mock_spawn_results,
	is_clone_call,
	ORIGIN_URL,
	expect_cloned
} from './deploy_task_test_helpers.ts';
import { get_mock_project } from './fs_mocks.ts';
import { git_ops } from './git_mocks.ts';

// Mock dependencies
vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

describe('deploy_task target branch creation (remote does not exist)', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();

		// Default: remote target does NOT exist
		const { git_remote_branch_exists } = git_ops;
		vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('deploy directory cleanup', () => {
		test('deletes existing deploy directory', async () => {
			const { exists, remove, mkdir } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true); // deploy dir exists

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should delete deploy dir
			expect(remove).toHaveBeenCalledWith('.gro/deploy', { recursive: true });
			// Should recreate it
			expect(mkdir).toHaveBeenCalledWith('.gro/deploy', { recursive: true });
		});

		test('skips deletion when deploy directory does not exist', async () => {
			const { exists, remove, mkdir } = await get_mock_project();

			// deploy dir doesn't exist, but build_dir does
			vi.mocked(exists).mockImplementation((path: any) => String(path).includes('build'));

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should NOT delete (doesn't exist)
			expect(remove).not.toHaveBeenCalledWith('.gro/deploy', expect.anything());
			// Should NOT recreate (only does that if it existed before), only make `.gro/` for the clone
			expect(mkdir).not.toHaveBeenCalledWith('.gro/deploy', expect.anything());
			expect(mkdir).toHaveBeenCalledWith('.gro');
		});

		test('uses custom deploy_dir path', async () => {
			const { exists, remove, mkdir } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				deploy_dir: '.gro/custom/path',
				dry: true
			});

			await deploy_task.run(ctx);

			expect(remove).toHaveBeenCalledWith('.gro/custom/path', { recursive: true });
			expect(mkdir).toHaveBeenCalledWith('.gro/custom/path', { recursive: true });
		});
	});

	describe('local target branch cleanup', () => {
		test('deletes local target branch if it exists', async () => {
			const { git_local_branch_exists, git_delete_local_branch } = git_ops;

			// Mock target branch exists locally
			vi.mocked(git_local_branch_exists).mockImplementation(async (branch) => {
				return branch === 'deploy'; // target branch exists
			});

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should delete local target branch
			expect(git_delete_local_branch).toHaveBeenCalledWith('deploy');
		});

		test('skips deletion when local target branch does not exist', async () => {
			const { git_local_branch_exists, git_delete_local_branch } = git_ops;

			// Mock target branch doesn't exist locally
			vi.mocked(git_local_branch_exists).mockResolvedValue(false);

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should NOT delete (doesn't exist)
			expect(git_delete_local_branch).not.toHaveBeenCalledWith('deploy');
		});
	});

	describe('orphan branch creation', () => {
		test('clones source branch locally to deploy dir', async () => {
			const { git_command: spawn } = git_ops;

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				deploy_dir: '.gro/deploy',
				dry: true
			});

			await deploy_task.run(ctx);

			// clones the source branch from the cwd, then points the clone's origin at the project's
			expect(spawn).toHaveBeenCalledWith('git', [
				'clone',
				'--origin',
				'origin',
				'-b',
				'main',
				'--single-branch',
				'--end-of-options',
				process.cwd(),
				resolve('.gro/deploy')
			]);
			expect(spawn).toHaveBeenCalledWith(
				'git',
				['remote', 'set-url', '--end-of-options', 'origin', ORIGIN_URL],
				{
					cwd: resolve('.gro/deploy')
				}
			);
			// tagged, so a later deploy knows it for a clone Gro made
			expect(spawn).toHaveBeenCalledWith(
				'git',
				['config', '--local', DEPLOY_CLONE_CONFIG_KEY, 'true'],
				{ cwd: resolve('.gro/deploy') }
			);
		});

		test('creates orphan target branch in deploy dir', async () => {
			const { git_command: spawn } = git_ops;

			const ctx = create_mock_deploy_task_context({
				target: 'deploy',
				dry: true
			});

			await deploy_task.run(ctx);

			// Should create orphan branch
			expect(spawn).toHaveBeenCalledWith('git', ['checkout', '--orphan', 'deploy'], {
				cwd: expect.stringContaining('.gro/deploy')
			});
		});

		test('removes all files in orphan branch', async () => {
			const { git_command: spawn } = git_ops;

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should remove all files
			expect(spawn).toHaveBeenCalledWith('git', ['rm', '-rf', '.'], {
				cwd: expect.stringContaining('.gro/deploy')
			});
		});

		test('creates initial .gitkeep file', async () => {
			const { write } = await get_mock_project();

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should create .gitkeep
			expect(write).toHaveBeenCalledWith('.gro/deploy/.gitkeep', '');
		});

		test('adds .gitkeep to git', async () => {
			const { git_command: spawn } = git_ops;

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should add .gitkeep
			expect(spawn).toHaveBeenCalledWith('git', ['add', '--', '.gitkeep'], {
				cwd: expect.stringContaining('.gro/deploy')
			});
		});

		test('commits initial commit', async () => {
			const { git_command: spawn } = git_ops;

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should commit with 'init' message
			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'init'], {
				cwd: expect.stringContaining('.gro/deploy')
			});
		});

		test('operations happen in correct order', async () => {
			const { git_command: spawn } = git_ops;

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Get all spawn calls for the deploy dir
			const deploy_calls = spawn.mock.calls.filter((call) =>
				(call[2]?.cwd as string | undefined)?.includes('.gro/deploy')
			);

			// the git subcommands and the initial file's write, by when each ran
			const { write } = await get_mock_project();
			const git_order = (subcommand: string): number =>
				spawn.mock.invocationCallOrder[
					spawn.mock.calls.findIndex((call) => call[1]?.[0] === subcommand)
				]!;
			const write_order =
				write.mock.invocationCallOrder[
					vi.mocked(write).mock.calls.findIndex(([path]) => path === '.gro/deploy/.gitkeep')
				]!;

			// Should happen in order: checkout --orphan, rm, the file, add, commit, push (skipped in dry)
			expect(deploy_calls.length).toBeGreaterThan(0);
			expect(git_order('checkout')).toBeLessThan(git_order('rm'));
			expect(git_order('rm')).toBeLessThan(write_order);
			expect(write_order).toBeLessThan(git_order('add'));
			expect(git_order('add')).toBeLessThan(git_order('commit'));
		});
	});

	describe('remote branch creation', () => {
		test('pushes to create remote target branch', async () => {
			const { git_push_to_create } = git_ops;

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should push to create remote branch
			expect(git_push_to_create).toHaveBeenCalledWith('origin', 'deploy', {
				cwd: expect.stringContaining('.gro/deploy')
			});
		});

		test('uses custom origin and target', async () => {
			const { git_push_to_create } = git_ops;

			const ctx = create_mock_deploy_task_context({
				origin: 'upstream',
				target: 'gh-pages',
				force: true,
				dry: false
			});

			await deploy_task.run(ctx);

			expect(git_push_to_create).toHaveBeenCalledWith('upstream', 'gh-pages', {
				cwd: expect.stringContaining('.gro/deploy')
			});
			// `remote set-url upstream` needs the clone's remote under that name
			await expect_cloned('main', '.gro/deploy', 'upstream');
		});
	});

	describe('cleanup after creation', () => {
		test('deletes source branch from deploy dir', async () => {
			const { git_delete_local_branch } = git_ops;

			const ctx = create_mock_deploy_task_context({
				source: 'main',
				dry: true
			});

			await deploy_task.run(ctx);

			// Should delete source branch from deploy dir (last delete call)
			const delete_calls = git_delete_local_branch.mock.calls;
			const last_call = delete_calls[delete_calls.length - 1];
			expect(last_call).toEqual(['main', { cwd: expect.stringContaining('.gro/deploy') }]);
		});

		test('deletes custom source branch from deploy dir', async () => {
			const { git_delete_local_branch } = git_ops;

			const ctx = create_mock_deploy_task_context({
				source: 'develop',
				dry: true
			});

			await deploy_task.run(ctx);

			const delete_calls = git_delete_local_branch.mock.calls;
			const last_call = delete_calls[delete_calls.length - 1];
			expect(last_call).toEqual(['develop', { cwd: expect.stringContaining('.gro/deploy') }]);
		});
	});

	describe('error handling', () => {
		test('a failed clone fails the deploy before setting any remote', async () => {
			const { git_command: spawn } = git_ops;
			await mock_spawn_results(is_clone_call);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow(
				'`git clone -b main --single-branch` failed'
			);
			expect(spawn.mock.calls.some(([, args]) => args?.includes('set-url'))).toBe(false);
		});

		test('propagates error when orphan branch creation fails', async () => {
			await mock_spawn_results(
				(cmd, args) => cmd === 'git' && args[0] === 'checkout' && args[1] === '--orphan'
			);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow(
				'`git checkout --orphan deploy` failed creating the target branch'
			);
		});

		test('propagates error when push fails', async () => {
			const { git_push_to_create } = git_ops;

			vi.mocked(git_push_to_create).mockRejectedValue(new Error('Push failed'));

			const ctx = create_mock_deploy_task_context({ dry: false });

			await expect(deploy_task.run(ctx)).rejects.toThrow('Push failed');
		});
	});
});

describe('deploy_task initial file failure', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();
		vi.mocked(git_ops.git_remote_branch_exists).mockResolvedValue(false);
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	test('an unwritable .gitkeep fails the deploy before committing and pushing', async () => {
		const { write } = await get_mock_project();
		const { git_command: spawn, git_push_to_create } = git_ops;
		vi.mocked(write).mockImplementation(async (path) => {
			if (path.endsWith('.gitkeep')) {
				throw new FsError('permission_denied', path);
			}
			return 'created';
		});
		const ctx = create_mock_deploy_task_context({ dry: false });

		await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);

		expect(spawn).not.toHaveBeenCalledWith('git', ['add', '--', '.gitkeep'], expect.anything());
		expect(git_push_to_create).not.toHaveBeenCalled();
		expect(ctx.log.error).toHaveBeenCalled();
	});
});
