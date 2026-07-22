import { z } from 'zod';

import { format_directory } from './format_directory.ts';
import { paths } from './paths.ts';
import { TaskError, type Task } from './task.ts';

/** @nodocs */
export const Args = z.strictObject({
	_: z.array(z.string()).meta({ description: 'files or globs to format' }).optional(),
	check: z
		.boolean()
		.meta({ description: 'exit with a nonzero code if any files are unformatted' })
		.default(false)
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'format source files',
	Args,
	run: async ({ args, log, config }) => {
		const { _: patterns, check } = args;

		const result = await format_directory(
			log,
			paths.source,
			check,
			config.search_filters,
			patterns
		);
		if (!result.ok) {
			throw new TaskError(`Failed ${check ? 'formatting check' : 'to format'}.`);
		}
	}
};
