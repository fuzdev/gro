import {args_serialize} from '@fuzdev/fuz_util/args.ts';
import {print_spawn_result} from '@fuzdev/fuz_util/process.ts';
import {z} from 'zod';

import {to_forwarded_args} from './args.ts';
import {find_cli, spawn_cli} from './cli.ts';
import {TaskError, type Task} from './task.ts';

const ESLINT_CLI = 'eslint';

/** @nodocs */
export const Args = z.strictObject({
	_: z.array(z.string()).meta({description: 'paths to serve'}).default([]),
	eslint_cli: z.string().meta({description: 'the ESLint CLI to use'}).default(ESLINT_CLI),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'run eslint',
	Args,
	run: async ({log, args}): Promise<void> => {
		const {_, eslint_cli} = args;

		const found_eslint_cli = await find_cli(eslint_cli);
		if (!found_eslint_cli) {
			// TODO maybe make this an option?
			log.info('ESLint is not installed; skipping linting');
			return;
		}

		const forwarded_args = {_, 'max-warnings': 0, ...to_forwarded_args(eslint_cli)};
		const serialized_args = args_serialize(forwarded_args);
		const eslintResult = await spawn_cli(found_eslint_cli, serialized_args, log);
		if (!eslintResult?.ok) {
			throw new TaskError(`ESLint found some problems. ${print_spawn_result(eslintResult!)}`);
		}
	},
};
