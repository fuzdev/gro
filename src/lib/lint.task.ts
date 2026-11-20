import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

import { has_forwarded_flag, to_forwarded_argv } from './task_args.ts';
import { assert_no_forwarded_info_flag } from './internal/forwarded_info_flags.ts';
import { spawn_cli, spawn_result_is_ok, to_spawn_result_message } from './cli.ts';
import { TaskError } from './task_error.ts';
import { ESLINT_CLI } from './constants.ts';
import type { FsPaths } from './fs_paths.ts';
import { project_has_dependency } from './package_json.ts';
import type { BuiltinPrograms, BuiltinTask, BuiltinTaskGrants } from './builtin_host.ts';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(meta(s.array(s.string), { description: 'paths to lint' }), () => [])
});
export type Args = s.Infer<typeof Args>;

const grants = {
	// reads `package.json` for the eslint dependency
	fs: { read: ['.'] },
	// ESLint loads the project's config and plugins, code whose environment Gro can't know
	programs: {
		eslint_cli: { name: ESLINT_CLI, local: true, pass_env: 'all' }
	}
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'run eslint',
	Args,
	grants,
	run: ({ task_args, raw_rest_args, log, host }) =>
		run_lint(
			task_args,
			{ raw_rest_args },
			{ fs_paths: host.fs_paths, programs: host.programs, log }
		)
};

/**
 * The data `run_lint` takes beside its args.
 */
export interface LintOptions {
	/**
	 * The CLI args from the first `--` on, whose section for the ESLint CLI is forwarded to it.
	 */
	raw_rest_args: ReadonlyArray<string>;
}

/**
 * What `run_lint` reaches, picked from a builtin's `host` and context.
 */
export interface LintDeps {
	fs_paths: FsPaths;
	programs: Pick<BuiltinPrograms<'eslint_cli'>, 'find'>;
	log: Logger;
}

/**
 * Lints the project with ESLint, `gro lint`'s body.
 * ESLint runs only when `package.json` declares it, as vitest for `gro test`.
 * @param args - the task's parsed args
 * @param options - the forwarded args
 * @param deps - the project's boundary, ESLint, and the log
 * @throws `TaskError` when eslint isn't declared or installed, or finds problems
 */
export const run_lint = async (args: Args, options: LintOptions, deps: LintDeps): Promise<void> => {
	const { _ } = args;
	const { fs_paths, programs, log } = deps;

	if (!(await project_has_dependency(fs_paths, ESLINT_CLI, log))) {
		throw new TaskError('no linter found, add eslint to devDependencies');
	}

	const found_eslint_cli = programs.find('eslint_cli');
	if (!found_eslint_cli) {
		throw new TaskError('eslint is a dependency but not installed; run `npm i`?');
	}

	// the forwarded text goes verbatim after Gro's own args, which drop what it overrides
	const forwarded_argv = to_forwarded_argv(ESLINT_CLI, options.raw_rest_args);
	assert_no_forwarded_info_flag(ESLINT_CLI, forwarded_argv, true);
	const serialized_args = [..._];
	if (!has_forwarded_flag(forwarded_argv, 'max-warnings')) {
		serialized_args.push('--max-warnings', '0');
	}
	serialized_args.push(...forwarded_argv);
	const eslint_result = await spawn_cli(found_eslint_cli, serialized_args, log);
	if (!spawn_result_is_ok(eslint_result)) {
		throw new TaskError(`ESLint found some problems. ${to_spawn_result_message(eslint_result)}`);
	}
};
