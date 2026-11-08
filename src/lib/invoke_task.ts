import type { Args } from '@fuzdev/fuz_util/args.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';
import { print_ms, print_timings } from '@fuzdev/fuz_util/print.ts';
import { create_stopwatch, Timings } from '@fuzdev/fuz_util/timings.ts';
import { join, resolve } from 'node:path';
import { styleText as st } from 'node:util';

import {
	merge_forwarded_args,
	RUN_TASK_NAME,
	to_forwarded_args,
	type CliTaskArgs
} from './args.ts';
import { run_task } from './run_task.ts';
import {
	is_builtin_task_name,
	is_silent_error,
	is_task_error,
	is_task_path,
	is_task_path_input,
	load_task,
	SilentError,
	to_builtin_task_id,
	to_task_command,
	to_task_path,
	TASK_FILE_SUFFIX_TS,
	TASK_VERB,
	type TaskModuleMeta
} from './task.ts';
import {
	TASK_PATH_SUFFIX_RULE,
	find_task_file_for_input,
	is_removed_builtin_prefix,
	to_removed_builtin_prefix_message,
	to_run_task_file_hint,
	to_task_path_hint,
	to_task_path_without_verb_message
} from './internal/task_hints.ts';
import { package_json_load_for_gro } from './package_json.ts';
import { log_builtin_tasks, log_error_reasons, log_tasks } from './task_logging.ts';
import type { GroConfig } from './gro_config.ts';
import { create_node_module_graph } from './module_graph_node.ts';
import type { ModuleGraph } from './module_graph.ts';
import { task_registry_find } from './task_registry.ts';
import { GRO_BUILTIN_TASK_SUMMARIES } from './gro_builtin_tasks.ts';
import { LIB_DIR, SOURCE_DIR } from './constants.ts';
import { print_path } from './paths.ts';
import type { FileInfo } from './fs.ts';
import { is_fs_error } from './fs_error.ts';
import type { FsPaths } from './fs_paths.ts';

/**
 * What `invoke_task` runs a task with, after the task name and args.
 */
export interface InvokeTaskOptions {
	/**
	 * The Gro configuration.
	 */
	config: GroConfig;
	/**
	 * Opens the task's path and the paths of the hints for a missing one,
	 * built by the composition root and passed to nested tasks.
	 */
	fs_paths: FsPaths;
	/**
	 * The `graph` from the invoking task,
	 * created from `config.watch` and closed on finish when absent.
	 * A nested task gets its invoking task's even when invoked with its own `config`,
	 * so its `config.watch` is ignored.
	 */
	graph?: ModuleGraph;
	/**
	 * The invoking task's timings, shared by nested tasks;
	 * absent for the top-level task, which creates them and prints them on success.
	 * The CLI passes its own for `gro run`, which prints none, like `node`.
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
	const { config, fs_paths, graph: initial_graph, timings: initial_timings, parent_log } = options;
	// Create child logger if parent exists, otherwise root logger,
	// labeled by the name the task is displayed by when it's a normalized path
	const log_label = is_task_path(task_name) ? to_task_path(task_name) : task_name || 'gro';
	const log = parent_log ? parent_log.child(log_label) : new Logger(log_label);
	log.info('invoking', st('cyan', log_label));

	const resolved = await resolve_task(fs_paths, task_name);
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
		if (!is_task_error(error)) throw error;
		log_error_reasons(log, [error.message]);
		throw new SilentError();
	}

	// the graph loads lazily, so a task that never uses it never loads chokidar
	const owns_graph = !initial_graph;
	const graph =
		initial_graph ?? create_node_module_graph({ watch: config.watch, log: log.child('graph') });

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
			fs_paths,
			graph,
			log,
			timings
		});
	} finally {
		// close only what we created, failed or not, a no-op when it never loaded
		if (owns_graph) await graph.close();
	}
	timing_to_run_task();
	if (!result.ok) {
		log.info(`${st('red', '🞩')} ${st('cyan', task.name)}`);
		log_error_reasons(log, [result.reason]);
		// an unexpected error's stack is logged here, once, as nothing above reports it
		if (!is_task_error(result.error) && !is_silent_error(result.error)) log.error(result.error);
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
 * @param fs_paths - stats the task file, and for a name that's no builtin, the file it hints
 * @param task_name - a builtin name or a task path
 * @param root - the project root task paths are relative to, defaults to the project of `fs_paths`
 */
export const resolve_task = async (
	fs_paths: FsPaths,
	task_name: string,
	root: string = fs_paths.project.path
): Promise<ResolveTaskResult> => {
	if (!is_task_path_input(task_name)) {
		if (task_name === TASK_VERB) {
			return {
				ok: false,
				reasons: [
					`\`${TASK_VERB}\` is the CLI verb \`gro ${TASK_VERB} <path>\`, not a builtin: ` +
						`invoke a project task by its path, like \`${LIB_DIR}foo${TASK_FILE_SUFFIX_TS}\`.`
				]
			};
		}
		if (is_builtin_task_name(task_name)) {
			return { ok: true, id: to_builtin_task_id(task_name), name: task_name };
		}
		return { ok: false, reasons: await to_no_builtin_reasons(fs_paths, task_name, root) };
	}
	if (!is_task_path(task_name)) {
		if (is_removed_builtin_prefix(task_name)) {
			return { ok: false, reasons: [to_removed_builtin_prefix_message(task_name)] };
		}
		const reasons = [`\`${task_name}\` is not a task path: ${TASK_PATH_SUFFIX_RULE}.`];
		const hint = to_task_path_hint(task_name, root);
		if (hint !== null) reasons.push(hint);
		return { ok: false, reasons };
	}
	const name = to_task_path(task_name, root);
	const id = resolve(root, name);
	const stat = await stat_path(fs_paths, id);
	if (!stat.ok) {
		return { ok: false, reasons: [`Can't read \`${name}\`: ${stat.message}`] };
	}
	const { info } = stat;
	if (info?.kind !== 'file') {
		return {
			ok: false,
			reasons: [
				info
					? `\`${name}\` is not a task file: it's a directory or a special file.`
					: `No task file at \`${name}\`.`
			]
		};
	}
	return { ok: true, id, name };
};

/**
 * Stats a path from outside, following symlinks, `info` `null` when nothing is there
 * or a file is where a directory would be, both meaning no task is at the path,
 * and any other fs failure the message for a reason, never thrown.
 */
const stat_path = async (
	fs_paths: FsPaths,
	path: string
): Promise<{ ok: true; info: FileInfo | null } | { ok: false; message: string }> => {
	try {
		const opened = await fs_paths.open_path(path);
		return { ok: true, info: await opened.dir.stat(opened.path) };
	} catch (error) {
		if (is_fs_error(error, 'not_a_directory')) return { ok: true, info: null };
		if (is_fs_error(error)) return { ok: false, message: error.message };
		throw error;
	}
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
const to_no_builtin_reasons = async (
	fs_paths: FsPaths,
	task_name: string,
	root: string
): Promise<Array<string>> => {
	const reasons = [
		`No builtin \`${task_name}\`.`,
		`Builtins: ${Object.keys(GRO_BUILTIN_TASK_SUMMARIES).join(', ')}`
	];
	if (!TASK_STEM_MATCHER.test(task_name)) return reasons; // like `task.ts`, no sensible path to hint
	const hint_path = LIB_DIR + task_name + TASK_FILE_SUFFIX_TS;
	const hint_file = await fs_paths.open_path(join(root, hint_path));
	reasons.push(
		(await hint_file.dir.exists(hint_file.path))
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
 * @param fs_paths - opens the paths the args name, passed to the task it invokes
 * @param task_args - the parsed CLI args
 * @param config - the Gro configuration
 */
export const invoke_cli = async (
	fs_paths: FsPaths,
	task_args: CliTaskArgs,
	config: GroConfig
): Promise<void> => {
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
			await invoke_task(task_path, args, { config, fs_paths });
			return;
		}
		await list_tasks(fs_paths, task_path, config, new Logger('gro task'));
		return;
	}

	if (is_task_path_input(task_name) && !is_removed_builtin_prefix(task_name)) {
		const log = new Logger('gro');
		log_error_reasons(log, [to_task_path_without_verb_message(task_name)]);
		throw new SilentError();
	}

	// `gro run` is like `node`, so the CLI gives it timings no one prints
	await invoke_task(task_name, args, {
		config,
		fs_paths,
		timings: task_name === RUN_TASK_NAME ? new Timings() : undefined
	});
};

/**
 * Lists the task files under `dir`, importing each to read its summary,
 * then reports each task that fails to load and fails.
 * @param dir - the directory the user named, `null` for the default `src/`
 */
const list_tasks = async (
	fs_paths: FsPaths,
	dir: string | null,
	config: GroConfig,
	log: Logger
): Promise<void> => {
	const dir_id = resolve(fs_paths.project.path, dir ?? SOURCE_DIR);
	const stat = await stat_path(fs_paths, dir_id);
	if (!stat.ok) {
		log_error_reasons(log, [`Can't read \`${dir ?? SOURCE_DIR}\`: ${stat.message}`]);
		throw new SilentError();
	}
	if (stat.info?.kind !== 'directory') {
		// the user named no path, so don't blame one
		if (dir === null) {
			log_error_reasons(log, [
				`\`gro ${TASK_VERB}\` lists the tasks under \`${SOURCE_DIR}\`, and there's no \`${SOURCE_DIR}\` directory here: ` +
					`run a task file with \`gro ${TASK_VERB} <path>\` or list a directory's with \`gro ${TASK_VERB} <dir>\`.`
			]);
			throw new SilentError();
		}
		const reasons = [`\`${dir}\` is not a directory or a task file: ${TASK_PATH_SUFFIX_RULE}.`];
		if (is_builtin_task_name(dir)) {
			reasons.push(`\`${dir}\` is a builtin: run it with \`gro ${dir}\`.`);
		} else {
			const hint = to_task_path_hint(dir);
			if (hint !== null) reasons.push(hint);
		}
		log_error_reasons(log, reasons);
		throw new SilentError();
	}
	const entries = await task_registry_find(fs_paths, dir_id, config.search_filters);
	const modules: Array<TaskModuleMeta> = [];
	const reasons: Array<string> = [];
	for (const { id, name } of entries) {
		const loaded = await load_task(id, name);
		if (loaded.ok) modules.push(loaded.value);
		else reasons.push(loaded.reason);
	}
	// list what loads, so one broken task hides none of the others
	if (modules.length || !reasons.length) log_tasks(log, modules, print_path(dir_id));
	// a directory with no tasks beside a task file of its name, like `update/` and `update.task.ts`
	if (!modules.length && !reasons.length && dir !== null) {
		const task_file = find_task_file_for_input(dir);
		if (task_file !== null) log.info(to_run_task_file_hint(task_file));
	}
	if (reasons.length) {
		log_error_reasons(log, reasons);
		throw new SilentError();
	}
};
