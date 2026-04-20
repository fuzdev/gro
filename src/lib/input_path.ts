import {dirname, isAbsolute, join, resolve} from 'node:path';
import {stat} from 'node:fs/promises';
import {fs_exists, fs_search} from '@fuzdev/fuz_util/fs.js';
import {strip_start} from '@fuzdev/fuz_util/string.js';
import {z} from 'zod';
import type {Flavored} from '@fuzdev/fuz_util/types.js';
import type {PathInfo, PathId, ResolvedPath} from '@fuzdev/fuz_util/path.js';

import {GRO_PACKAGE_DIR, GRO_DIST_DIR} from './paths.ts';
import {TASK_FILE_SUFFIX_JS} from './task.ts';

// TODO Flavored doesn't work when used in schemas, use Zod brand instead? problem is ergonomics
export const InputPath = z.string();
export type InputPath = Flavored<z.infer<typeof InputPath>, 'InputPath'>;

export const RawInputPath = z.string();
export type RawInputPath = Flavored<z.infer<typeof RawInputPath>, 'RawInputPath'>;

/**
 * Raw input paths are paths that users provide to Gro to reference files for tasks and gen.
 *
 * A raw input path can be to a file or directory in the following forms:
 *
 * - an absolute path, preserved
 * - an explicit relative path, e.g. `./src/foo`, resolved to `root_path` which defaults to the cwd
 * - an implicit relative path, e.g. `src/foo`, preserved
 * - an implicit relative path prefixed with `gro/`, transformed to absolute in the Gro directory
 *
 * Thus, input paths are either absolute or implicitly relative.
 */
export const to_input_path = (
	raw_input_path: RawInputPath,
	root_path = process.cwd(), // TODO @many isn't passed in anywhere, maybe hoist to `invoke_task` and others
): InputPath => {
	if (raw_input_path.startsWith(GRO_PACKAGE_DIR)) {
		return GRO_DIST_DIR + strip_start(raw_input_path, GRO_PACKAGE_DIR);
	} else if (raw_input_path[0] === '.') {
		return resolve(root_path, raw_input_path);
	}
	return raw_input_path as InputPath;
};

export const to_input_paths = (
	raw_input_paths: Array<RawInputPath>,
	root_path?: string, // TODO @many isn't passed in anywhere, maybe hoist to `invoke_task` and others
): Array<InputPath> => raw_input_paths.map((p) => to_input_path(p, root_path));

export interface PossiblePath {
	id: PathId;
	input_path: InputPath;
	root_dir: PathId;
}

/**
 * Gets a list of possible source ids for each input path with `extensions`,
 * duplicating each under `root_dirs`, without checking the filesystem.
 */
export const get_possible_paths = async (
	input_path: InputPath,
	root_dirs: Array<PathId>,
	extensions: Array<string>,
): Promise<Array<PossiblePath>> => {
	const possible_paths: Set<PossiblePath> = new Set();

	const add_possible_paths = (path: string, root_dir: PathId) => {
		// Specifically for paths to the Gro package dist, optimize by only looking for `.task.js`.
		if (path.startsWith(GRO_DIST_DIR)) {
			possible_paths.add({
				id: (path.endsWith('/') || path.endsWith(TASK_FILE_SUFFIX_JS)
					? path
					: path + TASK_FILE_SUFFIX_JS) as PathId,
				input_path,
				root_dir,
			});
		} else {
			possible_paths.add({id: path as PathId, input_path, root_dir});
			if (!path.endsWith('/') && !extensions.some((e) => path.endsWith(e))) {
				for (const extension of extensions) {
					possible_paths.add({id: path + extension, input_path, root_dir});
				}
			}
		}
	};

	if (isAbsolute(input_path)) {
		const exists = await fs_exists(input_path);
		const is_directory = exists && (await stat(input_path)).isDirectory();
		add_possible_paths(input_path, is_directory ? input_path : dirname(input_path));
	} else {
		for (const root_dir of root_dirs) {
			add_possible_paths(join(root_dir, input_path), root_dir);
		}
	}
	return Array.from(possible_paths);
};

export interface ResolvedInputPath {
	input_path: InputPath;
	id: PathId;
	is_directory: boolean;
	root_dir: PathId;
}

export interface ResolvedInputFile {
	id: PathId;
	input_path: InputPath;
	resolved_input_path: ResolvedInputPath;
}

export interface ResolvedInputPaths {
	resolved_input_paths: Array<ResolvedInputPath>;
	possible_paths_by_input_path: Map<InputPath, Array<PossiblePath>>;
	unmapped_input_paths: Array<InputPath>;
}

/**
 * Gets the path data for each input path, checking the filesystem for the possibilities
 * and stopping at the first existing file or falling back to the first existing directory.
 * If none is found for an input path, it's added to `unmapped_input_paths`.
 */
export const resolve_input_paths = async (
	input_paths: Array<InputPath>,
	root_dirs: Array<PathId>,
	extensions: Array<string>,
): Promise<ResolvedInputPaths> => {
	const resolved_input_paths: Array<ResolvedInputPath> = [];
	const possible_paths_by_input_path: Map<InputPath, Array<PossiblePath>> = new Map();
	const unmapped_input_paths: Array<InputPath> = [];
	for (const input_path of input_paths) {
		let found_file: [PathInfo, PossiblePath] | null = null;
		let found_dirs: Array<[PathInfo, PossiblePath]> | null = null;

		const possible_paths = await get_possible_paths(input_path, root_dirs, extensions);
		possible_paths_by_input_path.set(input_path, possible_paths);

		// Find the first existing file path or fallback to the first directory path.
		for (const possible_path of possible_paths) {
			if (!(await fs_exists(possible_path.id))) continue;

			const stats = await stat(possible_path.id);
			if (stats.isDirectory()) {
				found_dirs ??= [];
				found_dirs.push([{id: possible_path.id, is_directory: stats.isDirectory()}, possible_path]);
			} else {
				found_file = [{id: possible_path.id, is_directory: stats.isDirectory()}, possible_path];
				break;
			}
		}
		if (found_file) {
			resolved_input_paths.push({
				input_path,
				id: found_file[0].id,
				is_directory: found_file[0].is_directory,
				root_dir: found_file[1].root_dir,
			});
		} else if (found_dirs) {
			for (const found_dir of found_dirs) {
				resolved_input_paths.push({
					input_path,
					id: found_dir[0].id,
					is_directory: found_dir[0].is_directory,
					root_dir: found_dir[1].root_dir,
				});
			}
		} else {
			unmapped_input_paths.push(input_path);
		}
	}
	return {
		resolved_input_paths,
		possible_paths_by_input_path,
		unmapped_input_paths,
	};
};

export interface ResolvedInputFiles {
	resolved_input_files: Array<ResolvedInputFile>;
	resolved_input_files_by_root_dir: Map<PathId, Array<ResolvedInputFile>>;
	input_directories_with_no_files: Array<InputPath>;
}

/**
 * Finds all of the matching files for the given input paths.
 * De-dupes source ids.
 */
export const resolve_input_files = async (
	resolved_input_paths: Array<ResolvedInputPath>,
	search: (dir: string) => Promise<Array<ResolvedPath>> = fs_search,
): Promise<ResolvedInputFiles> => {
	const resolved_input_files: Array<ResolvedInputFile> = [];
	// Add all input paths initially, and remove each when resolved to a file.
	const existing_path_ids: Set<PathId> = new Set();

	let remaining = resolved_input_paths.slice();
	const handle_found = (input_path: InputPath, id: PathId) => {
		remaining = remaining.filter(
			(r) => !(r.id === id || r.input_path === input_path || r.input_path === id), // `r.input_path === id` may be unnecessary
		);
	};

	// TODO parallelize but would need to de-dupe and retain order
	for (const resolved_input_path of resolved_input_paths) {
		const {input_path, id, is_directory} = resolved_input_path;
		if (is_directory) {
			// Handle input paths that resolve to directories.

			const files = await search(id);
			if (!files.length) continue;
			const path_ids: Array<PathId> = [];
			for (const {path, is_directory} of files) {
				if (is_directory) continue;
				const path_id = join(id, path);
				if (!existing_path_ids.has(path_id)) {
					existing_path_ids.add(path_id);
					path_ids.push(path_id);
				}
				handle_found(input_path, path_id);
			}
			if (!path_ids.length) continue;
			const resolved_input_files_for_input_path: Array<ResolvedInputFile> = [];
			for (const path_id of path_ids) {
				const resolved_input_file: ResolvedInputFile = {
					id: path_id,
					input_path,
					resolved_input_path,
				};
				resolved_input_files.push(resolved_input_file);
				resolved_input_files_for_input_path.push(resolved_input_file);
			}
		} else {
			if (!existing_path_ids.has(id)) {
				// Handle input paths that resolve to files.
				existing_path_ids.add(id);
				const resolved_input_file: ResolvedInputFile = {id, input_path, resolved_input_path};
				resolved_input_files.push(resolved_input_file);
			}
			handle_found(input_path, id);
		}
	}
	return {
		resolved_input_files,
		resolved_input_files_by_root_dir: resolved_input_files.reduce((map, resolved_input_file) => {
			const {root_dir} = resolved_input_file.resolved_input_path;
			if (map.has(root_dir)) {
				map.get(root_dir)!.push(resolved_input_file);
			} else {
				map.set(root_dir, [resolved_input_file]);
			}
			return map;
		}, new Map<PathId, Array<ResolvedInputFile>>()),
		input_directories_with_no_files: remaining.map((r) => r.input_path),
	};
};
