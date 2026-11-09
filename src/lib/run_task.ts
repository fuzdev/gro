import { args_parse, type Args } from '@fuzdev/fuz_util/args.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { Timings } from '@fuzdev/fuz_util/timings.ts';
import { styleText as st } from 'node:util';
import { format_issue } from '@fuzdev/fuz_schema/schema.ts';

import type { ModuleGraph } from './module_graph.ts';
import type { GroConfig } from './gro_config.ts';
import type { FsPaths } from './fs_paths.ts';
import type { invoke_task as base_invoke_task } from './invoke_task.ts';
import { load_default_svelte_config } from './svelte_config.ts';
import {
	is_silent_error,
	is_task_error,
	TaskError,
	type InvokeTaskContextOptions,
	type TaskModuleMeta
} from './task.ts';
import { log_task_help } from './task_logging.ts';

/**
 * A task's run: its return value as `output`, or why it failed, with the error.
 */
export type RunTaskResult =
	| {
			ok: true;
			output: unknown;
	  }
	| {
			ok: false;
			reason: string;
			error: Error;
	  };

/**
 * What `run_task` builds the task's context from:
 * `config`, `graph`, `log`, and `timings` become its fields of the same name,
 * and `fs_paths` and `raw_rest_args` pass to the tasks it invokes, never onto its context.
 */
export interface RunTaskOptions {
	/**
	 * The `invoke_task` the task's own `invoke_task` calls,
	 * passing it this task's `config`, `fs_paths`, `raw_rest_args`, `graph`, `timings`, and `log`.
	 */
	invoke_task: typeof base_invoke_task;
	config: GroConfig;
	/**
	 * The invoking root's boundary, passed to the tasks this task invokes.
	 */
	fs_paths: FsPaths;
	/**
	 * The CLI args from the first `--` on, passed to the tasks this task invokes.
	 */
	raw_rest_args: ReadonlyArray<string>;
	graph: ModuleGraph;
	log: Logger;
	timings: Timings;
}

/**
 * Validates `unparsed_args` against the task's `Args` and runs the task,
 * or logs its help for `--help`.
 * A failure, invalid args or a thrown error, is returned, not thrown,
 * so the invoking `invoke_task` reports it once.
 * @param task_meta - the loaded task module
 * @param unparsed_args - the args before validation, forwarded args already merged
 * @param options - what the task's context is built from
 */
export const run_task = async (
	task_meta: TaskModuleMeta,
	unparsed_args: Args,
	options: RunTaskOptions
): Promise<RunTaskResult> => {
	const { invoke_task, config, fs_paths, raw_rest_args, graph, log, timings } = options;
	const { task } = task_meta.mod;

	if (unparsed_args.help) {
		log_task_help(log, task_meta);
		return { ok: true, output: null };
	}

	// Parse and validate args.
	let args = unparsed_args;
	if (task.Args) {
		const parsed = args_parse(unparsed_args, task.Args);
		if (!parsed.ok) {
			const error = new TaskError(
				`Failed task args validation for task '${task_meta.name}':\n${parsed.issues
					.map(format_issue)
					.join('\n')}`
			);
			return { ok: false, reason: st('red', error.message), error };
		}
		args = parsed.value;
	}

	// Run the task.
	let output: unknown; // TODO generic
	try {
		output = await task.run({
			args,
			config,
			get svelte_config() {
				return load_default_svelte_config();
			},
			graph,
			log,
			timings,
			// the registry typing is for callers, the runtime takes any name and args
			invoke_task: (
				invoked_task_name: string,
				invoked_args?: Args,
				invoked_options?: InvokeTaskContextOptions
			) =>
				invoke_task(invoked_task_name, invoked_args, {
					config: invoked_options?.config ?? config,
					fs_paths,
					raw_rest_args,
					graph,
					timings,
					parent_log: log
				})
		});
	} catch (error) {
		return {
			ok: false,
			reason: st(
				'red',
				is_task_error(error)
					? error.message
					: is_silent_error(error) // already reported, like a failed nested `invoke_task`
						? `Task ${st('cyan', task_meta.name)} failed.`
						: `Unexpected error running task ${st(
								'cyan',
								task_meta.name
							)}. If this is unexpected try running \`${config.pm_cli} install\` and \`gro clean\`.`
			),
			error
		};
	}
	return { ok: true, output };
};
