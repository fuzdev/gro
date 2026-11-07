import { rm } from 'node:fs/promises';

import { paths } from './paths.ts';
import {
	NODE_MODULES_DIRNAME,
	SVELTEKIT_DEV_DIRNAME,
	SVELTEKIT_BUILD_DIRNAME,
	SVELTEKIT_VITE_CACHE_PATH,
	SVELTEKIT_DIST_DIRNAME
} from './constants.ts';
import { discover_dist_dirs } from './build_cache.ts';

export interface CleanFsOptions {
	/**
	 * Delete the `.gro/` directory.
	 */
	gro_dir?: boolean;
	/**
	 * Delete the `dist_*` plugin output directories at the project root.
	 */
	dist_dirs?: boolean;
	/**
	 * Delete the SvelteKit directory, its build and dist outputs, and the Vite cache.
	 */
	sveltekit?: boolean;
	/**
	 * Delete `node_modules/`.
	 */
	nodemodules?: boolean;
}

const RM_OPTIONS = { force: true, recursive: true };

/**
 * Deletes the selected temporary and build directories, each independently.
 */
export const clean_fs = async ({
	gro_dir = false,
	dist_dirs = false,
	sveltekit = false,
	nodemodules = false
}: CleanFsOptions): Promise<void> => {
	const paths_to_rm: Array<string> = [];
	if (gro_dir) paths_to_rm.push(paths.build);
	if (dist_dirs) paths_to_rm.push(...(await discover_dist_dirs()));
	if (sveltekit) {
		paths_to_rm.push(
			SVELTEKIT_DEV_DIRNAME,
			SVELTEKIT_BUILD_DIRNAME,
			SVELTEKIT_DIST_DIRNAME,
			SVELTEKIT_VITE_CACHE_PATH
		);
	}
	if (nodemodules) paths_to_rm.push(NODE_MODULES_DIRNAME);
	await Promise.all(paths_to_rm.map((p) => rm(p, RM_OPTIONS)));
};
