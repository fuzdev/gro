import { expect, vi } from 'vitest';
import { resolve } from 'node:path';

import type { TaskContext } from '#lib/task.ts';
import type { Args } from '#lib/deploy.task.ts';
import type { GroConfig } from '#lib/gro_config.ts';

import type { SpawnResult } from '#lib/spawn.ts';

import { create_mock_task_context } from './test_helpers.ts';
import { get_mock_project, to_mock_file_info, to_mock_listing } from './fs_mocks.ts';
import type { FsDir } from '#lib/fs.ts';
import { git_ops } from './git_mocks.ts';

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
		git_current_branch_name,
		git_delete_local_branch,
		git_push_to_create,
		git_current_branch_first_commit_hash
	} = git_ops;

	vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
	vi.mocked(git_check_setting_pull_rebase).mockResolvedValue(true);
	vi.mocked(git_local_branch_exists).mockResolvedValue(true);
	vi.mocked(git_fetch).mockResolvedValue(undefined);
	vi.mocked(git_checkout).mockResolvedValue(null);
	vi.mocked(git_pull).mockResolvedValue(undefined);
	vi.mocked(git_remote_branch_exists).mockResolvedValue(true);
	vi.mocked(git_current_branch_name).mockResolvedValue('deploy');
	vi.mocked(git_delete_local_branch).mockResolvedValue(undefined);
	vi.mocked(git_push_to_create).mockResolvedValue(undefined);
	vi.mocked(git_current_branch_first_commit_hash).mockResolvedValue(FIRST_COMMIT_HASH);
};

/**
 * The entries of the mocked deploy dir before it's emptied, `.git` among them.
 */
export const DEPLOY_DIR_ENTRIES = ['.git', 'index.html', 'assets'];

/**
 * The entries of the mocked build dir, copied into the deploy dir.
 */
export const BUILD_DIR_ENTRIES = ['index.html', 'assets'];

/**
 * Mocks `fs.project.list`: the build dir lists `build_entries`, any other directory,
 * like the deploy dir, lists `DEPLOY_DIR_ENTRIES`.
 */
export const mock_listings = async (
	build_entries: Array<string> = BUILD_DIR_ENTRIES,
	build_dir = 'build'
) => {
	const { list } = await get_mock_project();
	vi.mocked(list).mockImplementation((path) =>
		path === build_dir
			? to_mock_listing(build_entries)
			: to_mock_listing(DEPLOY_DIR_ENTRIES, ['.git', 'assets'])
	);
};

/**
 * Sets up common fs mocks for successful scenarios.
 */
export const setup_successful_fs_mocks = async () => {
	const project = await get_mock_project();
	const { exists, remove, copy, mkdir, stat, write, dir } = project;

	// a `cwd` handle on the deploy dir carries its path for the call to report
	vi.mocked(dir).mockImplementation(
		(path) => ({ path: resolve(project.path, path) }) as unknown as FsDir
	);
	vi.mocked(exists).mockResolvedValue(true);
	await mock_listings();
	vi.mocked(remove).mockResolvedValue(undefined);
	vi.mocked(copy).mockResolvedValue(undefined);
	vi.mocked(mkdir).mockResolvedValue(undefined);
	// `.gro/` is missing, so it's made with its marker before the deploy dir is cloned into it,
	// and every other path, like the deploy dir and its `.git`, is a directory
	vi.mocked(stat).mockImplementation(async (path) =>
		path === '.gro' || path.includes('.gro_marker')
			? null
			: { ...to_mock_file_info(), kind: 'directory' }
	);
	vi.mocked(write).mockResolvedValue('created');
};

/**
 * The `origin` remote's URL in the mocked project and in the deploy clones Gro makes.
 */
export const ORIGIN_URL = 'git@example.com:me/project.git';

/**
 * Whether a mocked `spawn` call is the deploy dir's `git clone`.
 */
export const is_clone_call = (command: string, args: ReadonlyArray<string> = []): boolean =>
	command === 'git' && args[0] === 'clone';

/**
 * The mocked `spawn` calls that cloned the deploy dir, each with its call order.
 */
export const get_clone_calls = async (): Promise<
	Array<{ args: ReadonlyArray<string>; order: number }>
> => {
	const { git_command: spawn } = git_ops;
	return spawn.mock.calls.flatMap(([command, args], i) =>
		is_clone_call(command, args) ? [{ args: args!, order: spawn.mock.invocationCallOrder[i]! }] : []
	);
};

/**
 * A spawn result for a command that exited with `code`.
 */
export const to_spawn_result = (code = 0): SpawnResult => ({ kind: 'exited', code });

/**
 * Mocks the git commands with no operation of their own for a successful deploy, with `fail` returning a failed result
 * for the commands it matches; `git diff --cached --quiet` exits 1, so the build has changes to commit,
 * and `git ls-files` prints `ignored_paths` NUL-separated, the files the deploy clone ignores.
 * An existing deploy dir is a clone Gro made: `git rev-parse --show-toplevel` prints the cwd
 * it runs in, `--absolute-git-dir` the cwd's own `.git`, `git config --get` prints `true` for its deploy-clone tag,
 * and `git remote get-url` prints `ORIGIN_URL` in the project and the deploy dir alike.
 */
export const mock_spawn_results = async (
	fail?: (command: string, args: ReadonlyArray<string>) => boolean,
	ignored_paths: ReadonlyArray<string> = []
) => {
	const { git_command: spawn, git_command_out: spawn_out } = git_ops;
	spawn.mockImplementation(async (command, args = []) => {
		if (fail?.(command, args)) return to_spawn_result(1);
		if (command === 'git' && args[0] === 'diff') return to_spawn_result(1);
		return to_spawn_result(0);
	});
	spawn_out.mockImplementation(async (command, args = [], options) => {
		if (fail?.(command, args))
			return { result: to_spawn_result(1), stdout: '', stderr: 'fatal: boom\n' };
		const stdout =
			command !== 'git'
				? ''
				: args[0] === 'ls-files'
					? ignored_paths.map((p) => p + '\0').join('')
					: args[0] === 'rev-parse'
						? String(options?.cwd) + (args[1] === '--absolute-git-dir' ? '/.git\n' : '\n')
						: args[0] === 'remote'
							? ORIGIN_URL + '\n'
							: args[0] === 'config'
								? 'true\n'
								: '';
		return { result: to_spawn_result(0), stdout, stderr: '' };
	});
};

/**
 * Sets up common spawn mock for successful scenarios.
 */
export const setup_successful_spawn_mock = async () => {
	await mock_spawn_results();
};

/**
 * Expects the deploy dir to have been cloned once, from the cwd's `branch`.
 */
export const expect_cloned = async (branch: string, deploy_dir = '.gro/deploy') => {
	const calls = await get_clone_calls();
	expect(calls.map((c) => c.args)).toEqual([
		[
			'clone',
			'-b',
			branch,
			'--single-branch',
			'--end-of-options',
			process.cwd(),
			resolve(deploy_dir)
		]
	]);
};
