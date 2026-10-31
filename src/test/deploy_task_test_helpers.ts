import { vi } from 'vitest';
import type { SpawnResult } from '@fuzdev/fuz_util/process.ts';

import type { TaskContext } from '#lib/task.ts';
import type { Args } from '#lib/deploy.task.ts';
import type { GroConfig } from '#lib/gro_config.ts';

import { create_mock_task_context } from './test_helpers.ts';

/* eslint-disable @typescript-eslint/explicit-module-boundary-types */

/**
 * Creates a mock task context with deploy task Args defaults.
 */
export const create_mock_deploy_task_context = (
	args: Partial<Args> = {},
	config: Partial<GroConfig> = {}
): TaskContext<Args> =>
	create_mock_task_context(args, config, {
		source: 'main',
		target: 'deploy',
		origin: 'origin',
		deploy_dir: '.gro/deploy',
		build_dir: 'build',
		dry: false,
		force: false,
		dangerous: false,
		reset: false,
		build: true,
		'no-build': false,
		sync: true,
		'no-sync': false,
		gen: true,
		'no-gen': false,
		install: true,
		'no-install': false,
		force_build: false,
		pull: true,
		'no-pull': false
	});

/**
 * Creates mock spawn options for deploy directory operations.
 */
export const create_mock_spawn_options = (deploy_dir = '.gro/deploy') => ({
	cwd: deploy_dir
});

/**
 * The first commit of the mocked target branch, which `--reset` resets to.
 */
export const FIRST_COMMIT_HASH = 'abc1234';

/**
 * Sets up common git mocks for successful scenarios.
 */
export const setup_successful_git_mocks = async () => {
	const {
		git_check_clean_workspace,
		git_check_setting_pull_rebase,
		git_local_branch_exists,
		git_fetch,
		git_checkout,
		git_pull,
		git_remote_branch_exists,
		git_clone_locally,
		git_current_branch_name,
		git_delete_local_branch,
		git_push_to_create,
		git_current_branch_first_commit_hash
	} = vi.mocked(await import('@fuzdev/fuz_util/git.ts'));

	vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
	vi.mocked(git_check_setting_pull_rebase).mockResolvedValue(true);
	vi.mocked(git_local_branch_exists).mockResolvedValue(true);
	vi.mocked(git_fetch).mockResolvedValue(undefined);
	vi.mocked(git_checkout).mockResolvedValue(null);
	vi.mocked(git_pull).mockResolvedValue(undefined);
	vi.mocked(git_remote_branch_exists).mockResolvedValue(true);
	vi.mocked(git_clone_locally).mockResolvedValue(undefined);
	vi.mocked(git_current_branch_name).mockResolvedValue('deploy');
	vi.mocked(git_delete_local_branch).mockResolvedValue(undefined);
	vi.mocked(git_push_to_create).mockResolvedValue(undefined);
	vi.mocked(git_current_branch_first_commit_hash).mockResolvedValue(FIRST_COMMIT_HASH);
};

/**
 * Sets up common fs mocks for successful scenarios.
 */
export const setup_successful_fs_mocks = async () => {
	const { fs_exists } = await import('@fuzdev/fuz_util/fs.ts');
	const { cp, mkdir, rm, readdir } = await import('node:fs/promises');

	vi.mocked(fs_exists).mockResolvedValue(true);
	vi.mocked(readdir).mockResolvedValue(['index.html', 'assets'] as any);
	vi.mocked(cp).mockResolvedValue(undefined);
	vi.mocked(mkdir).mockResolvedValue(undefined);
	vi.mocked(rm).mockResolvedValue(undefined);
};

/**
 * A spawn result for a command that exited with `code`.
 */
export const to_spawn_result = (code = 0): SpawnResult =>
	({ kind: 'exited', ok: code === 0, code }) as SpawnResult;

/**
 * Mocks `spawn` for a successful deploy, with `fail` returning a failed result
 * for the commands it matches; `git diff --cached --quiet` exits 1, so the build has changes to commit.
 */
export const mock_spawn_results = async (
	fail?: (command: string, args: ReadonlyArray<string>) => boolean
) => {
	const { spawn } = vi.mocked(await import('@fuzdev/fuz_util/process.ts'));
	vi.mocked(spawn).mockImplementation(async (command, args = []) => {
		if (fail?.(command, args)) return to_spawn_result(1);
		if (command === 'git' && args[0] === 'diff') return to_spawn_result(1);
		return to_spawn_result(0);
	});
};

/**
 * Sets up common spawn mock for successful scenarios.
 */
export const setup_successful_spawn_mock = async () => {
	await mock_spawn_results();
};
