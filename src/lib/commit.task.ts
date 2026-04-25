import {spawn} from '@fuzdev/fuz_util/process.js';
import {z} from 'zod';
import {GitOrigin, git_current_branch_name, git_push} from '@fuzdev/fuz_util/git.js';

import type {Task} from './task.ts';

/** @nodocs */
export const Args = z.strictObject({
	_: z
		.array(z.string())
		.meta({
			description: 'the git commit message, the same as git commit -m or --message',
		})
		.default([]),
	origin: GitOrigin.meta({description: 'git origin to commit to'}).default('origin'),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'commit and push to a new branch',
	Args,
	run: async ({args}): Promise<void> => {
		const {
			_: [message],
			origin,
		} = args;

		const branch = await git_current_branch_name();

		await spawn('git', ['commit', '-a', '-m', message!]);
		await git_push(origin, branch, undefined, true);
	},
};
