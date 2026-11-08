/**
 * The boundary bound to the fs host module and the working directory, for the library
 * modules that don't yet take an `FsPaths` from their composition root.
 *
 * Each op is `FsPaths`'s (`fs_paths.ts`), over `fs`, with a relative path resolved
 * against the cwd rather than the project, as these modules have always resolved theirs.
 *
 * @module
 */

import { resolve } from 'node:path';

import { fs } from '../fs.ts';
import type { FsPaths } from '../fs_paths.ts';
import { create_fs_boundary, to_protected_dirs } from './fs_boundary.ts';

// TODO: delete once every library module takes `FsPaths` from its composition root

/**
 * The boundary over `fs`, relative paths resolved against the cwd.
 */
export const cwd_fs_paths: FsPaths = create_fs_boundary(fs, (path) => resolve(path));

export const {
	open_path: fs_open_path,
	open_dir_path: fs_open_dir_path,
	search_path: fs_search_path,
	copy_path: fs_copy_path,
	empty_dir_path: fs_empty_dir_path,
	remove_path: fs_remove_path
} = cwd_fs_paths;

/**
 * The directories no removal or emptying may touch, beside `/` and their ancestors:
 * the project, the working directory, and `$HOME` as given and as its real path. Resolved once.
 */
export const fs_protected_dirs = (): Promise<Array<string>> => to_protected_dirs(fs.project.path);
