import { print_spawn_result } from '@fuzdev/fuz_util/process.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { has_forwarded_flag, to_forwarded_argv } from './args.ts';
import { find_cli, spawn_cli } from './cli.ts';
import { TaskError, type Task } from './task.ts';

const ESLINT_CLI = 'eslint';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(meta(s.array(s.string), { description: 'paths to lint' }), () => []),
	eslint_cli: s.with_default(meta(s.string, { description: 'the ESLint CLI to use' }), ESLINT_CLI)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'run eslint',
	Args,
	run: async ({ log, args }): Promise<void> => {
		const { _, eslint_cli } = args;

		const found_eslint_cli = await find_cli(eslint_cli);
		if (!found_eslint_cli) {
			// TODO maybe make this an option?
			log.info('ESLint is not installed; skipping linting');
			return;
		}

		// the forwarded text goes verbatim after Gro's own args, which drop what it overrides
		const forwarded_argv = to_forwarded_argv(eslint_cli);
		const serialized_args = [..._];
		if (!has_forwarded_flag(forwarded_argv, 'max-warnings')) {
			serialized_args.push('--max-warnings', '0');
		}
		serialized_args.push(...forwarded_argv);
		const eslint_result = await spawn_cli(found_eslint_cli, serialized_args, log);
		if (!eslint_result?.ok) {
			throw new TaskError(`ESLint found some problems. ${print_spawn_result(eslint_result!)}`);
		}
	}
};
