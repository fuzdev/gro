import { dirname } from 'node:path';

import type { Gen } from './gen.ts';
import { fs } from './fs.ts';
import { create_fs_paths } from './fs_paths.ts';
import { task_registry_find_builtins, task_registry_render_builtins } from './task_registry.ts';

/**
 * Writes `GroBuiltinTasks` and the builtin summaries from Gro's own tasks.
 *
 * @nodocs
 */
export const gen: Gen = {
	generate: async ({ origin_id, origin_path }) => {
		// Gro's `src/lib/`, this genfile's own directory
		const opened = await create_fs_paths(fs).open_dir_path(dirname(origin_id));
		if (!opened) throw Error(`no directory at ${dirname(origin_id)}`);
		const src_lib = opened.dir.dir(opened.path).readonly();
		const entries = await task_registry_find_builtins(src_lib);
		return task_registry_render_builtins(entries, dirname(origin_id), origin_path);
	},
	inputs: { patterns: ['*.task.ts'] }
};
