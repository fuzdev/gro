import { spawn_result_to_message } from '@fuzdev/fuz_util/process.ts';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { to_forwarded_argv, to_implicit_forwarded_argv } from './args.ts';
import { find_cli, spawn_cli } from './cli.ts';
import { VITEST_CLI } from './constants.ts';
import { package_json_has_dependency, package_json_load } from './package_json.ts';
import { paths } from './paths.ts';
import { TaskError, type Task } from './task.ts';

const TEST_TASK_NAME = 'test';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(
		meta(s.array(s.string), { description: 'file patterns to filter tests' }),
		() => ['.test.']
	),
	dir: s.with_default(meta(s.string, { description: 'working directory for tests' }), paths.source),
	fail_without_tests: s.with_default(
		meta(s.boolean, { description: 'opt out of `passWithNoTests`' }),
		false
	),
	t: s.optional(
		meta(s.string, {
			description: 'search pattern for test names, same as vitest -t and --testNamePattern'
		})
	)
});
export type Args = s.Infer<typeof Args>;

/** @nodocs */
export const task: Task<Args> = {
	summary: 'run tests with vitest',
	Args,
	run: async ({ args }): Promise<void> => {
		const { _: patterns, dir, fail_without_tests, t } = args;

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
		// `gro test -- --watch` when the CLI invoked `test`, and `-- vitest --watch` at any depth
		vitest_args.push(
			...to_implicit_forwarded_argv(TEST_TASK_NAME, VITEST_CLI),
			...to_forwarded_argv(VITEST_CLI)
		);

		const spawned = await spawn_cli(VITEST_CLI, vitest_args);
		if (!spawned?.ok) {
			throw new TaskError(
				`vitest failed: ${spawned ? spawn_result_to_message(spawned) : 'unknown error'}`
			);
		}
	}
};
