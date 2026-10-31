import { join, extname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ensure_end } from '@fuzdev/fuz_util/string.ts';
import { styleText as st } from 'node:util';
import type { PathFilter, PathId } from '@fuzdev/fuz_util/path.ts';

import { GRO_CONFIG_FILENAME, GRO_DEV_DIR, GRO_DIR, LIB_DIR, SOURCE_DIR } from './constants.ts';

/*

A path `id` is an absolute path to the source/.gro/dist directory.
It's the same name that Rollup uses.

*/

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
	/** @trailing_slash */
	build_dev: string;
	config: string;
}

export const create_paths = (root_dir: string): Paths => {
	// TODO remove reliance on trailing slash towards windows support
	const root = ensure_end(root_dir, '/');
	return {
		root,
		source: root + SOURCE_DIR,
		lib: root + LIB_DIR,
		build: root + GRO_DIR,
		build_dev: root + GRO_DEV_DIR,
		config: root + GRO_CONFIG_FILENAME
	};
};

export const infer_paths = (id: PathId): Paths => (is_gro_id(id) ? gro_paths : paths);

export const is_gro_id = (id: PathId): boolean =>
	id.startsWith(gro_paths.root) || gro_paths.root === ensure_end(id, '/');

// '/home/me/app/src/foo/bar/baz.ts' → 'src/foo/bar/baz.ts'
export const to_root_path = (id: PathId, p = infer_paths(id)): string =>
	relative(p.root, id) || './';

// '/home/me/app/src/foo/bar/baz.ts' → 'foo/bar/baz.ts'
export const path_id_to_base_path = (path_id: PathId, p = infer_paths(path_id)): string =>
	relative(p.source, path_id);

// TODO base_path is an obsolete concept, it was a remnant from forcing `src/`
// 'foo/bar/baz.ts' → '/home/me/app/src/foo/bar/baz.ts'
export const base_path_to_path_id = (base_path: string, p = infer_paths(base_path)): PathId =>
	join(p.source, base_path);

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
export const paths = create_paths(process.cwd());

// TODO document these conditions with comments
// TODO there's probably a more robust way to do this
const filename = fileURLToPath(import.meta.url);
const gro_package_dir_path = join(
	filename,
	filename.includes('/gro/src/lib/')
		? '../../../'
		: filename.includes('/gro/dist/')
			? '../../'
			: '../'
);
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
 * A path outside `root` is tested as its absolute id.
 * @param search_filters - the filters each file and directory must pass, usually `config.search_filters`
 * @param root - the project root the paths are relative to, usually `paths.root`
 * @returns filters for `fs_search`, which passes them absolute ids
 */
export const to_search_filters_relative = (
	search_filters: PathFilter | Array<PathFilter> | null | undefined,
	root: string
): Array<PathFilter> => {
	if (!search_filters) return [];
	const filters = Array.isArray(search_filters) ? search_filters : [search_filters];
	const prefix = ensure_end(resolve(root), '/');
	return filters.map(
		(filter) => (id, is_directory) =>
			filter(id.startsWith(prefix) ? id.slice(prefix.length) : id, is_directory)
	);
};
