import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

import { has_forwarded_flag, to_forwarded_argv } from './task_args.ts';
import { assert_no_forwarded_info_flag } from './internal/forwarded_info_flags.ts';
import { spawn_cli, spawn_result_is_ok, to_spawn_result_message } from './cli.ts';
import { VITEST_CLI } from './constants.ts';
import { create_paths } from './paths.ts';
import { cwd } from './cwd.ts';
import type { FsPaths } from './fs_paths.ts';
import { TaskError } from './task_error.ts';
import { TEST_FILE_PATTERN } from './vitest_helpers.ts';
import { project_has_dependency } from './package_json.ts';
import {
	project_has_sveltekit_app,
	SVELTEKIT_CLI_GRANT,
	sveltekit_sync
} from './sveltekit_helpers.ts';
import type { BuiltinPrograms, BuiltinTask, BuiltinTaskGrants } from './builtin_host.ts';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(
		meta(s.array(s.string), { description: 'file patterns to filter tests' }),
		() => [TEST_FILE_PATTERN]
	),
	dir: s.with_default_fn(
		meta(s.string, { description: 'working directory for tests' }),
		() => create_paths(cwd()).source
	),
	fail_without_tests: s.with_default(
		meta(s.boolean, { description: 'opt out of `passWithNoTests`' }),
		false
	),
	t: s.optional(
		meta(s.string, {
			description: 'search pattern for test names, same as vitest -t and --testNamePattern'
		})
	),
	sync: s.with_default(meta(s.boolean, { description: 'dual of no-sync' }), true),
	'no-sync': s.with_default(
		meta(s.boolean, { description: 'opt out of syncing SvelteKit before the tests' }),
		false
	)
});
export type Args = s.Infer<typeof Args>;

const grants = {
	// reads `package.json` for the vitest dependency
	fs: { read: ['.'] },
	programs: {
		// syncs SvelteKit's files before vitest, which then finds them current
		// and writes nothing of `node_modules/$app`, which only `svelte-kit` writes
		sveltekit_cli: SVELTEKIT_CLI_GRANT,
		vitest_cli: {
			name: VITEST_CLI,
			local: true,
			// vitest loads the project's config and tests, code whose environment Gro can't know
			pass_env: 'all',
			// SvelteKit's plugin writes the modules a dev server loads and the route types,
			// Vite bundles the config, and Vitest keeps its own cache
			write: [
				'.svelte-kit/generated/dev',
				'.svelte-kit/types',
				'node_modules/.vite-temp',
				'node_modules/.vite/vitest'
			]
		}
	}
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'run tests with vitest',
	Args,
	grants,
	run: ({ task_args, raw_rest_args, implicit_rest_args, log, host }) =>
		run_test(
			task_args,
			{ raw_rest_args, implicit_rest_args },
			{ fs_paths: host.fs_paths, programs: host.programs, log }
		)
};

/**
 * The data `run_test` takes beside its args.
 */
export interface TestOptions {
	/**
	 * The CLI args from the first `--` on, whose `vitest` section is forwarded to vitest.
	 */
	raw_rest_args: ReadonlyArray<string>;
	/**
	 * The section forwarded to vitest without naming it, like `gro test -- --watch`,
	 * dropped when it starts with `vitest`, as that's the explicit section.
	 */
	implicit_rest_args: ReadonlyArray<string>;
}

/**
 * What `run_test` reaches, picked from a builtin's `host` and context.
 */
export interface TestDeps {
	fs_paths: FsPaths;
	programs: Pick<BuiltinPrograms<'sveltekit_cli' | 'vitest_cli'>, 'find'>;
	log: Logger;
}

/**
 * Runs the project's tests with vitest, `gro test`'s body,
 * Vitest's module cache off unless a forwarded flag names it.
 * A SvelteKit project is synced first, as `gro typecheck` syncs it, unless `sync` is off,
 * as `gro check` turns it off after its typecheck has synced:
 * SvelteKit's plugin in vitest then finds its tsconfig and types in `node_modules/$app` current
 * and writes nothing there, which only `svelte-kit` writes, since a later typecheck, `dev`, `build`,
 * and the editor trust what's there.
 * @param args - the task's parsed args
 * @param options - the forwarded args
 * @param deps - the project's boundary, `svelte-kit` and vitest, and the log
 */
export const run_test = async (args: Args, options: TestOptions, deps: TestDeps): Promise<void> => {
	const { _: patterns, dir, fail_without_tests, t, sync } = args;
	const { raw_rest_args, implicit_rest_args } = options;
	const { fs_paths, programs, log } = deps;

	if (!(await project_has_dependency(fs_paths, VITEST_CLI, log))) {
		throw new TaskError('no test runner found, add vitest to devDependencies');
	}

	const vitest = programs.find('vitest_cli');
	if (!vitest) {
		throw new TaskError('vitest is a dependency but not installed; run `npm i`?');
	}

	if (sync && (await project_has_sveltekit_app(fs_paths, log))) {
		await sveltekit_sync({ find: () => programs.find('sveltekit_cli') });
	}

	// `gro test -- --watch` when the CLI invoked `test`, and `-- vitest --watch` at any depth;
	// Vitest ignores a version flag after `run`, so only help is refused
	const implicit_argv = implicit_rest_args[0] === VITEST_CLI ? [] : implicit_rest_args;
	const forwarded_argv = to_forwarded_argv(VITEST_CLI, raw_rest_args);
	assert_no_forwarded_info_flag(VITEST_CLI, implicit_argv, false);
	assert_no_forwarded_info_flag(VITEST_CLI, forwarded_argv, false);

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
	const user_argv = [...implicit_argv, ...forwarded_argv];
	// Vitest runs a cached module's code as read in later runs, so a test could plant code there;
	// a forwarded flag decides instead, in camel or kebab case, as Vitest reads both
	if (
		!has_forwarded_flag(user_argv, 'fsModuleCache') &&
		!has_forwarded_flag(user_argv, 'fs-module-cache')
	) {
		vitest_args.push('--fsModuleCache=false');
	}
	vitest_args.push(...user_argv);

	const result = await spawn_cli(vitest, vitest_args);
	if (!spawn_result_is_ok(result)) {
		throw new TaskError(`vitest failed: ${to_spawn_result_message(result)}`);
	}
};
