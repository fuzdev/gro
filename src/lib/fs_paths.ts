/**
 * Gro's boundary from paths that arrive from outside to the fs host module's handles.
 *
 * A path from a CLI arg, a config, or one of Gro's well-known paths names a file by
 * absolute or project-relative path, while a handle takes only relative paths beneath it.
 * An `FsPaths` converts one into the other where the path enters code that reaches the
 * filesystem. It follows symlinks the way `fs.open_dir` does, the same inside and outside
 * the project, so the confinement of a handle applies only to the paths code resolves
 * beneath a handle it already holds.
 *
 * It opens any path, so it's built from the root `Fs` and never from a handle:
 * a composition root that imports the host module builds one with `create_fs_paths(fs)`
 * and passes it down, and code given one reaches what the root's `fs` reaches.
 * Code that holds a handle gets narrower handles from it, never wider ones by path.
 *
 * @module
 */

import { resolve } from 'node:path';

import type { Fs, FsCopyOptions, FsDir, FsRemoveOptions } from './fs.ts';
import type { FsSearchOptions, FsSearchResult } from './fs_search.ts';
import { create_fs_boundary } from './internal/fs_boundary.ts';

/**
 * A path resolved to a handle and the relative path beneath it.
 */
export interface FsOpenedPath {
	dir: FsDir;
	/**
	 * Relative to `dir`, `''` for `dir` itself, keeping a requested trailing `/`.
	 */
	path: string;
}

/**
 * Opens paths from outside as handles. A relative path is relative to `project`,
 * resolved lexically, its `..` segments before any symlink is followed, as Node's
 * `path.resolve` does. Then it's resolved to its real path, every symlink in it followed,
 * a dangling final one to its missing target, and a missing part appended to its nearest
 * existing ancestor's real path. A real path at or beneath the project is relative to
 * `project`; any other opens its parent, or the parent's nearest existing ancestor,
 * with `fs.open_dir`. So the op on the result follows no symlink but ones created since,
 * and a symlink out of the project, at the end of the path or before it, reaches what
 * it points at.
 */
export interface FsPaths {
	/**
	 * The project directory, the `fs.project` of the `Fs` this was built from,
	 * the `dir` of every path that resolves beneath it.
	 */
	readonly project: FsDir;
	/**
	 * Converts a path into a handle and the relative path beneath it.
	 * @param path - absolute, or relative to the project; a trailing `/` is kept
	 * @throws `FsError` - `not_a_directory` when, outside the project, an ancestor of the path
	 * is a file (beneath the project, the op on the result throws it instead);
	 * `io_error` for a symlink loop (ELOOP); `permission_denied` or `io_error` when reading
	 * a directory or symlink on the way fails otherwise
	 */
	open_path(path: string): Promise<FsOpenedPath>;
	/**
	 * Opens a directory, resolved as `open_path` resolves it:
	 * beneath the project, `project` and the directory's path beneath it,
	 * else the directory as its own handle, so what's found beneath it reaches nothing above.
	 * @param path - absolute, or relative to the project
	 * @returns the handle and the directory's path beneath it; `null` when nothing is at a `path`
	 * outside the project, while beneath it the result is `project` and the path, missing or not,
	 * and the op on it reports a missing directory
	 * @throws `FsError` - `not_a_directory` when `path` is a file outside the project
	 * (beneath the project, the op on the result throws it instead), otherwise as `open_path` does
	 */
	open_dir_path(path: string): Promise<FsOpenedPath | null>;
	/**
	 * Searches a directory as `fs_search` does, with ids that start with the path as named,
	 * made absolute against the project. The directory is opened as `open_dir_path` opens it,
	 * so the search reaches nothing above it.
	 * @param path - absolute, or relative to the project
	 * @returns the found paths, or an empty array when nothing is at `path`
	 * @throws `FsError` - when `path` isn't a directory, or a listing fails
	 */
	search_path(
		path: string,
		options?: Omit<FsSearchOptions, 'path' | 'id_dir'>
	): Promise<Array<FsSearchResult>>;
	/**
	 * Copies a file, or with `recursive` a directory tree, as `FsDir.copy` does,
	 * symlinks in a tree copied as links. Both paths are resolved as `open_path` resolves them,
	 * and the copy runs beneath `project` when both are in the project,
	 * else beneath their nearest common directory.
	 * @param from - absolute, or relative to the project
	 * @param to - absolute, or relative to the project
	 * @throws `FsError` - as `open_path` and `FsDir.copy` do
	 */
	copy_path(from: string, to: string, options?: FsCopyOptions): Promise<void>;
	/**
	 * Empties a directory, its symlinks followed, removing everything in it
	 * but never the directory itself, so a symlink to it is kept.
	 * @param path - absolute, or relative to the project
	 * @throws `FsError` - `io_error` when the directory is the project, the process's working
	 * directory, `$HOME`, or an ancestor of one, `not_a_directory` when it's a file,
	 * otherwise as `open_dir_path` and `FsDir.remove` do
	 */
	empty_dir_path(path: string): Promise<void>;
	/**
	 * Removes the entry at a path, never what a final symlink points at:
	 * the parent is resolved as `open_path` resolves it, symlinks followed,
	 * and the final entry is removed as `FsDir.remove` does, a symlink removed itself.
	 * A trailing `/` removes only a directory, as in `FsDir.remove`.
	 * @param path - absolute, or relative to the project
	 * @throws `FsError` - `io_error` for the project, the process's working directory, `$HOME`,
	 * or an ancestor of one, otherwise as `open_path` and `FsDir.remove` do
	 */
	remove_path(path: string, options?: FsRemoveOptions): Promise<void>;
}

/**
 * Builds the boundary over the fs root, resolving relative paths against `fs.project`.
 * @param fs - the fs host module's root, `fs` from `fs.ts` or a mock of it
 */
export const create_fs_paths = (fs: Fs): FsPaths => {
	const project_path = fs.project.path;
	return create_fs_boundary(fs, (path) => resolve(project_path, path));
};
