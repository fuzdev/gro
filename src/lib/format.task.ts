import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { format_directory } from './format_directory.ts';
import { fs } from './fs.ts';
import { create_fs_paths } from './fs_paths.ts';
import { paths } from './paths.ts';
import { TaskError, type Task } from './task.ts';

/** @nodocs */
export const Args = s.object({
	_: s.optional(meta(s.array(s.string), { description: 'files or globs to format' })),
	check: s.with_default(
		meta(s.boolean, { description: 'exit with a nonzero code if any files are unformatted' }),
		false
	)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'format source files',
	Args,
	run: async ({ task_args, log, config }) => {
		const { _: patterns, check } = task_args;

		const result = await format_directory(create_fs_paths(fs), {
			log,
			dir: paths.source,
			check,
			filter: config.search_filters,
			patterns
		});
		if (!result.ok) {
			throw new TaskError(`Failed ${check ? 'formatting check' : 'to format'}.`);
		}
	}
};
