import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

import { TaskError } from './task_error.ts';
import { clean_fs } from './clean_fs.ts';
import type { FsPaths } from './fs_paths.ts';
import { GitOrigin, to_git_program } from './git.ts';
import { spawn_result_is_ok, to_spawn_result_message } from './cli.ts';
import type { BuiltinPrograms, BuiltinTask, BuiltinTaskGrants } from './builtin_host.ts';

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

const grants = {
	// removes the build and cache directories beneath the project,
	// and empties the directory a `.gro/` Gro marked links to, wherever it is (`clean_fs.ts`)
	fs: { read: ['.'], write: ['.'] },
	// `--git` queries the remote, so it may prompt, and credential helpers are the user's
	programs: { git: { pass_env: 'all' } }
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'remove build files, and optionally prune git branches',
	Args,
	grants,
	run: ({ task_args, log, host }) =>
		run_clean(task_args, { fs_paths: host.fs_paths, programs: host.programs, log })
};

/**
 * What `run_clean` reaches, picked from a builtin's `host` and context.
 */
export interface CleanDeps {
	fs_paths: FsPaths;
	programs: Pick<BuiltinPrograms<'git'>, 'find'>;
	log: Logger;
}

/**
 * Removes the build and cache directories, and with `--git` prunes stale branches,
 * `gro clean`'s body.
 * @param args - the task's parsed args
 * @param deps - the project's boundary, git, and the log
 */
export const run_clean = async (args: Args, deps: CleanDeps): Promise<void> => {
	const { build_dist, sveltekit, nodemodules, git, git_origin } = args;
	const { fs_paths, programs, log } = deps;

	await clean_fs(
		{ fs_paths, log },
		{ gro_dir: !build_dist, dist_dirs: true, sveltekit, nodemodules }
	);

	// lop off stale git branches
	if (git) {
		const result = await to_git_program(programs).run([
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
};
