import {print_spawn_result} from '@fuzdev/fuz_util/process.ts';
import {z} from 'zod';

import {to_implicit_forwarded_args} from './args.ts';
import {PRETTIER_CLI_DEFAULT} from './constants.ts';
import {format_directory} from './format_directory.ts';
import {paths} from './paths.ts';
import {TaskError, type Task} from './task.ts';

/** @nodocs */
export const Args = z.strictObject({
	_: z.array(z.string()).meta({description: 'files or directories to format'}).optional(),
	check: z
		.boolean()
		.meta({description: 'exit with a nonzero code if any files are unformatted'})
		.default(false),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'format source files',
	Args,
	run: async ({args, log, config}) => {
		const {_: patterns, check} = args;

		const format_result = await format_directory(
			log,
			paths.source,
			check,
			undefined,
			undefined,
			undefined,
			config.pm_cli,
			to_implicit_forwarded_args(PRETTIER_CLI_DEFAULT),
			patterns,
		);
		if (!format_result.ok) {
			throw new TaskError(
				`Failed ${check ? 'formatting check' : 'to format'}. ${print_spawn_result(format_result)}`,
			);
		}
	},
};
