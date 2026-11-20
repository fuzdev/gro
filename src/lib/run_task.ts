import { args_parse, type Args } from '@fuzdev/fuz_util/args.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { Timings } from '@fuzdev/fuz_util/timings.ts';
import { st } from '@fuzdev/fuz_util/print.ts';
import { format_issue } from '@fuzdev/fuz_schema/schema.ts';

import type { ModuleGraph } from './module_graph.ts';
import type { GroConfig } from './gro_config.ts';
import type { FsPaths } from './fs_paths.ts';
import type { ImportModule } from './modules.ts';
import type { invoke_task as base_invoke_task } from './invoke_task.ts';
import { load_default_svelte_config } from './svelte_config.ts';
import type { InvokeTaskContextOptions, TaskContext, TaskModuleMeta } from './task.ts';
import { is_reported_error, is_task_error, TaskError } from './task_error.ts';
import {
	create_builtin_host,
	type BuiltinHostRoot,
	type BuiltinTaskContext,
	type BuiltinTaskGrants
} from './builtin_host.ts';
import { log_task_help } from './task_logging.ts';
import { is_fs_error } from './fs_error.ts';
import { is_net_error } from './net_error.ts';
import { is_spawn_error } from './spawn_error.ts';
import { is_env_error } from './env_error.ts';
import { is_stdio_error } from './stdio_error.ts';
import { GRO_BUILTIN_TASK_SUMMARIES } from './gro_builtin_tasks.ts';

// copied when this module loads; the grants and the table are still read when a task runs,
// so code in the same realm isn't held by the invokes check
const { hasOwn, keys } = Object;

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
 * and `fs_paths`, `raw_rest_args`, `import_module`, and `host_root` pass to the tasks it invokes,
 * never onto its context, but for a builtin's `host`, cut from them by its `grants`.
 */
export interface RunTaskOptions {
	/**
	 * The `invoke_task` the task's own `invoke_task` calls,
	 * passing it this task's `config`, `fs_paths`, `raw_rest_args`, `import_module`, `graph`,
	 * `timings`, and `log`.
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
	/**
	 * Loads the modules of the tasks this task invokes.
	 */
	import_module: ImportModule;
	/**
	 * The root's handles, passed to the tasks this task invokes.
	 */
	host_root: BuiltinHostRoot;
	/**
	 * The task's grants when it's a builtin that declares them, cutting `host_root` into its `host`,
	 * and `null` for a project task, whose context has no `host`.
	 */
	grants: BuiltinTaskGrants | null;
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
	const {
		invoke_task,
		config,
		fs_paths,
		raw_rest_args,
		import_module,
		host_root,
		grants,
		graph,
		log,
		timings
	} = options;
	const { task } = task_meta.mod;

	if (unparsed_args.help) {
		log_task_help(log, task_meta);
		return { ok: true, output: null };
	}

	// a builtin's task run by path, or re-exported from a project module, has grants but no name,
	// and a path is a file, while a builtin's grants are known before it starts only by its name
	if (!grants && hasOwn(task, 'grants')) {
		const error = new TaskError(to_builtin_by_path_message(task_meta.name, task.summary));
		return { ok: false, reason: error.message, error };
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
			return { ok: false, reason: error.message, error };
		}
		args = parsed.value;
	}

	const ctx: TaskContext<Args> = {
		task_args: args,
		config,
		get svelte_config() {
			return load_default_svelte_config();
		},
		graph,
		log,
		timings,
		// the registry typing is for callers, the runtime takes any name and args
		invoke_task: async (
			invoked_task_name: string,
			invoked_args?: Args,
			invoked_options?: InvokeTaskContextOptions
		) => {
			if (grants && !is_invoke_granted(grants, invoked_task_name)) {
				throw new TaskError(
					`Task ${st('cyan', task_meta.name)} invokes \`${invoked_task_name}\`, which its grants don't name.`
				);
			}
			await invoke_task(invoked_task_name, invoked_args, {
				config: invoked_options?.config ?? config,
				fs_paths,
				raw_rest_args,
				import_module,
				host_root,
				graph,
				timings,
				parent_log: log
			});
		}
	};
	if (grants) {
		(ctx as BuiltinTaskContext<Args>).host = create_builtin_host(
			host_root,
			grants,
			fs_paths,
			raw_rest_args
		);
	}

	// Run the task.
	let output: unknown; // TODO generic
	try {
		output = await task.run(ctx);
	} catch (error) {
		return {
			ok: false,
			reason: is_task_error(error)
				? error.message
				: // already reported, like a failed nested `invoke_task`,
					// or a host module's, the task's own failure like a missing file, its stack logged
					is_reported_error(error) || is_host_module_error(error)
					? `Task ${st('cyan', task_meta.name)} failed.`
					: `Unexpected error running task ${st(
							'cyan',
							task_meta.name
						)}. If this is unexpected try running \`${config.pm_cli} install\` and \`gro clean\`.`,
			error
		};
	}
	return { ok: true, output };
};

// an error from a host module, from any copy of Gro, which a broken install doesn't explain
const is_host_module_error = (error: unknown): boolean =>
	is_fs_error(error) ||
	is_spawn_error(error) ||
	is_net_error(error) ||
	is_env_error(error) ||
	is_stdio_error(error);

/**
 * Whether `grants` names `task_name` among the builtins it `invokes`.
 */
const is_invoke_granted = (grants: BuiltinTaskGrants, task_name: string): boolean => {
	const { invokes } = grants;
	if (!invokes) return false;
	for (let i = 0; i < invokes.length; i++) {
		if (invokes[i] === task_name) return true;
	}
	return false;
};

/**
 * The refusal of a task with `grants` that wasn't invoked as a builtin by name,
 * naming the builtin whose summary it has, when one does.
 */
const to_builtin_by_path_message = (name: string, summary: string | undefined): string => {
	const builtin = keys(GRO_BUILTIN_TASK_SUMMARIES).find(
		(builtin_name) =>
			GRO_BUILTIN_TASK_SUMMARIES[builtin_name as keyof typeof GRO_BUILTIN_TASK_SUMMARIES] ===
			summary
	);
	return (
		`Task ${st('cyan', name)} declares \`grants\`, so it runs only as a builtin, by name` +
		(builtin === undefined ? '.' : `: \`gro ${builtin}\`, or \`invoke_task('${builtin}')\`.`)
	);
};
