import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';

import { to_forwarded_argv, to_implicit_forwarded_argv, to_raw_rest_args } from './task_args.ts';
import { find_cli, spawn_cli, spawn_result_is_ok, to_spawn_result_message } from './cli.ts';
import { VITEST_CLI } from './constants.ts';
import { paths } from './paths.ts';
import { fs } from './fs.ts';
import { spawn } from './spawn.ts';
import { process } from './process.ts';
import { create_fs_paths } from './fs_paths.ts';
import { TaskError, type Task } from './task.ts';
import { project_has_vitest, TEST_FILE_PATTERN } from './vitest_helpers.ts';

const TEST_TASK_NAME = 'test';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(
		meta(s.array(s.string), { description: 'file patterns to filter tests' }),
		() => [TEST_FILE_PATTERN]
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
	run: async ({ task_args, log }): Promise<void> => {
		const { _: patterns, dir, fail_without_tests, t } = task_args;

		if (!(await project_has_vitest(create_fs_paths(fs), log))) {
			throw new TaskError('no test runner found, add vitest to devDependencies');
		}

		const vitest = find_cli(spawn, VITEST_CLI, { pass_env: 'all' });
		if (!vitest) {
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
			...to_implicit_forwarded_argv(TEST_TASK_NAME, process.args, process.cwd(), VITEST_CLI),
			...to_forwarded_argv(VITEST_CLI, to_raw_rest_args(process.args))
		);

		const result = await spawn_cli(vitest, vitest_args);
		if (!spawn_result_is_ok(result)) {
			throw new TaskError(`vitest failed: ${to_spawn_result_message(result)}`);
		}
	}
};
