import {spawn} from '@fuzdev/fuz_util/process.js';
import {z} from 'zod';
import {GitOrigin} from '@fuzdev/fuz_util/git.js';

import type {Task} from './task.ts';
import {clean_fs} from './clean_fs.ts';

/** @nodocs */
export const Args = z.strictObject({
	build_dev: z.boolean().meta({description: 'delete the Gro build dev directory'}).default(false),
	build_dist: z.boolean().meta({description: 'delete the Gro build dist directory'}).default(false),
	sveltekit: z
		.boolean()
		.meta({description: 'delete the SvelteKit directory and Vite cache'})
		.default(false),
	nodemodules: z.boolean().meta({description: 'delete the node_modules directory'}).default(false),
	git: z
		.boolean()
		.meta({
			description:
				'run "git remote prune" to delete local branches referencing nonexistent remote branches',
		})
		.default(false),
	git_origin: GitOrigin.meta({description: 'the origin to "git remote prune"'}).default('origin'),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'remove temporary dev and build files, and optionally prune git branches',
	Args,
	run: async ({args}): Promise<void> => {
		const {build_dev, build_dist, sveltekit, nodemodules, git, git_origin} = args;

		await clean_fs({
			build: !build_dev && !build_dist,
			build_dev,
			build_dist,
			sveltekit,
			nodemodules,
		});

		// lop off stale git branches
		if (git) {
			await spawn('git', ['remote', 'prune', git_origin]);
		}
	},
};
