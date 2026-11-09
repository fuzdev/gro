import { print_error } from '@fuzdev/fuz_util/print.ts';
import { styleText as st } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { join, resolve } from 'node:path';

import { is_silent_error, TaskError, type Task } from './task.ts';
import {
	git_check_clean_workspace,
	git_checkout,
	git_local_branch_exists,
	git_remote_branch_exists,
	GitOrigin,
	GitBranch,
	git_delete_local_branch,
	git_push_to_create,
	git_current_branch_first_commit_hash,
	git_pull,
	git_fetch,
	git_check_setting_pull_rebase,
	git_current_branch_name,
	to_git_program
} from './git.ts';
import { paths, print_path } from './paths.ts';
import { GRO_DIRNAME, GIT_DIRNAME, SVELTEKIT_BUILD_DIRNAME } from './constants.ts';
import { fs, type FsDir } from './fs.ts';
import { create_fs_paths, type FsOpenedPath, type FsPaths } from './fs_paths.ts';
import { spawn_result_is_ok } from './cli.ts';
import { spawn, type SpawnCallOptions, type SpawnProgram } from './spawn.ts';
import { is_fs_error } from './fs_error.ts';
import {
	is_protected_dir,
	PROTECTED_DIRS_TEXT,
	to_protected_dirs
} from './internal/fs_boundary.ts';
import { gro_dir_ensure } from './internal/gro_dir.ts';
import { process } from './process.ts';

// docs at ../docs/deploy.md

// terminal command for testing:
// npm run bootstrap && rm -rf .gro && clear && gro deploy --source no-git-workspace --no-build --dry

// TODO customize
const dir = process.cwd();
const INITIAL_FILE_PATH = '.gitkeep';
const DEPLOY_DIR = GRO_DIRNAME + '/deploy';
const SOURCE_BRANCH = 'main';
const TARGET_BRANCH = 'deploy';
const DANGEROUS_BRANCHES = [SOURCE_BRANCH, 'master'];

/** @nodocs */
export const Args = s.object({
	source: s.with_default(
		meta(GitBranch, { description: 'git source branch to build and deploy from' }),
		SOURCE_BRANCH
	),
	target: s.with_default(
		meta(GitBranch, { description: 'git target branch to deploy to' }),
		TARGET_BRANCH
	),
	origin: s.with_default(meta(GitOrigin, { description: 'git origin to deploy to' }), 'origin'),
	deploy_dir: s.with_default(
		meta(s.string, { description: 'the deploy output directory' }),
		DEPLOY_DIR
	),
	build_dir: s.with_default(
		meta(s.string, { description: 'the SvelteKit build directory' }),
		SVELTEKIT_BUILD_DIRNAME
	),
	dry: s.with_default(
		meta(s.boolean, {
			description: 'build and prepare to deploy without actually deploying'
		}),
		false
	),
	force: s.with_default(
		meta(s.boolean, {
			description: 'caution!! destroys the target branch both locally and remotely'
		}),
		false
	),
	dangerous: s.with_default(
		meta(s.boolean, {
			description: 'caution!! enables destruction of branches like main and master'
		}),
		false
	),
	reset: s.with_default(
		meta(s.boolean, {
			description: 'if true, resets the target branch back to the first commit before deploying'
		}),
		false
	),
	build: s.with_default(meta(s.boolean, { description: 'dual of no-build' }), true),
	'no-build': s.with_default(meta(s.boolean, { description: 'opt out of building' }), false),
	sync: s.with_default(meta(s.boolean, { description: 'dual of no-sync' }), true),
	'no-sync': s.with_default(
		meta(s.boolean, { description: 'opt out of gro sync in build' }),
		false
	),
	gen: s.with_default(meta(s.boolean, { description: 'dual of no-gen' }), true),
	'no-gen': s.with_default(meta(s.boolean, { description: 'opt out of gro gen in build' }), false),
	install: s.with_default(meta(s.boolean, { description: 'dual of no-install' }), true),
	'no-install': s.with_default(
		meta(s.boolean, { description: 'opt out of installing packages before building' }),
		false
	),
	force_build: s.with_default(
		meta(s.boolean, { description: 'force a fresh build, ignoring the cache' }),
		false
	),
	pull: s.with_default(meta(s.boolean, { description: 'dual of no-pull' }), true),
	'no-pull': s.with_default(meta(s.boolean, { description: 'opt out of git pull' }), false)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'deploy to a branch',
	Args,
	run: async ({ args, log, invoke_task }): Promise<void> => {
		const {
			source,
			target,
			origin,
			build_dir,
			deploy_dir,
			dry,
			force,
			dangerous,
			reset,
			build,
			sync,
			gen,
			install,
			force_build,
			pull
		} = args;

		const fs_paths = create_fs_paths(fs);
		// a named unconfined phase, `-A`-equivalent: it signs and pushes with the user's git setup
		const git = to_git_program(spawn, { pass_env: 'all' });

		// Checks
		// the deploy dir is removed, emptied, and run git in below, so before anything else,
		// it must be somewhere Gro may own, and if it exists, a clone Gro made
		const resolved_deploy_dir = resolve(deploy_dir);
		await assert_deploy_dir_owned(
			git,
			await open_deploy_dir(fs_paths, resolved_deploy_dir),
			origin
		);
		if (!force && target !== TARGET_BRANCH) {
			throw new TaskError(
				`Warning! You are deploying to a custom target branch '${target}',` +
					` instead of the default '${TARGET_BRANCH}' branch.` +
					` This is destructive to your '${target}' branch!` +
					` If you understand and are OK with deleting your branch '${target}',` +
					` both locally and remotely, pass --force to suppress this error.`
			);
		}
		if (!dangerous && DANGEROUS_BRANCHES.includes(target)) {
			throw new TaskError(
				`Warning! You are deploying to a custom target branch '${target}'` +
					` and that appears very dangerous: it is destructive to your '${target}' branch!` +
					` If you understand and are OK with deleting your branch '${target}',` +
					` both locally and remotely, pass --dangerous to suppress this error.`
			);
		}
		const clean_error_message = await git_check_clean_workspace(git);
		if (clean_error_message) {
			throw new TaskError(
				'Deploy failed because the git workspace has uncommitted changes: ' + clean_error_message
			);
		}
		if (!(await git_check_setting_pull_rebase(git))) {
			throw new TaskError(
				'Deploying currently requires `git config --global pull.rebase true`,' +
					' but this restriction could be lifted with more work'
			);
		}

		// Fetch the source branch in the cwd if it's not there
		if (!(await git_local_branch_exists(git, source))) {
			await git_fetch(git, origin, source);
		}

		// Prepare the source branch in the cwd
		await git_checkout(git, source);
		if (pull) {
			await git_pull(git, origin, source);
		}
		if (await git_check_clean_workspace(git)) {
			throw new TaskError(
				'Deploy failed because the local source branch is out of sync with the remote one,' +
					' finish rebasing manually or reset with `git rebase --abort`'
			);
		}

		// Prepare the target branch remotely and locally
		// resolved at each check, since the steps below remove and recreate the directory
		const deploy_dir_exists = async (): Promise<boolean> => {
			const opened = await fs_paths.open_path(resolved_deploy_dir);
			return opened.dir.exists(opened.path);
		};
		// reopened for each call, since the steps below remove and recreate the directory
		const to_target_cwd = async (): Promise<SpawnCallOptions> => ({
			cwd: await open_deploy_cwd(fs_paths, resolved_deploy_dir)
		});
		const remote_target_exists = await git_remote_branch_exists(git, origin, target);
		if (remote_target_exists) {
			// Remote target branch already exists, so sync up efficiently

			// First, check if the deploy dir exists, and if so, attempt to sync it.
			// If anything goes wrong, delete the directory and we'll initialize it
			// using the same code path as if it didn't exist in the first place.
			if (await deploy_dir_exists()) {
				if (target !== (await git_current_branch_name(git, await to_target_cwd()))) {
					// We're in a bad state because the target branch has changed,
					// so delete the directory and continue as if it wasn't there.
					await fs_paths.remove_path(resolved_deploy_dir, { recursive: true });
				} else {
					// in case it's dirty
					const reset_result = await git.run(['reset', '--hard'], await to_target_cwd());
					if (!spawn_result_is_ok(reset_result)) {
						// We're in a bad state because the deploy dir can't be reset,
						// so delete the directory and continue as if it wasn't there.
						log.warn(
							'`git reset --hard` failed in',
							print_path(resolved_deploy_dir),
							'- recreating it'
						);
						await fs_paths.remove_path(resolved_deploy_dir, { recursive: true });
					} else if (!reset) {
						// Skip pulling target branch when resetting (optimization - we reset after anyway)
						await git_pull(git, origin, target, await to_target_cwd());
						if (await git_check_clean_workspace(git, await to_target_cwd())) {
							// We're in a bad state because the local branch lost continuity with the remote,
							// so delete the directory and continue as if it wasn't there.
							await fs_paths.remove_path(resolved_deploy_dir, { recursive: true });
						}
					}
				}
			}

			// Second, initialize the deploy dir if needed.
			// It may not exist, or it may have been deleted after failing to sync above.
			if (!(await deploy_dir_exists())) {
				const local_deploy_branch_exists = await git_local_branch_exists(git, target);
				await git_fetch(git, origin, '+' + target + ':' + target); // fetch+merge and allow non-fastforward updates with the +
				await clone_deploy_dir(git, fs_paths, origin, target, resolved_deploy_dir);
				// Clean up if we created the target branch in the cwd
				if (!local_deploy_branch_exists) {
					await git_delete_local_branch(git, target);
				}
			}

			// Local target branch is now synced with remote, but do we need to reset?
			// The reset is local, so a dry deploy pushes nothing, and the final force push publishes it.
			if (reset) {
				const first_commit_hash = await git_current_branch_first_commit_hash(
					git,
					await to_target_cwd()
				);
				const reset_result = await git.run(
					['reset', '--hard', first_commit_hash],
					await to_target_cwd()
				);
				if (!spawn_result_is_ok(reset_result)) {
					throw new TaskError(
						`\`git reset --hard ${first_commit_hash}\` failed resetting the target branch '${target}'` +
							` in ${print_path(resolved_deploy_dir)}, so nothing was pushed - see the output above,` +
							' resolve the failure, then deploy again'
					);
				}
			}
		} else {
			// Remote target branch does not exist, so start from scratch

			// Delete the deploy dir and recreate it
			if (await deploy_dir_exists()) {
				await fs_paths.remove_path(resolved_deploy_dir, { recursive: true });
				const recreated = await fs_paths.open_path(resolved_deploy_dir);
				await recreated.dir.mkdir(recreated.path, { recursive: true });
			}

			// Delete the target branch locally in the cwd if it exists,
			// which a dry deploy leaves alone, as everything below happens in the deploy clone
			if (!dry && (await git_local_branch_exists(git, target))) {
				await git_delete_local_branch(git, target);
			}

			// Create the target branch locally and remotely.
			// This is more complex to avoid churning the cwd.
			await clone_deploy_dir(git, fs_paths, origin, source, resolved_deploy_dir);
			const to_init_error = (command: string): TaskError =>
				new TaskError(
					`\`${command}\` failed creating the target branch '${target}'` +
						` in ${print_path(resolved_deploy_dir)}, so nothing was pushed - see the output above,` +
						' resolve the failure, then deploy again'
				);
			const git_init = async (git_args: Array<string>): Promise<void> => {
				const result = await git.run(git_args, await to_target_cwd());
				if (!spawn_result_is_ok(result)) throw to_init_error(`git ${git_args.join(' ')}`);
			};
			await git_init(['checkout', '--orphan', target]);
			// TODO there's definitely a better way to do this
			await git_init(['rm', '-rf', '.']);
			try {
				const initial = await open_deploy_dir(fs_paths, resolved_deploy_dir);
				await initial.dir.write(join(initial.path, INITIAL_FILE_PATH), '');
			} catch (error) {
				if (!is_fs_error(error)) throw error;
				log.error(error.message);
				throw to_init_error(`writing ${INITIAL_FILE_PATH}`);
			}
			await git_init(['add', '--', INITIAL_FILE_PATH]);
			await git_init(['commit', '-m', 'init']);
			// a dry deploy pushes nothing; a real one creates the remote branch here, with upstream tracking
			if (!dry) await git_push_to_create(git, origin, target, await to_target_cwd());
			await git_delete_local_branch(git, source, await to_target_cwd());
		}

		// Remove everything except .git from the deploy directory to avoid stale files,
		// opened now that it's in its final form, so the emptying never acts on a replaced directory
		const deploy = await open_deploy_dir(fs_paths, resolved_deploy_dir);
		const stale: Array<string> = [];
		for await (const { name } of deploy.dir.list(deploy.path)) {
			if (name !== GIT_DIRNAME) stale.push(name);
		}
		await Promise.all(
			stale.map((name) => deploy.dir.remove(join(deploy.path, name), { recursive: true }))
		);

		// Build
		try {
			if (build) {
				await invoke_task('build', { sync, gen, install, force_build });
			}
		} catch (error) {
			log.error(
				st('red', 'build failed'),
				// the only push before the build creates a remote target branch that didn't exist
				...(remote_target_exists || dry
					? ['but', st('green', 'nothing was pushed')]
					: [
							'and',
							st('yellow', `the new remote branch '${target}' has only its init commit`),
							'- nothing was deployed'
						]),
				// a `SilentError` means the build's failure is already reported above
				...(is_silent_error(error) ? [] : [print_error(error)])
			);
			if (dry) {
				log.info(st('red', 'dry deploy failed'));
			}
			throw new TaskError(`Deploy safely canceled due to build failure. See the error above.`);
		}

		// Verify build output exists
		const build_output = await fs_paths.open_path(build_dir);
		if (!(await build_output.dir.exists(build_output.path))) {
			throw new TaskError(`Directory to deploy does not exist after building: ${build_dir}`);
		}

		// Copy the build
		const build_entries: Array<string> = [];
		for await (const { name } of build_output.dir.list(build_output.path)) {
			build_entries.push(name);
		}
		await Promise.all(
			build_entries.map((name) =>
				fs_paths.copy_path(join(build_dir, name), join(resolved_deploy_dir, name), {
					recursive: true
				})
			)
		);

		// At this point, `dist/` is ready to be committed and deployed!
		if (dry) {
			log.info(st('green', 'dry deploy complete:'), 'files at', print_path(resolved_deploy_dir));
			return;
		}

		// Commit and push
		const bad_state_message = (step: string): string =>
			`Deploy failed in a bad state: built but not pushed: \`${step}\` failed` +
			` in ${print_path(resolved_deploy_dir)} - see the output above,` +
			` resolve the failure, then deploy again`;
		// no `-f`: the deploy dir is its own clone, so only its own ignore rules apply -
		// a `.gitignore` the build emits and the user's global excludes - and they're respected
		const commit_cwd = await to_target_cwd();
		const add_result = await git.run(['add', '.'], commit_cwd);
		if (!spawn_result_is_ok(add_result)) {
			throw new TaskError(bad_state_message('git add'));
		}
		// fail loud on what the add skipped: without `--directory`, files in ignored directories
		// are listed one by one, and `-z` keeps names unquoted
		const ignored = await git.out(
			['ls-files', '-z', '--others', '--ignored', '--exclude-standard'],
			commit_cwd
		);
		if (!spawn_result_is_ok(ignored)) {
			if (ignored.stderr) log.error(ignored.stderr.trimEnd());
			throw new TaskError(bad_state_message('git ls-files --others --ignored'));
		}
		const ignored_paths = ignored.stdout.split('\0').filter(Boolean);
		if (ignored_paths.length) {
			log.warn(
				st('yellow', 'not deploying files the deploy clone ignores') +
					' (a `.gitignore` in the build output or your global git excludes):\n' +
					ignored_paths.map((p) => '  ' + p).join('\n') +
					'\nto deploy them, remove the matching rule' +
					` - \`git check-ignore -v <path>\` in ${print_path(resolved_deploy_dir)} names it`
			);
		}
		// `git diff --cached --quiet` exits 0 with nothing staged and 1 with changes,
		// so a redeploy of an identical build commits nothing and still pushes
		const diff_result = await git.run(['diff', '--cached', '--quiet'], commit_cwd);
		const changed = !spawn_result_is_ok(diff_result);
		if (changed && !(diff_result.kind === 'exited' && diff_result.code === 1)) {
			throw new TaskError(bad_state_message('git diff --cached --quiet'));
		}
		if (changed) {
			const commit_result = await git.run(['commit', '-m', 'deployment'], commit_cwd);
			if (!spawn_result_is_ok(commit_result)) {
				throw new TaskError(bad_state_message('git commit'));
			}
		}
		// force push because we may be resetting the branch, see the checks above to make this safer
		const push_result = await git.run(
			['push', '-f', '--end-of-options', origin, target],
			commit_cwd
		);
		if (!spawn_result_is_ok(push_result)) {
			throw new TaskError(bad_state_message(`git push ${origin} ${target} -f`));
		}

		log.info(st('green', changed ? 'deployed' : 'deployed with no changes'));
	}
};

/**
 * Opens the deploy dir, following its symlinks, and refuses one where Gro may own nothing:
 * the project, `$HOME`, or an ancestor of either, or a directory inside the project
 * other than one strictly beneath `.gro/`, where the project's own files are.
 */
const open_deploy_dir = async (
	fs_paths: FsPaths,
	resolved_deploy_dir: string
): Promise<FsOpenedPath> => {
	const opened = await fs_paths.open_path(resolved_deploy_dir);
	const real = to_opened_real_path(opened);
	if (is_protected_dir(real, await to_protected_dirs(fs_paths.project.path))) {
		throw new TaskError(
			`The deploy dir ${print_path(resolved_deploy_dir)} resolves to ${real},` +
				` ${PROTECTED_DIRS_TEXT}`
		);
	}
	if (opened.dir === fs_paths.project) {
		const segments = opened.path.split('/').filter(Boolean);
		if (segments[0] !== GRO_DIRNAME || segments.length < 2) {
			throw new TaskError(
				`The deploy dir ${print_path(resolved_deploy_dir)} resolves to ${real}, inside the project` +
					` but not beneath \`${GRO_DIRNAME}/\`, so it could hold the project's own files` +
					` - choose a deploy dir beneath \`${GRO_DIRNAME}/\` or outside the project`
			);
		}
	}
	return opened;
};

const join_beneath = (dir_path: string, name: string): string =>
	dir_path ? dir_path.replace(/\/+$/, '') + '/' + name : name;

const to_opened_real_path = (opened: FsOpenedPath): string =>
	opened.path ? join(opened.dir.path, opened.path.replace(/\/+$/, '')) : opened.dir.path;

/**
 * The local git config key `clone_deploy_dir` sets to `true` in each deploy clone it makes,
 * so an ordinary checkout of the project, with the same `origin`, is never taken for one.
 * @nodocs
 */
export const DEPLOY_CLONE_CONFIG_KEY = 'gro.deploy-clone';

/**
 * Refuses an existing deploy dir that isn't a clone Gro made, before anything removes it or
 * runs git in it: it must be a directory and its own repo's top level, not a directory
 * in another repo, with its own `.git` directory as its git dir, carry the local config `DEPLOY_CLONE_CONFIG_KEY` set to `true`, and have
 * the `origin` remote `clone_deploy_dir` sets, the project's. A missing one passes.
 * Gro never removes a refused deploy dir, which the message leaves to the user.
 */
const assert_deploy_dir_owned = async (
	git: SpawnProgram,
	opened: FsOpenedPath,
	origin: GitOrigin
): Promise<void> => {
	if (!(await opened.dir.exists(opened.path))) return;
	const real = to_opened_real_path(opened);
	const refuse = (reason: string): TaskError =>
		new TaskError(
			`The deploy dir ${print_path(real)} exists but isn't a clone \`gro deploy\` made: ${reason}.` +
				' Gro never removes it - if nothing in it is needed, remove it yourself, then deploy again'
		);
	// before any git call, which would run with a file as its cwd
	if ((await opened.dir.stat(opened.path))?.kind !== 'directory') {
		throw refuse('it is not a directory');
	}
	const cwd = opened.path ? opened.dir.dir(opened.path) : opened.dir;
	const toplevel = await git.out(['rev-parse', '--show-toplevel'], { cwd });
	const toplevel_path = spawn_result_is_ok(toplevel) ? toplevel.stdout.trim() : undefined;
	if (!toplevel_path) throw refuse('it is not in a git repo');
	if (toplevel_path !== real) throw refuse(`it is inside the git repo at ${toplevel_path}`);
	// its own `.git` directory, so a layout like a committed bare repo with `core.worktree`,
	// whose config could carry a forged tag, isn't taken for a clone
	const own_git_dir = real + '/' + GIT_DIRNAME;
	const git_dir = await git.out(['rev-parse', '--absolute-git-dir'], { cwd });
	const git_dir_path = spawn_result_is_ok(git_dir) ? git_dir.stdout.trim() : undefined;
	if (git_dir_path !== own_git_dir) {
		throw refuse(`its git dir is ${git_dir_path ?? 'unknown'}, not its own ${own_git_dir}`);
	}
	const git_dir_info = await opened.dir.stat(join_beneath(opened.path, GIT_DIRNAME), {
		follow: false
	});
	if (git_dir_info?.kind !== 'directory') {
		throw refuse(`its ${GIT_DIRNAME} is not a directory`);
	}
	const expected = await to_origin_url(git, origin);
	if (expected === null) {
		throw refuse(`the project has no \`${origin}\` remote to compare it with`);
	}
	const actual = await to_origin_url(git, origin, { cwd });
	if (actual !== expected) {
		throw refuse(
			actual === null
				? `it has no \`${origin}\` remote`
				: `its \`${origin}\` remote is ${actual}, not the project's ${expected}`
		);
	}
	const tag = await git.out(['config', '--local', '--get', DEPLOY_CLONE_CONFIG_KEY], { cwd });
	if (!spawn_result_is_ok(tag) || tag.stdout.trim() !== 'true') {
		throw refuse(
			`it lacks the local git config \`${DEPLOY_CLONE_CONFIG_KEY} true\` that marks a deploy clone` +
				' (a clone from before the mark, which Gro re-clones once it is removed, or another checkout)'
		);
	}
};

const to_origin_url = async (
	git: SpawnProgram,
	origin: GitOrigin,
	options?: SpawnCallOptions
): Promise<string | null> => {
	const output = await git.out(['remote', 'get-url', '--end-of-options', origin], options);
	return (spawn_result_is_ok(output) && output.stdout.trim()) || null;
};

/**
 * Opens the deploy dir as a handle, for a `cwd`.
 */
const open_deploy_cwd = async (fs_paths: FsPaths, resolved_deploy_dir: string): Promise<FsDir> => {
	const opened = await fs_paths.open_path(resolved_deploy_dir);
	return opened.path ? opened.dir.dir(opened.path) : opened.dir;
};

/**
 * Clones the project's `branch` into the deploy dir, points the clone's `origin` remote
 * at the project's, and tags it with `DEPLOY_CLONE_CONFIG_KEY`, failing at the first step that fails, so `remote set-url`
 * never runs after a failed clone, in a directory that could be inside another repo.
 * Makes the project's `.gro/` first, with its marker, when the deploy dir is beneath it.
 */
const clone_deploy_dir = async (
	git: SpawnProgram,
	fs_paths: FsPaths,
	origin: GitOrigin,
	branch: GitBranch,
	resolved_deploy_dir: string
): Promise<void> => {
	if (resolved_deploy_dir.startsWith(paths.build)) await gro_dir_ensure(fs_paths);
	const fail = (step: string): TaskError =>
		new TaskError(
			`\`${step}\` failed preparing the deploy dir ${print_path(resolved_deploy_dir)},` +
				' so nothing was pushed - see the output above, resolve the failure, then deploy again'
		);
	const clone = await git.run([
		'clone',
		'-b',
		branch,
		'--single-branch',
		'--end-of-options',
		dir,
		resolved_deploy_dir
	]);
	if (!spawn_result_is_ok(clone)) throw fail(`git clone -b ${branch} --single-branch`);
	const url = await to_origin_url(git, origin);
	if (url === null) throw fail(`git remote get-url ${origin}`);
	const cwd = await open_deploy_cwd(fs_paths, resolved_deploy_dir);
	const set_url = await git.run(['remote', 'set-url', '--end-of-options', origin, url], { cwd });
	if (!spawn_result_is_ok(set_url)) throw fail(`git remote set-url ${origin}`);
	const set_tag = await git.run(['config', '--local', DEPLOY_CLONE_CONFIG_KEY, 'true'], { cwd });
	if (!spawn_result_is_ok(set_tag))
		throw fail(`git config --local ${DEPLOY_CLONE_CONFIG_KEY} true`);
};
