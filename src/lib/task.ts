import type { Args } from '@fuzdev/fuz_util/args.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import type { PathId } from '@fuzdev/fuz_util/path.ts';
import { print_error } from '@fuzdev/fuz_util/print.ts';
import type { Result } from '@fuzdev/fuz_util/result.ts';
import type { Timings } from '@fuzdev/fuz_util/timings.ts';
import { relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Schema } from '@fuzdev/fuz_schema/schema.ts';
import type { GroConfig } from './gro_config.ts';
import type { ParsedSvelteConfig } from './svelte_config.ts';
import { print_path } from './paths.ts';
import { load_module, type ModuleMeta } from './modules.ts';
import type { ModuleGraph, Watcher } from './module_graph.ts';
import { GRO_BUILTIN_TASK_SUMMARIES, type GroBuiltinTasks } from './gro_builtin_tasks.ts';
import type { TaskArgsInput, TaskName } from './task_registry.ts';

export interface Task<
	TArgs = Args,
	TArgsSchema extends Schema<Args, Args> = Schema<Args, Args>, // TODO improve type? separate input/output?
	TReturn = unknown
> {
	run: (ctx: TaskContext<TArgs>) => TReturn | Promise<TReturn>; // TODO unused return value
	summary?: string;
	Args?: TArgsSchema;
}

export interface TaskContext<TArgs = object> {
	args: TArgs;
	config: GroConfig;
	/**
	 * Resolved on first access, so tasks that don't touch it
	 * never pay to read the SvelteKit config.
	 */
	svelte_config: Promise<ParsedSvelteConfig>;
	/**
	 * The project's module graph, loaded on first use.
	 */
	graph: ModuleGraph;
	/**
	 * Watches files with no module graph, loaded on first use.
	 */
	watcher: Watcher;
	log: Logger;
	timings: Timings;
	invoke_task: InvokeTask;
}

/**
 * Invokes a task: a name always means a builtin, and a path always means a task file,
 * relative to the cwd and ending in `.task.ts` or `.task.js`.
 *
 * Literal names and paths are checked against `TaskRegistry`
 * and args against the task's `Args` schema input.
 * A literal task path not in the registry, or a name widened to `string`, takes loose `Args`.
 */
export type InvokeTask = <TName extends string>(
	task_name: InvokeTaskName<TName>,
	...rest: InvokeTaskRest<TName>
) => Promise<void>;

/**
 * A literal task file path, ending in `.task.ts` or `.task.js`.
 */
export type TaskPathLiteral = `${string}.task.ts` | `${string}.task.js`;

/**
 * A literal must be in `TaskRegistry` or be a task file path, a widened `string` passes through.
 */
export type InvokeTaskName<TName extends string> = string extends TName
	? TName
	: TName extends TaskName | TaskPathLiteral
		? TName
		: TaskName;

/**
 * Strips any leading `./` from a literal task path,
 * so `./src/lib/x.task.ts` keys the registry like `src/lib/x.task.ts`.
 */
export type TaskPathNormalized<TName extends string> = TName extends `./${infer TRest}`
	? TaskPathNormalized<TRest>
	: TName;

/**
 * The args `InvokeTask` accepts for `TName`:
 * the input type of the registered task's `Args` schema,
 * keyed by its name or normalized path, or loose `Args` for a task not in the registry.
 */
export type InvokeTaskArgs<TName extends string> =
	TaskPathNormalized<TName> extends infer TKey extends TaskName ? TaskArgsInput<TKey> : Args;

/**
 * Makes `args` required when the task's `Args` has required fields.
 */
export type InvokeTaskRest<TName extends string> =
	// eslint-disable-next-line @typescript-eslint/no-empty-object-type -- `{}` assignable means no required fields
	{} extends InvokeTaskArgs<TName>
		? [args?: InvokeTaskArgs<TName>, options?: InvokeTaskContextOptions]
		: [args: InvokeTaskArgs<TName>, options?: InvokeTaskContextOptions];

/**
 * The options a task's `invoke_task` takes, after the task name and args.
 */
export interface InvokeTaskContextOptions {
	/**
	 * The config the invoked task runs with, defaults to the invoking task's.
	 * The invoked task still shares the invoking task's `graph` and `watcher`,
	 * so this config's `watch` doesn't apply.
	 */
	config?: GroConfig;
}

/**
 * The suffix of a TypeScript task file, which `gro task <path>` requires with `TASK_FILE_SUFFIX_JS`.
 */
export const TASK_FILE_SUFFIX_TS = '.task.ts';
/**
 * The suffix of a JS task file, which `gro task <path>` requires with `TASK_FILE_SUFFIX_TS`.
 */
export const TASK_FILE_SUFFIX_JS = '.task.js';
/**
 * Every task file suffix, the only files `gro task` runs and lists.
 */
export const TASK_FILE_SUFFIXES = [TASK_FILE_SUFFIX_TS, TASK_FILE_SUFFIX_JS];

/**
 * The CLI verb that runs or lists project tasks by path, `gro task [path]`.
 */
export const TASK_VERB = 'task';

/**
 * Whether `path` ends in a task file suffix, `.task.ts` or `.task.js`.
 */
export const is_task_path = (path: string): boolean =>
	path.endsWith(TASK_FILE_SUFFIX_TS) || path.endsWith(TASK_FILE_SUFFIX_JS);

/**
 * Whether a task input names a path rather than a builtin:
 * it contains a `/` or ends in a task file suffix.
 * Anything else is a builtin name, never looked up on the filesystem.
 */
export const is_task_path_input = (input: string): boolean =>
	input.includes('/') || is_task_path(input);

/**
 * Normalizes a task path to be relative to `root`,
 * the form tasks are displayed, keyed, and forwarded args by.
 * A relative `path` is relative to `root`, with or without a leading `./`.
 * @param path - a relative or absolute task path
 * @param root - the project root, defaults to the cwd
 */
export const to_task_path = (path: string, root: string = process.cwd()): string =>
	relative(root, resolve(root, path));

/**
 * The command that runs the task `name`, a builtin name or a normalized task path,
 * which is also the key its forwarded args are given by, like `gro check -- gro task src/x.task.ts --a`.
 */
export const to_task_command = (name: string): string =>
	is_task_path_input(name) ? `gro ${TASK_VERB} ${name}` : `gro ${name}`;

/**
 * Whether `name` is one of Gro's builtin tasks.
 */
export const is_builtin_task_name = (name: string): name is keyof GroBuiltinTasks =>
	Object.hasOwn(GRO_BUILTIN_TASK_SUMMARIES, name);

// builtins sit beside this module, `.ts` in Gro's source and `.js` in its dist
const BUILTIN_TASK_FILE_SUFFIX = import.meta.url.endsWith('.ts')
	? TASK_FILE_SUFFIX_TS
	: TASK_FILE_SUFFIX_JS;

/**
 * The module id of the builtin task `name`.
 */
export const to_builtin_task_id = (name: keyof GroBuiltinTasks): PathId =>
	fileURLToPath(new URL('./' + name + BUILTIN_TASK_FILE_SUFFIX, import.meta.url));

/**
 * Options for a `TaskError`.
 */
export interface TaskErrorOptions extends ErrorOptions {
	/**
	 * The code the CLI exits with when this error fails the task it invoked,
	 * like `gro run`'s script's own, an integer 1–255; absent or any other value means `1`.
	 */
	exit_code?: number;
}

/**
 * This is used by tasks to signal a known failure.
 * It's useful for cleaning up logging because
 * we usually don't need their stack trace.
 */
export class TaskError extends Error {
	/**
	 * The code the CLI exits with when this error fails the task it invoked,
	 * an integer 1–255; absent or any other value means `1`.
	 */
	readonly exit_code: number | undefined;

	constructor(message?: string, options?: TaskErrorOptions) {
		super(message, options);
		this.exit_code = options?.exit_code;
	}
}

/**
 * This is used to tell Gro to exit silently, usually still with a non-zero exit code.
 * Using it means error logging is handled by the code that threw it.
 */
export class SilentError extends Error {}

/**
 * A task file's module, which exports the `task`.
 */
export interface TaskModule {
	task: Task;
}

/**
 * A loaded task module with its id and display name.
 */
export interface TaskModuleMeta extends ModuleMeta<TaskModule> {
	/**
	 * The builtin name or normalized root-relative path the task is displayed and keyed by.
	 */
	name: string;
}

/**
 * A loaded task module, or why it didn't load.
 */
export type LoadTaskResult = Result<{ value: TaskModuleMeta }, { reason: string }>;

/**
 * Imports the task module at `id`, displayed as `name`.
 */
export const load_task = async (id: PathId, name: string): Promise<LoadTaskResult> => {
	const loaded = await load_module(id, validate_task_module);
	if (loaded.ok) return { ok: true, value: { id, name, mod: loaded.mod } };
	return {
		ok: false,
		reason:
			loaded.type === 'failed_import'
				? `Task module ${print_path(id)} failed to import: ${print_error(loaded.error)}`
				: `Task module ${print_path(id)} has no \`task\` export with a \`run\` function.`
	};
};

/**
 * Whether a module is a task module, exporting a `task` with a `run` function.
 * @param mod - the imported module
 */
export const validate_task_module = (mod: Record<string, any>): mod is TaskModule =>
	!!mod.task && typeof mod.task.run === 'function';
