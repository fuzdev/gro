import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { TaskError, type Task } from './task.ts';
import { clean_fs } from './clean_fs.ts';
import { fs } from './fs.ts';
import { create_fs_paths } from './fs_paths.ts';
import { GitOrigin, to_git_program } from './git.ts';
import { spawn } from './spawn.ts';
import { spawn_result_is_ok, to_spawn_result_message } from './cli.ts';

/** @nodocs */
export const Args = s.object({
	build_dist: s.with_default(
		meta(s.boolean, {
			description: 'delete only the `dist_*` plugin output directories, keeping `.gro/`'
		}),
		false
	),
	sveltekit: s.with_default(
		meta(s.boolean, { description: 'delete the SvelteKit directory and Vite cache' }),
		false
	),
	nodemodules: s.with_default(
		meta(s.boolean, { description: 'delete the node_modules directory' }),
		false
	),
	git: s.with_default(
		meta(s.boolean, {
			description:
				'run "git remote prune" to delete local branches referencing nonexistent remote branches'
		}),
		false
	),
	git_origin: s.with_default(
		meta(GitOrigin, { description: 'the origin to "git remote prune"' }),
		'origin'
	)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'remove build files, and optionally prune git branches',
	Args,
	run: async ({ args, log }): Promise<void> => {
		const { build_dist, sveltekit, nodemodules, git, git_origin } = args;

		await clean_fs(
			{ fs_paths: create_fs_paths(fs), log },
			{ gro_dir: !build_dist, dist_dirs: true, sveltekit, nodemodules }
		);

		// lop off stale git branches
		if (git) {
			// queries the remote, so it may prompt, and credential helpers are the user's
			const result = await to_git_program(spawn, { pass_env: 'all' }).run([
				'remote',
				'prune',
				'--end-of-options',
				git_origin
			]);
			if (!spawn_result_is_ok(result)) {
				throw new TaskError(
					`\`git remote prune ${git_origin}\` failed: ${to_spawn_result_message(result)}`
				);
			}
		}
	}
};
