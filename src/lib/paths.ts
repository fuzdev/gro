import { join, extname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ensure_end } from '@fuzdev/fuz_util/string.ts';
import { styleText as st } from 'node:util';
import type { PathFilter, PathId } from '@fuzdev/fuz_util/path.ts';

import { GRO_DIR, LIB_DIR, SOURCE_DIR } from './constants.ts';
import { to_checked_cwd } from './internal/cwd.ts';

/*

`paths` is built from the conventional locations in `./constants.ts`, never from the
SvelteKit `files` config values - reading those costs a full Vite config resolution,
which is too expensive for a module every Gro invocation imports.

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

export const infer_paths = (id: PathId): Paths => (is_gro_id(id) ? gro_paths : paths);

export const is_gro_id = (id: PathId): boolean =>
	id.startsWith(gro_paths.root) || gro_paths.root === ensure_end(id, '/');

// '/home/me/app/src/foo/bar/baz.ts' → 'src/foo/bar/baz.ts'
export const to_root_path = (id: PathId, p = infer_paths(id)): string =>
	relative(p.root, id) || './';

export const print_path = (path: string, p = infer_paths(path)): string => {
	const root_path = to_root_path(path, p);
	return st('gray', root_path[0] === '.' ? root_path : './' + root_path);
};

export const replace_extension = (path: string, new_extension: string): string => {
	const { length } = extname(path);
	return (length === 0 ? path : path.substring(0, path.length - length)) + new_extension;
};

/**
 * `Paths` for the user repo.
 */
export const paths = create_paths(to_checked_cwd());

/**
 * Finds the Gro package directory from the path of Gro's own `paths` module,
 * by its location in the package (`src/lib/paths.ts` from source, `dist/paths.js` built),
 * so a checkout or worktree of any name works.
 * @param module_path - the absolute path of Gro's `paths` module
 * @returns the package directory with a trailing slash
 * @internal
 */
export const to_gro_package_dir = (module_path: PathId): string =>
	join(
		module_path,
		module_path.endsWith('/src/lib/paths.ts')
			? '../../../'
			: module_path.endsWith('/dist/paths.js')
				? '../../'
				: '../'
	);

const gro_package_dir_path = to_gro_package_dir(fileURLToPath(import.meta.url));
export const IS_THIS_GRO = gro_package_dir_path === paths.root;
/**
 * `Paths` for the Gro package being used by the user repo.
 */
export const gro_paths = IS_THIS_GRO ? paths : create_paths(gro_package_dir_path);

/**
 * Binds `search_filters` to the project root,
 * so each filter tests the path relative to `root` rather than the absolute id,
 * the same path whichever directory a search starts in,
 * and a project under a `.`-prefixed directory like `~/.cache/` still finds its files.
 * A path outside `root` but under `search_dir` is tested relative to `search_dir`,
 * so a search of a directory outside the project, like `~/.cache/other/src`, finds its files too,
 * and any other path is tested as its absolute id.
 * @param search_filters - the filters each file and directory must pass, usually `config.search_filters`
 * @param root - the project root the paths are relative to, usually `paths.root`
 * @param search_dir - the directory being searched, the base for the paths outside `root`
 * @returns filters for `fs_search`, which passes them absolute ids
 */
export const to_search_filters_relative = (
	search_filters: PathFilter | Array<PathFilter> | null | undefined,
	root: string,
	search_dir?: string
): Array<PathFilter> => {
	if (!search_filters) return [];
	const filters = Array.isArray(search_filters) ? search_filters : [search_filters];
	const root_prefix = ensure_end(resolve(root), '/');
	const dir_prefix = search_dir === undefined ? null : ensure_end(resolve(search_dir), '/');
	const to_path = (id: string): string =>
		id.startsWith(root_prefix)
			? id.slice(root_prefix.length)
			: dir_prefix !== null && id.startsWith(dir_prefix)
				? id.slice(dir_prefix.length)
				: id;
	return filters.map((filter) => (id, is_directory) => filter(to_path(id), is_directory));
};
