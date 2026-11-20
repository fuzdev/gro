import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

import { has_forwarded_flag, to_forwarded_argv } from './task_args.ts';
import { spawn_cli, spawn_result_is_ok, to_spawn_result_message } from './cli.ts';
import { TaskError } from './task_error.ts';
import type { BuiltinPrograms, BuiltinTask, BuiltinTaskGrants } from './builtin_host.ts';

const ESLINT_CLI = 'eslint';

/** @nodocs */
export const Args = s.object({
	_: s.with_default_fn(meta(s.array(s.string), { description: 'paths to lint' }), () => []),
	eslint_cli: s.with_default(meta(s.string, { description: 'the ESLint CLI to use' }), ESLINT_CLI)
});
export type Args = s.Infer<typeof Args>;

const grants = {
	// ESLint loads the project's config and plugins, code whose environment Gro can't know
	programs: {
		eslint_cli: {
			name: ESLINT_CLI,
			name_from: 'arg',
			local_first: true,
			pass_env: 'all'
		}
	}
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'run eslint',
	Args,
	grants,
	run: ({ task_args, log, host }) =>
		run_lint(task_args, { raw_rest_args: host.raw_rest_args }, { programs: host.programs, log })
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
	programs: Pick<BuiltinPrograms<'eslint_cli'>, 'find'>;
	log: Logger;
}

/**
 * Lints the project with ESLint, `gro lint`'s body.
 * @param args - the task's parsed args
 * @param options - the forwarded args
 * @param deps - ESLint and the log
 */
export const run_lint = async (args: Args, options: LintOptions, deps: LintDeps): Promise<void> => {
	const { _, eslint_cli } = args;
	const { programs, log } = deps;

	const found_eslint_cli = programs.find('eslint_cli', eslint_cli);
	if (!found_eslint_cli) {
		// TODO maybe make this an option?
		log.info('ESLint is not installed; skipping linting');
		return;
	}

	// the forwarded text goes verbatim after Gro's own args, which drop what it overrides
	const forwarded_argv = to_forwarded_argv(eslint_cli, options.raw_rest_args);
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
