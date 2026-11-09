import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import { resolve } from 'node:path';

import { task as deploy_task } from '#lib/deploy.task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
	FIRST_COMMIT_HASH,
	get_clone_calls
} from './deploy_task_test_helpers.ts';
import { get_mock_project } from './fs_mocks.ts';
import { git_ops } from './git_mocks.ts';

// Mock dependencies
vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

describe('deploy_task integration scenarios', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('first deployment (no remote branch)', () => {
		test('complete first deployment flow', async () => {
			const {
				git_checkout,
				git_pull,
				git_remote_branch_exists,
				git_local_branch_exists,
				git_delete_local_branch,
				git_push_to_create
			} = git_ops;
			const { git_command: spawn } = git_ops;
			const { exists } = await get_mock_project();

			// No remote branch exists (first deploy)
			vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
			vi.mocked(git_local_branch_exists).mockResolvedValue(false);
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should checkout and pull source
			expect(git_checkout).toHaveBeenCalledWith('main');
			expect(git_pull).toHaveBeenCalledWith('origin', 'main');

			// Should create orphan branch
			expect(await get_clone_calls()).toHaveLength(1);
			expect(spawn).toHaveBeenCalledWith(
				'git',
				['checkout', '--orphan', 'deploy'],
				expect.anything()
			);
			expect(spawn).toHaveBeenCalledWith('git', ['rm', '-rf', '.'], expect.anything());
			expect((await get_mock_project()).write).toHaveBeenCalledWith('.gro/deploy/.gitkeep', '');
			expect(spawn).toHaveBeenCalledWith('git', ['add', '--', '.gitkeep'], expect.anything());
			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'init'], expect.anything());

			// Should push to create remote
			expect(git_push_to_create).toHaveBeenCalled();

			// Should clean up source branch in deploy dir
			expect(git_delete_local_branch).toHaveBeenCalledWith('main', expect.anything());

			// Should build and copy files
			expect(ctx.invoke_task).toHaveBeenCalledWith('build', {
				sync: true,
				gen: true,
				install: true,
				force_build: false
			});

			// Should commit and push deployment
			expect(spawn).toHaveBeenCalledWith('git', ['add', '.'], expect.anything());
			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'deployment'], expect.anything());
			expect(spawn).toHaveBeenCalledWith(
				'git',
				['push', '-f', '--end-of-options', 'origin', 'deploy'],
				expect.anything()
			);

			// Should log success
			expect(ctx.log.info).toHaveBeenCalledWith(expect.stringContaining('deployed'));
		});

		test('first deployment with custom branches', async () => {
			const { git_checkout, git_remote_branch_exists, git_push_to_create } = git_ops;
			const { git_command: spawn } = git_ops;
			const { exists } = await get_mock_project();

			vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				source: 'develop',
				target: 'gh-pages',
				origin: 'upstream',
				force: true,
				dry: false
			});

			await deploy_task.run(ctx);

			// Should use custom branches
			expect(git_checkout).toHaveBeenCalledWith('develop');
			expect(spawn).toHaveBeenCalledWith(
				'git',
				['checkout', '--orphan', 'gh-pages'],
				expect.anything()
			);
			expect(git_push_to_create).toHaveBeenCalledWith('upstream', 'gh-pages', expect.anything());
			expect(spawn).toHaveBeenCalledWith(
				'git',
				['push', '-f', '--end-of-options', 'upstream', 'gh-pages'],
				expect.anything()
			);
		});
	});

	describe('subsequent deployments (remote branch exists)', () => {
		test('complete subsequent deployment flow with existing deploy dir', async () => {
			const { git_checkout, git_pull, git_current_branch_name } = git_ops;
			const { git_command: spawn } = git_ops;
			const { exists, remove } = await get_mock_project();

			// Remote branch exists, deploy dir exists with correct branch
			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(git_current_branch_name).mockResolvedValue('deploy');

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should checkout and pull source
			expect(git_checkout).toHaveBeenCalledWith('main');
			expect(git_pull).toHaveBeenNthCalledWith(1, 'origin', 'main');

			// Should sync deploy dir (reset + pull)
			expect(spawn).toHaveBeenCalledWith('git', ['reset', '--hard'], {
				cwd: resolve('.gro/deploy')
			});
			expect(git_pull).toHaveBeenNthCalledWith(2, 'origin', 'deploy', {
				cwd: resolve('.gro/deploy')
			});

			// Should empty deploy dir
			expect(remove).toHaveBeenCalled();

			// Should build and copy
			expect(ctx.invoke_task).toHaveBeenCalledWith('build', {
				sync: true,
				gen: true,
				install: true,
				force_build: false
			});

			// Should commit and push
			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'deployment'], expect.anything());
			expect(spawn).toHaveBeenCalledWith(
				'git',
				['push', '-f', '--end-of-options', 'origin', 'deploy'],
				expect.anything()
			);
		});

		test('subsequent deployment with reset flag', async () => {
			const { git_pull } = git_ops;
			const { git_command: spawn } = git_ops;
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				reset: true,
				dry: false
			});

			await deploy_task.run(ctx);

			// Should reset branch to first commit locally, then publish it with the final force push
			expect(spawn).toHaveBeenCalledWith('git', ['reset', '--hard', FIRST_COMMIT_HASH], {
				cwd: resolve('.gro/deploy')
			});
			const reset_index = spawn.mock.calls.findIndex(
				(call) => call[1]?.[0] === 'reset' && call[1][2] === FIRST_COMMIT_HASH
			);
			const push_index = spawn.mock.calls.findIndex(
				(call) => call[1]?.join(' ') === 'push -f --end-of-options origin deploy'
			);
			expect(reset_index).toBeGreaterThan(-1);
			expect(push_index).toBeGreaterThan(reset_index);

			// Should NOT pull target branch (optimization when resetting)
			expect(git_pull).toHaveBeenCalledTimes(1); // Only source branch
			expect(git_pull).toHaveBeenCalledWith('origin', 'main');
		});

		test('subsequent deployment with reset and no-pull flags', async () => {
			const { git_pull } = git_ops;
			const { git_command: spawn } = git_ops;
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				reset: true,
				pull: false, // Don't pull source branch
				dry: false
			});

			await deploy_task.run(ctx);

			// Should still reset branch even without pulling source
			expect(spawn).toHaveBeenCalledWith('git', ['reset', '--hard', FIRST_COMMIT_HASH], {
				cwd: resolve('.gro/deploy')
			});

			// Should NOT pull either source or target branch
			expect(git_pull).not.toHaveBeenCalled();
		});

		test('subsequent deployment reinitializes when deploy dir has wrong branch', async () => {
			const { git_current_branch_name, git_fetch } = git_ops;
			const { remove, exists } = await get_mock_project();

			// Deploy dir exists but with wrong branch
			let exists_call_count = 0;
			vi.mocked(exists).mockImplementation((path: any) => {
				const path_str = String(path);
				if (path_str.includes('build')) return true; // build_dir always exists
				exists_call_count++;
				return exists_call_count <= 2; // the ownership check and the first deploy_dir check
			});
			vi.mocked(git_current_branch_name).mockResolvedValue('wrong-branch');

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Should delete and reinitialize
			expect(remove).toHaveBeenCalledWith('.gro/deploy', { recursive: true });
			expect(git_fetch).toHaveBeenCalled();
			expect(await get_clone_calls()).toHaveLength(1);
		});
	});

	describe('deployment with no-build flag', () => {
		test('deploys using existing build output', async () => {
			const { git_command: spawn } = git_ops;
			const { exists, copy } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				build: false,
				dry: false
			});

			await deploy_task.run(ctx);

			// Should NOT invoke build
			expect(ctx.invoke_task).not.toHaveBeenCalled();

			// Should still copy files and deploy
			expect(copy).toHaveBeenCalled();
			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'deployment'], expect.anything());
		});
	});

	describe('deployment with no-pull flag', () => {
		test('deploys without pulling source branch', async () => {
			const { git_pull, git_checkout } = git_ops;
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				pull: false,
				dry: false
			});

			await deploy_task.run(ctx);

			// Should checkout but not pull source
			expect(git_checkout).toHaveBeenCalled();
			expect(git_pull).not.toHaveBeenCalledWith('origin', 'main');

			// Should still pull target branch to sync deploy dir
			expect(git_pull).toHaveBeenCalledTimes(1);
			expect(git_pull).toHaveBeenCalledWith('origin', 'deploy', { cwd: resolve('.gro/deploy') });
		});
	});

	describe('dry deployment scenarios', () => {
		test('dry deployment performs all prep but skips push', async () => {
			const { git_checkout } = git_ops;
			const { git_command: spawn } = git_ops;
			const { exists, copy } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			await deploy_task.run(ctx);

			// Should do all prep work
			expect(git_checkout).toHaveBeenCalled();
			expect(ctx.invoke_task).toHaveBeenCalledWith('build', {
				sync: true,
				gen: true,
				install: true,
				force_build: false
			});
			expect(copy).toHaveBeenCalled();

			// Should NOT push
			expect(spawn).not.toHaveBeenCalledWith(
				'git',
				expect.arrayContaining(['push']),
				expect.anything()
			);

			// Should log dry success
			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.stringContaining('dry deploy complete'),
				expect.anything(),
				expect.anything()
			);
		});

		test('dry deployment with all custom options', async () => {
			const { git_checkout } = git_ops;
			const { git_command: spawn } = git_ops;
			const { exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: true,
				source: 'develop',
				target: 'staging',
				origin: 'upstream',
				deploy_dir: '.gro/custom/deploy',
				build_dir: 'dist',
				build: false,
				pull: false,
				reset: true,
				force: true
			});

			await deploy_task.run(ctx);

			// Should use all custom settings
			expect(git_checkout).toHaveBeenCalledWith('develop');
			expect(spawn).toHaveBeenCalledWith('git', ['reset', '--hard', FIRST_COMMIT_HASH], {
				cwd: resolve('.gro/custom/deploy')
			});
			expect(ctx.invoke_task).not.toHaveBeenCalled(); // no-build

			// Should still complete successfully
			expect(ctx.log.info).toHaveBeenCalledWith(
				expect.stringContaining('dry deploy complete'),
				expect.anything(),
				expect.stringContaining('.gro/custom/deploy')
			);
		});
	});

	describe('complete workflow verification', () => {
		test('all operations happen in expected order for new deployment', async () => {
			const {
				git_check_clean_workspace,
				git_check_setting_pull_rebase,
				git_checkout,
				git_pull,
				git_remote_branch_exists,
				git_push_to_create
			} = git_ops;
			const { git_command: spawn } = git_ops;
			const { remove, copy, exists } = await get_mock_project();

			vi.mocked(git_remote_branch_exists).mockResolvedValue(false);
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Verify operation order using invocation call orders
			const check_workspace_order = git_check_clean_workspace.mock.invocationCallOrder[0]!;
			const check_rebase_order = git_check_setting_pull_rebase.mock.invocationCallOrder[0]!;
			const checkout_order = git_checkout.mock.invocationCallOrder[0]!;
			const pull_order = git_pull.mock.invocationCallOrder[0]!;
			const remote_exists_order = git_remote_branch_exists.mock.invocationCallOrder[0]!;
			const clone_order = (await get_clone_calls())[0]!.order;
			const push_create_order = git_push_to_create.mock.invocationCallOrder[0]!;
			// the last removal empties the deploy dir, after the first removed the whole of it
			const empty_order = remove.mock.invocationCallOrder.at(-1)!;
			const build_order = (ctx.invoke_task as any).mock.invocationCallOrder[0]!;
			const copy_order = copy.mock.invocationCallOrder[0]!;

			// Safety checks first
			expect(check_workspace_order).toBeLessThan(checkout_order);
			expect(check_rebase_order).toBeLessThan(checkout_order);

			// Source branch prep
			expect(checkout_order).toBeLessThan(pull_order);
			expect(pull_order).toBeLessThan(remote_exists_order);

			// Target branch setup
			expect(remote_exists_order).toBeLessThan(clone_order);
			expect(clone_order).toBeLessThan(push_create_order);

			// Build and deploy
			expect(push_create_order).toBeLessThan(empty_order);
			expect(empty_order).toBeLessThan(build_order);
			expect(build_order).toBeLessThan(copy_order);

			// Final commit and push
			const final_push_call = spawn.mock.calls.findIndex(
				(call) => call[0] === 'git' && call[1]?.[0] === 'push' && call[1][1] === '-f'
			);
			const final_push_order = spawn.mock.invocationCallOrder[final_push_call]!;
			expect(copy_order).toBeLessThan(final_push_order);
		});

		test('all operations happen in expected order for existing deployment', async () => {
			const { git_checkout, git_pull, git_current_branch_name } = git_ops;
			const { git_command: spawn } = git_ops;
			const { remove, copy, exists } = await get_mock_project();

			vi.mocked(exists).mockResolvedValue(true);
			vi.mocked(git_current_branch_name).mockResolvedValue('deploy');

			const ctx = create_mock_deploy_task_context({ dry: false });

			await deploy_task.run(ctx);

			// Source prep
			const checkout_order = git_checkout.mock.invocationCallOrder[0]!;
			const pull_source_order = git_pull.mock.invocationCallOrder[0]!;

			// Target sync
			const branch_name_order = git_current_branch_name.mock.invocationCallOrder.at(-1)!;
			const reset_call = spawn.mock.calls.findIndex(
				(call) => call[0] === 'git' && call[1]?.[0] === 'reset'
			);
			const reset_order = spawn.mock.invocationCallOrder[reset_call]!;

			// Build and deploy
			const empty_order = remove.mock.invocationCallOrder[0]!;
			const build_order = (ctx.invoke_task as any).mock.invocationCallOrder[0]!;
			const copy_order = copy.mock.invocationCallOrder[0]!;

			// Verify order
			expect(checkout_order).toBeLessThan(pull_source_order);
			expect(pull_source_order).toBeLessThan(branch_name_order);
			expect(branch_name_order).toBeLessThan(reset_order);
			expect(reset_order).toBeLessThan(empty_order);
			expect(empty_order).toBeLessThan(build_order);
			expect(build_order).toBeLessThan(copy_order);
		});
	});
});
