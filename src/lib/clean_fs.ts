import { basename, dirname, resolve } from 'node:path';
import { strip_end } from '@fuzdev/fuz_util/string.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';

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
import {
	GRO_DIR_MARKER_FILENAME,
	gro_dir_has_marker,
	gro_dir_is_in_git_work_tree,
	gro_dir_mark
} from './internal/gro_dir.ts';

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
	/**
	 * Warned when a linked `.gro/` is unlinked rather than emptied, saying why.
	 */
	log?: Logger;
}

/**
 * Deletes the selected temporary and build directories, each independently.
 * A symlink at one of their paths is removed itself, never followed,
 * except a `.gro/` linked to a directory outside the project that holds Gro's marker
 * (`internal/gro_dir.ts`) and isn't inside a git work tree, like one on a tmpfs,
 * which keeps its link and has the directory it points at emptied.
 */
export const clean_fs = async ({
	gro_dir = false,
	dist_dirs = false,
	sveltekit = false,
	nodemodules = false,
	log
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
		gro_dir ? clean_gro_dir(paths.build, log) : null,
		...paths_to_rm.map((p) => remove_path(p))
	]);
};

// without a trailing `/`, which would refuse a symlink rather than remove it
const remove_path = (path: string): Promise<void> =>
	fs_remove_path(strip_end(path, '/'), { recursive: true, missing_ok: true });

/**
 * Removes `.gro/`, or when it's a symlink to a directory outside the project that holds
 * Gro's marker and isn't inside a git work tree, keeps the link and empties the directory,
 * marker rewritten. A dangling symlink is left alone, with nothing to clean. A symlink to
 * anything else, inside the project, to an unmarked directory like a sibling repo, to one
 * in a git work tree, to a file, or in a loop, is removed itself, as any other cleaned path
 * is, its target untouched, with a warning when the target is a directory outside the
 * project, unmarked or in a git work tree.
 */
const clean_gro_dir = async (path: string, log: Logger | undefined): Promise<void> => {
	const resolved = resolve(strip_end(path, '/'));
	const parent = await fs_open_path(dirname(resolved));
	const name = basename(resolved);
	const info = await parent.dir.stat(parent.path ? parent.path + '/' + name : name, {
		follow: false
	});
	if (info === null) return;
	if (info.kind === 'symlink') {
		// dangling, like a link to a tmpfs path gone after a reboot: nothing to clean,
		// and Gro creates and marks the target on its next write through the link
		// followed wherever it points, as `fs_open_path` follows, not confined beneath the project
		let dangling = false;
		try {
			const opened = await fs_open_path(resolved);
			dangling = (await opened.dir.stat(opened.path)) === null;
		} catch (error) {
			// a loop, a path through a file, or unreadable, all unlinked below
			if (!is_fs_error(error)) throw error;
		}
		if (dangling) return;
		let target: FsOpenedPath | null = null;
		try {
			target = await fs_open_dir_path(resolved);
		} catch (error) {
			// a loop, a link to a file, or unreadable, all unlinked below
			if (!is_fs_error(error)) throw error;
		}
		if (target !== null && target.dir !== fs.project) {
			const target_path = target.path ? target.dir.path + '/' + target.path : target.dir.path;
			const why = !(await gro_dir_has_marker(target.dir, target.path))
				? `it has no \`${GRO_DIR_MARKER_FILENAME}\`, so Gro didn't create it` +
					' - Gro marks a `.gro/` target only when it creates it, so link `.gro/`' +
					' to a path that does not exist yet, like a fresh one on a tmpfs'
				: (await gro_dir_is_in_git_work_tree(target_path))
					? 'it is inside a git work tree, where its marker could be a committed file' +
						' - link `.gro/` to a path outside any git work tree'
					: null;
			if (why === null) {
				await fs_empty_dir_path(resolved);
				await gro_dir_mark(target.dir, target.path);
				return;
			}
			log?.warn(
				`unlinking ${resolved}, which points at ${target_path}, without emptying it: ${why}`
			);
		}
	}
	await remove_path(resolved);
};
