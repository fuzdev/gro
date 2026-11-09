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
 * A branch or tag name from a caller passes `assert_git_ref_name` before it reaches git, so it
 * is never a refspec like `:deploy` or `a:b`, a pattern like `ma*`, or a shorthand like `@{-1}`.
 * It's still a name git resolves, so `git_checkout` detaches `HEAD` for a tag or a short hash.
 * An origin goes into argv after `--end-of-options`, as a ref name does
 * too, so one beginning with `-` can't be read as an option like `--upload-pack`. `git pull`
 * drops that marker before it calls `git fetch`, so `git_pull` refuses such an origin itself.
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
export const GIT_LOCAL_PASS_ENV: ReadonlyArray<string> = Object.freeze([
	'GIT_*',
	'XDG_CONFIG_HOME',
	'SUDO_UID'
]);

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
 * Whether `name` is one plain ref name, by the rules of `git check-ref-format --branch` plus
 * refusals for names it allows but a command reads as more than a branch: a leading `+`,
 * which forces a refspec, a leading `refs/`, which names a ref of any kind, and `@` and
 * `@{-1}`-style shorthands, which that command expands. A branch can still have a name like `feature/a.b`.
 */
const is_plain_git_ref_name = (name: string): boolean => {
	if (name === 'HEAD' || name === '@' || /^(?:[-+]|refs\/)/.test(name)) return false;
	if (/\.\.|@\{|[~^:?*[\\]|\.$/.test(name)) return false;
	for (let i = 0; i < name.length; i++) {
		const code = name.charCodeAt(i);
		if (code <= 32 || code === 127) return false;
	}
	return name
		.split('/')
		.every((part) => part !== '' && !part.startsWith('.') && !part.endsWith('.lock'));
};

/**
 * Refuses a name git would read as more than one ref. Every function here taking a ref name
 * from a caller goes through this one check.
 * @throws `TaskError` - if `name` isn't one plain ref name
 */
const assert_git_ref_name = (name: string, what: 'branch' | 'tag'): void => {
	if (is_plain_git_ref_name(name)) return;
	if (name === 'HEAD' && what === 'branch') {
		throw new TaskError(
			'gro refuses the git branch HEAD: it names no branch, and a detached HEAD has none, so check out a branch first'
		);
	}
	throw new TaskError(
		`gro refuses the git ${what} '${name}': it must be legal by \`git check-ref-format\`,` +
			' not `HEAD` or `@`, without `@{`, and not begin with -, +, or refs/,' +
			' so git reads it as one name and nothing more'
	);
};

/**
 * The branch a function acts on, `branch` or else the current one, checked as a ref name.
 * @throws `TaskError` - if it isn't one plain ref name, or the current branch is unborn or detached
 */
const to_final_branch = async (
	git: SpawnProgram,
	branch: GitBranch | undefined,
	options: SpawnCallOptions | undefined
): Promise<GitBranch> => {
	const final_branch = branch ?? (await git_current_branch_name(git, options));
	assert_git_ref_name(final_branch, 'branch');
	return final_branch;
};

/**
 * Returns the current git branch name, or the literal string `HEAD` on a detached `HEAD`,
 * which callers comparing it to a branch name read as not that branch.
 * @throws `TaskError` - on an unborn branch, one with no commits yet
 * @throws Error - if `git` prints no name, as outside a repo, or was stopped or truncated
 */
export const git_current_branch_name = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<GitBranch> => {
	const output = await git_out(git, ['rev-parse', '--abbrev-ref', 'HEAD'], options);
	const name = output.stdout.trim();
	if (spawn_result_is_ok(output) && name) return name;
	// on an unborn branch `rev-parse` prints its argument back, then fails to resolve it
	if (name === 'HEAD') {
		throw new TaskError(
			'the current git branch is unborn, with no commits yet: make a first commit, or check out a branch that has one'
		);
	}
	throw Error('git_current_branch_name failed');
};

/**
 * Checks if a remote git branch exists. Contacts the remote, so it may prompt.
 * @throws `TaskError` - if the branch isn't one plain ref name, as on a detached `HEAD`
 * @throws Error - if `git ls-remote` fails for a reason other than the branch not existing
 */
export const git_remote_branch_exists = async (
	git: SpawnProgram,
	origin: GitOrigin = 'origin',
	branch?: GitBranch,
	options?: SpawnCallOptions
): Promise<boolean> => {
	const final_branch = await to_final_branch(git, branch, options);
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
 * @throws `TaskError` - if `branch` isn't one plain ref name
 */
export const git_local_branch_exists = async (
	git: SpawnProgram,
	branch: GitBranch,
	options?: SpawnCallOptions
): Promise<boolean> => {
	assert_git_ref_name(branch, 'branch');
	const output = await git_out(
		git,
		['show-ref', '--quiet', '--end-of-options', 'refs/heads/' + branch],
		options
	);
	return spawn_result_is_ok(output);
};

/**
 * @returns `true` if the local tag exists
 * @throws `TaskError` - if `tag` isn't one plain ref name
 * @throws Error - if `git` was stopped rather than exiting
 */
export const git_tag_exists = async (
	git: SpawnProgram,
	tag: string,
	options?: SpawnCallOptions
): Promise<boolean> => {
	assert_git_ref_name(tag, 'tag');
	const output = await git_out(
		git,
		['show-ref', '--quiet', '--end-of-options', 'refs/tags/' + tag],
		options
	);
	return spawn_result_is_ok(output);
};

/**
 * @returns the subject line of the commit `HEAD` points at, or `null` when there is none
 * @throws Error - if `git` was stopped rather than exiting, or its output was truncated
 */
export const git_head_subject = async (
	git: SpawnProgram,
	options?: SpawnCallOptions
): Promise<string | null> => {
	const output = await git_out(git, ['log', '-1', '--format=%s'], options);
	return spawn_result_is_ok(output) ? output.stdout.trim() : null;
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

const run_git_fetch = async (
	git: SpawnProgram,
	origin: GitOrigin,
	refspec: string | undefined,
	options: SpawnCallOptions | undefined
): Promise<void> => {
	const args = ['fetch', '--end-of-options', origin];
	if (refspec !== undefined) args.push(refspec);
	const result = await git.run(args, options);
	if (!spawn_result_is_ok(result)) {
		throw Error(
			`git_fetch failed for origin '${origin}' and branch '${
				refspec
			}' with ${to_spawn_result_message(result)}`
		);
	}
};

/**
 * Calls `git fetch`. Contacts the remote, so it may prompt.
 * @throws `TaskError` - if `branch` isn't one plain ref name
 * @throws Error - if the underlying git command fails
 */
export const git_fetch = async (
	git: SpawnProgram,
	origin: GitOrigin = 'origin',
	branch?: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	if (branch !== undefined) assert_git_ref_name(branch, 'branch');
	await run_git_fetch(git, origin, branch, options);
};

/**
 * Sets the local `branch` to the remote's, even when that drops commits only the local one has,
 * as the refspec `+branch:branch`. Contacts the remote, so it may prompt.
 * @throws `TaskError` - if `branch` isn't one plain ref name
 * @throws Error - if the underlying git command fails
 */
export const git_fetch_force = async (
	git: SpawnProgram,
	origin: GitOrigin,
	branch: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	assert_git_ref_name(branch, 'branch');
	await run_git_fetch(git, origin, `+${branch}:${branch}`, options);
};

/**
 * Calls `git checkout`.
 * @returns the previous branch name, if it changed
 * @throws `TaskError` - if `branch` isn't one plain ref name, or the current branch is unborn
 * @throws Error - if the underlying git command fails
 */
export const git_checkout = async (
	git: SpawnProgram,
	branch: GitBranch,
	options?: SpawnCallOptions
): Promise<GitBranch | null> => {
	assert_git_ref_name(branch, 'branch');
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
 * Calls `git pull`. Contacts the remote, so it may prompt.
 * @throws `TaskError` - if `branch` isn't one plain ref name
 * @throws Error - if the underlying git command fails, or `origin` begins with `-`
 */
export const git_pull = async (
	git: SpawnProgram,
	origin: GitOrigin = 'origin',
	branch?: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	if (origin.startsWith('-')) {
		throw Error(`git_pull refuses the origin '${origin}' for beginning with -`);
	}
	if (branch !== undefined) assert_git_ref_name(branch, 'branch');
	const args = ['pull', origin];
	if (branch !== undefined) args.push(branch);
	const result = await git.run(args, options);
	if (!spawn_result_is_ok(result)) {
		throw Error(`git_pull failed for branch '${branch}' with ${to_spawn_result_message(result)}`);
	}
};

/**
 * Calls `git push`. Contacts the remote, so it may prompt.
 * @throws `TaskError` - if the branch isn't one plain ref name, as on a detached `HEAD`
 * @throws Error - if the underlying git command fails
 */
export const git_push = async (
	git: SpawnProgram,
	origin: GitOrigin,
	branch?: GitBranch,
	options?: SpawnCallOptions,
	set_upstream = false
): Promise<void> => {
	const final_branch = await to_final_branch(git, branch, options);
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
 * @throws `TaskError` - if the branch isn't one plain ref name, as on a detached `HEAD`
 * @throws Error - if the underlying git command fails
 */
export const git_push_to_create = async (
	git: SpawnProgram,
	origin: GitOrigin = 'origin',
	branch?: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	const final_branch = await to_final_branch(git, branch, options);
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
 * @throws `TaskError` - if `branch` isn't one plain ref name
 * @throws Error - if the underlying git command fails
 */
export const git_delete_local_branch = async (
	git: SpawnProgram,
	branch: GitBranch,
	options?: SpawnCallOptions
): Promise<void> => {
	assert_git_ref_name(branch, 'branch');
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
