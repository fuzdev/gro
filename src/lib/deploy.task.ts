import { spawn, spawn_out } from '@fuzdev/fuz_util/process.ts';
import { print_error } from '@fuzdev/fuz_util/print.ts';
import { styleText as st } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { join, resolve } from 'node:path';
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
	git_current_branch_name
} from '@fuzdev/fuz_util/git.ts';

import { is_silent_error, TaskError, type Task } from './task.ts';
import { paths, print_path } from './paths.ts';
import { GRO_DIRNAME, GIT_DIRNAME, SVELTEKIT_BUILD_DIRNAME } from './constants.ts';
import { fs } from './fs.ts';
import {
	fs_copy_path,
	fs_open_path,
	fs_protected_dirs,
	fs_remove_path,
	is_protected_dir,
	PROTECTED_DIRS_TEXT,
	type FsOpenedPath
} from './internal/fs_path.ts';
import { gro_dir_ensure } from './internal/gro_dir.ts';

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

		// Checks
		// the deploy dir is removed, emptied, and run git in below, so before anything else,
		// it must be somewhere Gro may own, and if it exists, a clone Gro made
		const resolved_deploy_dir = resolve(deploy_dir);
		await assert_deploy_dir_owned(await open_deploy_dir(resolved_deploy_dir), origin);
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
		const clean_error_message = await git_check_clean_workspace();
		if (clean_error_message) {
			throw new TaskError(
				'Deploy failed because the git workspace has uncommitted changes: ' + clean_error_message
			);
		}
		if (!(await git_check_setting_pull_rebase())) {
			throw new TaskError(
				'Deploying currently requires `git config --global pull.rebase true`,' +
					' but this restriction could be lifted with more work'
			);
		}

		// Fetch the source branch in the cwd if it's not there
		if (!(await git_local_branch_exists(source))) {
			await git_fetch(origin, source);
		}

		// Prepare the source branch in the cwd
		await git_checkout(source);
		if (pull) {
			await git_pull(origin, source);
		}
		if (await git_check_clean_workspace()) {
			throw new TaskError(
				'Deploy failed because the local source branch is out of sync with the remote one,' +
					' finish rebasing manually or reset with `git rebase --abort`'
			);
		}

		// Prepare the target branch remotely and locally
		// resolved at each check, since the steps below remove and recreate the directory
		const deploy_dir_exists = async (): Promise<boolean> => {
			const opened = await fs_open_path(resolved_deploy_dir);
			return opened.dir.exists(opened.path);
		};
		const target_spawn_options = { cwd: resolved_deploy_dir };
		const remote_target_exists = await git_remote_branch_exists(origin, target);
		if (remote_target_exists) {
			// Remote target branch already exists, so sync up efficiently

			// First, check if the deploy dir exists, and if so, attempt to sync it.
			// If anything goes wrong, delete the directory and we'll initialize it
			// using the same code path as if it didn't exist in the first place.
			if (await deploy_dir_exists()) {
				if (target !== (await git_current_branch_name(target_spawn_options))) {
					// We're in a bad state because the target branch has changed,
					// so delete the directory and continue as if it wasn't there.
					await fs_remove_path(resolved_deploy_dir, { recursive: true });
				} else {
					// in case it's dirty
					const reset_result = await spawn('git', ['reset', '--hard'], target_spawn_options);
					if (!reset_result.ok) {
						// We're in a bad state because the deploy dir can't be reset,
						// so delete the directory and continue as if it wasn't there.
						log.warn(
							'`git reset --hard` failed in',
							print_path(resolved_deploy_dir),
							'- recreating it'
						);
						await fs_remove_path(resolved_deploy_dir, { recursive: true });
					} else if (!reset) {
						// Skip pulling target branch when resetting (optimization - we reset after anyway)
						await git_pull(origin, target, target_spawn_options);
						if (await git_check_clean_workspace(target_spawn_options)) {
							// We're in a bad state because the local branch lost continuity with the remote,
							// so delete the directory and continue as if it wasn't there.
							await fs_remove_path(resolved_deploy_dir, { recursive: true });
						}
					}
				}
			}

			// Second, initialize the deploy dir if needed.
			// It may not exist, or it may have been deleted after failing to sync above.
			if (!(await deploy_dir_exists())) {
				const local_deploy_branch_exists = await git_local_branch_exists(target);
				await git_fetch(origin, '+' + target + ':' + target); // fetch+merge and allow non-fastforward updates with the +
				await clone_deploy_dir(origin, target, resolved_deploy_dir);
				// Clean up if we created the target branch in the cwd
				if (!local_deploy_branch_exists) {
					await git_delete_local_branch(target);
				}
			}

			// Local target branch is now synced with remote, but do we need to reset?
			// The reset is local, so a dry deploy pushes nothing, and the final force push publishes it.
			if (reset) {
				const first_commit_hash = await git_current_branch_first_commit_hash(target_spawn_options);
				const reset_result = await spawn(
					'git',
					['reset', '--hard', first_commit_hash],
					target_spawn_options
				);
				if (!reset_result.ok) {
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
				await fs_remove_path(resolved_deploy_dir, { recursive: true });
				const recreated = await fs_open_path(resolved_deploy_dir);
				await recreated.dir.mkdir(recreated.path, { recursive: true });
			}

			// Delete the target branch locally in the cwd if it exists
			if (await git_local_branch_exists(target)) {
				await git_delete_local_branch(target);
			}

			// Create the target branch locally and remotely.
			// This is more complex to avoid churning the cwd.
			await clone_deploy_dir(origin, source, resolved_deploy_dir);
			const spawn_init = async (command: string, command_args: Array<string>): Promise<void> => {
				const result = await spawn(command, command_args, target_spawn_options);
				if (!result.ok) {
					throw new TaskError(
						`\`${command} ${command_args.join(' ')}\` failed creating the target branch '${target}'` +
							` in ${print_path(resolved_deploy_dir)}, so nothing was pushed - see the output above,` +
							' resolve the failure, then deploy again'
					);
				}
			};
			await spawn_init('git', ['checkout', '--orphan', target]);
			// TODO there's definitely a better way to do this
			await spawn_init('git', ['rm', '-rf', '.']);
			await spawn_init('touch', [INITIAL_FILE_PATH]);
			await spawn_init('git', ['add', INITIAL_FILE_PATH]);
			await spawn_init('git', ['commit', '-m', 'init']);
			// a dry deploy pushes nothing; a real one creates the remote branch here, with upstream tracking
			if (!dry) await git_push_to_create(origin, target, target_spawn_options);
			await git_delete_local_branch(source, target_spawn_options);
		}

		// Remove everything except .git from the deploy directory to avoid stale files,
		// opened now that it's in its final form, so the emptying never acts on a replaced directory
		const deploy = await open_deploy_dir(resolved_deploy_dir);
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
				'but',
				st('green', 'no changes were made to git'),
				// a `SilentError` means the build's failure is already reported above
				...(is_silent_error(error) ? [] : [print_error(error)])
			);
			if (dry) {
				log.info(st('red', 'dry deploy failed'));
			}
			throw new TaskError(`Deploy safely canceled due to build failure. See the error above.`);
		}

		// Verify build output exists
		const build_output = await fs_open_path(build_dir);
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
				fs_copy_path(join(build_dir, name), join(resolved_deploy_dir, name), { recursive: true })
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
		const add_result = await spawn('git', ['add', '.'], target_spawn_options);
		if (!add_result.ok) {
			throw new TaskError(bad_state_message('git add'));
		}
		// fail loud on what the add skipped: without `--directory`, files in ignored directories
		// are listed one by one, and `-z` keeps names unquoted
		const ignored = await spawn_out(
			'git',
			['ls-files', '-z', '--others', '--ignored', '--exclude-standard'],
			target_spawn_options
		);
		if (!ignored.result.ok) {
			if (ignored.stderr) log.error(ignored.stderr.trimEnd());
			throw new TaskError(bad_state_message('git ls-files --others --ignored'));
		}
		const ignored_paths = ignored.stdout?.split('\0').filter(Boolean) ?? [];
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
		const diff_result = await spawn('git', ['diff', '--cached', '--quiet'], target_spawn_options);
		const changed = !diff_result.ok;
		if (changed && !(diff_result.kind === 'exited' && diff_result.code === 1)) {
			throw new TaskError(bad_state_message('git diff --cached --quiet'));
		}
		if (changed) {
			const commit_result = await spawn(
				'git',
				['commit', '-m', 'deployment'],
				target_spawn_options
			);
			if (!commit_result.ok) {
				throw new TaskError(bad_state_message('git commit'));
			}
		}
		// force push because we may be resetting the branch, see the checks above to make this safer
		const push_result = await spawn('git', ['push', origin, target, '-f'], target_spawn_options);
		if (!push_result.ok) {
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
const open_deploy_dir = async (resolved_deploy_dir: string): Promise<FsOpenedPath> => {
	const opened = await fs_open_path(resolved_deploy_dir);
	const real = to_opened_real_path(opened);
	if (is_protected_dir(real, await fs_protected_dirs())) {
		throw new TaskError(
			`The deploy dir ${print_path(resolved_deploy_dir)} resolves to ${real},` +
				` ${PROTECTED_DIRS_TEXT}`
		);
	}
	if (opened.dir === fs.project) {
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
const assert_deploy_dir_owned = async (opened: FsOpenedPath, origin: GitOrigin): Promise<void> => {
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
	const toplevel = await spawn_out('git', ['rev-parse', '--show-toplevel'], { cwd: real });
	const toplevel_path = toplevel.result.ok ? toplevel.stdout?.trim() : undefined;
	if (!toplevel_path) throw refuse('it is not in a git repo');
	if (toplevel_path !== real) throw refuse(`it is inside the git repo at ${toplevel_path}`);
	// its own `.git` directory, so a layout like a committed bare repo with `core.worktree`,
	// whose config could carry a forged tag, isn't taken for a clone
	const own_git_dir = real + '/' + GIT_DIRNAME;
	const git_dir = await spawn_out('git', ['rev-parse', '--absolute-git-dir'], { cwd: real });
	const git_dir_path = git_dir.result.ok ? git_dir.stdout?.trim() : undefined;
	if (git_dir_path !== own_git_dir) {
		throw refuse(`its git dir is ${git_dir_path ?? 'unknown'}, not its own ${own_git_dir}`);
	}
	const git_dir_info = await opened.dir.stat(join_beneath(opened.path, GIT_DIRNAME), {
		follow: false
	});
	if (git_dir_info?.kind !== 'directory') {
		throw refuse(`its ${GIT_DIRNAME} is not a directory`);
	}
	const expected = await to_origin_url(origin, dir);
	if (expected === null) {
		throw refuse(`the project has no \`${origin}\` remote to compare it with`);
	}
	const actual = await to_origin_url(origin, real);
	if (actual !== expected) {
		throw refuse(
			actual === null
				? `it has no \`${origin}\` remote`
				: `its \`${origin}\` remote is ${actual}, not the project's ${expected}`
		);
	}
	const tag = await spawn_out('git', ['config', '--local', '--get', DEPLOY_CLONE_CONFIG_KEY], {
		cwd: real
	});
	if (!tag.result.ok || tag.stdout?.trim() !== 'true') {
		throw refuse(
			`it lacks the local git config \`${DEPLOY_CLONE_CONFIG_KEY} true\` that marks a deploy clone` +
				' (a clone from before the mark, which Gro re-clones once it is removed, or another checkout)'
		);
	}
};

const to_origin_url = async (origin: GitOrigin, cwd: string): Promise<string | null> => {
	const { result, stdout } = await spawn_out('git', ['remote', 'get-url', origin], { cwd });
	return (result.ok && stdout?.trim()) || null;
};

/**
 * Clones the project's `branch` into the deploy dir, points the clone's `origin` remote
 * at the project's, and tags it with `DEPLOY_CLONE_CONFIG_KEY`, failing at the first step that fails, so `remote set-url`
 * never runs after a failed clone, in a directory that could be inside another repo.
 * Makes the project's `.gro/` first, with its marker, when the deploy dir is beneath it.
 */
const clone_deploy_dir = async (
	origin: GitOrigin,
	branch: GitBranch,
	resolved_deploy_dir: string
): Promise<void> => {
	if (resolved_deploy_dir.startsWith(paths.build)) await gro_dir_ensure();
	const fail = (step: string): TaskError =>
		new TaskError(
			`\`${step}\` failed preparing the deploy dir ${print_path(resolved_deploy_dir)},` +
				' so nothing was pushed - see the output above, resolve the failure, then deploy again'
		);
	const clone = await spawn('git', [
		'clone',
		'-b',
		branch,
		'--single-branch',
		dir,
		resolved_deploy_dir
	]);
	if (!clone.ok) throw fail(`git clone -b ${branch} --single-branch`);
	const url = await to_origin_url(origin, dir);
	if (url === null) throw fail(`git remote get-url ${origin}`);
	const set_url = await spawn('git', ['remote', 'set-url', origin, url], {
		cwd: resolved_deploy_dir
	});
	if (!set_url.ok) throw fail(`git remote set-url ${origin}`);
	const set_tag = await spawn('git', ['config', '--local', DEPLOY_CLONE_CONFIG_KEY, 'true'], {
		cwd: resolved_deploy_dir
	});
	if (!set_tag.ok) throw fail(`git config --local ${DEPLOY_CLONE_CONFIG_KEY} true`);
};
