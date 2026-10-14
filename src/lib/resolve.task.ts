import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import { styleText as st } from 'node:util';

import { TASK_FILE_SUFFIXES, type Task } from './task.ts';
import { resolve_input_paths, to_input_paths } from './input_path.ts';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(
		meta(s.array(s.string), { description: 'the input paths to resolve' }),
		() => ['']
	),
	verbose: s.with_default(meta(s.boolean, { description: 'log diagnostics' }), false)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'diagnostic that logs resolved filesystem info for the given input paths',
	Args,
	run: async ({ args, config, log }): Promise<void> => {
		const { _, verbose } = args;

		if (verbose) log.info('raw input paths:', _);

		const input_paths = to_input_paths(_);
		if (verbose) log.info('input paths:', input_paths);

		const { task_root_dirs } = config;
		if (verbose) log.info('task root paths:', task_root_dirs);

		const { resolved_input_paths, possible_paths_by_input_path, unmapped_input_paths } =
			await resolve_input_paths(input_paths, task_root_dirs, TASK_FILE_SUFFIXES);
		if (verbose) log.info('resolved_input_paths:', resolved_input_paths);
		if (verbose) log.info('possible_paths_by_input_path:', possible_paths_by_input_path);
		if (verbose) log.info('unmapped_input_paths:', unmapped_input_paths);

		for (const p of resolved_input_paths) {
			log.info('resolved:', st('green', p.id));
		}

		if (!resolved_input_paths.length) {
			log.warn(st('yellow', 'no input paths were resolved'));
		}
	}
};
