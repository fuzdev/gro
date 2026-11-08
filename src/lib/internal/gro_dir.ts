/**
 * The marker Gro keeps in a `.gro/` directory it made, so `gro clean` can tell one it may
 * empty from whatever else a symlinked `.gro/` points at.
 *
 * `.gro/` is usually a plain directory in the project, which `gro clean` removes whole.
 * Linked elsewhere, like to a directory on a tmpfs, `gro clean` keeps the link and empties
 * what it points at, but only when that directory holds the marker, a small file named
 * `GRO_DIR_MARKER_FILENAME`, and isn't inside a git work tree, where the marker could be
 * a committed file. Gro writes the marker only into a `.gro/` directory it creates itself,
 * like the missing target of a `.gro/` linked to a fresh tmpfs path, never into one that
 * already exists, however empty, so a link to any other directory never gets one.
 *
 * @module
 */

import { strip_end } from '@fuzdev/fuz_util/string.ts';

import { dirname } from 'node:path';

import { GIT_DIRNAME, GRO_DIRNAME } from '../constants.ts';
import type { FsDir, FsReadDir } from '../fs.ts';
import { is_fs_error } from '../fs_error.ts';
import type { FsPaths } from '../fs_paths.ts';

/**
 * The name of the marker in a `.gro/` directory Gro made.
 */
export const GRO_DIR_MARKER_FILENAME = '.gro_marker';

/**
 * The marker's text, which says what it's for to someone who finds it.
 */
export const GRO_DIR_MARKER_CONTENT =
	'Gro made this directory as a `.gro/`, and `gro clean` empties it.\n';

const to_marker_path = (path: string): string =>
	path ? path + '/' + GRO_DIR_MARKER_FILENAME : GRO_DIR_MARKER_FILENAME;

/**
 * Whether the directory at `path` beneath `dir` holds the marker, a regular file,
 * never a symlink.
 */
export const gro_dir_has_marker = async (dir: FsReadDir, path: string): Promise<boolean> =>
	(await dir.stat(to_marker_path(path), { follow: false }))?.kind === 'file';

/**
 * Writes the marker into the directory at `path` beneath `dir`, leaving one that's there.
 */
export const gro_dir_mark = async (dir: FsDir, path: string): Promise<void> => {
	await dir.write(to_marker_path(path), GRO_DIR_MARKER_CONTENT, { create_new: true });
};

/**
 * Whether the absolute real path `real` is inside a git work tree, by a `.git` entry
 * (a directory, a file for a linked worktree or submodule, or a symlink, never followed)
 * at it or any ancestor, one no-follow `stat` each.
 * Conservative: a `.git` anywhere above counts, whatever git would make of it,
 * and so does an ancestor that can't be read.
 * @param fs_paths - opens each directory
 * @param real - an absolute real path, so its ancestors hold no symlink
 */
export const gro_dir_is_in_git_work_tree = async (
	fs_paths: FsPaths,
	real: string
): Promise<boolean> => {
	for (let path = real; ; path = dirname(path)) {
		try {
			const opened = await fs_paths.open_dir_path(path);
			if (opened === null) return true;
			const git_path = opened.path ? opened.path + '/' + GIT_DIRNAME : GIT_DIRNAME;
			if ((await opened.dir.stat(git_path, { follow: false })) !== null) return true;
		} catch {
			return true;
		}
		if (path === '/') return false;
	}
};

/**
 * Makes a missing `.gro/` directory, its symlinks followed, and marks it as Gro's.
 * Only a directory this call creates is marked: one that exists, empty or not,
 * is left as it is, so the marker never claims a directory Gro didn't make.
 * @param fs_paths - opens `gro_dir`
 * @param gro_dir - absolute, or relative to the project, defaults to the project's `.gro/`
 */
export const gro_dir_ensure = async (
	fs_paths: FsPaths,
	gro_dir: string = GRO_DIRNAME
): Promise<void> => {
	const { dir, path } = await fs_paths.open_path(strip_end(gro_dir, '/'));
	if ((await dir.stat(path)) !== null) return;
	const parent = dirname(path);
	if (parent !== '.') await dir.mkdir(parent, { recursive: true });
	try {
		// not recursive, so a directory made since the stat fails here and stays unmarked
		await dir.mkdir(path);
	} catch (error) {
		if (is_fs_error(error, 'already_exists')) return;
		throw error;
	}
	await gro_dir_mark(dir, path);
};
