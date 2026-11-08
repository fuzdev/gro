import { basename, dirname, resolve } from 'node:path';
import { strip_end } from '@fuzdev/fuz_util/string.ts';

import { paths } from './paths.ts';
import {
	NODE_MODULES_DIRNAME,
	SVELTEKIT_DEV_DIRNAME,
	SVELTEKIT_BUILD_DIRNAME,
	SVELTEKIT_VITE_CACHE_PATH,
	SVELTEKIT_DIST_DIRNAME
} from './constants.ts';
import { discover_dist_dirs } from './build_cache.ts';
import { fs, is_fs_error } from './fs.ts';
import {
	fs_empty_dir_path,
	fs_open_dir_path,
	fs_open_path,
	fs_remove_path,
	type FsOpenedPath
} from './internal/fs_path.ts';

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

/**
 * Deletes the selected temporary and build directories, each independently.
 * A symlink at one of their paths is removed itself, never followed,
 * except a `.gro/` linked to a directory outside the project, like one on a tmpfs,
 * which keeps its link and has the directory it points at emptied.
 */
export const clean_fs = async ({
	gro_dir = false,
	dist_dirs = false,
	sveltekit = false,
	nodemodules = false
}: CleanFsOptions): Promise<void> => {
	const paths_to_rm: Array<string> = [];
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
	await Promise.all([
		gro_dir ? clean_gro_dir(paths.build) : null,
		...paths_to_rm.map((p) => remove_path(p))
	]);
};

// without a trailing `/`, which would refuse a symlink rather than remove it
const remove_path = (path: string): Promise<void> =>
	fs_remove_path(strip_end(path, '/'), { recursive: true, missing_ok: true });

/**
 * Removes `.gro/`, or when it's a symlink to a directory outside the project,
 * keeps the link and empties the directory. A symlink to anything else, inside the
 * project, dangling, or in a loop, is removed itself, as any other cleaned path is.
 */
const clean_gro_dir = async (path: string): Promise<void> => {
	const resolved = resolve(strip_end(path, '/'));
	const parent = await fs_open_path(dirname(resolved));
	const name = basename(resolved);
	const info = await parent.dir.stat(parent.path ? parent.path + '/' + name : name, {
		follow: false
	});
	if (info === null) return;
	if (info.kind === 'symlink') {
		let target: FsOpenedPath | null = null;
		try {
			target = await fs_open_dir_path(resolved);
		} catch (error) {
			// a loop, a link to a file, or unreadable, all unlinked below
			if (!is_fs_error(error)) throw error;
		}
		if (target !== null && target.dir !== fs.project) {
			await fs_empty_dir_path(resolved);
			return;
		}
	}
	await remove_path(resolved);
};
