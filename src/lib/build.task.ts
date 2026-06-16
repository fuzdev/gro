import {z} from 'zod';
import {styleText as st} from 'node:util';
import {git_check_clean_workspace, git_current_commit_hash} from '@fuzdev/fuz_util/git.ts';
import {rm} from 'node:fs/promises';
import {join} from 'node:path';
import {fs_exists} from '@fuzdev/fuz_util/fs.ts';

import {TaskError, type Task} from './task.ts';
import {Plugins} from './plugin.ts';
import {clean_fs} from './clean_fs.ts';
import {
	is_build_cache_valid,
	create_build_cache_metadata,
	save_build_cache_metadata,
	discover_build_output_dirs,
} from './build_cache.ts';
import {paths} from './paths.ts';

/** @nodocs */
export const Args = z.strictObject({
	sync: z.boolean().meta({description: 'dual of no-sync'}).default(true),
	'no-sync': z.boolean().meta({description: 'opt out of gro sync'}).default(false),
	gen: z.boolean().meta({description: 'dual of no-gen'}).default(true),
	'no-gen': z.boolean().meta({description: 'opt out of gro gen'}).default(false),
	install: z.boolean().meta({description: 'dual of no-install'}).default(true),
	'no-install': z // convenience, same as `gro build -- gro sync --no-install` but the latter takes precedence
		.boolean()
		.meta({description: 'opt out of installing packages before building'})
		.default(false),
	force_build: z
		.boolean()
		.meta({description: 'force a fresh build, ignoring the cache'})
		.default(false),
	allow_dirty: z
		.boolean()
		.meta({description: 'skip the post-build dirty workspace check'})
		.default(false),
});
export type Args = z.infer<typeof Args>;

/**
 * Length of git commit hash when displayed in logs (standard git convention).
 */
export const GIT_SHORT_HASH_LENGTH = 7;

/**
 * Formats a git commit hash for display in logs.
 * Returns '[none]' if hash is null (e.g., not in a git repo).
 */
const format_commit_hash = (hash: string | null): string =>
	hash?.slice(0, GIT_SHORT_HASH_LENGTH) ?? '[none]';

/** @nodocs */
export const task: Task<Args> = {
	summary: 'build the project',
	Args,
	run: async (ctx): Promise<void> => {
		const {args, invoke_task, log, config} = ctx;
		const {sync, gen, install, force_build, allow_dirty} = args;

		if (sync || install) {
			if (!sync) log.warn('sync is false but install is true, so ignoring the sync option');
			await invoke_task('sync', {install, gen: false});
		}

		if (gen) {
			await invoke_task('gen');
		}

		// Batch git calls upfront for performance (spawning processes is expensive)
		const [workspace_status, initial_commit] = await Promise.all([
			git_check_clean_workspace(),
			git_current_commit_hash(),
		]);
		const workspace_dirty = !!workspace_status;

		// Discover build output directories once to avoid redundant filesystem scans
		let build_dirs: Array<string> | undefined;

		// Check build cache unless force_build is set or workspace is dirty
		if (!workspace_dirty && !force_build) {
			const cache_valid = await is_build_cache_valid(config, log, initial_commit);
			if (cache_valid) {
				log.info(
					st('cyan', 'skipping build, cache is valid'),
					st('dim', '(use --force_build to rebuild)'),
				);
				return;
			}
		} else if (workspace_dirty) {
			// IMPORTANT: When workspace is dirty, we delete cache AND all outputs to prevent stale state.
			// Rationale: Uncommitted changes could be reverted, leaving cached outputs from reverted code.
			// This conservative approach prioritizes safety over convenience during development.
			const cache_path = join(paths.build, 'build.json');
			if (await fs_exists(cache_path)) {
				await rm(cache_path, {force: true});
			}

			// Delete all build output directories
			build_dirs = await discover_build_output_dirs();
			await Promise.all(build_dirs.map((dir) => rm(dir, {recursive: true, force: true})));

			log.info(st('yellow', 'workspace has uncommitted changes - skipping build cache'));
			// Skip clean_fs - already manually cleaned cache and all build outputs above
		} else {
			log.info(st('yellow', 'forcing fresh build, ignoring cache'));
		}

		// Clean build outputs (skip if workspace was dirty - already cleaned manually above)
		if (!workspace_dirty) {
			await clean_fs({build_dist: true});
		}

		const plugins = await Plugins.create({...ctx, dev: false, watch: false});
		await plugins.setup();
		await plugins.adapt();
		await plugins.teardown();

		// Verify workspace didn't become dirty during build
		if (!allow_dirty) {
			const final_workspace_status = await git_check_clean_workspace();
			if (final_workspace_status !== workspace_status) {
				// Workspace state changed during build - this indicates a problem
				throw new TaskError(
					'Build process modified tracked files or created untracked files.\n\n' +
						'Git status after build:\n' +
						final_workspace_status +
						'\n\n' +
						'Builds should only write to output directories (build/, dist/, etc.).\n' +
						'This usually indicates a plugin or build step is incorrectly modifying source files.',
				);
			}
		}

		// Save build cache metadata after successful build (only if workspace is clean)
		if (!workspace_dirty) {
			// Race condition protection: verify git commit didn't change during build
			const current_commit = await git_current_commit_hash();

			if (current_commit !== initial_commit) {
				log.warn(
					st('yellow', 'git commit changed during build'),
					st(
						'dim',
						`(${format_commit_hash(initial_commit)} → ${format_commit_hash(current_commit)})`,
					),
					'- cache not saved',
				);
			} else {
				// Commit is stable - safe to save cache
				const metadata = await create_build_cache_metadata(config, log, initial_commit, build_dirs);
				await save_build_cache_metadata(metadata, log);
				log.debug('Build cache metadata saved');
			}
		}
	},
};
