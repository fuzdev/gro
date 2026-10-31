import type { Args } from '@fuzdev/fuz_util/args.ts';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { print_ms, print_timings } from '@fuzdev/fuz_util/print.ts';
import { create_stopwatch, Timings } from '@fuzdev/fuz_util/timings.ts';
import { stat } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { styleText as st } from 'node:util';

import { merge_forwarded_args, to_forwarded_args, type CliTaskArgs } from './args.ts';
import { run_task } from './run_task.ts';
import {
	is_builtin_task_name,
	is_task_path,
	is_task_path_input,
	load_task,
	SilentError,
	TaskError,
	to_builtin_task_id,
	to_task_command,
	to_task_path,
	TASK_FILE_SUFFIX_TS,
	TASK_VERB,
	type TaskContext,
	type TaskModuleMeta
} from './task.ts';
import { package_json_load_for_gro } from './package_json.ts';
import { log_builtin_tasks, log_error_reasons, log_tasks } from './task_logging.ts';
import type { GroConfig } from './gro_config.ts';
import { create_node_module_graph, create_node_watcher } from './module_graph_node.ts';
import { task_registry_find } from './task_registry.ts';
import { GRO_BUILTIN_TASK_SUMMARIES } from './gro_builtin_tasks.ts';
import { LIB_DIR, SOURCE_DIR } from './constants.ts';
import { print_path } from './paths.ts';

/**
 * The prefix that once forced a builtin, now an error naming the plain builtin.
 */
const REMOVED_BUILTIN_PREFIX = 'gro/';

/**
 * What `invoke_task` runs a task with, after the task name and args.
 */
export interface InvokeTaskOptions {
	/**
	 * The Gro configuration.
	 */
	config: GroConfig;
	/**
	 * The `graph` and `watcher` from the invoking task,
	 * created from `config.watch` and closed on finish when absent.
	 */
	watching?: Pick<TaskContext, 'graph' | 'watcher'>;
	/**
	 * The invoking task's timings, shared by nested tasks;
	 * absent for the top-level task, which creates them and prints them on success.
	 */
	timings?: Timings;
	/**
	 * The invoking task's logger, which labels this task's logger as its child.
	 */
	parent_log?: Logger;
}

/**
 * Invokes a task by builtin name or by path.
 *
 * A name always means one of Gro's builtins, so a project task never shadows one.
 * A path, an input containing a `/` or ending in `.task.ts` or `.task.js`,
 * always means a task file, relative to the cwd (the project root) or absolute,
 * and must end in a task suffix.
 * The task is displayed, and its forwarded args keyed, by its builtin name
 * or its path relative to the project root, like `gro check -- gro task src/x.task.ts --a`.
 * Nothing is searched for: a bare name that's not a builtin is an error.
 *
 * @param task_name - a builtin name or a task path
 * @param args - the CLI args to pass to the task
 * @param options - the config and what the invoking task shares
 */
export const invoke_task = async (
	task_name: string,
	args: Args | undefined,
	options: InvokeTaskOptions
): Promise<void> => {
	const { config, watching: initial_watching, timings: initial_timings, parent_log } = options;
	// Create child logger if parent exists, otherwise root logger,
	// labeled by the name the task is displayed by when it's a normalized path
	const log_label = is_task_path(task_name) ? to_task_path(task_name) : task_name || 'gro';
	const log = parent_log ? parent_log.child(log_label) : new Logger(log_label);
	log.info('invoking', st('cyan', log_label));

	const resolved = await resolve_task(task_name);
	if (!resolved.ok) {
		log_error_reasons(log, resolved.reasons);
		throw new SilentError();
	}
	const loaded = await load_task(resolved.id, resolved.name);
	if (!loaded.ok) {
		log_error_reasons(log, [loaded.reason]);
		throw new SilentError();
	}
	const task = loaded.value;

	let forwarded: Args;
	try {
		forwarded = to_forwarded_args(to_task_command(task.name));
	} catch (error) {
		if (!(error instanceof TaskError)) throw error;
		log_error_reasons(log, [error.message]);
		throw new SilentError();
	}

	// the graph and watcher load lazily, so a task that never uses them never loads chokidar
	const owns_watching = !initial_watching;
	const { graph, watcher } = initial_watching ?? {
		graph: create_node_module_graph({ watch: config.watch, log: log.child('graph') }),
		watcher: create_node_watcher({ watch: config.watch })
	};

	const owns_timings = !initial_timings;
	const timings = initial_timings ?? new Timings();

	const total_timing = create_stopwatch();

	log.info(
		`→ ${st('cyan', task.name)} ${
			(task.mod.task.summary && st('gray', task.mod.task.summary)) ?? ''
		}`
	);

	const timing_to_run_task = timings.start('run task ' + task.name);
	let result;
	try {
		result = await run_task(task, merge_forwarded_args(args, forwarded), {
			invoke_task,
			config,
			graph,
			watcher,
			log,
			timings
		});
	} finally {
		// close only what we created, failed or not, a no-op when it never loaded
		if (owns_watching) {
			await Promise.all([graph.close(), watcher.close()]);
		}
	}
	timing_to_run_task();
	if (!result.ok) {
		log.info(`${st('red', '🞩')} ${st('cyan', task.name)}`);
		log_error_reasons(log, [result.reason]);
		// an unexpected error's stack is logged here, once, as nothing above reports it;
		// checked by name since a task module can load a different copy of the error classes
		const error_name = (result.error as Error | null)?.constructor?.name;
		if (error_name !== 'TaskError' && error_name !== 'SilentError') log.error(result.error);
		// reported, so the invoking task says only that this one failed and the CLI exits quietly,
		// with the original as `cause` for a task that catches it
		throw new SilentError(undefined, { cause: result.error });
	}
	log.info(`✓ ${st('cyan', task.name)}`);

	// only the top-level task prints the timings, which nested tasks share
	if (owns_timings) {
		print_timings(timings, log);
		log.info(`🕒 ${print_ms(total_timing())}`);
	}
};

/**
 * A resolved task's module id and display name, or why it didn't resolve.
 */
export type ResolveTaskResult =
	{ ok: true; id: string; name: string } | { ok: false; reasons: Array<string> };

/**
 * Resolves a builtin name or a task path to the task module's id and display name,
 * with no search: a name is a builtin or an error, a path is a task file or an error.
 * @param task_name - a builtin name or a task path
 * @param root - the project root task paths are relative to, defaults to the cwd
 */
export const resolve_task = async (
	task_name: string,
	root: string = process.cwd()
): Promise<ResolveTaskResult> => {
	if (!is_task_path_input(task_name)) {
		if (is_builtin_task_name(task_name)) {
			return { ok: true, id: to_builtin_task_id(task_name), name: task_name };
		}
		return { ok: false, reasons: await to_no_builtin_reasons(task_name, root) };
	}
	if (!is_task_path(task_name)) {
		if (task_name.startsWith(REMOVED_BUILTIN_PREFIX)) {
			const builtin_name = task_name.slice(REMOVED_BUILTIN_PREFIX.length);
			return {
				ok: false,
				reasons: [
					is_builtin_task_name(builtin_name)
						? `The \`${REMOVED_BUILTIN_PREFIX}\` prefix is removed, builtins are plain names: use \`${builtin_name}\`, as in \`gro ${builtin_name}\`.`
						: builtin_name
							? `The \`${REMOVED_BUILTIN_PREFIX}\` prefix is removed, and there's no builtin \`${builtin_name}\`.`
							: `The \`${REMOVED_BUILTIN_PREFIX}\` prefix is removed, builtins are plain names: run \`gro\` to list them.`
				]
			};
		}
		return {
			ok: false,
			reasons: [
				`\`${task_name}\` is not a task path: task paths end in \`.task.ts\` or \`.task.js\`.`
			]
		};
	}
	const name = to_task_path(task_name, root);
	const id = resolve(root, name);
	if (!(await fs_exists(id))) {
		return { ok: false, reasons: [`No task file at \`${name}\`.`] };
	}
	return { ok: true, id, name };
};

/**
 * A name that could be a task file's stem, the only kind the missing-builtin error hints a path for.
 */
const TASK_STEM_MATCHER = /^[\w-]+$/;

/**
 * The error for a bare name that's not a builtin:
 * the builtin list and, for a name that could be a file's stem,
 * the path that runs a project task of that name, with one stat to say whether that file exists.
 */
const to_no_builtin_reasons = async (task_name: string, root: string): Promise<Array<string>> => {
	const reasons = [
		`No builtin \`${task_name}\`.`,
		`Builtins: ${Object.keys(GRO_BUILTIN_TASK_SUMMARIES).join(', ')}`
	];
	if (!TASK_STEM_MATCHER.test(task_name)) return reasons; // like `task.ts`, no sensible path to hint
	const hint_path = LIB_DIR + task_name + TASK_FILE_SUFFIX_TS;
	reasons.push(
		(await fs_exists(join(root, hint_path)))
			? `\`${hint_path}\` exists, run it by path: \`gro task ${hint_path}\``
			: `Project tasks run by path, like \`gro task ${hint_path}\` (no file there).`
	);
	return reasons;
};

/**
 * Invokes what the CLI's args name:
 * `gro` lists the builtins and usage, importing no task module;
 * `gro <name>` runs a builtin;
 * `gro task` lists the project's tasks under `src/`, importing each;
 * `gro task <dir>` lists the tasks under `dir`;
 * `gro task <path>` runs the task file at `path`.
 * A path without `gro task` is an error with a hint, as listing is never a fallthrough.
 * @param task_args - the parsed CLI args
 * @param config - the Gro configuration
 */
export const invoke_cli = async (task_args: CliTaskArgs, config: GroConfig): Promise<void> => {
	const { task_name, task_path, args } = task_args;

	if (!task_name) {
		const log = new Logger('gro');
		if (args.version || args.v) {
			const gro_package_json = await package_json_load_for_gro();
			log.info(`${st('gray', 'v')}${st('cyan', gro_package_json.version ?? '[no version]')}`);
			return;
		}
		log_builtin_tasks(log);
		return;
	}

	if (task_name === TASK_VERB) {
		if (task_path !== null && is_task_path(task_path)) {
			await invoke_task(task_path, args, { config });
			return;
		}
		await list_tasks(task_path, config, new Logger('gro task'));
		return;
	}

	if (is_task_path_input(task_name) && !is_removed_builtin_prefix(task_name)) {
		const log = new Logger('gro');
		log_error_reasons(log, [
			`\`${task_name}\` is a path: ${
				is_task_path(task_name)
					? `run a task file with \`gro task ${task_name}\``
					: `list a directory's tasks with \`gro task ${task_name}\``
			}.`
		]);
		throw new SilentError();
	}

	await invoke_task(task_name, args, { config });
};

const is_removed_builtin_prefix = (task_name: string): boolean =>
	task_name.startsWith(REMOVED_BUILTIN_PREFIX) && !is_task_path(task_name);

/**
 * Lists the task files under `dir`, importing each to read its summary,
 * then reports each task that fails to load and fails.
 * @param dir - the directory the user named, `null` for the default `src/`
 */
const list_tasks = async (dir: string | null, config: GroConfig, log: Logger): Promise<void> => {
	const dir_id = resolve(dir ?? SOURCE_DIR);
	const stats = await stat(dir_id).catch(() => null);
	if (!stats?.isDirectory()) {
		// the user named no path, so don't blame one
		if (dir === null) {
			log_error_reasons(log, [
				`\`gro ${TASK_VERB}\` lists the tasks under \`${SOURCE_DIR}\`, and there's no \`${SOURCE_DIR}\` directory here: ` +
					`run a task file with \`gro ${TASK_VERB} <path>\` or list a directory's with \`gro ${TASK_VERB} <dir>\`.`
			]);
			throw new SilentError();
		}
		const reasons = [
			`\`${dir}\` is not a directory or a task file: task paths end in \`.task.ts\` or \`.task.js\`.`
		];
		if (is_builtin_task_name(dir)) {
			reasons.push(`\`${dir}\` is a builtin: run it with \`gro ${dir}\`.`);
		}
		log_error_reasons(log, reasons);
		throw new SilentError();
	}
	const entries = await task_registry_find(dir_id, config.search_filters);
	const modules: Array<TaskModuleMeta> = [];
	const reasons: Array<string> = [];
	for (const { id, name } of entries) {
		const loaded = await load_task(id, name);
		if (loaded.ok) modules.push(loaded.value);
		else reasons.push(loaded.reason);
	}
	// list what loads, so one broken task hides none of the others
	if (modules.length || !reasons.length) log_tasks(log, modules, print_path(dir_id));
	if (reasons.length) {
		log_error_reasons(log, reasons);
		throw new SilentError();
	}
};
