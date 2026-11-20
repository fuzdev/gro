import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { plural } from '@fuzdev/fuz_util/string.ts';

import { has_forwarded_flag, to_forwarded_argv } from './task_args.ts';
import { assert_no_forwarded_info_flag } from './internal/forwarded_info_flags.ts';
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
import { create_paths, to_root_path } from './paths.ts';
import { project_has_dependency } from './package_json.ts';
import type { FsSearchResult } from './fs_search.ts';
import { is_fs_error } from './fs_error.ts';
import { format_error_detail } from './error_detail.ts';
import { find_svelte_files } from './svelte_helpers.ts';
import { project_has_sveltekit_app, sveltekit_sync } from './sveltekit_helpers.ts';
import { TaskError } from './task_error.ts';
import type { BuiltinPrograms, BuiltinTask, BuiltinTaskGrants } from './builtin_host.ts';

const TYPESCRIPT_CLI = 'tsc';
// the package that installs `tsc`
const TYPESCRIPT_DEP_NAME = 'typescript';

const SVELTE_FILES_PRINTED_MAX = 5;

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
	// reads `package.json` for SvelteKit and the declared checker, and `src/` for Svelte files
	fs: { read: ['.'] },
	// each loads the project's config and code, whose environment Gro can't know
	programs: {
		sveltekit_cli: { name: SVELTEKIT_CLI, local: true, pass_env: 'all' },
		svelte_check_cli: {
			name: SVELTE_CHECK_CLI,
			local: true,
			pass_env: 'all',
			// for svelte-check's colors through the pipe its output is mapped on
			env: { FORCE_COLOR: '1' }
		},
		typescript_cli: { name: TYPESCRIPT_CLI, local: true, pass_env: 'all' }
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
	run: ({ task_args, raw_rest_args, log, host }) =>
		run_typecheck(
			task_args,
			{ raw_rest_args },
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
 * Each runs only when `package.json` declares it, as vitest for `gro test`:
 * svelte-check when declared, else tsc, unless the project's `src/` has Svelte files,
 * which tsc can't check, so their project fails naming them.
 * @param args - the task's parsed args
 * @param options - the forwarded args
 * @param deps - the project's boundary, the checkers, the output streams, and the log
 * @throws `TaskError` when the checker isn't declared or installed, or the project doesn't typecheck
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

	if (await project_has_dependency(fs_paths, SVELTE_CHECK_CLI, log)) {
		// args forwarded to tsc ask for it, and svelte-check runs in its place
		if (to_forwarded_argv(TYPESCRIPT_CLI, raw_rest_args).length > 0) {
			throw new TaskError(
				`args are forwarded to ${TYPESCRIPT_CLI}, which doesn't run as ${SVELTE_CHECK_CLI} is declared` +
					` - drop the \`-- ${TYPESCRIPT_CLI}\` section, or forward to ${SVELTE_CHECK_CLI} with a \`-- ${SVELTE_CHECK_CLI}\` one`
			);
		}
		const found_svelte_check_cli = programs.find('svelte_check_cli');
		if (!found_svelte_check_cli) {
			throw new TaskError(`${SVELTE_CHECK_CLI} is a dependency but not installed; run \`npm i\`?`);
		}
		const forwarded_argv = to_forwarded_argv(SVELTE_CHECK_CLI, raw_rest_args);
		assert_no_forwarded_info_flag(SVELTE_CHECK_CLI, forwarded_argv, true);
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

	// args forwarded to svelte-check ask for it, as a `-- eslint` section asks `gro check` for lint
	if (to_forwarded_argv(SVELTE_CHECK_CLI, raw_rest_args).length > 0) {
		throw new TaskError(
			`args are forwarded to ${SVELTE_CHECK_CLI}, which isn't declared in package.json` +
				` - add ${SVELTE_CHECK_CLI} to devDependencies, or drop the \`-- ${SVELTE_CHECK_CLI}\` section`
		);
	}

	const source_dir = create_paths(fs_paths.project.path).source;
	const svelte_files = await find_svelte_files_or_throw(fs_paths, source_dir);
	if (svelte_files.length > 0) {
		throw new TaskError(
			to_undeclared_svelte_check_message(svelte_files, fs_paths.project.path, source_dir)
		);
	}

	if (!(await project_has_dependency(fs_paths, TYPESCRIPT_DEP_NAME, log))) {
		throw new TaskError(`no typechecker found, add ${TYPESCRIPT_DEP_NAME} to devDependencies`);
	}
	const found_typescript_cli = programs.find('typescript_cli');
	if (!found_typescript_cli) {
		throw new TaskError(`${TYPESCRIPT_DEP_NAME} is a dependency but not installed; run \`npm i\`?`);
	}
	const forwarded_argv = to_forwarded_argv(TYPESCRIPT_CLI, raw_rest_args);
	assert_no_forwarded_info_flag(TYPESCRIPT_CLI, forwarded_argv, true);
	// tsc's flags are case-insensitive, so a forwarded `--noemit` overrides too,
	// and `--noEmit` goes last because `--build` must be tsc's first arg
	const tsc_argv = has_forwarded_flag(forwarded_argv, 'noEmit', true)
		? forwarded_argv
		: [...forwarded_argv, '--noEmit'];
	const tsc_result = await spawn_cli(found_typescript_cli, tsc_argv, log);
	if (!spawn_result_is_ok(tsc_result)) {
		throw new TaskError(`Failed to typecheck. ${to_spawn_result_message(tsc_result)}`);
	}
};

const find_svelte_files_or_throw = async (
	fs_paths: FsPaths,
	dir: string
): Promise<Array<FsSearchResult>> => {
	try {
		return await find_svelte_files(fs_paths, dir);
	} catch (error) {
		if (!is_fs_error(error)) throw error;
		throw new TaskError(
			`failed to search ${to_root_path(dir, fs_paths.project.path)} for Svelte files, as ${SVELTE_CHECK_CLI} isn't declared: ` +
				format_error_detail(error),
			{ cause: error }
		);
	}
};

const to_undeclared_svelte_check_message = (
	svelte_files: Array<FsSearchResult>,
	root: string,
	source_dir: string
): string => {
	const count = svelte_files.length;
	const printed = svelte_files
		.slice(0, SVELTE_FILES_PRINTED_MAX)
		.map((f) => to_root_path(f.id, root));
	const unprinted = count - printed.length;
	return (
		`found ${count} Svelte file${plural(count)} in ${to_root_path(source_dir, root)} but ${SVELTE_CHECK_CLI} isn't declared in package.json: ` +
		printed.join(', ') +
		(unprinted > 0 ? ` and ${unprinted} more` : '') +
		` - add ${SVELTE_CHECK_CLI} to devDependencies, remove the Svelte files, or pass --no-typecheck to gro check`
	);
};
