import type { Gen } from './gen.ts';
import { fs } from './fs.ts';
import { create_fs_paths } from './fs_paths.ts';
import { cwd } from './cwd.ts';
import { env } from './env.ts';
import {
	task_registry_find_builtins,
	task_registry_render_builtin_imports,
	task_registry_render_builtins
} from './task_registry.ts';
import { import_module_node } from './internal/node_import_module.ts';
import { posix_dirname } from './internal/posix_path.ts';

/**
 * Writes `GroBuiltinTasks`, the builtin summaries and grants,
 * and the literal import of each builtin, from Gro's own tasks.
 *
 * @nodocs
 */
export const gen: Gen = {
	generate: async ({ origin_id, origin_path }) => {
		const fs_paths = create_fs_paths(fs, { cwd: cwd(), home: env.get('HOME') });
		// Gro's `src/lib/`, this genfile's own directory
		const opened = await fs_paths.open_dir_path(posix_dirname(origin_id));
		if (!opened) throw Error(`no directory at ${posix_dirname(origin_id)}`);
		const src_lib = opened.dir.dir(opened.path).readonly();
		const entries = await task_registry_find_builtins(
			src_lib,
			import_module_node,
			fs_paths.project.path
		);
		const dir = posix_dirname(origin_id);
		return [
			task_registry_render_builtins(entries, dir, origin_path),
			{
				filename: 'gro_builtin_task_imports.ts',
				content: task_registry_render_builtin_imports(entries, dir, origin_path)
			}
		];
	},
	inputs: { patterns: ['*.task.ts'] }
};
