import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { styleText as st } from 'node:util';

import { TaskError, type Task } from './task.ts';
import { Plugins, to_plugin_context } from './plugin.ts';
import { clean_fs } from './clean_fs.ts';
import {
	is_build_cache_valid,
	create_build_cache_metadata,
	save_build_cache_metadata,
	discover_build_output_dirs,
	BUILD_CACHE_METADATA_PATH
} from './build_cache.ts';
import { load_build_cache_dependencies } from './build_cache_dependencies.ts';
import { fs } from './fs.ts';
import { is_fs_error } from './fs_error.ts';
import { create_fs_paths } from './fs_paths.ts';
import {
	GIT_LOCAL_PASS_ENV,
	git_check_clean_workspace,
	git_current_commit_hash,
	to_git_program
} from './git.ts';
import { spawn } from './spawn.ts';
import { process } from './process.ts';

/** @nodocs */
export const Args = s.object({
	sync: s.with_default(meta(s.boolean, { description: 'dual of no-sync' }), true),
	'no-sync': s.with_default(meta(s.boolean, { description: 'opt out of gro sync' }), false),
	gen: s.with_default(meta(s.boolean, { description: 'dual of no-gen' }), true),
	'no-gen': s.with_default(meta(s.boolean, { description: 'opt out of gro gen' }), false),
	install: s.with_default(meta(s.boolean, { description: 'dual of no-install' }), true),
	// convenience, same as `gro build -- gro sync --no-install` but the latter takes precedence
	'no-install': s.with_default(
		meta(s.boolean, { description: 'opt out of installing packages before building' }),
		false
	),
	force_build: s.with_default(
		meta(s.boolean, { description: 'force a fresh build, ignoring the cache' }),
		false
	),
	allow_dirty: s.with_default(
		meta(s.boolean, { description: 'skip the post-build dirty workspace check' }),
		false
	)
});
export type Args = s.Infer<typeof Args>;

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
		const { task_args, invoke_task, log, config } = ctx;
		const { sync, gen, install, force_build, allow_dirty } = task_args;
		const fs_paths = create_fs_paths(fs);
		// only reads the local repo, so the named set is all it needs
		const git = to_git_program(spawn, { pass_env: GIT_LOCAL_PASS_ENV });
		const cache_deps = { fs_paths, log, git };

		if (sync || install) {
			if (!sync) log.warn('sync is false but install is true, so ignoring the sync option');
			await invoke_task('sync', { install, gen: false });
		}

		if (gen) {
			await invoke_task('gen');
		}

		// Batch git calls upfront for performance (spawning processes is expensive),
		// and load the dependency key components once so the check and the save agree
		const [workspace_status, initial_commit, cache_dependencies] = await Promise.all([
			git_check_clean_workspace(git),
			git_current_commit_hash(git),
			load_build_cache_dependencies(fs_paths, process.runtime_version)
		]);
		const workspace_dirty = !!workspace_status;

		// Discover build output directories once to avoid redundant filesystem scans
		let build_dirs: Array<string> | undefined;

		// Check build cache unless force_build is set or workspace is dirty
		if (!workspace_dirty && !force_build) {
			const cache_valid = await is_build_cache_valid(cache_deps, config, {
				dependencies: cache_dependencies,
				git_commit: initial_commit
			});
			if (cache_valid) {
				log.info(
					st('cyan', 'skipping build, cache is valid'),
					st('dim', '(use --force_build to rebuild)')
				);
				return;
			}
		} else if (workspace_dirty) {
			// IMPORTANT: When workspace is dirty, we delete cache AND all outputs to prevent stale state.
			// Rationale: Uncommitted changes could be reverted, leaving cached outputs from reverted code.
			// This conservative approach prioritizes safety over convenience during development.
			try {
				await fs_paths.remove_path(BUILD_CACHE_METADATA_PATH, { missing_ok: true });
			} catch (error) {
				// `.gro` as a file means there's no cache to remove
				if (!is_fs_error(error, 'not_a_directory')) throw error;
			}

			// Delete all build output directories
			build_dirs = await discover_build_output_dirs(fs_paths);
			await Promise.all(
				build_dirs.map((dir) => fs_paths.remove_path(dir, { recursive: true, missing_ok: true }))
			);

			log.info(st('yellow', 'workspace has uncommitted changes - skipping build cache'));
			// Skip clean_fs - already manually cleaned cache and all build outputs above
		} else {
			log.info(st('yellow', 'forcing fresh build, ignoring cache'));
		}

		// Clean build outputs (skip if workspace was dirty - already cleaned manually above)
		if (!workspace_dirty) {
			await clean_fs({ fs_paths }, { dist_dirs: true });
		}

		const plugins = await Plugins.create(to_plugin_context(ctx, false, false));
		await plugins.setup();
		await plugins.adapt();
		await plugins.teardown();

		// Only a build that started clean can be verified or cached. A dirty start has someone
		// mid-edit, and publish and deploy start clean, so a misbehaving build is caught there.
		if (!workspace_dirty) {
			const final_workspace_status = await git_check_clean_workspace(git);
			if (final_workspace_status) {
				if (!allow_dirty) {
					throw new TaskError(
						'Build process modified tracked files or created untracked files.\n\n' +
							'Git status after build:\n' +
							final_workspace_status +
							'\n\n' +
							'Builds should only write to output directories (build/, dist/, etc.).\n' +
							'This usually indicates a plugin or build step is incorrectly modifying source files.'
					);
				}
				log.warn(st('yellow', 'workspace changed during build'), '- cache not saved');
				return;
			}

			// Race condition protection: verify git commit didn't change during build
			const current_commit = await git_current_commit_hash(git);

			if (current_commit !== initial_commit) {
				log.warn(
					st('yellow', 'git commit changed during build'),
					st(
						'dim',
						`(${format_commit_hash(initial_commit)} → ${format_commit_hash(current_commit)})`
					),
					'- cache not saved'
				);
			} else {
				// Commit is stable - safe to save cache
				const metadata = await create_build_cache_metadata(cache_deps, config, {
					dependencies: cache_dependencies,
					git_commit: initial_commit,
					build_dirs
				});
				await save_build_cache_metadata(cache_deps, metadata);
				log.debug('Build cache metadata saved');
			}
		}
	}
};
