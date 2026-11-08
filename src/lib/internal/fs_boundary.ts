/**
 * The machinery of `FsPaths` (`fs_paths.ts`), Gro's boundary from paths that arrive
 * from outside to the fs host module's handles.
 *
 * The realpath resolution here reads symlinks through `node:fs` itself, the authority
 * the host module keeps out of a handle's reach, so it's internal: code gets the boundary
 * as an `FsPaths` built from the root `Fs`, never this resolution on its own.
 *
 * @module
 */

import { readlink, realpath } from 'node:fs/promises';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';

import type { Fs, FsCopyOptions, FsRemoveOptions } from '../fs.ts';
import { FsError, is_fs_error, type FsErrorKind } from '../fs_error.ts';
import { fs_search, type FsSearchOptions, type FsSearchResult } from '../fs_search.ts';
import type { FsOpenedPath, FsPaths } from '../fs_paths.ts';
import { to_checked_cwd } from './cwd.ts';

// TODO: the realpath and readlink this boundary reads from `node:fs` become an op on the `Fs`
// passed in, a root op beside `open_dir`, reached through it and never imported, so building
// an `FsPaths` grants nothing beyond what that `Fs` holds

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
	let real: Uint8Array | null = null;
	try {
		// as bytes, so a real path that isn't UTF-8 is refused rather than decoded lossily
		real = await realpath(path, { encoding: 'buffer' });
	} catch (error) {
		const code = code_of(error);
		if (code !== 'ENOENT' && code !== 'ENOTDIR') throw to_resolve_error(error, path);
	}
	if (real !== null) return to_utf8_path(path, real);
	const parent = dirname(path);
	if (parent === path) return path;
	const candidate = join(await to_real_path(parent, links), basename(path));
	// the parent is real, so the candidate is missing, or a symlink whose target is
	let target_bytes: Uint8Array;
	try {
		target_bytes = await readlink(candidate, { encoding: 'buffer' });
	} catch (error) {
		// missing, not a symlink, or beneath a file, which the op on the result reports
		const code = code_of(error);
		if (code === 'ENOENT' || code === 'EINVAL' || code === 'ENOTDIR') return candidate;
		throw to_resolve_error(error, candidate);
	}
	if (++links.count > MAX_SYMLINKS) throw to_loop_error(path);
	return to_real_path(
		resolve(
			dirname(candidate),
			to_utf8_path(candidate, target_bytes, 'is a symlink whose target is a path')
		),
		links
	);
};

// fatal, so a target that isn't UTF-8 throws rather than decoding to U+FFFD, BOM kept
const utf8_decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

/**
 * A symlink's target or a real path read as bytes, refused with `io_error` when it isn't
 * UTF-8, since decoded lossily it would name a different path than the kernel follows.
 */
const to_utf8_path = (path: string, bytes: Uint8Array, what = 'resolves to a path'): string => {
	try {
		return utf8_decoder.decode(bytes);
	} catch {
		throw new FsError('io_error', path, `io_error: ${path} ${what} that isn't UTF-8`);
	}
};

// read once, when the module loads; an unset, empty, or relative `HOME` protects nothing
const HOME = process.env.HOME;

// the process's working directory, the project Gro runs in, protected whatever project
// an `FsPaths` is over, read once when the module loads, as `fs.project` is
const CWD = to_checked_cwd();

const protected_dirs_cache: Map<string, Promise<Array<string>>> = new Map();

/**
 * The directories no removal or emptying may touch, beside `/` and their ancestors:
 * the `FsPaths`' project, the process's working directory when the module loaded,
 * and `$HOME` as given and as its real path. Resolved once per project.
 * @param project_path - the project directory's absolute real path
 */
export const to_protected_dirs = (project_path: string): Promise<Array<string>> => {
	let dirs = protected_dirs_cache.get(project_path);
	if (dirs === undefined) {
		dirs = (async () => {
			const found = project_path === CWD ? [CWD] : [CWD, project_path];
			if (HOME && isAbsolute(HOME)) {
				const home = resolve(HOME);
				found.push(home);
				const home_real = await realpath(home).catch(() => null);
				if (home_real !== null && home_real !== home) found.push(home_real);
			}
			return found;
		})();
		protected_dirs_cache.set(project_path, dirs);
	}
	return dirs;
};

/**
 * Whether the absolute, normalized `real` is `/`, one of `dirs`, or an ancestor of one,
 * the directories no removal or emptying may touch. A trailing `/` on any is ignored.
 * @param real - an absolute real path
 * @param dirs - absolute paths, like `to_protected_dirs`'s: the project, the cwd, and `$HOME`
 */
export const is_protected_dir = (real: string, dirs: ReadonlyArray<string>): boolean => {
	const target = strip_trailing_slashes(real);
	if (target === '/') return true;
	for (const dir of dirs) {
		const dir_path = strip_trailing_slashes(dir);
		if (target === dir_path || dir_path.startsWith(target + '/')) return true;
	}
	return false;
};

/**
 * How a refusal names the protected directories.
 */
export const PROTECTED_DIRS_TEXT =
	'the project, the working directory, the home directory, or one of their ancestors';

// the resolved target a refusal names, with the path as given when it differs
const to_refusal_subject = (path: string, target: string): string =>
	path === target ? target : `${target} (from ${path})`;

const strip_trailing_slashes = (path: string): string => path.replace(/(.)\/+$/, '$1');

/**
 * Builds the boundary's ops over `fs`, each path resolved to absolute by `to_absolute`,
 * lexically, before any symlink is followed. `FsPaths` documents each op.
 * @param fs - the fs root, whose `project` a path beneath the project is relative to
 * and whose `open_dir` opens every other directory
 * @param to_absolute - resolves a path as given to an absolute, normalized one
 */
export const create_fs_boundary = (fs: Fs, to_absolute: (path: string) => string): FsPaths => {
	const { project } = fs;

	const to_project_relative = (real: string): string | null => {
		const project_path = project.path;
		if (real === project_path) return '';
		const prefix = project_path === '/' ? '/' : project_path + '/';
		return real.startsWith(prefix) ? real.slice(prefix.length) : null;
	};

	const open_path = async (path: string): Promise<FsOpenedPath> => {
		const suffix = path.endsWith('/') ? '/' : '';
		const real = await to_real_path(to_absolute(path));
		const relative = to_project_relative(real);
		if (relative !== null) {
			return { dir: project, path: relative && relative + suffix };
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

	const open_dir_path = async (path: string): Promise<FsOpenedPath | null> => {
		const real = await to_real_path(to_absolute(path));
		const relative = to_project_relative(real);
		if (relative !== null) return { dir: project, path: relative };
		try {
			return { dir: await fs.open_dir(real), path: '' };
		} catch (error) {
			if (is_fs_error(error, 'not_found')) return null;
			throw error;
		}
	};

	const search_path = async (
		path: string,
		options?: Omit<FsSearchOptions, 'path' | 'id_dir'>
	): Promise<Array<FsSearchResult>> => {
		const opened = await open_dir_path(path);
		if (opened === null) return [];
		return fs_search(opened.dir, { ...options, path: opened.path, id_dir: to_absolute(path) });
	};

	const copy_path = async (from: string, to: string, options?: FsCopyOptions): Promise<void> => {
		const [real_from, real_to] = await Promise.all([
			to_real_path(to_absolute(from)),
			to_real_path(to_absolute(to))
		]);
		const from_relative = to_project_relative(real_from);
		const to_relative = to_project_relative(real_to);
		if (from_relative !== null && to_relative !== null) {
			await project.copy(from_relative, to_relative, options);
			return;
		}
		let common = dirname(real_from);
		while (common !== '/' && !real_to.startsWith(common + '/')) common = dirname(common);
		const dir = await fs.open_dir(common);
		const start = common === '/' ? 1 : common.length + 1;
		await dir.copy(real_from.slice(start), real_to.slice(start), options);
	};

	const empty_dir_path = async (path: string): Promise<void> => {
		const opened = await open_dir_path(path);
		if (opened === null) return;
		const target = opened.path ? join(opened.dir.path, opened.path) : opened.dir.path;
		if (is_protected_dir(target, await to_protected_dirs(project.path))) {
			throw new FsError(
				'io_error',
				to_absolute(path),
				`io_error: ${to_refusal_subject(path, target)} is ${PROTECTED_DIRS_TEXT}, which can't be emptied`
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

	const remove_path = async (path: string, options?: FsRemoveOptions): Promise<void> => {
		const resolved = to_absolute(path);
		const name = basename(resolved);
		if (!name) throw new FsError('io_error', resolved, `io_error: the root can't be removed`);
		const parent = await open_path(dirname(resolved));
		const entry = parent.path ? parent.path + '/' + name : name;
		const target = join(parent.dir.path, entry);
		if (is_protected_dir(target, await to_protected_dirs(project.path))) {
			throw new FsError(
				'io_error',
				resolved,
				`io_error: ${to_refusal_subject(path, target)} is ${PROTECTED_DIRS_TEXT}, which can't be removed`
			);
		}
		await parent.dir.remove(path.endsWith('/') ? entry + '/' : entry, options);
	};

	return Object.freeze({
		project,
		open_path,
		open_dir_path,
		search_path,
		copy_path,
		empty_dir_path,
		remove_path
	});
};
