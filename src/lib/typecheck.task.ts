import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

import { has_forwarded_flag, to_forwarded_argv } from './task_args.ts';
import type { ProcessOutput } from './stdout.ts';
import { configure_colored_output_with_path_replacement } from './child_process_logging.ts';
import {
	spawn_cli,
	spawn_cli_process,
	spawn_result_is_ok,
	to_spawn_result_message
} from './cli.ts';
import { SVELTE_CHECK_CLI, SVELTEKIT_CLI } from './constants.ts';
import type { FsPaths } from './fs_paths.ts';
import { cwd as process_cwd } from './cwd.ts';
import { create_paths } from './paths.ts';
import { project_has_sveltekit_app, sveltekit_sync } from './sveltekit_helpers.ts';
import { TaskError } from './task_error.ts';
import type { BuiltinPrograms, BuiltinTask, BuiltinTaskGrants } from './builtin_host.ts';

const TYPESCRIPT_CLI = 'tsc';

/** @nodocs */
export const Args = s.object({
	path_replacement: s.with_default(
		meta(s.string, { description: 'replacement string for current working directory in output' }),
		'.'
	),
	cwd: s.with_default_fn(
		meta(s.string, { description: 'current working directory' }),
		() => create_paths(process_cwd()).root
	)
});
export type Args = s.Infer<typeof Args>;

const grants = {
	// reads `package.json` for SvelteKit
	fs: { read: ['.'] },
	// each loads the project's config and code, whose environment Gro can't know
	programs: {
		sveltekit_cli: { name: SVELTEKIT_CLI, local_first: true, pass_env: 'all' },
		svelte_check_cli: {
			name: SVELTE_CHECK_CLI,
			local_first: true,
			pass_env: 'all',
			// for svelte-check's colors through the pipe its output is mapped on
			env: { FORCE_COLOR: '1' }
		},
		typescript_cli: { name: TYPESCRIPT_CLI, local_first: true, pass_env: 'all' }
	},
	// svelte-check's output, its paths replaced
	stdout: true,
	stderr: true
} as const satisfies BuiltinTaskGrants;

/** @nodocs */
export const task: BuiltinTask<Args, typeof grants> = {
	summary: 'run svelte-check or tsc on the project without emitting any files',
	Args,
	grants,
	run: ({ task_args, log, host }) =>
		run_typecheck(
			task_args,
			{ raw_rest_args: host.raw_rest_args },
			{
				fs_paths: host.fs_paths,
				programs: host.programs,
				stdout: host.stdout,
				stderr: host.stderr,
				log
			}
		)
};

/**
 * The data `run_typecheck` takes beside its args.
 */
export interface TypecheckOptions {
	/**
	 * The CLI args from the first `--` on, whose section for the checker that runs is forwarded to it.
	 */
	raw_rest_args: ReadonlyArray<string>;
}

/**
 * What `run_typecheck` reaches, picked from a builtin's `host` and context.
 */
export interface TypecheckDeps {
	fs_paths: FsPaths;
	programs: Pick<BuiltinPrograms<'sveltekit_cli' | 'svelte_check_cli' | 'typescript_cli'>, 'find'>;
	stdout: ProcessOutput;
	stderr: ProcessOutput;
	log: Logger;
}

/**
 * Typechecks the project with svelte-check, or tsc without it, `gro typecheck`'s body.
 * @param args - the task's parsed args
 * @param options - the forwarded args
 * @param deps - the project's boundary, the checkers, the output streams, and the log
 */
export const run_typecheck = async (
	args: Args,
	options: TypecheckOptions,
	deps: TypecheckDeps
): Promise<void> => {
	const { path_replacement, cwd } = args;
	const { raw_rest_args } = options;
	const { fs_paths, programs, stdout, stderr, log } = deps;

	if (await project_has_sveltekit_app(fs_paths, log)) {
		await sveltekit_sync({ find: () => programs.find('sveltekit_cli') });
	}

	// Prefer svelte-check if available.
	const found_svelte_check_cli = programs.find('svelte_check_cli');
	if (found_svelte_check_cli) {
		const forwarded_argv = to_forwarded_argv(SVELTE_CHECK_CLI, raw_rest_args);
		const svelte_check_child = await spawn_cli_process(
			found_svelte_check_cli,
			forwarded_argv,
			undefined,
			{ stdio: ['inherit', 'pipe', 'pipe'] }
		);

		// Configure process output with path replacement while preserving colors
		const output_written = configure_colored_output_with_path_replacement(
			svelte_check_child,
			path_replacement,
			cwd,
			{ stdout, stderr }
		);
		const [svelte_check_result] = await Promise.all([svelte_check_child.closed, output_written]);

		if (!spawn_result_is_ok(svelte_check_result)) {
			throw new TaskError(`Failed to typecheck. ${to_spawn_result_message(svelte_check_result)}`);
		}

		return;
	}

	// Fall back to tsc.
	const found_typescript_cli = programs.find('typescript_cli');
	if (found_typescript_cli) {
		const forwarded_argv = to_forwarded_argv(TYPESCRIPT_CLI, raw_rest_args);
		// tsc's flags are case-insensitive, so a forwarded `--noemit` overrides too,
		// and `--noEmit` goes last because `--build` must be tsc's first arg
		const tsc_argv = has_forwarded_flag(forwarded_argv, 'noEmit', true)
			? forwarded_argv
			: [...forwarded_argv, '--noEmit'];
		const tsc_result = await spawn_cli(found_typescript_cli, tsc_argv, log);
		if (!spawn_result_is_ok(tsc_result)) {
			throw new TaskError(`Failed to typecheck. ${to_spawn_result_message(tsc_result)}`);
		}
		return;
	}

	throw new TaskError(
		`Failed to typecheck because neither \`${SVELTE_CHECK_CLI}\` nor \`${TYPESCRIPT_CLI}\` was found`
	);
};
