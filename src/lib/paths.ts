import { ensure_end } from '@fuzdev/fuz_util/string.ts';
import { st } from '@fuzdev/fuz_util/print.ts';
import type { PathFilter, PathId } from '@fuzdev/fuz_util/path.ts';

import { GRO_DIR, LIB_DIR, SOURCE_DIR } from './constants.ts';
import { posix_extname, posix_relative, posix_resolve } from './internal/posix_path.ts';

/*

`Paths` are built from the conventional locations in `./constants.ts`, never from the
SvelteKit `files` config values - reading those costs a full Vite config resolution,
which is too expensive for a module every Gro invocation imports.

Nothing here reads the process: a project's root comes from the caller,
like `create_paths(cwd())` at a composition root or `fs_paths.project.path` below one.

*/

export interface Paths {
	/** @trailing_slash */
	root: string;
	/** @trailing_slash */
	source: string;
	/** @trailing_slash */
	lib: string;
	/** @trailing_slash */
	build: string;
}

export const create_paths = (root_dir: string): Paths => {
	// TODO remove reliance on trailing slash towards windows support
	const root = ensure_end(root_dir, '/');
	return {
		root,
		source: root + SOURCE_DIR,
		lib: root + LIB_DIR,
		build: root + GRO_DIR
	};
};

/**
 * A path relative to the project root, like `src/foo/bar/baz.ts` for
 * `/home/me/app/src/foo/bar/baz.ts`, `./` for the root itself.
 * @param id - an absolute path, or one relative to `root`
 * @param root - the absolute project root
 */
export const to_root_path = (id: PathId, root: string): string =>
	posix_relative(root, posix_resolve(root, id)) || './';

/**
 * A path for display, relative to the project root as `to_root_path` reads it, starting `.`.
 * @param path - an absolute path, or one relative to `root`
 * @param root - the absolute project root
 */
export const print_path = (path: string, root: string): string => {
	const root_path = to_root_path(path, root);
	return st('gray', root_path[0] === '.' ? root_path : './' + root_path);
};

export const replace_extension = (path: string, new_extension: string): string => {
	const { length } = posix_extname(path);
	return (length === 0 ? path : path.substring(0, path.length - length)) + new_extension;
};

/**
 * Binds `search_filters` to the project root,
 * so each filter tests the path relative to `root` rather than the absolute id,
 * the same path whichever directory a search starts in,
 * and a project under a `.`-prefixed directory like `~/.cache/` still finds its files.
 * A path outside `root` but under `search_dir` is tested relative to `search_dir`,
 * so a search of a directory outside the project, like `~/.cache/other/src`, finds its files too,
 * and any other path is tested as its absolute id.
 * @param search_filters - the filters each file and directory must pass, usually `config.search_filters`
 * @param root - the absolute project root the paths are relative to, usually `fs_paths.project.path`
 * @param search_dir - the absolute directory being searched, the base for the paths outside `root`
 * @returns filters for `fs_search`, which passes them absolute ids
 */
export const to_search_filters_relative = (
	search_filters: PathFilter | Array<PathFilter> | null | undefined,
	root: string,
	search_dir?: string
): Array<PathFilter> => {
	if (!search_filters) return [];
	const filters = Array.isArray(search_filters) ? search_filters : [search_filters];
	const root_prefix = ensure_end(posix_resolve(root, '.'), '/');
	const dir_prefix =
		search_dir === undefined ? null : ensure_end(posix_resolve(search_dir, '.'), '/');
	const to_path = (id: string): string =>
		id.startsWith(root_prefix)
			? id.slice(root_prefix.length)
			: dir_prefix !== null && id.startsWith(dir_prefix)
				? id.slice(dir_prefix.length)
				: id;
	return filters.map((filter) => (id, is_directory) => filter(to_path(id), is_directory));
};
