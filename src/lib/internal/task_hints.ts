import { statSync, type Stats } from 'node:fs';
import { resolve } from 'node:path';

import {
	is_builtin_task_name,
	is_task_path,
	TASK_FILE_SUFFIX_JS,
	TASK_FILE_SUFFIX_TS,
	TASK_VERB
} from '../task.ts';

/**
 * The rule every task path follows, for errors about one that doesn't.
 */
export const TASK_PATH_SUFFIX_RULE = `task paths end in \`${TASK_FILE_SUFFIX_TS}\` or \`${TASK_FILE_SUFFIX_JS}\``;

/**
 * The prefix that once forced a builtin, now an error naming the plain builtin.
 */
const REMOVED_BUILTIN_PREFIX = 'gro/';

/**
 * Whether a task input uses the removed `gro/` prefix, as opposed to a task path that starts with `gro/`.
 */
export const is_removed_builtin_prefix = (input: string): boolean =>
	input.startsWith(REMOVED_BUILTIN_PREFIX) && !is_task_path(input);

/**
 * The error for a task input with the removed `gro/` prefix, naming the plain builtin when there is one.
 * @param input - a task input for which `is_removed_builtin_prefix` is true
 */
export const to_removed_builtin_prefix_message = (input: string): string => {
	const builtin_name = input.slice(REMOVED_BUILTIN_PREFIX.length);
	return is_builtin_task_name(builtin_name)
		? `The \`${REMOVED_BUILTIN_PREFIX}\` prefix is removed, builtins are plain names: use \`${builtin_name}\`, as in \`gro ${builtin_name}\`.`
		: builtin_name
			? `The \`${REMOVED_BUILTIN_PREFIX}\` prefix is removed, and there's no builtin \`${builtin_name}\`.`
			: `The \`${REMOVED_BUILTIN_PREFIX}\` prefix is removed, builtins are plain names: run \`gro\` to list them.`;
};

/**
 * The error for a path given where a task name goes, like `gro src/lib/x.task.ts`,
 * hinting the `gro task` form that runs or lists it.
 * A path without a task suffix gets `to_task_path_hint`, which says what's there.
 * @param input - a task input for which `is_task_path_input` is true
 * @param root - the project root a relative `input` is relative to, defaults to the cwd
 * @param listable - whether the hint may name listing a directory, `false` for a forwarded section
 */
export const to_task_path_without_verb_message = (
	input: string,
	root: string = process.cwd(),
	listable = true
): string => {
	if (is_task_path(input)) {
		return `\`${input}\` is a path: run a task file with \`gro ${TASK_VERB} ${input}\`.`;
	}
	const subject = `\`${input}\` is a path, not a builtin name`;
	const hint = to_task_path_hint(input, root, listable);
	return hint === null ? `${subject}: ${TASK_PATH_SUFFIX_RULE}.` : `${subject}. ${hint}`;
};

/**
 * The task file paths an input without a task suffix likely meant:
 * `x.task` with `.ts` or `.js` added, and anything else with `.task.ts` or `.task.js`.
 */
const to_task_file_candidates = (input: string): Array<string> =>
	input.endsWith('.task')
		? [input + '.ts', input + '.js']
		: [input + TASK_FILE_SUFFIX_TS, input + TASK_FILE_SUFFIX_JS];

/**
 * Finds the task file an input without a task suffix likely meant, like `src/x/update.task.ts`
 * for `src/x/update`, preferring `.ts`.
 * Only error messages and hints call it, so its synchronous stats never slow a run.
 * @param input - a task input for which `is_task_path` is false
 * @param root - the project root a relative `input` is relative to, defaults to the cwd
 * @returns the task file's path as `input` names it, or `null`
 */
export const find_task_file_for_input = (
	input: string,
	root: string = process.cwd()
): string | null => {
	if (input.endsWith('/')) return null;
	for (const candidate of to_task_file_candidates(input)) {
		if (stat_sync_or_null(resolve(root, candidate))?.isFile()) return candidate;
	}
	return null;
};

/**
 * The hint that runs the task file at `path`.
 */
export const to_run_task_file_hint = (path: string): string =>
	`Run the task file with \`gro ${TASK_VERB} ${path}\`.`;

/**
 * Says what's at a task input that isn't a task path, like `src/test/fixtures/update`:
 * the task file it likely meant (`find_task_file_for_input`), which `gro task` runs,
 * and the directory, which it lists, or that nothing is at any of them.
 * Only error messages call it, so its synchronous stats never slow a run.
 * @param input - a task input for which `is_task_path` is false
 * @param root - the project root a relative `input` is relative to, defaults to the cwd
 * @param listable - whether to name listing a directory, `false` for a forwarded section,
 *   which can only run a task
 * @returns the hint, or `null` when `input` names something else,
 *   like a file without a task suffix, or a directory when not `listable`
 */
export const to_task_path_hint = (
	input: string,
	root: string = process.cwd(),
	listable = true
): string | null => {
	const hints: Array<string> = [];
	const task_file = find_task_file_for_input(input, root);
	if (task_file !== null) hints.push(to_run_task_file_hint(task_file));
	const stats = stat_sync_or_null(resolve(root, input));
	if (listable && stats?.isDirectory()) {
		hints.push(`List the directory's tasks with \`gro ${TASK_VERB} ${input}\`.`);
	}
	if (hints.length) return hints.join(' ');
	if (stats) return null;
	return input.endsWith('/')
		? `Nothing is at \`${input}\`.`
		: `Nothing is at \`${input}\`, ${to_task_file_candidates(input)
				.map((c) => `\`${c}\``)
				.join(', or ')}.`;
};

/**
 * Stats `path` synchronously for a hint, `null` for any error,
 * like `ENOTDIR` for a path through a file, since then there's nothing to hint.
 */
const stat_sync_or_null = (path: string): Stats | null => {
	try {
		return statSync(path, { throwIfNoEntry: false }) ?? null;
	} catch {
		return null;
	}
};
