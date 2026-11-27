/**
 * The Node Gro's `GroPlatform`, what its composition roots hand every task's run.
 *
 * Node-only: it finds Gro's package directory from this module's own URL.
 * Another root hands Gro its own.
 *
 * @module
 */

import { to_file_path, type PathId } from '@fuzdev/fuz_util/path.ts';

import type { GroPlatform } from '../invoke_task.ts';
import type { FsPaths } from '../fs_paths.ts';
import type { GroPackage } from '../package_json.ts';
import { create_node_module_graph } from '../module_graph_node.ts';
import { load_default_svelte_config } from '../svelte_config.ts';
import { posix_join } from './posix_path.ts';

/**
 * Finds the Gro package directory from the path of this module,
 * by its location in the package (`src/lib/internal/` from source, `dist/internal/` built),
 * so a checkout or worktree of any name works.
 * @param module_path - the absolute path of this module
 * @returns the package directory with a trailing slash
 */
export const to_gro_package_dir = (module_path: PathId): string =>
	posix_join(
		module_path,
		module_path.endsWith('/src/lib/internal/node_platform.ts')
			? '../../../../'
			: module_path.endsWith('/dist/internal/node_platform.js')
				? '../../../'
				: '../'
	);

/**
 * Reads the running Gro's version from the `package.json` in its package directory.
 * @param fs_paths - opens Gro's `package.json`, beneath the project or not
 * @param dir - Gro's package directory
 */
export const load_gro_package = async (fs_paths: FsPaths, dir: string): Promise<GroPackage> => {
	const opened = await fs_paths.open_path(posix_join(dir, 'package.json'));
	const { version } = JSON.parse(await opened.dir.read_text(opened.path));
	return { version: typeof version === 'string' ? version : '', dir };
};

/**
 * Creates the Node Gro's platform, reading the running Gro's version,
 * with Vitest's module cache off, since its Vitest runs unconfined.
 * @param fs_paths - opens Gro's `package.json`
 */
export const create_node_platform = async (fs_paths: FsPaths): Promise<GroPlatform> => ({
	create_module_graph: create_node_module_graph,
	load_svelte_config: load_default_svelte_config,
	gro: await load_gro_package(fs_paths, to_gro_package_dir(to_file_path(new URL(import.meta.url)))),
	cli_name: 'gro',
	vitest_module_cache: null
});
