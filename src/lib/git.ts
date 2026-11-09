/**
 * Git over a `SpawnProgram`: each function takes the `git` handle a composition root built
 * with `to_git_program`, and a `cwd` handle in its options. The root decides the
 * environment and `git` is pinned, so nothing here reads `PATH` or the host's environment.
 *
 * A read of the local repo captures with `out`, where a child with no inherited stdio leads
 * its own group and loses the terminal. Anything that may prompt runs with `run`, which stays
 * in the terminal's foreground group so `/dev/tty` still reaches ssh, a credential helper,
 * or a gpg pinentry: every `fetch`, `pull`, `push`, `ls-remote`, `checkout`, `commit`,
 * and `reset`. `branch -D` can't prompt, and runs with `run` so its output reaches the user.
 * No function here parses a remote call's output.
 *
 * A name from a caller goes into argv after `--end-of-options`, so one beginning with `-`
 * can't be read as an option like `--upload-pack`. `git pull` drops that marker before it
 * calls `git fetch`, so `git_pull` refuses such a name itself.
 *
 * @module
 */

import * as s from '@fuzdev/fuz_schema/schema.ts';

import { spawn_result_is_ok, to_spawn_result_message } from './cli.ts';
import type {
	Spawn,
	SpawnCallOptions,
	SpawnOutput,
	SpawnProgram,
	SpawnProgramOptions
} from './spawn.ts';
import { TaskError } from './task.ts';

/**
 * The host variables a `git` that only reads the local repo needs beyond the base set.
 * `GIT_*` carries what a repo reached through a hook or a worktree is (`GIT_DIR`,
 * `GIT_INDEX_FILE`) and `XDG_CONFIG_HOME` where the user's config is. `SUDO_UID` is the owner
 * git's `safe.directory` check accepts when the effective user is root, so under `sudo` the
 * invoking user's repo isn't refused as dubiously owned.
 * A `git` that commits or reaches a remote gets more, decided at its root.
 */
export const GIT_LOCAL_PASS_ENV: ReadonlyArray<string> = ['GIT_*', 'XDG_CONFIG_HOME', 'SUDO_UID'];

/**
 * A git remote's name, such as `origin`.
 */
export const GitOrigin = s.flavor(s.string, 'GitOrigin');
export type GitOrigin = s.Infer<typeof GitOrigin>;

/**
 * A git branch's name.
 */
export const GitBranch = s.flavor(s.string, 'GitBranch');
export type GitBranch = s.Infer<typeof GitBranch>;

/**
 * Finds `git` for a composition root, on `PATH` only, so a `node_modules/.bin/git` can't
 * shadow it.
 * @param spawn - the root's spawn host module
 * @param options - the program's environment, fixed when its handle is made
 * @throws `TaskError` - if `git` isn't on `PATH`
 */
export const to_git_program = (spawn: Spawn, options?: SpawnProgramOptions): SpawnProgram => {
	const git = spawn.find('git', options);
	if (!git) throw new TaskError('git not found on PATH: install git to run this task');
	return git;
};

/**
 * Captures a local read. A program stopped before it finished, or whose output was cut,
 * has no output to read a result from, so it throws rather than letting partial output
 * pass for a clean answer.
 * @throws Error - if `git` was stopped rather than exiting, or its output was truncated
 */
const git_out = async (
	git: SpawnProgram,
	args: ReadonlyArray<string>,
	options?: SpawnCallOptions
): Promise<SpawnOutput> => {
	const output = await git.out(args, options);
	if (output.kind !== 'exited') {
		throw Error(`\`git ${args[0]}\` ended by ${to_spawn_result_message(output)}`);
	}
	if (output.truncated) {
		throw Error(`\`git ${args[0]}\` output was truncated, so it can't be read as a result`);
	}
	return output;
};

/**
 * Returns the current git branch name, or the literal string `HEAD` on a detached `HEAD`
 * or an unborn branch, which callers comparing it to a branch name read as not that branch.
 * @throws Error - if `git` prints no name, as outside a repo, or was stopped or truncated
 */
export const git_current_branch_name = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<GitBranch> => {
	const { stdout } = await git_out(git, ['rev-parse', '--abbrev-ref', 'HEAD'], options);
	if (!stdout) throw Error('git_current_branch_name failed');
	return stdout.trim();
};

/**
 * Checks if a remote git branch exists. Contacts the remote, so it may prompt.
 * @throws Error - if `git ls-remote` fails for a reason other than the branch not existing
 */
export const git_remote_branch_exists = async (
	git: SpawnProgram,
	origin: GitOrigin = 'origin',
	branch?: GitBranch,
	options?: SpawnCallOptions
): Promise<boolean> => {
	const final_branch = branch ?? (await git_current_branch_name(git, options));
	const result = await git.run(
		[
			'ls-remote',
			'--exit-code',
			'--heads',
			'--end-of-options',
			origin,
			'refs/heads/' + final_branch
		],
		options
	);
	if (spawn_result_is_ok(result)) return true;
	if (result.kind === 'exited' && result.code === 2) return false;
	throw Error(
		`git_remote_branch_exists failed for origin '${origin}' and branch '${
			final_branch
		}' with ${to_spawn_result_message(result)}`
	);
};

/**
 * Checks if a local git branch exists.
 */
export const git_local_branch_exists = async (
	git: SpawnProgram,
	branch: GitBranch,
	options?: SpawnCallOptions
): Promise<boolean> => {
	const output = await git_out(
		git,
		['show-ref', '--quiet', '--end-of-options', 'refs/heads/' + branch],
		options
	);
	return spawn_result_is_ok(output);
};

/**
 * Git workspace status flags indicating which types of changes are present.
 */
export interface GitWorkspaceStatus {
	unstaged_changes: boolean;
	staged_changes: boolean;
	untracked_files: boolean;
}

/**
 * Parses the output of `git status --porcelain -z` (v1 format) into a status object.
 *
 * Format: `XY path\0` where `X` is the staged status (index), `Y` the unstaged status
 * (work tree), and `path` is unescaped, with renames and copies as `XY new\0old\0`.
 * Submodule-specific status codes are interpreted as changes.
 *
 * @param stdout - the raw output from `git status --porcelain -z`
 */
export const git_parse_workspace_status = (stdout: string | null): GitWorkspaceStatus => {
	const entries = stdout?.split('\0').filter(Boolean) ?? [];

	// a rename or copy is followed by the old filename's entry, skipped here
	let skip_next = false;
	const lines: Array<string> = [];

	for (const entry of entries) {
		if (skip_next) {
			skip_next = false;
			continue;
		}
		if (
			entry.length >= 3 &&
			(entry[0] === 'R' || entry[0] === 'C' || entry[1] === 'R' || entry[1] === 'C')
		) {
			skip_next = true;
		}
		lines.push(entry);
	}

	return {
		// Y position, any non-space, non-?, non-!
		unstaged_changes: lines.some(
			(line) => line.length >= 3 && line[1] !== ' ' && line[1] !== '?' && line[1] !== '!'
		),
		// X position, any non-space, non-?, non-!
		staged_changes: lines.some(
			(line) => line.length >= 3 && line[0] !== ' ' && line[0] !== '?' && line[0] !== '!'
		),
		untracked_files: lines.some((line) => line.startsWith('??'))
	};
};

/**
 * Whether a failed git command's stderr says the directory isn't a repo. Git exits the same for
 * a repo it refuses, like one with a corrupt index, a bad config, or dubious ownership, and
 * `git rev-parse --git-dir` fails alike on those last two, so the message tells them apart.
 * Only discovery's wording counts, `(or any of the parent directories)` and its mount-point
 * form: `fatal: not a git repository: <path>` names a broken submodule, worktree, or `GIT_DIR`,
 * which must not read as clean. The base environment's `LC_ALL=C` keeps it in English; a
 * `pass_env: 'all'` program with a translated locale reads as a refused repo, the safe way to
 * be wrong.
 */
const is_not_a_repo_error = (stderr: string): boolean =>
	/^fatal: not a git repository \(or any /m.test(stderr);

/**
 * Checks the git workspace status using a single `git status --porcelain -z` call.
 * A directory that isn't a repo reports no changes.
 * @throws `TaskError` - if it failed on a repo it couldn't read, which must not read as clean
 * @throws Error - if `git` was stopped rather than exiting, or its output was truncated
 */
export const git_check_workspace = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<GitWorkspaceStatus> => {
	const output = await git_out(git, ['status', '--porcelain', '-z'], options);
	if (spawn_result_is_ok(output)) return git_parse_workspace_status(output.stdout);
	if (is_not_a_repo_error(output.stderr)) return git_parse_workspace_status(null);
	throw new TaskError(
		`git status failed with ${to_spawn_result_message(output)}: ${output.stderr.trim()}`
	);
};

/**
 * @returns `true` if the workspace has no changes at all
 */
export const git_workspace_is_clean = (status: GitWorkspaceStatus): boolean =>
	!status.unstaged_changes && !status.staged_changes && !status.untracked_files;

/**
 * @returns `true` if the workspace has no unstaged changes and no untracked files (staged changes are OK)
 */
export const git_workspace_is_fully_staged = (status: GitWorkspaceStatus): boolean =>
	!status.unstaged_changes && !status.untracked_files;

/**
 * Converts a workspace status to a human-readable message.
 */
export const git_workspace_status_message = (status: GitWorkspaceStatus): string => {
	if (git_workspace_is_clean(status)) return 'workspace is clean';
	const issues: Array<string> = [];
	if (status.unstaged_changes) issues.push('unstaged changes');
	if (status.staged_changes) issues.push('staged but uncommitted changes');
	if (status.untracked_files) issues.push('untracked files');
	return `git has ${issues.join(', ')}`;
};

/**
 * @returns an error message if the git workspace has any unstaged or uncommitted changes, or `null` if it's clean
 */
export const git_check_clean_workspace = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<string | null> => {
	const status = await git_check_workspace(git, options);
	return git_workspace_is_clean(status) ? null : git_workspace_status_message(status);
};

/**
 * @returns an error message if the git workspace has any unstaged changes or untracked files, or `null` if fully staged
 */
export const git_check_fully_staged_workspace = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<string | null> => {
	const status = await git_check_workspace(git, options);
	return git_workspace_is_fully_staged(status) ? null : git_workspace_status_message(status);
};

/**
 * Calls `git fetch`. Contacts the remote, so it may prompt.
 * @param branch - a branch name or a refspec like `+deploy:deploy`
 * @throws Error - if the underlying git command fails
 */
export const git_fetch = async (
	git: SpawnProgram,
	origin: GitOrigin = 'origin',
	branch?: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	const args = ['fetch', '--end-of-options', origin];
	if (branch) args.push(branch);
	const result = await git.run(args, options);
	if (!spawn_result_is_ok(result)) {
		throw Error(
			`git_fetch failed for origin '${origin}' and branch '${
				branch
			}' with ${to_spawn_result_message(result)}`
		);
	}
};

/**
 * Calls `git checkout`.
 * @returns the previous branch name, if it changed
 * @throws Error - if the underlying git command fails
 */
export const git_checkout = async (
	git: SpawnProgram,
	branch: GitBranch,
	options?: SpawnCallOptions
): Promise<GitBranch | null> => {
	const current_branch = await git_current_branch_name(git, options);
	if (branch === current_branch) return null;
	const result = await git.run(['checkout', '--end-of-options', branch], options);
	if (!spawn_result_is_ok(result)) {
		throw Error(
			`git_checkout failed for branch '${branch}' with ${to_spawn_result_message(result)}`
		);
	}
	return current_branch;
};

/**
 * Refuses a name `git pull` would read as an option, as `--end-of-options` can't help it.
 * @throws Error - if `value` begins with `-`
 */
const assert_not_pull_option = (value: string, what: string): void => {
	if (value.startsWith('-')) {
		throw Error(`git_pull refuses the ${what} '${value}' for beginning with -`);
	}
};

/**
 * Calls `git pull`. Contacts the remote, so it may prompt.
 * @throws Error - if the underlying git command fails, or a name begins with `-`
 */
export const git_pull = async (
	git: SpawnProgram,
	origin: GitOrigin = 'origin',
	branch?: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	assert_not_pull_option(origin, 'origin');
	if (branch) assert_not_pull_option(branch, 'branch');
	const args = ['pull', origin];
	if (branch) args.push(branch);
	const result = await git.run(args, options);
	if (!spawn_result_is_ok(result)) {
		throw Error(`git_pull failed for branch '${branch}' with ${to_spawn_result_message(result)}`);
	}
};

/**
 * Calls `git push`. Contacts the remote, so it may prompt.
 * @throws Error - if the underlying git command fails
 */
export const git_push = async (
	git: SpawnProgram,
	origin: GitOrigin,
	branch?: GitBranch,
	options?: SpawnCallOptions,
	set_upstream = false
): Promise<void> => {
	const final_branch = branch ?? (await git_current_branch_name(git, options));
	const args = ['push'];
	if (set_upstream) args.push('-u');
	args.push('--end-of-options', origin, final_branch);
	const result = await git.run(args, options);
	if (!spawn_result_is_ok(result)) {
		throw Error(
			`git_push failed for branch '${final_branch}' with ${to_spawn_result_message(result)}`
		);
	}
};

/**
 * Calls `git push`, setting upstream if the remote branch does not yet exist.
 * @throws Error - if the underlying git command fails
 */
export const git_push_to_create = async (
	git: SpawnProgram,
	origin: GitOrigin = 'origin',
	branch?: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	const final_branch = branch ?? (await git_current_branch_name(git, options));
	const args = ['push'];
	if (!(await git_remote_branch_exists(git, origin, final_branch, options))) args.push('-u');
	args.push('--end-of-options', origin, final_branch);
	const result = await git.run(args, options);
	if (!spawn_result_is_ok(result)) {
		throw Error(
			`git_push failed for branch '${final_branch}' with ${to_spawn_result_message(result)}`
		);
	}
};

/**
 * Deletes a branch locally.
 * @throws Error - if the underlying git command fails
 */
export const git_delete_local_branch = async (
	git: SpawnProgram,
	branch: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	const result = await git.run(['branch', '-D', '--end-of-options', branch], options);
	if (!spawn_result_is_ok(result)) {
		throw Error(
			`git_delete_local_branch failed for branch '${branch}' with ${to_spawn_result_message(
				result
			)}`
		);
	}
};

/**
 * Returns the hash of the commit `HEAD` points at, or `null` when there is none:
 * an unborn branch, or a directory that isn't a repo.
 * A detached `HEAD` reads as its commit.
 * @throws Error - if `git` was stopped rather than exiting, or its output was truncated
 */
export const git_current_commit_hash = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<string | null> => {
	const { stdout } = await git_out(
		git,
		['rev-parse', '--verify', '--quiet', 'HEAD^{commit}'],
		options
	);
	return stdout.trim() || null;
};

/**
 * Returns the hash of the current branch's first commit.
 * @throws Error - if the underlying git command fails
 */
export const git_current_branch_first_commit_hash = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<string> => {
	const { stdout } = await git_out(
		git,
		['rev-list', '--max-parents=0', '--abbrev-commit', 'HEAD'],
		options
	);
	if (!stdout) throw Error('git_current_branch_first_commit_hash failed');
	return stdout.trim();
};

/**
 * Returns the global git config setting for `pull.rebase`.
 */
export const git_check_setting_pull_rebase = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<boolean> => {
	const { stdout } = await git_out(git, ['config', '--global', 'pull.rebase'], options);
	return stdout.trim() === 'true';
};
