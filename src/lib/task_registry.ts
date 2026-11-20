import type { Args } from '@fuzdev/fuz_util/args.ts';
import type { PathFilter, PathId } from '@fuzdev/fuz_util/path.ts';
import { strip_end } from '@fuzdev/fuz_util/string.ts';
import type { AnySchema, InferInput } from '@fuzdev/fuz_schema/schema.ts';

import type { GenConfig } from './gen.ts';
import type { GroBuiltinTasks } from './gro_builtin_tasks.ts';
import type { BuiltinTaskGrants } from './builtin_host.ts';
import { gro_paths, to_search_filters_relative } from './paths.ts';
import type { FsReadDir } from './fs.ts';
import type { FsPaths } from './fs_paths.ts';
import type { ImportModule } from './modules.ts';
import { SOURCE_DIRNAME } from './constants.ts';
import {
	is_task_path,
	load_task,
	TASK_FILE_SUFFIX_TS,
	TASK_FILE_SUFFIXES,
	to_task_path
} from './task.ts';
import { TaskError } from './task_error.ts';
import { posix_dirname, posix_join, posix_relative } from './internal/posix_path.ts';

/**
 * The project's own tasks by normalized path, empty until augmented by a generated file.
 * Create a genfile like `src/gro_tasks.gen.ts` exporting `gen_task_registry(create_fs_paths(fs, options))`
 * as `gen` to type the project's task paths and args for `invoke_task`.
 */
// eslint-disable-next-line @typescript-eslint/no-empty-object-type
export interface ProjectTasks {}

/**
 * Every task `invoke_task` types, mapped to its module's type:
 * Gro's builtins by name and the project's tasks by path relative to the project root.
 * Names never contain a `/` or end in a task suffix and paths always do, so nothing shadows.
 */
export type TaskRegistry = GroBuiltinTasks & ProjectTasks;

/**
 * A builtin name or project task path in `TaskRegistry`, the literals `invoke_task` checks.
 */
export type TaskName = Extract<keyof TaskRegistry, string>;

/**
 * The input type of the task's `Args` schema, so defaulted args are optional,
 * or loose `Args` for a task module without an `Args` export.
 * A member of the input with no keys becomes `Record<string, never>`,
 * so an arg-less task refuses every arg, and a union keeps each option's keys.
 */
export type TaskArgsInput<TName extends TaskName> = TaskRegistry[TName] extends {
	Args: infer TSchema extends AnySchema;
}
	? InferInput<TSchema> extends infer TInput
		? TInput extends unknown // distributes, `keyof` of a union is only the shared keys
			? [keyof TInput] extends [never]
				? // a keyless member, such as the `undefined` a `with_default_fn` admits
					Record<string, never>
				: TInput
			: never
		: never
	: Args;

export interface TaskRegistryEntry {
	/**
	 * The builtin name, or the project task's path relative to the project root.
	 */
	name: string;
	id: PathId;
}

/**
 * Finds the task files under `dir`, named by their paths relative to `root`,
 * the names `invoke_task` and `gro task` give them, sorted by name.
 * @param fs_paths - searches `dir`
 * @param dir - the directory to search, absolute or relative to the project of `fs_paths`
 * @param search_filters - the filters each file and directory must pass, given paths relative to `root`,
 * or to `dir` for the paths outside `root`
 * @param root - the project root the names and filtered paths are relative to,
 * defaults to the project of `fs_paths`
 */
export const task_registry_find = async (
	fs_paths: FsPaths,
	dir: PathId,
	search_filters: Array<PathFilter>,
	root: PathId = fs_paths.project.path
): Promise<Array<TaskRegistryEntry>> => {
	const found = await fs_paths.search_path(dir, {
		filter: to_search_filters_relative(search_filters, root, dir),
		file_filter: is_task_path
	});
	return found
		.map(({ id }) => ({ name: to_task_path(id, root), id }))
		.sort((a, b) => a.name.localeCompare(b.name));
};

const IDENTIFIER_MATCHER = /^[A-Za-z_$][\w$]*$/;

const render_key = (name: string): string =>
	IDENTIFIER_MATCHER.test(name) ? name : JSON.stringify(name);

/**
 * Renders the type-only imports of the task modules and their registry properties,
 * with import specifiers relative to `dir`, where the rendered file is written.
 */
const render_entries = (
	entries: Array<TaskRegistryEntry>,
	dir: PathId,
	indent: string
): { imports: string; properties: string } => {
	const used: Set<string> = new Set();
	let imports = '';
	let properties = '';
	for (const { name, id } of entries) {
		let stem = name;
		for (const suffix of TASK_FILE_SUFFIXES) stem = strip_end(stem, suffix);
		const base = stem.replace(/[^\w$]/g, '_').replace(/^\d/, '_$&') + '_task'; // a valid identifier
		let identifier = base;
		for (let i = 2; used.has(identifier); i++) identifier = base + i;
		used.add(identifier);
		let specifier = posix_relative(dir, id);
		// a path beneath `dir`, like `.hidden/x.task.ts`, needs `./` to be a path, not bare,
		// and any other starts `../`
		if (!specifier.startsWith('../')) specifier = './' + specifier;
		imports += `import type * as ${identifier} from '${specifier}';\n`;
		properties += `${indent}${render_key(name)}: typeof ${identifier};\n`;
	}
	return { imports, properties };
};

/**
 * Renders the module augmentation of `ProjectTasks` written by `gen_task_registry`.
 */
export const task_registry_render_project = (
	entries: Array<TaskRegistryEntry>,
	output_dir: PathId,
	origin_path: string
): string => {
	const { imports, properties } = render_entries(entries, output_dir, '\t\t');
	return `// generated by ${origin_path} - DO NOT EDIT DIRECTLY!
// types the project's tasks for \`invoke_task\` by path, see \`@fuzdev/gro/task_registry.ts\`

${imports}
declare module '@fuzdev/gro/task_registry.ts' {
	interface ProjectTasks {
${properties}	}
}

export {}; // always a module, so the declaration augments rather than declares
`;
};

/**
 * A builtin task with the summary `gro` lists it by
 * and its `task.grants`, `null` for a task that declares none.
 */
export interface BuiltinTaskEntry extends TaskRegistryEntry {
	summary: string;
	grants: BuiltinTaskGrants | null;
}

/**
 * Finds Gro's builtin tasks, the `*.task.ts` files directly in `src_lib`,
 * importing each to read its summary and grants.
 * @param src_lib - Gro's `src/lib/`
 * @param import_module - loads each task module
 */
export const task_registry_find_builtins = async (
	src_lib: FsReadDir,
	import_module: ImportModule
): Promise<Array<BuiltinTaskEntry>> => {
	const filenames: Array<string> = [];
	for await (const { name } of src_lib.list()) {
		if (name.endsWith(TASK_FILE_SUFFIX_TS)) filenames.push(name);
	}
	filenames.sort();
	const entries: Array<BuiltinTaskEntry> = [];
	for (const filename of filenames) {
		const name = strip_end(filename, TASK_FILE_SUFFIX_TS);
		const id = posix_join(src_lib.path, filename);
		const loaded = await load_task(id, name, gro_paths.root, import_module);
		if (!loaded.ok) throw new TaskError(loaded.reason);
		const { task } = loaded.value.mod;
		const { grants } = task as { grants?: BuiltinTaskGrants };
		entries.push({ name, id, summary: task.summary ?? '', grants: grants ?? null });
	}
	return entries;
};

/**
 * Renders the `GroBuiltinTasks` interface for Gro's own tasks
 * and their summaries and grants, so listing them or reading what they need imports no task module,
 * the grants deep-frozen with null-prototype objects when the rendered module loads.
 */
export const task_registry_render_builtins = (
	entries: Array<BuiltinTaskEntry>,
	output_dir: PathId,
	origin_path: string
): string => {
	const { imports, properties } = render_entries(entries, output_dir, '\t');
	let summaries = '';
	let grants = '';
	for (const entry of entries) {
		const key = render_key(entry.name);
		summaries += `\t${key}: ${JSON.stringify(entry.summary)},\n`;
		grants += `\t${key}: ${JSON.stringify(entry.grants)},\n`;
	}
	return `// generated by ${origin_path} - DO NOT EDIT DIRECTLY!

import type { BuiltinTaskGrants } from './builtin_host.ts';
${imports}
/**
 * Gro's builtin tasks by name, mapped to their module types.
 */
export interface GroBuiltinTasks {
${properties}}

/**
 * The summary of each of Gro's builtin tasks by name, so listing them imports no task module.
 */
export const GRO_BUILTIN_TASK_SUMMARIES: Record<keyof GroBuiltinTasks, string> = {
${summaries}};

const { create, freeze, keys } = Object;

// a deep-frozen copy of \`value\`, each object null-prototype, so a property added to
// \`Object.prototype\` widens no grant, a guardrail against accidental pollution,
// not against a module built to defeat it
const to_frozen_record = <T>(value: T): T => {
	if (value === null || typeof value !== 'object') return value;
	if (Array.isArray(value)) return freeze(value.map(to_frozen_record)) as T;
	const record: Record<string, unknown> = create(null);
	for (const key of keys(value)) {
		record[key] = to_frozen_record((value as Record<string, unknown>)[key]);
	}
	return freeze(record) as T;
};

/**
 * The grants of each of Gro's builtin tasks by name, as data a runtime reads without importing
 * the task, \`null\` for a builtin that declares none and gets no \`host\`,
 * deep-frozen with null-prototype objects.
 */
export const GRO_BUILTIN_TASK_GRANTS: Record<keyof GroBuiltinTasks, BuiltinTaskGrants | null> =
	to_frozen_record({
${grants}});
`;
};

/**
 * Creates the genfile config that types a project's tasks for `invoke_task`.
 * Export it as `gen` from a genfile like `src/gro_tasks.gen.ts`,
 * `export const gen = gen_task_registry(create_fs_paths(fs, {cwd: cwd(), home: env.get('HOME')}));`,
 * which writes `src/gro_tasks.ts` augmenting `ProjectTasks`
 * with every task file under `src/`, keyed by its path relative to the project root.
 * In `gro dev` it regenerates when a task file is added or changed;
 * `gro dev` ignores deletes, so run `gro gen` after removing a task.
 * @param fs_paths - searches the project's `src/`, its project the root the tasks are keyed relative to
 */
export const gen_task_registry = (fs_paths: FsPaths): GenConfig => ({
	generate: async ({ config, origin_id, origin_path }) => {
		const root = fs_paths.project.path;
		const entries = await task_registry_find(
			fs_paths,
			posix_join(root, SOURCE_DIRNAME),
			config.search_filters,
			root
		);
		return task_registry_render_project(entries, posix_dirname(origin_id), origin_path);
	},
	inputs: { patterns: ['*.task.ts', '*.task.js'] }
});
