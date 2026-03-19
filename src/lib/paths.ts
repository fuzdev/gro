import {join, extname, relative, basename} from 'node:path';
import {fileURLToPath} from 'node:url';
import {ensure_end, strip_end} from '@fuzdev/fuz_util/string.js';
import {styleText as st} from 'node:util';
import type {PathId} from '@fuzdev/fuz_util/path.js';

import {
	GRO_CONFIG_FILENAME,
	GRO_DEV_DIR,
	GRO_DIR,
	SOURCE_DIR,
	SVELTEKIT_DIST_DIRNAME,
} from './constants.ts';
import {default_svelte_config} from './svelte_config.ts';

/*

A path `id` is an absolute path to the source/.gro/dist directory.
It's the same name that Rollup uses.

*/

export const LIB_DIRNAME = basename(default_svelte_config.lib_path);
export const LIB_PATH = SOURCE_DIR + LIB_DIRNAME;
/** @trailing_slash */
export const LIB_DIR = LIB_PATH + '/';
export const ROUTES_DIRNAME = basename(default_svelte_config.routes_path);

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
		config: root + GRO_CONFIG_FILENAME,
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
	let final_path =
		strip_end(path, '/') === strip_end(GRO_DIST_DIR, '/') ? 'gro' : to_root_path(path, p);
	final_path =
		final_path === 'gro' ? final_path : final_path[0] === '.' ? final_path : './' + final_path;
	return st('gray', final_path);
};

export const replace_extension = (path: string, new_extension: string): string => {
	const {length} = extname(path);
	return (length === 0 ? path : path.substring(0, path.length - length)) + new_extension;
};

/**
 * `Paths` for the user repo.
 */
export const paths = create_paths(process.cwd());

/** @trailing_slash */
export const GRO_PACKAGE_DIR = 'gro/';
// TODO document these conditions with comments
// TODO there's probably a more robust way to do this
const filename = fileURLToPath(import.meta.url);
const gro_package_dir_path = join(
	filename,
	filename.includes('/gro/src/lib/')
		? '../../../'
		: filename.includes('/gro/dist/')
			? '../../'
			: '../',
);
export const IS_THIS_GRO = gro_package_dir_path === paths.root;
/**
 * `Paths` for the Gro package being used by the user repo.
 */
export const gro_paths = IS_THIS_GRO ? paths : create_paths(gro_package_dir_path);
/** @trailing_slash */
export const GRO_DIST_DIR = gro_paths.root + SVELTEKIT_DIST_DIRNAME + '/';
