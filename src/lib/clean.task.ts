import { spawn } from '@fuzdev/fuz_util/process.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { GitOrigin } from '@fuzdev/fuz_util/git.ts';

import type { Task } from './task.ts';
import { clean_fs } from './clean_fs.ts';

/** @nodocs */
export const Args = s.object({
	build_dev: s.with_default(
		meta(s.boolean, { description: 'delete the Gro build dev directory' }),
		false
	),
	build_dist: s.with_default(
		meta(s.boolean, { description: 'delete the Gro build dist directory' }),
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
	summary: 'remove temporary dev and build files, and optionally prune git branches',
	Args,
	run: async ({ args }): Promise<void> => {
		const { build_dev, build_dist, sveltekit, nodemodules, git, git_origin } = args;

		await clean_fs({
			build: !build_dev && !build_dist,
			build_dev,
			build_dist,
			sveltekit,
			nodemodules
		});

		// lop off stale git branches
		if (git) {
			await spawn('git', ['remote', 'prune', git_origin]);
		}
	}
};
