import {spawn} from '@fuzdev/fuz_util/process.js';
import {print_error} from '@fuzdev/fuz_util/print.js';
import {styleText as st} from 'node:util';
import {z} from 'zod';
import {cp, mkdir, readdir, rm} from 'node:fs/promises';
import {join, resolve} from 'node:path';
import {fs_exists, fs_empty_dir} from '@fuzdev/fuz_util/fs.js';
import {
	git_check_clean_workspace,
	git_checkout,
	git_local_branch_exists,
	git_remote_branch_exists,
	GitOrigin,
	GitBranch,
	git_delete_local_branch,
	git_push_to_create,
	git_reset_branch_to_first_commit,
	git_pull,
	git_fetch,
	git_check_setting_pull_rebase,
	git_clone_locally,
	git_current_branch_name,
} from '@fuzdev/fuz_util/git.js';

import {TaskError, type Task} from './task.ts';
import {print_path} from './paths.ts';
import {GRO_DIRNAME, GIT_DIRNAME, SVELTEKIT_BUILD_DIRNAME} from './constants.ts';

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
export const Args = z.strictObject({
	source: GitBranch.meta({description: 'git source branch to build and deploy from'}).default(
		SOURCE_BRANCH,
	),
	target: GitBranch.meta({description: 'git target branch to deploy to'}).default(TARGET_BRANCH),
	origin: GitOrigin.meta({description: 'git origin to deploy to'}).default('origin'),
	deploy_dir: z.string().meta({description: 'the deploy output directory'}).default(DEPLOY_DIR),
	build_dir: z
		.string()
		.meta({description: 'the SvelteKit build directory'})
		.default(SVELTEKIT_BUILD_DIRNAME),
	dry: z
		.boolean()
		.meta({
			description: 'build and prepare to deploy without actually deploying',
		})
		.default(false),
	force: z
		.boolean()
		.meta({description: 'caution!! destroys the target branch both locally and remotely'})
		.default(false),
	dangerous: z
		.boolean()
		.meta({description: 'caution!! enables destruction of branches like main and master'})
		.default(false),
	reset: z
		.boolean()
		.meta({
			description: 'if true, resets the target branch back to the first commit before deploying',
		})
		.default(false),
	build: z.boolean().meta({description: 'dual of no-build'}).default(true),
	'no-build': z.boolean().meta({description: 'opt out of building'}).default(false),
	sync: z.boolean().meta({description: 'dual of no-sync'}).default(true),
	'no-sync': z.boolean().meta({description: 'opt out of gro sync in build'}).default(false),
	gen: z.boolean().meta({description: 'dual of no-gen'}).default(true),
	'no-gen': z.boolean().meta({description: 'opt out of gro gen in build'}).default(false),
	install: z.boolean().meta({description: 'dual of no-install'}).default(true),
	'no-install': z
		.boolean()
		.meta({description: 'opt out of installing packages before building'})
		.default(false),
	force_build: z
		.boolean()
		.meta({description: 'force a fresh build, ignoring the cache'})
		.default(false),
	pull: z.boolean().meta({description: 'dual of no-pull'}).default(true),
	'no-pull': z.boolean().meta({description: 'opt out of git pull'}).default(false),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'deploy to a branch',
	Args,
	run: async ({args, log, invoke_task}): Promise<void> => {
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
			pull,
		} = args;

		// Checks
		if (!force && target !== TARGET_BRANCH) {
			throw new TaskError(
				`Warning! You are deploying to a custom target branch '${target}',` +
					` instead of the default '${TARGET_BRANCH}' branch.` +
					` This is destructive to your '${target}' branch!` +
					` If you understand and are OK with deleting your branch '${target}',` +
					` both locally and remotely, pass --force to suppress this error.`,
			);
		}
		if (!dangerous && DANGEROUS_BRANCHES.includes(target)) {
			throw new TaskError(
				`Warning! You are deploying to a custom target branch '${target}'` +
					` and that appears very dangerous: it is destructive to your '${target}' branch!` +
					` If you understand and are OK with deleting your branch '${target}',` +
					` both locally and remotely, pass --dangerous to suppress this error.`,
			);
		}
		const clean_error_message = await git_check_clean_workspace();
		if (clean_error_message) {
			throw new TaskError(
				'Deploy failed because the git workspace has uncommitted changes: ' + clean_error_message,
			);
		}
		if (!(await git_check_setting_pull_rebase())) {
			throw new TaskError(
				'Deploying currently requires `git config --global pull.rebase true`,' +
					' but this restriction could be lifted with more work',
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
					' finish rebasing manually or reset with `git rebase --abort`',
			);
		}

		// Prepare the target branch remotely and locally
		const resolved_deploy_dir = resolve(deploy_dir);
		const target_spawn_options = {cwd: resolved_deploy_dir};
		const remote_target_exists = await git_remote_branch_exists(origin, target);
		if (remote_target_exists) {
			// Remote target branch already exists, so sync up efficiently

			// First, check if the deploy dir exists, and if so, attempt to sync it.
			// If anything goes wrong, delete the directory and we'll initialize it
			// using the same code path as if it didn't exist in the first place.
			if (await fs_exists(resolved_deploy_dir)) {
				if (target !== (await git_current_branch_name(target_spawn_options))) {
					// We're in a bad state because the target branch has changed,
					// so delete the directory and continue as if it wasn't there.
					await rm(resolved_deploy_dir, {recursive: true});
				} else {
					await spawn('git', ['reset', '--hard'], target_spawn_options); // in case it's dirty
					// Skip pulling target branch when resetting (optimization - we reset after anyway)
					if (!reset) {
						await git_pull(origin, target, target_spawn_options);
						if (await git_check_clean_workspace(target_spawn_options)) {
							// We're in a bad state because the local branch lost continuity with the remote,
							// so delete the directory and continue as if it wasn't there.
							await rm(resolved_deploy_dir, {recursive: true});
						}
					}
				}
			}

			// Second, initialize the deploy dir if needed.
			// It may not exist, or it may have been deleted after failing to sync above.
			if (!(await fs_exists(resolved_deploy_dir))) {
				const local_deploy_branch_exists = await git_local_branch_exists(target);
				await git_fetch(origin, ('+' + target + ':' + target) as GitBranch); // fetch+merge and allow non-fastforward updates with the +
				await git_clone_locally(origin, target, dir, resolved_deploy_dir);
				// Clean up if we created the target branch in the cwd
				if (!local_deploy_branch_exists) {
					await git_delete_local_branch(target);
				}
			}

			// Local target branch is now synced with remote, but do we need to reset?
			if (reset) {
				await git_reset_branch_to_first_commit(origin, target, target_spawn_options);
			}
		} else {
			// Remote target branch does not exist, so start from scratch

			// Delete the deploy dir and recreate it
			if (await fs_exists(resolved_deploy_dir)) {
				await rm(resolved_deploy_dir, {recursive: true});
				await mkdir(resolved_deploy_dir, {recursive: true});
			}

			// Delete the target branch locally in the cwd if it exists
			if (await git_local_branch_exists(target)) {
				await git_delete_local_branch(target);
			}

			// Create the target branch locally and remotely.
			// This is more complex to avoid churning the cwd.
			await git_clone_locally(origin, source, dir, resolved_deploy_dir);
			await spawn('git', ['checkout', '--orphan', target], target_spawn_options);
			// TODO there's definitely a better way to do this
			await spawn('git', ['rm', '-rf', '.'], target_spawn_options);
			await spawn('touch', [INITIAL_FILE_PATH], target_spawn_options);
			await spawn('git', ['add', INITIAL_FILE_PATH], target_spawn_options);
			await spawn('git', ['commit', '-m', 'init'], target_spawn_options);
			await git_push_to_create(origin, target, target_spawn_options);
			await git_delete_local_branch(source, target_spawn_options);
		}

		// Remove everything except .git from the deploy directory to avoid stale files
		await fs_empty_dir(resolved_deploy_dir, (name) => name !== GIT_DIRNAME);

		// Build
		try {
			if (build) {
				await invoke_task('build', {sync, gen, install, force_build});
			}
		} catch (error) {
			log.error(
				st('red', 'build failed'),
				'but',
				st('green', 'no changes were made to git'),
				print_error(error),
			);
			if (dry) {
				log.info(st('red', 'dry deploy failed'));
			}
			throw new TaskError(`Deploy safely canceled due to build failure. See the error above.`);
		}

		// Verify build output exists
		if (!(await fs_exists(build_dir))) {
			throw new TaskError(`Directory to deploy does not exist after building: ${build_dir}`);
		}

		// Copy the build
		const build_entries = await readdir(build_dir);
		await Promise.all(
			build_entries.map((path) =>
				cp(join(build_dir, path), join(resolved_deploy_dir, path), {recursive: true}),
			),
		);

		// At this point, `dist/` is ready to be committed and deployed!
		if (dry) {
			log.info(st('green', 'dry deploy complete:'), 'files at', print_path(resolved_deploy_dir));
			return;
		}

		// Commit and push
		try {
			await spawn('git', ['add', '.', '-f'], target_spawn_options);
			await spawn('git', ['commit', '-m', 'deployment'], target_spawn_options);
			await spawn('git', ['push', origin, target, '-f'], target_spawn_options); // force push because we may be resetting the branch, see the checks above to make this safer
		} catch (error) {
			log.error(st('red', 'updating git failed:'), print_error(error));
			throw new TaskError(`Deploy failed in a bad state: built but not pushed, see error above.`);
		}

		log.info(st('green', 'deployed')); // TODO log a different message if "Everything up-to-date"
	},
};
