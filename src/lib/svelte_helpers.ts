import type { FsPaths } from './fs_paths.ts';
import type { FsSearchResult } from './fs_search.ts';
import { posix_basename } from './internal/posix_path.ts';

/**
 * The suffix of a Svelte component file, which only svelte-check typechecks.
 */
export const SVELTE_FILE_SUFFIX = '.svelte';

// the directory names never searched for Svelte files, at any depth
const SVELTE_FILES_EXCLUDED_NAMES: ReadonlySet<string> = new Set(['node_modules', '.git']);

/**
 * Finds the Svelte component files beneath `dir`, the evidence `gro typecheck` reads
 * that a project needs svelte-check, as `find_test_files` finds the tests that need vitest.
 * No `node_modules` or `.git` directory is searched, and a symlink is never followed,
 * so a linked `.svelte` name counts as a file wherever it points.
 * @param fs_paths - searches `dir`
 * @param dir - the directory to search, absolute, the project's `src/` for `gro typecheck`
 * @returns the files sorted by `path`, an empty array when nothing is at `dir`
 * @throws `FsError` - when `dir` isn't a directory, or a listing fails
 */
export const find_svelte_files = (fs_paths: FsPaths, dir: string): Promise<Array<FsSearchResult>> =>
	fs_paths.search_path(dir, {
		filter: (id, is_directory) =>
			!is_directory || !SVELTE_FILES_EXCLUDED_NAMES.has(posix_basename(id)),
		file_filter: (id) => id.endsWith(SVELTE_FILE_SUFFIX)
	});
