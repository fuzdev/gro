import { dirname } from 'node:path';

import type { Gen } from './gen.ts';
import { paths } from './paths.ts';
import { task_registry_find, task_registry_render_builtins } from './task_registry.ts';

/**
 * Writes `GroBuiltinTasks` from Gro's own tasks.
 *
 * @nodocs
 */
export const gen: Gen = {
	generate: async ({ config, origin_id, origin_path }) => {
		const entries = await task_registry_find([paths.lib], config.search_filters);
		return task_registry_render_builtins(entries, dirname(origin_id), origin_path);
	},
	dependencies: { patterns: [/\.task\.ts$/] }
};
