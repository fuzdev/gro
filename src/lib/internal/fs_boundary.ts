/**
 * The machinery of `FsPaths` (`fs_paths.ts`), Gro's boundary from paths that arrive
 * from outside to the fs host module's handles.
 *
 * It resolves a path's symlinks through the `Fs` it's given, `fs.real_path` and a handle's
 * `read_link`, so building an `FsPaths` grants nothing beyond what that `Fs` holds.
 * It's internal: code gets the boundary as an `FsPaths` built from the root `Fs`.
 *
 * @module
 */

import { basename, dirname, isAbsolute, join, resolve } from 'node:path';

import type { Fs, FsCopyOptions, FsDir, FsRemoveOptions } from '../fs.ts';
import { FsError, is_fs_error } from '../fs_error.ts';
import { fs_search, type FsSearchOptions, type FsSearchResult } from '../fs_search.ts';
import type { FsOpenedPath, FsPaths } from '../fs_paths.ts';
import { to_checked_cwd } from './cwd.ts';
import { create_record } from './records.ts';
import { env } from '../env.ts';

// the most symlinks one resolution follows, Linux's own limit
const MAX_SYMLINKS = 40;

const to_loop_error = (path: string): FsError =>
	new FsError('io_error', path, `io_error: ${path} reaches a symlink loop (ELOOP)`);

// missing, or beneath a file, which the op on the resolved path reports
const is_missing_error = (error: unknown): boolean =>
	is_fs_error(error, 'not_found') || is_fs_error(error, 'not_a_directory');

/**
 * The target of the symlink `name` in the directory at the real path `parent`, read through a
 * handle on `parent`, or `null` when there's none to follow: the entry or `parent` missing,
 * or `parent` a file. `fs.real_path` just found the path missing, so an entry that's there
 * is a dangling symlink, or one that replaced it since, which fails as `io_error`.
 * @throws `FsError` - any other failure, like a target that isn't UTF-8
 */
const read_link_target = async (fs: Fs, parent: string, name: string): Promise<string | null> => {
	try {
		return await (await fs.open_dir(parent)).read_link(name);
	} catch (error) {
		if (is_missing_error(error)) return null;
		throw error;
	}
};

/**
 * The real path of the absolute, normalized `path`, every symlink followed,
 * a dangling one to its missing target. A missing part is appended to its nearest
 * existing ancestor's real path. A symlink's target is resolved against the link's
 * directory lexically, its `..` segments before any symlink in it is followed.
 * @throws `TypeError` - for a path holding a NUL byte or a lone surrogate, as `fs.real_path` does
 */
const to_real_path = async (fs: Fs, path: string, links = { count: 0 }): Promise<string> => {
	try {
		return await fs.real_path(path);
	} catch (error) {
		if (!is_missing_error(error)) throw error;
	}
	const parent = dirname(path);
	if (parent === path) return path;
	const parent_real = await to_real_path(fs, parent, links);
	const name = basename(path);
	// the parent is real, so the entry is missing, or a symlink whose target is
	const target = await read_link_target(fs, parent_real, name);
	if (target === null) return join(parent_real, name);
	if (++links.count > MAX_SYMLINKS) throw to_loop_error(path);
	return to_real_path(fs, resolve(parent_real, target), links);
};

// read once, when the module loads; an unset, empty, or relative `HOME` protects nothing
const HOME = env.get('HOME');

// the process's working directory, the project Gro runs in, protected whatever project
// an `FsPaths` is over, read once when the module loads, as `fs.project` is
const CWD = to_checked_cwd();

const protected_dirs_cache: Map<string, Promise<Array<string>>> = new Map();

/**
 * The directories no removal or emptying may touch, beside `/` and their ancestors:
 * the `FsPaths`' project, the process's working directory when the module loaded,
 * and `$HOME` as given and as its real path. Resolved once per project,
 * with the first `fs` given.
 * @param fs - the fs root, which resolves `$HOME`'s real path
 * @param project_path - the project directory's absolute real path
 */
export const to_protected_dirs = (fs: Fs, project_path: string): Promise<Array<string>> => {
	let dirs = protected_dirs_cache.get(project_path);
	if (dirs === undefined) {
		dirs = (async () => {
			const found = project_path === CWD ? [CWD] : [CWD, project_path];
			if (HOME && isAbsolute(HOME)) {
				const home = resolve(HOME);
				found.push(home);
				const home_real = await fs.real_path(home).catch(() => null);
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

// null-prototype, as the fs handles are, so awaiting one adopts no polluted `Object.prototype.then`,
// which an opened path would hand its handle
const to_opened_path = (dir: FsDir, path: string): FsOpenedPath =>
	create_record<FsOpenedPath>({ dir, path });

/**
 * Builds the boundary's ops over `fs`, each path resolved to absolute by `to_absolute`,
 * lexically, before any symlink is followed. `FsPaths` documents each op.
 * @param fs - the fs root, whose `project` a path beneath the project is relative to,
 * whose `open_dir` opens every other directory, and whose `real_path` resolves symlinks
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
		const real = await to_real_path(fs, to_absolute(path));
		const relative = to_project_relative(real);
		if (relative !== null) {
			return to_opened_path(project, relative && relative + suffix);
		}
		if (real === '/') return to_opened_path(await fs.open_dir('/'), '');
		let dir_path = dirname(real);
		for (;;) {
			const rest = real.slice(dir_path === '/' ? 1 : dir_path.length + 1);
			try {
				return to_opened_path(await fs.open_dir(dir_path), rest + suffix);
			} catch (error) {
				if (!is_fs_error(error, 'not_found') || dir_path === '/') throw error;
				dir_path = dirname(dir_path);
			}
		}
	};

	const open_dir_path = async (path: string): Promise<FsOpenedPath | null> => {
		const real = await to_real_path(fs, to_absolute(path));
		const relative = to_project_relative(real);
		if (relative !== null) return to_opened_path(project, relative);
		try {
			return to_opened_path(await fs.open_dir(real), '');
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
			to_real_path(fs, to_absolute(from)),
			to_real_path(fs, to_absolute(to))
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
		if (is_protected_dir(target, await to_protected_dirs(fs, project.path))) {
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
		if (is_protected_dir(target, await to_protected_dirs(fs, project.path))) {
			throw new FsError(
				'io_error',
				resolved,
				`io_error: ${to_refusal_subject(path, target)} is ${PROTECTED_DIRS_TEXT}, which can't be removed`
			);
		}
		await parent.dir.remove(path.endsWith('/') ? entry + '/' : entry, options);
	};

	return Object.freeze(
		create_record<FsPaths>({
			project,
			open_path,
			open_dir_path,
			search_path,
			copy_path,
			empty_dir_path,
			remove_path
		})
	);
};
