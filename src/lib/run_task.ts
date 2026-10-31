import { args_parse, type Args } from '@fuzdev/fuz_util/args.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { Timings } from '@fuzdev/fuz_util/timings.ts';
import { styleText as st } from 'node:util';
import { format_issue } from '@fuzdev/fuz_schema/schema.ts';

import type { ModuleGraph, Watcher } from './module_graph.ts';
import type { GroConfig } from './gro_config.ts';
import type { invoke_task as base_invoke_task } from './invoke_task.ts';
import { load_default_svelte_config } from './svelte_config.ts';
import { TaskError, type TaskModuleMeta } from './task.ts';
import { log_task_help } from './task_logging.ts';

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

export const run_task = async (
	task_meta: TaskModuleMeta,
	unparsed_args: Args,
	invoke_task: typeof base_invoke_task,
	config: GroConfig,
	graph: ModuleGraph,
	watcher: Watcher,
	log: Logger,
	timings: Timings
): Promise<RunTaskResult> => {
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
			throw new TaskError(
				`Failed task args validation for task '${task_meta.name}':\n${parsed.issues
					.map(format_issue)
					.join('\n')}`
			);
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
			watcher,
			log,
			timings,
			// the registry typing is for callers, the runtime takes any name and args
			invoke_task: (invoked_task_name: string, invoked_args?: Args, invoked_config?: GroConfig) =>
				invoke_task(
					invoked_task_name,
					invoked_args,
					invoked_config ?? config,
					{ graph, watcher },
					timings,
					log
				)
		});
	} catch (error) {
		return {
			ok: false,
			reason: st(
				'red',
				error?.constructor?.name === 'TaskError'
					? (error.message as string)
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
