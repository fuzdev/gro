/**
 * Recursive search of a directory through an fs handle.
 *
 * Library code over `FsReadDir`: it takes a handle and reaches nothing else.
 *
 * @module
 */

import type { FileFilter, PathFilter, ResolvedPath } from '@fuzdev/fuz_util/path.ts';
import { ensure_end, strip_end } from '@fuzdev/fuz_util/string.ts';

import type { FsReadDir } from './fs.ts';

export interface FsSearchOptions {
	/**
	 * The directory to search, relative to the handle. Defaults to the handle's own directory.
	 */
	path?: string;
	/**
	 * The absolute directory each id starts with: an entry's id is `id_dir`, a `/`,
	 * and the entry's path relative to the searched directory.
	 * Defaults to the searched directory's real path, beneath the handle's.
	 * A caller that names the directory by another absolute path passes it,
	 * so the ids and the filters see the path the caller named.
	 */
	id_dir?: string;
	/**
	 * Filters each entry must pass, given its absolute id and whether it's a directory.
	 * A directory that fails isn't descended into.
	 */
	filter?: PathFilter | Array<PathFilter>;
	/**
	 * Filters each file must pass, given its absolute id.
	 */
	file_filter?: FileFilter | Array<FileFilter>;
	/**
	 * Whether the results are sorted by `path`, defaults to `true`.
	 * Without sorting the order is volatile.
	 */
	sort?: boolean;
}

/**
 * Finds the files beneath a directory, recursively.
 *
 * Each result is a file. Its `path` is relative to the searched directory and its `id`
 * is absolute (see `id_dir`). A symlink is never followed, so it's a result like a file
 * whatever it points at, as is any other entry that isn't a directory.
 *
 * @param dir - the handle to search beneath
 * @returns the found paths, or an empty array when nothing is at the searched path
 * @throws `FsError` - when the searched path isn't a directory, or a listing fails
 */
export const fs_search = async (
	dir: FsReadDir,
	options?: FsSearchOptions
): Promise<Array<ResolvedPath>> => {
	const { path = '', filter, file_filter, sort = true } = options ?? {};

	if ((await dir.stat(path)) === null) return [];

	const base = strip_end(path, '/');
	const id_dir = ensure_end(options?.id_dir ?? (base ? dir.path + '/' + base : dir.path), '/');
	const filters = to_filters(filter);
	const file_filters = to_filters(file_filter);

	const found: Array<ResolvedPath> = [];

	const search = async (relative_dir: string | null): Promise<void> => {
		const list_path =
			relative_dir === null ? base : base ? base + '/' + relative_dir : relative_dir;
		// a directory is listed in full before its subdirectories are searched, so its own
		// handle closes first, and sibling subdirectories are searched concurrently
		const subdirs: Array<string> = [];
		for await (const { name, kind } of dir.list(list_path)) {
			const entry_path = relative_dir === null ? name : relative_dir + '/' + name;
			const id = id_dir + entry_path;
			const is_directory = kind === 'directory';
			if (filters && !filters.every((f) => f(id, is_directory))) continue;
			if (is_directory) {
				subdirs.push(entry_path);
			} else if (!file_filters || file_filters.every((f) => f(id))) {
				found.push({ path: entry_path, id, is_directory: false });
			}
		}
		await Promise.all(subdirs.map(search));
	};
	await search(null);

	return sort ? found.sort(compare_paths) : found;
};

const compare_paths = (a: ResolvedPath, b: ResolvedPath): number => a.path.localeCompare(b.path);

const to_filters = <T>(filter: T | Array<T> | undefined): Array<T> | undefined =>
	filter === undefined
		? undefined
		: Array.isArray(filter)
			? filter.length
				? filter
				: undefined
			: [filter];
