import type { DirEntry } from './fs.ts';
import type { FsOpenedPath, FsPaths } from './fs_paths.ts';
import type { FsSearchResult } from './fs_search.ts';
import { is_fs_error } from './fs_error.ts';
import { compare_paths_by_segment } from './internal/code_points.ts';
import { posix_join } from './internal/posix_path.ts';

/**
 * The suffix of a Svelte component file, which only svelte-check typechecks.
 */
export const SVELTE_FILE_SUFFIX = '.svelte';

// the names TypeScript's `**` and `*` never match, so svelte-check never reads them with a tsconfig,
// beside every name starting with `.`, at any depth
const SVELTE_FILES_EXCLUDED_NAMES: ReadonlySet<string> = new Set([
	'node_modules',
	'bower_components',
	'jspm_packages'
]);

const is_excluded_name = (name: string): boolean =>
	name.startsWith('.') || SVELTE_FILES_EXCLUDED_NAMES.has(name);

/**
 * Finds the Svelte component files beneath `dir`, the evidence `gro typecheck` reads
 * that a project needs svelte-check, as `find_test_files` finds the tests that need vitest.
 * No name starting with `.`, nor a `node_modules`, `bower_components`, or `jspm_packages`
 * directory, is searched or counted, as TypeScript's wildcards skip them.
 *
 * Symlinks are followed as svelte-check follows them with a tsconfig, through TypeScript's
 * directory reads: a link to a directory is searched, a link to a file counts by its own name,
 * and one that doesn't resolve, dangling or a loop, is skipped. Each real directory is searched
 * once, the first time a depth-first walk in name order reaches it, so a loop of links ends
 * and a directory reached by two paths is named by the first.
 *
 * @param fs_paths - opens `dir` and each symlink beneath it
 * @param dir - the directory to search, absolute, the project's `src/` for `gro typecheck`
 * @returns the files sorted by `path`, named through the links, with `kind` the listed one;
 * an empty array when nothing is at `dir`
 * @throws `FsError` - when `dir` isn't a directory, or a listing fails
 */
export const find_svelte_files = async (
	fs_paths: FsPaths,
	dir: string
): Promise<Array<FsSearchResult>> => {
	const root = await fs_paths.open_dir_path(dir);
	if (root === null || (await root.dir.stat(root.path)) === null) return [];

	const found: Array<FsSearchResult> = [];
	// each real directory searched, by its path
	const searched: Set<string> = new Set();

	const search = async (opened: FsOpenedPath, relative_dir: string): Promise<void> => {
		const real_dir = posix_join(opened.dir.path, opened.path);
		if (searched.has(real_dir)) return;
		searched.add(real_dir);
		// listed in full before anything beneath it is searched, so its handle closes first
		const entries: Array<DirEntry & { path: string }> = [];
		for await (const { name, kind } of opened.dir.list(opened.path)) {
			entries.push({ path: relative_dir ? relative_dir + '/' + name : name, name, kind });
		}
		for (const { path, name, kind } of entries) {
			if (is_excluded_name(name)) continue;
			if (kind === 'directory') {
				await search({ dir: opened.dir, path: posix_join(opened.path, name) }, path);
			} else if (kind === 'symlink') {
				const target = await open_link_target(fs_paths, posix_join(dir, path));
				if (target === null) continue;
				if (target.is_directory) {
					await search(target.opened, path);
				} else if (name.endsWith(SVELTE_FILE_SUFFIX)) {
					found.push({ path, id: posix_join(dir, path), is_directory: false, kind });
				}
			} else if (name.endsWith(SVELTE_FILE_SUFFIX)) {
				found.push({ path, id: posix_join(dir, path), is_directory: false, kind });
			}
		}
	};
	await search(root, '');

	return found.sort((a, b) => compare_paths_by_segment(a.path, b.path));
};

/**
 * Opens what the symlink at `path` points at, `null` when it doesn't resolve,
 * as TypeScript skips a link it can't stat.
 */
const open_link_target = async (
	fs_paths: FsPaths,
	path: string
): Promise<{ opened: FsOpenedPath; is_directory: boolean } | null> => {
	let opened: FsOpenedPath;
	try {
		opened = await fs_paths.open_path(path);
	} catch (error) {
		if (is_fs_error(error)) return null;
		throw error;
	}
	const info = await opened.dir.stat(opened.path);
	if (info === null) return null;
	return { opened, is_directory: info.kind === 'directory' };
};
