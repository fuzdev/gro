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
import type { BuiltinTaskGrants } from './builtin_host.ts';
import { import_module_node } from './internal/node_import_module.ts';
import { posix_dirname } from './internal/posix_path.ts';

/**
 * The grants of the CLI's own steps before any task, rendered as `GRO_CLI_GRANTS`,
 * which `gro_main` cuts its host by, as `run_task` cuts a builtin's by `GRO_BUILTIN_TASK_GRANTS`:
 * reading the project, to resolve the command, and loading the project's code,
 * its config and the task modules `gro task` lists.
 */
const cli_grants = {
	// resolves a task path and lists the project's tasks
	fs: { read: ['.'] },
	// loads the project's code: its `gro.config.ts`, and the task modules `gro task` lists
	project_modules: true
} as const satisfies BuiltinTaskGrants;

/**
 * Writes `GroBuiltinTasks`, the builtin summaries and grants, the CLI's own grants,
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
			task_registry_render_builtins(entries, cli_grants, dir, origin_path),
			{
				filename: 'gro_builtin_task_imports.ts',
				content: task_registry_render_builtin_imports(entries, dir, origin_path)
			}
		];
	},
	inputs: { patterns: ['*.task.ts'] }
};
