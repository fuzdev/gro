import {args_parse, type Args} from '@fuzdev/fuz_util/args.ts';
import type {Logger} from '@fuzdev/fuz_util/log.ts';
import type {Timings} from '@fuzdev/fuz_util/timings.ts';
import {styleText as st} from 'node:util';
import {z} from 'zod';

import type {Filer} from './filer.ts';
import type {GroConfig} from './gro_config.ts';
import type {invoke_task as base_invoke_task} from './invoke_task.ts';
import {default_svelte_config} from './svelte_config.ts';
import {TaskError, type TaskModuleMeta} from './task.ts';
import {log_task_help} from './task_logging.ts';

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
	filer: Filer,
	log: Logger,
	timings: Timings,
): Promise<RunTaskResult> => {
	const {task} = task_meta.mod;

	if (unparsed_args.help) {
		log_task_help(log, task_meta);
		return {ok: true, output: null};
	}

	// Parse and validate args.
	let args = unparsed_args;
	if (task.Args) {
		const parsed = args_parse(unparsed_args, task.Args);
		if (!parsed.success) {
			throw new TaskError(
				`Failed task args validation for task '${task_meta.name}':\n${z.prettifyError(parsed.error)}`,
			);
		}
		args = parsed.data;
	}

	// Run the task.
	let output: unknown; // TODO generic
	try {
		output = await task.run({
			args,
			config,
			svelte_config: default_svelte_config,
			filer,
			log,
			timings,
			invoke_task: (invoked_task_name, invoked_args, invoked_config) =>
				invoke_task(invoked_task_name, invoked_args, invoked_config ?? config, filer, timings, log),
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
							task_meta.name,
						)}. If this is unexpected try running \`${config.pm_cli} install\` and \`gro clean\`.`,
			),
			error,
		};
	}
	return {ok: true, output};
};
