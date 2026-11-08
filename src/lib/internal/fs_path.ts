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

import {
	fs,
	FsError,
	is_fs_error,
	type FsCopyOptions,
	type FsDir,
	type FsErrorKind,
	type FsRemoveOptions
} from '../fs.ts';
import { fs_search, type FsSearchOptions, type FsSearchResult } from '../fs_search.ts';

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
 * Whether the absolute, normalized `real` is `project` or one of its ancestors, `/` included,
 * the directories no removal or emptying may touch. A trailing `/` on either is ignored.
 * @param real - an absolute real path
 * @param project - the project's absolute real path, defaults to `fs.project`'s
 */
export const is_project_or_ancestor = (
	real: string,
	project: string = fs.project.path
): boolean => {
	const target = strip_trailing_slashes(real);
	const project_path = strip_trailing_slashes(project);
	if (target === '/' || target === project_path) return true;
	return project_path.startsWith(target + '/');
};

// the resolved target a refusal names, with the path as given when it differs
const to_refusal_subject = (path: string, target: string): string =>
	path === target ? target : `${target} (from ${path})`;

const strip_trailing_slashes = (path: string): string => path.replace(/(.)\/+$/, '$1');

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
 * Opens a directory named by a path from outside, resolved as `fs_open_path` resolves it:
 * beneath the project, `fs.project` and the directory's path beneath it,
 * else the directory as its own handle, so what's found beneath it reaches nothing above.
 * @param path - absolute, or relative to the cwd
 * @returns the handle and the directory's path beneath it; `null` when nothing is at a `path`
 * outside the project, while beneath it the result is `fs.project` and the path, missing or not,
 * and the op on it reports a missing directory
 * @throws `FsError` - `not_a_directory` when `path` is a file outside the project
 * (beneath the project, the op on the result throws it instead), otherwise as `fs_open_path` does
 */
export const fs_open_dir_path = async (path: string): Promise<FsOpenedPath | null> => {
	const real = await to_real_path(resolve(path));
	const relative = to_project_relative(real);
	if (relative !== null) return { dir: fs.project, path: relative };
	try {
		return { dir: await fs.open_dir(real), path: '' };
	} catch (error) {
		if (is_fs_error(error, 'not_found')) return null;
		throw error;
	}
};

/**
 * Searches a directory named by a path from outside, as `fs_search` does,
 * with ids that start with the path as named, resolved against the cwd.
 * The directory is opened as `fs_open_dir_path` opens it, so the search reaches nothing above it.
 * @param path - absolute, or relative to the cwd
 * @returns the found paths, or an empty array when nothing is at `path`
 * @throws `FsError` - when `path` isn't a directory, or a listing fails
 */
export const fs_search_path = async (
	path: string,
	options?: Omit<FsSearchOptions, 'path' | 'id_dir'>
): Promise<Array<FsSearchResult>> => {
	const opened = await fs_open_dir_path(path);
	if (opened === null) return [];
	return fs_search(opened.dir, { ...options, path: opened.path, id_dir: resolve(path) });
};

/**
 * Copies a file, or with `recursive` a directory tree, between paths from outside,
 * as `FsDir.copy` does, symlinks in a tree copied as links.
 * Both paths are resolved as `fs_open_path` resolves them, and the copy runs beneath
 * `fs.project` when both are in the project, else beneath their nearest common directory.
 * @param from - absolute, or relative to the cwd
 * @param to - absolute, or relative to the cwd
 * @throws `FsError` - as `fs_open_path` and `FsDir.copy` do
 */
export const fs_copy_path = async (
	from: string,
	to: string,
	options?: FsCopyOptions
): Promise<void> => {
	const [real_from, real_to] = await Promise.all([
		to_real_path(resolve(from)),
		to_real_path(resolve(to))
	]);
	const from_relative = to_project_relative(real_from);
	const to_relative = to_project_relative(real_to);
	if (from_relative !== null && to_relative !== null) {
		await fs.project.copy(from_relative, to_relative, options);
		return;
	}
	let common = dirname(real_from);
	while (common !== '/' && !real_to.startsWith(common + '/')) common = dirname(common);
	const dir = await fs.open_dir(common);
	const start = common === '/' ? 1 : common.length + 1;
	await dir.copy(real_from.slice(start), real_to.slice(start), options);
};

/**
 * Empties a directory named by a path from outside, its symlinks followed,
 * removing everything in it but never the directory itself, so a symlink to it is kept.
 * @param path - absolute, or relative to the cwd
 * @throws `FsError` - `io_error` when the directory is the project or one of its ancestors,
 * `not_a_directory` when it's a file, otherwise as `fs_open_dir_path` and `FsDir.remove` do
 */
export const fs_empty_dir_path = async (path: string): Promise<void> => {
	const opened = await fs_open_dir_path(path);
	if (opened === null) return;
	const target = opened.path ? join(opened.dir.path, opened.path) : opened.dir.path;
	if (is_project_or_ancestor(target)) {
		throw new FsError(
			'io_error',
			resolve(path),
			`io_error: ${to_refusal_subject(path, target)} is the project or one of its ancestors, which can't be emptied`
		);
	}
	const names: Array<string> = [];
	try {
		for await (const { name } of opened.dir.list(opened.path)) names.push(name);
	} catch (error) {
		// beneath the project, a missing directory is found missing by the listing
		if (is_fs_error(error, 'not_found')) return;
		throw error;
	}
	await Promise.all(
		names.map((name) =>
			opened.dir.remove(opened.path ? opened.path + '/' + name : name, { recursive: true })
		)
	);
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
	if (is_project_or_ancestor(target)) {
		throw new FsError(
			'io_error',
			resolved,
			`io_error: ${to_refusal_subject(path, target)} is the project or one of its ancestors, which can't be removed`
		);
	}
	await parent.dir.remove(path.endsWith('/') ? entry + '/' : entry, options);
};
