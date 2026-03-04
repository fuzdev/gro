import {vi} from 'vitest';

import type {TaskContext} from '../lib/task.ts';
import type {Args} from '../lib/deploy.task.ts';
import type {GroConfig} from '../lib/gro_config.ts';

import {create_mock_task_context} from './test_helpers.ts';

/* eslint-disable @typescript-eslint/explicit-module-boundary-types */

/**
 * Creates a mock task context with deploy task Args defaults.
 */
export const create_mock_deploy_task_context = (
	args: Partial<Args> = {},
	config: Partial<GroConfig> = {},
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
		'no-pull': false,
	} as Args);

/**
 * Creates mock spawn options for deploy directory operations.
 */
export const create_mock_spawn_options = (deploy_dir = '.gro/deploy') => ({
	cwd: deploy_dir,
});

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
		git_reset_branch_to_first_commit,
	} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

	vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
	vi.mocked(git_check_setting_pull_rebase).mockResolvedValue(true);
	vi.mocked(git_local_branch_exists).mockResolvedValue(true);
	vi.mocked(git_fetch).mockResolvedValue(undefined);
	vi.mocked(git_checkout).mockResolvedValue(undefined);
	vi.mocked(git_pull).mockResolvedValue(undefined);
	vi.mocked(git_remote_branch_exists).mockResolvedValue(true);
	vi.mocked(git_clone_locally).mockResolvedValue(undefined);
	vi.mocked(git_current_branch_name).mockResolvedValue('deploy');
	vi.mocked(git_delete_local_branch).mockResolvedValue(undefined);
	vi.mocked(git_push_to_create).mockResolvedValue(undefined);
	vi.mocked(git_reset_branch_to_first_commit).mockResolvedValue(undefined);
};

/**
 * Sets up common fs mocks for successful scenarios.
 */
export const setup_successful_fs_mocks = async () => {
	const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');
	const {cp, mkdir, rm, readdir} = await import('node:fs/promises');

	vi.mocked(fs_exists).mockResolvedValue(true);
	vi.mocked(readdir).mockResolvedValue(['index.html', 'assets'] as any);
	vi.mocked(cp).mockResolvedValue(undefined);
	vi.mocked(mkdir).mockResolvedValue(undefined);
	vi.mocked(rm).mockResolvedValue(undefined);
};

/**
 * Sets up common spawn mock for successful scenarios.
 */
export const setup_successful_spawn_mock = async () => {
	const {spawn} = vi.mocked(await import('@fuzdev/fuz_util/process.js'));
	vi.mocked(spawn).mockResolvedValue({code: 0} as any);
};
