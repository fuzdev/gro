/**
 * The Gro CLI's body, which a root of the CLI calls with what it opened:
 * the Node Gro's entry, `invoke.ts`, and `binary_main`, the body of the binary's CLI root,
 * made for the entry the binary generates for a command.
 *
 * It splits off the grant flags, warning that they confine nothing here, parses the CLI args,
 * loads the project's config, and invokes what the args name, the CLI's primary concept,
 * a task; to learn more about them, see `src/docs/task.md`.
 * It imports no host module itself, reads nothing of the process itself,
 * and configures nothing global:
 * the root opens the handles, picks the project loader and the platform, and configures
 * fuz_util's print colors, if it colors at all, before it calls `gro_main`, and it reports what
 * `gro_main` throws with `log_cli_error` and exits with `to_exit_code`.
 *
 * @module
 */

import { Logger } from '@fuzdev/fuz_util/log.ts';
import { st } from '@fuzdev/fuz_util/print.ts';

import { invoke_cli, type GroPlatform } from './invoke_task.ts';
import { to_grant_flags, to_raw_rest_args, to_task_args, type CliTaskArgs } from './task_args.ts';
import { ReportedError, is_reported_error, is_task_error } from './task_error.ts';
import { load_gro_config } from './gro_config.ts';
import { create_builtin_host, type BuiltinHostRoot } from './builtin_host.ts';
import { GRO_CLI_GRANTS } from './gro_builtin_tasks.ts';
import type { FsPaths } from './fs_paths.ts';
import type { ImportModule } from './modules.ts';
import { print_error_with_causes } from './internal/error_report.ts';

/**
 * What a root of the CLI hands `gro_main`, everything it reaches.
 * It mixes data, `args`, with capabilities on purpose: both are what one root opened for one
 * run, the args being that run's input rather than a handle, and a root builds the whole
 * set together, so splitting them would add a parameter and separate nothing.
 */
export interface GroMainDeps {
	/**
	 * The CLI args after the CLI's own path, as the `args` host module has them.
	 */
	readonly args: ReadonlyArray<string>;
	/**
	 * The root's boundary, built from the fs host module with the working directory and `HOME`.
	 */
	readonly fs_paths: FsPaths;
	/**
	 * The root's env, spawn, and stream handles, cut by each builtin's grants into its `host`,
	 * and by `GRO_CLI_GRANTS` into the CLI's own,
	 * with stderr taking the warning for grant flags that confine nothing.
	 */
	readonly host_root: BuiltinHostRoot;
	/**
	 * Loads the project's modules: its config, through the CLI's own host, and its tasks and genfiles.
	 */
	readonly import_module: ImportModule;
	/**
	 * Creates the module graph, loads the Svelte config, and names the running Gro.
	 */
	readonly platform: GroPlatform;
}

/**
 * Runs the Gro CLI on `deps.args`.
 * @throws `ReportedError` - for a failure already reported, like a usage error or a failed task
 * @throws for a failure not yet reported, like one loading the config, which the root reports
 * with `log_cli_error`
 */
export const gro_main = async (deps: GroMainDeps): Promise<void> => {
	const { fs_paths, host_root, import_module, platform } = deps;
	const host = create_builtin_host(host_root, GRO_CLI_GRANTS, { fs_paths, import_module });

	// a root that enforces grant flags would strip them before calling this,
	// so any left here reach a run that confines nothing
	let args: Array<string>;
	let grant_flags: Array<string>;
	try {
		({ cli_args: args, grant_flags } = to_grant_flags(deps.args));
	} catch (error) {
		if (!is_task_error(error)) throw error;
		new Logger('gro').error(st('red', error.message)); // a usage error, no stack trace
		throw new ReportedError();
	}
	if (grant_flags.length) {
		// straight to stderr, as the only notice that nothing is confined, which no log level hides
		await host_root.stderr.write(to_grant_flags_unenforced_message(grant_flags) + '\n');
	}

	let task_args: CliTaskArgs;
	try {
		task_args = to_task_args(args, grant_flags);
	} catch (error) {
		if (!is_task_error(error)) throw error;
		new Logger('gro').error(st('red', error.message)); // a usage error, no stack trace
		throw new ReportedError();
	}
	await invoke_cli(
		fs_paths,
		task_args,
		// the project's config is its code, loaded through the CLI's host as its row grants
		await load_gro_config(fs_paths, host.import_module),
		to_raw_rest_args(args),
		import_module,
		host_root,
		platform
	);
};

/**
 * The warning, once per run, for grant flags given to a Gro that doesn't enforce them,
 * written to the root's stderr, not through a logger, so no log level hides it.
 * @param grant_flags - the flags, as written
 */
export const to_grant_flags_unenforced_message = (grant_flags: ReadonlyArray<string>): string =>
	`grant flags have no effect on the Node gro, which confines nothing: ` +
	grant_flags.map((flag) => `\`${flag}\``).join(', ');

/**
 * Logs an error `gro_main` threw, or another the root caught, the way the CLI reports it:
 * a `TaskError`'s message with no stack, nothing for a `ReportedError`, already reported,
 * and anything else with its stack and causes.
 * @param error - what was thrown, any value
 * @param label - the logger's label for an error other than a `TaskError`, like its origin
 */
export const log_cli_error = (error: unknown, label: string): void => {
	if (is_task_error(error)) {
		// a known failure, so its message with no stack
		new Logger('TaskError').error(st('red', error.message));
	} else if (!is_reported_error(error)) {
		// `print_error` reads properties, so a thrown `undefined` or `null` would throw here
		new Logger(label).error(
			error == null ? `Unknown error: ${error}` : print_error_with_causes(error as Error)
		);
	}
};

/**
 * The code the CLI exits with for an uncaught error:
 * a `TaskError`'s `exit_code`, like `gro run`'s script's own, when it's an integer 1–255
 * (a process exit code is a byte, so `256` would exit `0`)
 * and it's the uncaught error itself, as when thrown from `gro.config.ts` before any task runs,
 * or failed the task the CLI invoked (the `cause` of `invoke_task`'s `ReportedError`),
 * and `1` otherwise, including for a nested task's failure.
 */
export const to_exit_code = (error: unknown): number => {
	const value = error as
		{ exit_code?: unknown; cause?: { exit_code?: unknown } } | null | undefined;
	const exit_code = value?.exit_code ?? value?.cause?.exit_code;
	return typeof exit_code === 'number' &&
		Number.isInteger(exit_code) &&
		exit_code > 0 &&
		exit_code < 256
		? exit_code
		: 1;
};
