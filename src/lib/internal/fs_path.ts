/**
 * Gro's boundary from paths that arrive from outside to the fs host module's handles.
 *
 * A path from a CLI arg, a config, or one of Gro's well-known paths names a file by
 * absolute or cwd-relative path, while a handle takes only relative paths beneath it.
 * `fs_open_path` converts one into the other where the path enters code that reaches
 * the filesystem. It follows symlinks the way `fs.open_dir` does, the same inside and
 * outside the project, so the confinement of a handle applies only to the paths code
 * resolves beneath a handle it already holds. It's internal rather than part of the
 * host module because it opens any path, the authority the module's API keeps out of a
 * handle's reach: code that holds a handle gets narrower handles from it, not wider ones
 * by path.
 *
 * @module
 */

import { readlink, realpath } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import type { ResolvedPath } from '@fuzdev/fuz_util/path.ts';

import {
	fs,
	FsError,
	is_fs_error,
	type FsDir,
	type FsErrorKind,
	type FsRemoveOptions
} from '../fs.ts';
import { fs_search, type FsSearchOptions } from '../fs_search.ts';

// TODO: on the runtime, the realpath this boundary needs is a host op beside `fs.open_dir`

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

// the most symlinks one resolution follows, Linux's own limit
const MAX_SYMLINKS = 40;

const code_of = (error: unknown): unknown =>
	typeof error === 'object' && error !== null ? (error as { code?: unknown }).code : undefined;

const to_kind = (code: unknown): FsErrorKind =>
	code === 'EACCES' || code === 'EPERM' ? 'permission_denied' : 'io_error';

const to_loop_error = (path: string, cause?: unknown): FsError =>
	new FsError('io_error', path, `io_error: ${path} reaches a symlink loop (ELOOP)`, { cause });

const to_resolve_error = (error: unknown, path: string): FsError => {
	const code = code_of(error);
	if (code === 'ELOOP') return to_loop_error(path, error);
	const kind = to_kind(code);
	const detail = error instanceof Error ? error.message : String(error);
	return new FsError(kind, path, `${kind}: ${path} (${detail})`, { cause: error });
};

/**
 * The real path of the absolute, normalized `path`, every symlink followed,
 * a dangling one to its missing target. A missing part is appended to its nearest
 * existing ancestor's real path. A symlink's target is resolved against the link's
 * directory lexically, its `..` segments before any symlink in it is followed.
 */
const to_real_path = async (path: string, links = { count: 0 }): Promise<string> => {
	try {
		return await realpath(path);
	} catch (error) {
		const code = code_of(error);
		if (code !== 'ENOENT' && code !== 'ENOTDIR') throw to_resolve_error(error, path);
	}
	const parent = dirname(path);
	if (parent === path) return path;
	const candidate = join(await to_real_path(parent, links), basename(path));
	// the parent is real, so the candidate is missing, or a symlink whose target is
	let target: string;
	try {
		target = await readlink(candidate);
	} catch (error) {
		// missing, not a symlink, or beneath a file, which the op on the result reports
		const code = code_of(error);
		if (code === 'ENOENT' || code === 'EINVAL' || code === 'ENOTDIR') return candidate;
		throw to_resolve_error(error, candidate);
	}
	if (++links.count > MAX_SYMLINKS) throw to_loop_error(path);
	return to_real_path(resolve(dirname(candidate), target), links);
};

const to_project_relative = (real: string): string | null => {
	const project = fs.project.path;
	if (real === project) return '';
	const prefix = project === '/' ? '/' : project + '/';
	return real.startsWith(prefix) ? real.slice(prefix.length) : null;
};

/**
 * Converts a path from outside into a handle and the relative path beneath it.
 *
 * The path is resolved against the cwd lexically, its `..` segments before any symlink
 * is followed, as Node's `path.resolve` does. Then it's resolved to its real path,
 * every symlink in it followed, a dangling final one to its missing target, and a
 * missing part appended to its nearest existing ancestor's real path. A real path at or
 * beneath the project is relative to `fs.project`; any other opens its parent, or the
 * parent's nearest existing ancestor, with `fs.open_dir`. So the op on the result
 * follows no symlink but ones created since, and a symlink out of the project, at the
 * end of the path or before it, reaches what it points at, as it did before handles.
 *
 * @param path - absolute, or relative to the cwd; a trailing `/` is kept
 * @throws `FsError` - `not_a_directory` when, outside the project, an ancestor of the path
 * is a file (beneath the project, the op on the result throws it instead);
 * `io_error` for a symlink loop (ELOOP); `permission_denied` or `io_error` when reading
 * a directory or symlink on the way fails otherwise
 */
export const fs_open_path = async (path: string): Promise<FsOpenedPath> => {
	const suffix = path.endsWith('/') ? '/' : '';
	const real = await to_real_path(resolve(path));
	const relative = to_project_relative(real);
	if (relative !== null) {
		return { dir: fs.project, path: relative && relative + suffix };
	}
	if (real === '/') return { dir: await fs.open_dir('/'), path: '' };
	let dir_path = dirname(real);
	for (;;) {
		const rest = real.slice(dir_path === '/' ? 1 : dir_path.length + 1);
		try {
			return { dir: await fs.open_dir(dir_path), path: rest + suffix };
		} catch (error) {
			if (!is_fs_error(error, 'not_found') || dir_path === '/') throw error;
			dir_path = dirname(dir_path);
		}
	}
};

/**
 * Searches a directory named by a path from outside, as `fs_search` does,
 * with ids that start with the path as named, resolved against the cwd.
 * The directory is opened as `fs_open_path` resolves it: beneath the project through
 * `fs.project`, else as its own handle, so the search reaches nothing above it.
 * @param path - absolute, or relative to the cwd
 * @returns the found paths, or an empty array when nothing is at `path`
 * @throws `FsError` - when `path` isn't a directory, or a listing fails
 */
export const fs_search_path = async (
	path: string,
	options?: Omit<FsSearchOptions, 'path' | 'id_dir'>
): Promise<Array<ResolvedPath>> => {
	const id_dir = resolve(path);
	const real = await to_real_path(id_dir);
	const relative = to_project_relative(real);
	if (relative !== null) {
		return fs_search(fs.project, { ...options, path: relative, id_dir });
	}
	let dir: FsDir;
	try {
		dir = await fs.open_dir(real);
	} catch (error) {
		if (is_fs_error(error, 'not_found')) return [];
		throw error;
	}
	return fs_search(dir, { ...options, id_dir });
};

/**
 * Removes the entry at a path from outside, never what a final symlink points at:
 * the parent is resolved as `fs_open_path` resolves it, symlinks followed,
 * and the final entry is removed as `FsDir.remove` does, a symlink removed itself.
 * A trailing `/` removes only a directory, as in `FsDir.remove`.
 * @param path - absolute, or relative to the cwd
 * @throws `FsError` - `io_error` for the project or one of its ancestors,
 * otherwise as `fs_open_path` and `FsDir.remove` do
 */
export const fs_remove_path = async (path: string, options?: FsRemoveOptions): Promise<void> => {
	const resolved = resolve(path);
	const name = basename(resolved);
	if (!name) throw new FsError('io_error', resolved, `io_error: the root can't be removed`);
	const parent = await fs_open_path(dirname(resolved));
	const entry = parent.path ? parent.path + '/' + name : name;
	const target = join(parent.dir.path, entry);
	const project = fs.project.path;
	if (project === target || project.startsWith(target + '/')) {
		throw new FsError(
			'io_error',
			resolved,
			`io_error: ${resolved} is the project or one of its ancestors, which can't be removed`
		);
	}
	await parent.dir.remove(path.endsWith('/') ? entry + '/' : entry, options);
};
