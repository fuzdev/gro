import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { spawn_result_is_ok } from './cli.ts';
import { GitOrigin, git_push, to_git_program } from './git.ts';
import { spawn } from './spawn.ts';
import { TaskError, type Task } from './task.ts';

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
	run: async ({ task_args }): Promise<void> => {
		const {
			_: [message],
			origin
		} = task_args;

		// signs and pushes with whatever the user's git is set up with, so the whole environment
		const git = to_git_program(spawn, { pass_env: 'all' });

		const commit_result = await git.run(['commit', '-a', '-m', message!]);
		if (!spawn_result_is_ok(commit_result)) {
			throw new TaskError('`git commit` failed, so nothing was pushed');
		}
		// read after the commit, since a branch with no commit yet can't be read
		await git_push(git, origin, undefined, undefined, true);
	}
};
