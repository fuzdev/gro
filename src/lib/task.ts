import type {Args} from '@fuzdev/fuz_util/args.js';
import {fs_search} from '@fuzdev/fuz_util/fs.js';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import type {PathId} from '@fuzdev/fuz_util/path.js';
import type {Result} from '@fuzdev/fuz_util/result.js';
import {ensure_end, strip_end, strip_start} from '@fuzdev/fuz_util/string.js';
import type {Timings} from '@fuzdev/fuz_util/timings.js';
import {isAbsolute, join, relative} from 'node:path';
import {styleText as st} from 'node:util';
import type {z} from 'zod';
import type {GroConfig} from './gro_config.ts';
import type {ParsedSvelteConfig} from './svelte_config.ts';
import {
	resolve_input_files,
	resolve_input_paths,
	type InputPath,
	type ResolvedInputFile,
	type ResolvedInputPath,
} from './input_path.ts';
import {GRO_DIST_DIR, print_path} from './paths.ts';
import {load_modules, type LoadModulesFailure, type ModuleMeta} from './modules.ts';
import type {Filer} from './filer.ts';

export interface Task<
	TArgs = Args,
	TArgsSchema extends z.ZodType<Args, Args> = z.ZodType<Args, Args>, // TODO improve type? separate input/output?
	TReturn = unknown,
> {
	run: (ctx: TaskContext<TArgs>) => TReturn | Promise<TReturn>; // TODO unused return value
	summary?: string;
	Args?: TArgsSchema;
}

export interface TaskContext<TArgs = object> {
	args: TArgs;
	config: GroConfig;
	svelte_config: ParsedSvelteConfig;
	filer: Filer;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
}

export type InvokeTask = (task_name: string, args?: Args, config?: GroConfig) => Promise<void>;

export const TASK_FILE_SUFFIX_TS = '.task.ts';
export const TASK_FILE_SUFFIX_JS = '.task.js';
export const TASK_FILE_SUFFIXES = [TASK_FILE_SUFFIX_TS, TASK_FILE_SUFFIX_JS]; // TODO from `GroConfig`, but needs to be used everywhere the constants are

export const is_task_path = (path: string): boolean =>
	path.endsWith(TASK_FILE_SUFFIX_TS) || path.endsWith(TASK_FILE_SUFFIX_JS);

export const to_task_name = (
	id: PathId,
	task_root_dir: PathId,
	input_path: InputPath,
	root_path: PathId,
): string => {
	let task_name = id.startsWith(task_root_dir)
		? strip_start(strip_start(id, task_root_dir), '/')
		: id;
	for (const suffix of TASK_FILE_SUFFIXES) {
		task_name = strip_end(task_name, suffix);
	}
	if (ensure_end(task_root_dir, '/') === GRO_DIST_DIR) {
		// TODO ideally it would display this in some contexts like the task progress logs,
		// but not all, like printing the task list, UNLESS there's a local override
		// return 'gro/' + task_name;
		return task_name;
	}
	if (isAbsolute(input_path)) {
		return relative(root_path, join(task_root_dir, task_name));
	}
	return task_name;
};

/**
 * This is used by tasks to signal a known failure.
 * It's useful for cleaning up logging because
 * we usually don't need their stack trace.
 */
export class TaskError extends Error {}

/**
 * This is used to tell Gro to exit silently, usually still with with a non-zero exit code.
 * Using it means error logging is handled by the code that threw it.
 */
export class SilentError extends Error {}

export interface FoundTask {
	input_path: InputPath;
	id: PathId;
	task_root_dir: PathId;
}

export interface FoundTasks {
	resolved_input_files: Array<ResolvedInputFile>;
	resolved_input_files_by_root_dir: Map<PathId, Array<ResolvedInputFile>>;
	resolved_input_paths: Array<ResolvedInputPath>;
	input_paths: Array<InputPath>;
	task_root_dirs: Array<PathId>;
}

export type FindTasksResult = Result<{value: FoundTasks}, FindModulesFailure>;
export type FindModulesFailure =
	| {
			type: 'unmapped_input_paths';
			unmapped_input_paths: Array<InputPath>;
			resolved_input_paths: Array<ResolvedInputPath>;
			input_paths: Array<InputPath>;
			task_root_dirs: Array<PathId>;
			reasons: Array<string>;
	  }
	| {
			type: 'input_directories_with_no_files';
			input_directories_with_no_files: Array<InputPath>;
			resolved_input_files: Array<ResolvedInputFile>;
			resolved_input_files_by_root_dir: Map<PathId, Array<ResolvedInputFile>>;
			resolved_input_paths: Array<ResolvedInputPath>;
			input_paths: Array<InputPath>;
			task_root_dirs: Array<PathId>;
			reasons: Array<string>;
	  };

/**
 * Finds modules from input paths. (see `input_path.ts` for more)
 */
export const find_tasks = async (
	input_paths: Array<InputPath>,
	task_root_dirs: Array<PathId>,
	config: GroConfig,
	timings?: Timings,
): Promise<FindTasksResult> => {
	// Check which extension variation works - if it's a directory, prefer others first!
	const timing_to_resolve_input_paths = timings?.start('resolve input paths');
	const {resolved_input_paths, unmapped_input_paths} = await resolve_input_paths(
		input_paths,
		task_root_dirs,
		TASK_FILE_SUFFIXES,
	);
	timing_to_resolve_input_paths?.();

	// Error if any input path could not be mapped.
	if (unmapped_input_paths.length) {
		return {
			ok: false,
			type: 'unmapped_input_paths',
			unmapped_input_paths,
			resolved_input_paths,
			input_paths,
			task_root_dirs,
			reasons: unmapped_input_paths.map((input_path) =>
				st('red', `Input path ${print_path(input_path)} cannot be mapped to a file or directory.`),
			),
		};
	}

	// Find all of the files for any directories.
	const timing_to_resolve_input_files = timings?.start('resolve input files');
	const {resolved_input_files, resolved_input_files_by_root_dir, input_directories_with_no_files} =
		await resolve_input_files(
			resolved_input_paths,
			async (id) =>
				await fs_search(id, {
					filter: config.search_filters,
					file_filter: (p) => TASK_FILE_SUFFIXES.some((s) => p.endsWith(s)),
				}),
		);
	timing_to_resolve_input_files?.();

	// Error if any input path has no files. (means we have an empty directory)
	if (input_directories_with_no_files.length) {
		return {
			ok: false,
			type: 'input_directories_with_no_files',
			input_directories_with_no_files,
			resolved_input_files,
			resolved_input_files_by_root_dir,
			resolved_input_paths,
			input_paths,
			task_root_dirs,
			reasons: input_directories_with_no_files.map((input_path) =>
				st('red', `Input directory contains no matching files: ${print_path(input_path)}`),
			),
		};
	}

	return {
		ok: true,
		value: {
			resolved_input_files,
			resolved_input_files_by_root_dir,
			resolved_input_paths,
			input_paths,
			task_root_dirs,
		},
	};
};

export interface LoadedTasks {
	modules: Array<TaskModuleMeta>;
	found_tasks: FoundTasks;
}

export interface TaskModule {
	task: Task;
}

export interface TaskModuleMeta extends ModuleMeta<TaskModule> {
	name: string;
}

export type LoadTasksResult = Result<{value: LoadedTasks}, LoadTasksFailure>;
export type LoadTasksFailure = LoadModulesFailure<TaskModuleMeta>;

export const load_tasks = async (
	found_tasks: FoundTasks,
	root_path: PathId = process.cwd(), // TODO @many isn't passed in anywhere, maybe hoist to `invoke_task` and others
): Promise<LoadTasksResult> => {
	const loaded_modules = await load_modules(
		found_tasks.resolved_input_files,
		validate_task_module,
		(resolved_input_file, mod): TaskModuleMeta => ({
			id: resolved_input_file.id,
			mod,
			name: to_task_name(
				resolved_input_file.id,
				resolved_input_file.resolved_input_path.root_dir,
				resolved_input_file.resolved_input_path.input_path,
				root_path,
			),
		}),
	);
	if (!loaded_modules.ok) {
		return loaded_modules;
	}

	return {
		ok: true,
		value: {modules: loaded_modules.modules, found_tasks},
	};
};

export const validate_task_module = (mod: Record<string, any>): mod is TaskModule =>
	!!mod.task && typeof mod.task.run === 'function';
