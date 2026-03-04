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

describe('deploy_task commit and push', () => {
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

	describe('git add', () => {
		test('adds all files with force flag', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			expect(spawn).toHaveBeenCalledWith('git', ['add', '.', '-f'], {cwd: resolve('.gro/deploy')});
		});

		test('uses custom deploy_dir for git add', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: false,
				deploy_dir: 'custom/deploy',
			});

			await deploy_task.run(ctx);

			expect(spawn).toHaveBeenCalledWith('git', ['add', '.', '-f'], {
				cwd: resolve('custom/deploy'),
			});
		});

		test('git add happens after file copying', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {cp} = await import('node:fs/promises');
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			// Find cp and git add invocation orders
			const cp_order = (cp as any).mock.invocationCallOrder[0];
			const add_call = spawn.mock.calls.findIndex(
				(call) => call[0] === 'git' && call[1]?.[0] === 'add',
			);
			const add_order = spawn.mock.invocationCallOrder[add_call];

			expect(add_order).toBeGreaterThan(cp_order);
		});
	});

	describe('git commit', () => {
		test('commits with deployment message', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'deployment'], {
				cwd: resolve('.gro/deploy'),
			});
		});

		test('uses custom deploy_dir for git commit', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: false,
				deploy_dir: 'custom/deploy',
			});

			await deploy_task.run(ctx);

			expect(spawn).toHaveBeenCalledWith('git', ['commit', '-m', 'deployment'], {
				cwd: resolve('custom/deploy'),
			});
		});

		test('git commit happens after git add', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			// Find add and commit calls
			const add_call_index = spawn.mock.calls.findIndex(
				(call) => call[0] === 'git' && call[1]?.[0] === 'add',
			);
			const commit_call_index = spawn.mock.calls.findIndex(
				(call) => call[0] === 'git' && call[1]?.[0] === 'commit',
			);

			const add_order = spawn.mock.invocationCallOrder[add_call_index]!;
			const commit_order = spawn.mock.invocationCallOrder[commit_call_index]!;

			expect(commit_order).toBeGreaterThan(add_order);
		});
	});

	describe('git push', () => {
		test('pushes with force flag to target branch', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			expect(spawn).toHaveBeenCalledWith('git', ['push', 'origin', 'deploy', '-f'], {
				cwd: resolve('.gro/deploy'),
			});
		});

		test('uses custom origin and target branch', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: false,
				origin: 'upstream',
				target: 'gh-pages',
				force: true,
			});

			await deploy_task.run(ctx);

			expect(spawn).toHaveBeenCalledWith('git', ['push', 'upstream', 'gh-pages', '-f'], {
				cwd: resolve('.gro/deploy'),
			});
		});

		test('uses custom deploy_dir for git push', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				dry: false,
				deploy_dir: 'custom/deploy',
			});

			await deploy_task.run(ctx);

			expect(spawn).toHaveBeenCalledWith('git', expect.anything(), {cwd: resolve('custom/deploy')});
		});

		test('git push happens after git commit', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			// Find commit and push calls
			const commit_call_index = spawn.mock.calls.findIndex(
				(call) => call[0] === 'git' && call[1]?.[0] === 'commit',
			);
			const push_call_index = spawn.mock.calls.findIndex(
				(call) => call[0] === 'git' && call[1]?.[0] === 'push',
			);

			const commit_order = spawn.mock.invocationCallOrder[commit_call_index]!;
			const push_order = spawn.mock.invocationCallOrder[push_call_index]!;

			expect(push_order).toBeGreaterThan(commit_order);
		});

		test('force pushes to allow non-fastforward updates', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			// Should include -f flag
			const push_call = spawn.mock.calls.find(
				(call) => call[0] === 'git' && call[1]?.[0] === 'push',
			);
			expect(push_call![1]).toContain('-f');
		});
	});

	describe('success logging', () => {
		test('logs deployed message on success', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			expect(ctx.log.info).toHaveBeenCalledWith(expect.stringContaining('deployed'));
		});

		test('does not log dry deploy message when dry=false', async () => {
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			expect(ctx.log.info).not.toHaveBeenCalledWith(expect.stringContaining('dry deploy'));
		});
	});

	describe('error handling', () => {
		test('catches and logs git add failure', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			// Make git add fail
			vi.mocked(spawn).mockImplementation(async (cmd, args) => {
				if (cmd === 'git' && args?.[0] === 'add') {
					throw new Error('Git add failed');
				}
				return {code: 0} as any;
			});

			const ctx = create_mock_deploy_task_context({dry: false});

			let error: TaskError | undefined;
			try {
				await deploy_task.run(ctx);
			} catch (e) {
				error = e as TaskError;
			}
			expect(error).toBeInstanceOf(TaskError);
			expect(error!.message).toContain('Deploy failed in a bad state');

			expect(ctx.log.error).toHaveBeenCalledWith(
				expect.stringContaining('updating git failed'),
				expect.anything(),
			);
		});

		test('catches and logs git commit failure', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			// Make git commit fail
			vi.mocked(spawn).mockImplementation(async (cmd, args) => {
				if (cmd === 'git' && args?.[0] === 'commit') {
					throw new Error('Nothing to commit');
				}
				return {code: 0} as any;
			});

			const ctx = create_mock_deploy_task_context({dry: false});

			let error: TaskError | undefined;
			try {
				await deploy_task.run(ctx);
			} catch (e) {
				error = e as TaskError;
			}
			expect(error).toBeInstanceOf(TaskError);
			expect(error!.message).toContain('Deploy failed in a bad state');

			expect(ctx.log.error).toHaveBeenCalledWith(
				expect.stringContaining('updating git failed'),
				expect.anything(),
			);
		});

		test('catches and logs git push failure', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			// Make git push fail
			vi.mocked(spawn).mockImplementation(async (cmd, args) => {
				if (cmd === 'git' && args?.[0] === 'push') {
					throw new Error('Push rejected');
				}
				return {code: 0} as any;
			});

			const ctx = create_mock_deploy_task_context({dry: false});

			let error: TaskError | undefined;
			try {
				await deploy_task.run(ctx);
			} catch (e) {
				error = e as TaskError;
			}
			expect(error).toBeInstanceOf(TaskError);
			expect(error!.message).toContain('Deploy failed in a bad state');

			expect(ctx.log.error).toHaveBeenCalledWith(
				expect.stringContaining('updating git failed'),
				expect.anything(),
			);
		});

		test('error message indicates bad state (built but not pushed)', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			// Make push fail
			vi.mocked(spawn).mockImplementation(async (cmd, args) => {
				if (cmd === 'git' && args?.[0] === 'push') {
					throw new Error('Network error');
				}
				return {code: 0} as any;
			});

			const ctx = create_mock_deploy_task_context({dry: false});

			let error: TaskError | undefined;
			try {
				await deploy_task.run(ctx);
			} catch (e) {
				error = e as TaskError;
			}
			expect(error!.message).toContain('built but not pushed');
		});
	});

	describe('operation order', () => {
		test('all operations happen in correct sequence', async () => {
			const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
			const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
			vi.mocked(fs_exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({dry: false});

			await deploy_task.run(ctx);

			// Find the three git operations in deploy dir
			const deploy_git_calls = spawn.mock.calls
				.map((call, idx) => ({call, idx}))
				.filter(({call}) => {
					return call[0] === 'git' && (call[2]?.cwd as string).includes('deploy');
				});

			const add_call = deploy_git_calls.find(({call}) => call[1]?.[0] === 'add');
			const commit_call = deploy_git_calls.find(({call}) => call[1]?.[0] === 'commit');
			const push_call = deploy_git_calls.find(({call}) => call[1]?.[0] === 'push');

			// All should exist
			expect(add_call).toBeDefined();
			expect(commit_call).toBeDefined();
			expect(push_call).toBeDefined();

			// Should be in order: add < commit < push
			expect(add_call!.idx).toBeLessThan(commit_call!.idx);
			expect(commit_call!.idx).toBeLessThan(push_call!.idx);
		});
	});
});
