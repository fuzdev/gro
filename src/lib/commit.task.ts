import { spawn } from '@fuzdev/fuz_util/process.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { GitOrigin, git_current_branch_name, git_push } from '@fuzdev/fuz_util/git.ts';

import type { Task } from './task.ts';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(
		meta(s.array(s.string), {
			description: 'the git commit message, the same as git commit -m or --message'
		}),
		() => []
	),
	origin: s.with_default(meta(GitOrigin, { description: 'git origin to commit to' }), 'origin')
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'commit and push to a new branch',
	Args,
	run: async ({ args }): Promise<void> => {
		const {
			_: [message],
			origin
		} = args;

		const branch = await git_current_branch_name();

		await spawn('git', ['commit', '-a', '-m', message!]);
		await git_push(origin, branch, undefined, true);
	}
};
