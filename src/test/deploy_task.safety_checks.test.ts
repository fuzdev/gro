import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, realpathSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { task as deploy_task, DEPLOY_CLONE_CONFIG_KEY, type Args } from '#lib/deploy.task.ts';
import { TaskError } from '#lib/task.ts';

import {
	create_mock_deploy_task_context,
	setup_successful_git_mocks,
	setup_successful_fs_mocks,
	setup_successful_spawn_mock,
	to_spawn_result,
	ORIGIN_URL
} from './deploy_task_test_helpers.ts';
import { create_mock_fs_dir, get_mock_project, to_mock_file_info } from './fs_mocks.ts';
import { git_ops } from './git_mocks.ts';

// Mock dependencies
vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('@fuzdev/fuz_util/process.js', () => ({
	spawn: vi.fn(),
	spawn_out: vi.fn()
}));

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

describe('deploy_task safety checks', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		await setup_successful_git_mocks();
		await setup_successful_fs_mocks();
		await setup_successful_spawn_mock();
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	describe('deploy dir validation', () => {
		const assert_refused_before_git = async (deploy_dir: string): Promise<void> => {
			const git = git_ops;
			const { spawn, spawn_out } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { remove } = await get_mock_project();

			const ctx = create_mock_deploy_task_context({ deploy_dir, dry: true });

			await expect(deploy_task.run(ctx)).rejects.toThrow(
				'the project, the working directory, the home directory, or one of their ancestors'
			);
			for (const fn of [
				git.git_check_clean_workspace,
				git.git_checkout,
				git.git_pull,
				git.git_remote_branch_exists,
				spawn,
				spawn_out,
				remove
			]) {
				expect(fn).not.toHaveBeenCalled();
			}
		};

		test('refuses the project before any git call', async () => {
			await assert_refused_before_git('.');
		});

		test('refuses an ancestor of the project before any git call', async () => {
			// the mock fs opens a directory outside the project as a mock handle
			const { fs } = vi.mocked(await import('#lib/fs.ts'));
			vi.mocked(fs.open_dir).mockImplementation(async (path) => create_mock_fs_dir(path) as any);
			await assert_refused_before_git('..');
		});

		test('refuses a symlink resolving to the project before any git call', async () => {
			const dir = realpathSync(mkdtempSync(join(tmpdir(), 'gro-deploy-dir-')));
			try {
				symlinkSync(process.cwd(), join(dir, 'link'));
				await assert_refused_before_git(join(dir, 'link'));
			} finally {
				rmSync(dir, { recursive: true, force: true });
			}
		});
	});

	describe('deploy dir ownership', () => {
		const assert_refused = async (
			deploy_dir: string,
			message: string,
			args: Partial<Args> = {}
		): Promise<void> => {
			const git = git_ops;
			const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { remove, mkdir } = await get_mock_project();

			const ctx = create_mock_deploy_task_context({ deploy_dir, dry: true, ...args });

			await expect(deploy_task.run(ctx)).rejects.toThrow(message);
			for (const fn of [
				git.git_check_clean_workspace,
				git.git_checkout,
				git.git_pull,
				git.git_remote_branch_exists,
				git.git_current_branch_name,
				spawn,
				remove,
				mkdir
			]) {
				expect(fn).not.toHaveBeenCalled();
			}
		};

		test('refuses a deploy dir inside the project but not beneath `.gro/`', async () => {
			for (const deploy_dir of ['build', 'src/deploy', '.gro', '.gro/', 'deploy']) {
				await assert_refused(deploy_dir, 'inside the project but not beneath `.gro/`');
			}
		});

		// `tag` is what `git config --get` prints for the deploy-clone tag, `null` failing as unset
		const mock_git_out = async (
			out: (args: ReadonlyArray<string>, cwd: string | undefined) => string | null,
			tag: string | null = 'true\n',
			git_dir: (cwd: string | undefined) => string | null = (cwd) => cwd + '/.git\n'
		): Promise<void> => {
			const { spawn_out } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			vi.mocked(spawn_out).mockImplementation(async (_command, args = [], options) => {
				// no cwd is the project, spawn's default
				const cwd = options?.cwd === undefined ? process.cwd() : String(options.cwd);
				const stdout =
					args[0] === 'config'
						? tag
						: args[1] === '--absolute-git-dir'
							? git_dir(cwd)
							: out(args, cwd);
				return stdout === null
					? { result: to_spawn_result(128), stdout: '', stderr: 'fatal: boom\n' }
					: { result: to_spawn_result(0), stdout, stderr: '' };
			});
		};

		// both the remote-branch-exists path and the missing one, which remove the deploy dir
		for (const remote_target_exists of [true, false]) {
			describe(
				remote_target_exists ? 'with the remote target' : 'without the remote target',
				() => {
					beforeEach(async () => {
						const { git_remote_branch_exists } = git_ops;
						vi.mocked(git_remote_branch_exists).mockResolvedValue(remote_target_exists);
					});

					test('refuses an existing deploy dir inside another repo, removing nothing', async () => {
						await mock_git_out((args) =>
							args[0] === 'rev-parse' ? process.cwd() + '\n' : ORIGIN_URL
						);
						await assert_refused('.gro/deploy', `inside the git repo at ${process.cwd()}`);
					});

					test('refuses an existing deploy dir in no repo', async () => {
						await mock_git_out((args) => (args[0] === 'rev-parse' ? null : ORIGIN_URL));
						await assert_refused('.gro/deploy', 'it is not in a git repo');
					});

					test("refuses an existing deploy dir whose origin is not the project's", async () => {
						await mock_git_out((args, cwd) =>
							args[0] === 'rev-parse'
								? cwd + '\n'
								: cwd === process.cwd()
									? ORIGIN_URL
									: 'git@example.com:someone/else.git'
						);
						await assert_refused('.gro/deploy', 'remote is git@example.com:someone/else.git');
					});

					test('refuses an existing deploy dir with no origin', async () => {
						await mock_git_out((args, cwd) =>
							args[0] === 'rev-parse' ? cwd + '\n' : cwd === process.cwd() ? ORIGIN_URL : null
						);
						await assert_refused('.gro/deploy', 'it has no `origin` remote', { dry: false });
					});

					test('refuses a same-origin checkout without the deploy-clone tag', async () => {
						const same_origin = (args: ReadonlyArray<string>, cwd: string | undefined) =>
							args[0] === 'rev-parse' ? cwd + '\n' : ORIGIN_URL;
						for (const tag of [null, 'false\n', '']) {
							await mock_git_out(same_origin, tag);
							await assert_refused(
								'.gro/deploy',
								`lacks the local git config \`${DEPLOY_CLONE_CONFIG_KEY} true\``
							);
						}
					});

					test('refuses a deploy dir whose git dir is not its own `.git` directory', async () => {
						const same_origin = (args: ReadonlyArray<string>, cwd: string | undefined) =>
							args[0] === 'rev-parse' ? cwd + '\n' : ORIGIN_URL;
						// a bare repo elsewhere, its `core.worktree` pointing here
						await mock_git_out(same_origin, 'true\n', () => '/elsewhere/bare.git\n');
						await assert_refused('.gro/deploy', 'its git dir is /elsewhere/bare.git');
						// `.git` a file, as for a linked worktree, though git names the path
						const { stat } = await get_mock_project();
						vi.mocked(stat).mockImplementation(async (path) =>
							path.endsWith('/.git')
								? to_mock_file_info()
								: { ...to_mock_file_info(), kind: 'directory' }
						);
						await mock_git_out(same_origin);
						await assert_refused('.gro/deploy', 'its .git is not a directory');
					});

					test('refuses a deploy dir that is a file, before any git call', async () => {
						const { stat } = await get_mock_project();
						const { spawn_out } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
						vi.mocked(stat).mockResolvedValue(to_mock_file_info());
						await assert_refused('.gro/deploy', 'it is not a directory');
						expect(spawn_out).not.toHaveBeenCalled();
					});

					test('the refusal tells the user to remove it themselves', async () => {
						await mock_git_out(() => null);
						await assert_refused('.gro/deploy', 'remove it yourself');
					});
				}
			);
		}

		test('a missing deploy dir is never checked with git', async () => {
			const { spawn_out } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockImplementation(async (path) => path !== '.gro/deploy');

			await deploy_task.run(create_mock_deploy_task_context({ dry: true }));

			expect(spawn_out).not.toHaveBeenCalledWith(
				'git',
				['rev-parse', '--show-toplevel'],
				expect.anything()
			);
		});

		test("a tagged clone with the project's origin is accepted and synced", async () => {
			const { git_pull } = git_ops;
			const { spawn_out } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));

			await deploy_task.run(create_mock_deploy_task_context({ dry: true }));

			expect(spawn_out).toHaveBeenCalledWith(
				'git',
				['config', '--local', '--get', DEPLOY_CLONE_CONFIG_KEY],
				{ cwd: resolve('.gro/deploy') }
			);
			expect(git_pull).toHaveBeenCalledWith('origin', 'deploy', { cwd: resolve('.gro/deploy') });
		});
	});

	describe('custom target branch validation', () => {
		test('throws TaskError when target is custom and force=false', async () => {
			const ctx = create_mock_deploy_task_context({
				target: 'custom-branch',
				force: false
			});

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/custom target branch/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/custom-branch/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/--force/);
		});

		test('allows custom target branch with force=true', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				target: 'custom-branch',
				force: true,
				dry: true // Skip push
			});

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});

		test('allows default target branch without force flag', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				target: 'deploy', // default
				force: false,
				dry: true
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
				dangerous: false
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
				dangerous: false
			});

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/appears very dangerous/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/master/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/--dangerous/);
		});

		test('allows dangerous branch with dangerous=true', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				target: 'main',
				force: true, // Also need force for custom target
				dangerous: true,
				dry: true
			});

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});

		test('requires both force and dangerous for dangerous custom branches', async () => {
			// Missing force
			const ctx1 = create_mock_deploy_task_context({
				target: 'main',
				force: false,
				dangerous: true
			});

			await expect(deploy_task.run(ctx1)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx1)).rejects.toThrow(/--force/);

			// Missing dangerous
			const ctx2 = create_mock_deploy_task_context({
				target: 'main',
				force: true,
				dangerous: false
			});

			await expect(deploy_task.run(ctx2)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx2)).rejects.toThrow(/--dangerous/);
		});
	});

	describe('workspace cleanliness check', () => {
		test('throws TaskError when workspace has uncommitted changes', async () => {
			const { git_check_clean_workspace } = git_ops;

			vi.mocked(git_check_clean_workspace).mockResolvedValue(
				'Modified files:\n  src/lib/foo.ts\n  src/lib/bar.ts'
			);

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/uncommitted changes/);
		});

		test('throws TaskError when workspace has untracked files', async () => {
			const { git_check_clean_workspace } = git_ops;

			vi.mocked(git_check_clean_workspace).mockResolvedValue('Untracked files:\n  temp.ts');

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/uncommitted changes/);
		});

		test('allows deploy when workspace is clean', async () => {
			const { git_check_clean_workspace } = git_ops;
			const { exists } = await get_mock_project();

			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});

	describe('git pull.rebase setting check', () => {
		test('throws TaskError when pull.rebase is not configured', async () => {
			const { git_check_setting_pull_rebase } = git_ops;

			vi.mocked(git_check_setting_pull_rebase).mockResolvedValue(false);

			const ctx = create_mock_deploy_task_context();

			await expect(deploy_task.run(ctx)).rejects.toThrow(TaskError);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/pull\.rebase/);
			await expect(deploy_task.run(ctx)).rejects.toThrow(/git config --global pull\.rebase true/);
		});

		test('allows deploy when pull.rebase is configured', async () => {
			const { git_check_setting_pull_rebase } = git_ops;
			const { exists } = await get_mock_project();

			vi.mocked(git_check_setting_pull_rebase).mockResolvedValue(true);
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});

	describe('post-pull workspace check', () => {
		test('throws TaskError when workspace is dirty after pull', async () => {
			const { git_check_clean_workspace } = git_ops;

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
			const { git_check_clean_workspace } = git_ops;
			const { exists } = await get_mock_project();

			// Clean throughout
			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({ dry: true });

			// Should not throw
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});

	describe('combined safety checks', () => {
		test('checks are performed in correct order (custom target checked first)', async () => {
			const { git_check_clean_workspace } = git_ops;

			// Set dirty workspace
			vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified: src/foo.ts');

			const ctx = create_mock_deploy_task_context({
				target: 'custom',
				force: false
			});

			// Should fail on custom target check, not workspace check
			await expect(deploy_task.run(ctx)).rejects.toThrow(/custom target branch/);
		});

		test('checks dangerous branch before workspace check', async () => {
			const { git_check_clean_workspace } = git_ops;

			// Set dirty workspace
			vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified: src/foo.ts');

			const ctx = create_mock_deploy_task_context({
				target: 'main',
				force: true,
				dangerous: false
			});

			// Should fail on dangerous branch check, not workspace check
			await expect(deploy_task.run(ctx)).rejects.toThrow(/appears very dangerous/);
		});

		test('all safety checks pass in normal scenario', async () => {
			const { exists } = await get_mock_project();
			vi.mocked(exists).mockResolvedValue(true);

			const ctx = create_mock_deploy_task_context({
				target: 'deploy', // default, safe
				force: false, // not needed for default
				dangerous: false, // not needed for safe branch
				dry: true
			});

			// All checks should pass
			await expect(deploy_task.run(ctx)).resolves.toBeUndefined();
		});
	});
});
