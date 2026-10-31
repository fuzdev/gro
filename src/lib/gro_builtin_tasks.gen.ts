import { dirname } from 'node:path';

import type { Gen } from './gen.ts';
import { paths } from './paths.ts';
import { task_registry_find_builtins, task_registry_render_builtins } from './task_registry.ts';

/**
 * Writes `GroBuiltinTasks` and the builtin summaries from Gro's own tasks.
 *
 * @nodocs
 */
export const gen: Gen = {
	generate: async ({ origin_id, origin_path }) => {
		const entries = await task_registry_find_builtins(paths.lib);
		return task_registry_render_builtins(entries, dirname(origin_id), origin_path);
	},
	inputs: { patterns: ['*.task.ts'] }
};
