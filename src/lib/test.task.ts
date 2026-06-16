import {args_serialize} from '@fuzdev/fuz_util/args.ts';
import {spawn_result_to_message} from '@fuzdev/fuz_util/process.ts';
import {spawn_cli} from '@fuzdev/gro/cli.ts';
import {z} from 'zod';

import {to_implicit_forwarded_args} from './args.ts';
import {find_cli} from './cli.ts';
import {VITEST_CLI} from './constants.ts';
import {package_json_has_dependency, package_json_load} from './package_json.ts';
import {paths} from './paths.ts';
import {TaskError, type Task} from './task.ts';

/** @nodocs */
export const Args = z.strictObject({
	_: z.array(z.string()).meta({description: 'file patterns to filter tests'}).default(['.test.']),
	dir: z.string().meta({description: 'working directory for tests'}).default(paths.source),
	fail_without_tests: z
		.boolean()
		.meta({description: 'opt out of `passWithNoTests`'})
		.default(false),
	t: z
		.string()
		.meta({description: 'search pattern for test names, same as vitest -t and --testNamePattern'})
		.optional(),
});
export type Args = z.infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'run tests with vitest',
	Args,
	run: async ({args}): Promise<void> => {
		const {_: patterns, dir, fail_without_tests, t} = args;

		const package_json = await package_json_load();
		if (!package_json_has_dependency(VITEST_CLI, package_json)) {
			throw new TaskError('no test runner found, install vitest');
		}

		if (!(await find_cli(VITEST_CLI))) {
			throw new TaskError('vitest is a dependency but not installed; run `npm i`?');
		}

		const vitest_args = ['run', ...patterns];
		if (dir) {
			vitest_args.push('--dir', dir);
		}
		if (!fail_without_tests) {
			vitest_args.push('--passWithNoTests');
		}
		if (t) {
			vitest_args.push('-t', t);
		}
		vitest_args.push(...args_serialize(to_implicit_forwarded_args(VITEST_CLI)));

		const spawned = await spawn_cli(VITEST_CLI, vitest_args);
		if (!spawned?.ok) {
			throw new TaskError(
				`vitest failed: ${spawned ? spawn_result_to_message(spawned) : 'unknown error'}`,
			);
		}
	},
};
