import { vi, type Mock } from 'vitest';
import { relative, resolve } from 'node:path';

import type { DirEntry, FileInfo, FsDir, FsStatOptions } from '#lib/fs.ts';
import { FsError } from '#lib/fs_error.ts';
import type { FsOpenedPath, FsPaths } from '#lib/fs_paths.ts';

/**
 * An `FsDir` whose ops are all `vi.fn` mocks.
 */
export type MockFsDir = {
	[K in keyof FsDir]: FsDir[K] extends (...args: infer A) => infer R
		? Mock<(...args: A) => R>
		: FsDir[K];
};

const OPS = [
	'dir',
	'readonly',
	'stat',
	'exists',
	'read',
	'read_text',
	'read_if_exists',
	'read_text_if_exists',
	'list',
	'stat_sync',
	'exists_sync',
	'read_text_sync',
	'write',
	'mkdir',
	'remove',
	'rename',
	'copy'
] as const;

/**
 * Creates a mock `FsDir` whose every op throws until a test sets it,
 * so code reaching an op the test didn't expect fails loudly.
 * `vi.resetAllMocks` restores the throwing stubs.
 */
export const create_mock_fs_dir = (path: string): MockFsDir => {
	const dir: Record<string, unknown> = { path };
	for (const op of OPS) {
		dir[op] = vi.fn(() => {
			throw Error(`mock FsDir.${op} not set`);
		});
	}
	return dir as MockFsDir;
};

/**
 * The fs host module with `fs.project` a mock directory on the cwd,
 * for a test of a task, the composition root that imports it:
 * `vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module())`.
 * `open_dir` and `temp_dir` throw, so a path outside the project fails loudly.
 */
export const create_mock_fs_module = async (): Promise<typeof import('#lib/fs.ts')> => ({
	...(await vi.importActual<typeof import('#lib/fs.ts')>('#lib/fs.ts')),
	fs: {
		project: create_mock_fs_dir(process.cwd()) as unknown as FsDir,
		open_dir: vi.fn(async (path: string) => {
			throw Error(`the mock fs opens no directory outside the project: ${path}`);
		}),
		temp_dir: vi.fn(async () => {
			throw Error('the mock fs makes no temp directory');
		})
	}
});

/**
 * The mocked `fs.project`, in a test that mocks `#lib/fs.ts` with `create_mock_fs_module`.
 */
export const get_mock_project = async (): Promise<MockFsDir> =>
	(await import('#lib/fs.ts')).fs.project as unknown as MockFsDir;

/**
 * A listing for a mocked `list`, every entry a file unless named in `directories`.
 */
export const to_mock_listing = (
	names: Array<string>,
	directories: Array<string> = []
): AsyncIterable<DirEntry> =>
	(async function* () {
		for (const name of names) {
			yield { name, kind: directories.includes(name) ? 'directory' : 'file' };
		}
	})();

/**
 * A file's info for a mocked `stat`.
 */
export const to_mock_file_info = (size = 1024): FileInfo => ({
	kind: 'file',
	size,
	mtime_ms: 1729512000000,
	mode: 0o644
});

/**
 * A file in a `mock_tree`, its text and info, defaulting to `'content'` and `to_mock_file_info`,
 * or with `target` a symlink to that path in the tree, absent for a dangling one.
 */
export interface MockTreeFile {
	content?: string;
	size?: number;
	target?: string;
}

/**
 * Sets a mock directory's read ops to serve a virtual tree keyed by relative path,
 * each directory implied by the paths beneath it, or named with a trailing `/` when empty.
 * As in the real module, `stat` follows a final symlink unless `follow` is `false`,
 * a read follows it, `list` reports it as a symlink, `list` on a file throws `not_a_directory`,
 * and reading a directory throws `is_a_directory`. Unlike the real module, a path through
 * a symlinked directory (`link/x`) isn't resolved, and a path through a file is missing.
 */
export const mock_tree = (dir: MockFsDir, tree: Record<string, string | MockTreeFile>): void => {
	const files: Map<string, { content: string; size: number; target: string | null }> = new Map();
	const dirs: Set<string> = new Set(['']);
	for (const [key, value] of Object.entries(tree)) {
		const file = typeof value === 'string' ? { content: value } : value;
		const path = key.endsWith('/') ? key.slice(0, -1) : key;
		const parts = path.split('/');
		for (let i = 1; i < parts.length; i++) dirs.add(parts.slice(0, i).join('/'));
		if (key.endsWith('/')) {
			dirs.add(path);
		} else {
			const content = file.content ?? 'content';
			files.set(path, {
				content,
				size: file.size ?? content.length,
				target: file.target === undefined ? null : file.target.replace(/\/+$/, '')
			});
		}
	}
	const normalize = (path: string): string => path.replace(/\/+$/, '');
	// a final symlink followed to what it points at, `null` when dangling
	const resolve_link = (path: string, follow: boolean): string | null => {
		let p = normalize(path);
		if (!follow) return p;
		for (let i = 0; i < 40; i++) {
			const target = files.get(p)?.target;
			if (target === null || target === undefined) return p;
			if (!dirs.has(target) && !files.has(target)) return null;
			p = target;
		}
		return p;
	};
	const stat = (path: string, options?: FsStatOptions): FileInfo | null => {
		const p = resolve_link(path, options?.follow ?? true);
		if (p === null) return null;
		if (dirs.has(p)) return { ...to_mock_file_info(), kind: 'directory' };
		const file = files.get(p);
		if (!file) return null;
		return { ...to_mock_file_info(file.size), kind: file.target === null ? 'file' : 'symlink' };
	};
	const read_text = (path: string): string | null => {
		const p = resolve_link(path, true);
		if (p === null) return null;
		if (dirs.has(p)) throw new FsError('is_a_directory', path);
		return files.get(p)?.content ?? null;
	};
	const read_or_throw = (path: string): string => {
		const text = read_text(path);
		if (text === null) throw new FsError('not_found', path);
		return text;
	};
	dir.stat.mockImplementation(async (path, options) => stat(path, options));
	dir.stat_sync.mockImplementation((path, options) => stat(path, options));
	dir.exists.mockImplementation(async (path) => stat(path) !== null);
	dir.exists_sync.mockImplementation((path) => stat(path) !== null);
	dir.read_text_if_exists.mockImplementation(async (path) => read_text(path));
	dir.read_if_exists.mockImplementation(async (path) => {
		const text = read_text(path);
		return text === null ? null : new TextEncoder().encode(text);
	});
	dir.read_text.mockImplementation(async (path) => read_or_throw(path));
	dir.read_text_sync.mockImplementation((path) => read_or_throw(path));
	dir.read.mockImplementation(async (path) => new TextEncoder().encode(read_or_throw(path)));
	dir.list.mockImplementation((path = '') =>
		(async function* () {
			yield* list(path);
		})()
	);
	const list = (path: string): Array<DirEntry> => {
		const base = resolve_link(path, true);
		if (base === null || (!dirs.has(base) && !files.has(base))) {
			throw new FsError('not_found', path);
		}
		if (!dirs.has(base)) throw new FsError('not_a_directory', path);
		const prefix = base ? base + '/' : '';
		const entries: Array<DirEntry> = [];
		for (const d of dirs) {
			if (d !== base && d.startsWith(prefix) && !d.slice(prefix.length).includes('/')) {
				entries.push({ name: d.slice(prefix.length), kind: 'directory' });
			}
		}
		for (const [p, file] of files) {
			if (p.startsWith(prefix) && !p.slice(prefix.length).includes('/')) {
				entries.push({
					name: p.slice(prefix.length),
					kind: file.target === null ? 'file' : 'symlink'
				});
			}
		}
		return entries;
	};
};

/**
 * An `FsPaths` whose ops are all `vi.fn` mocks, over a mock project.
 */
export type MockFsPaths = FsPaths & {
	[K in Exclude<keyof FsPaths, 'project'>]: FsPaths[K] extends (...args: infer A) => infer R
		? Mock<(...args: A) => R>
		: FsPaths[K];
} & { readonly project: FsDir & MockFsDir };

const FS_PATHS_OPS = [
	'open_path',
	'open_dir_path',
	'search_path',
	'copy_path',
	'empty_dir_path',
	'remove_path'
] as const;

/**
 * Creates a mock `FsPaths` over `project` whose every op is a `vi.fn` of its override,
 * or else throws, so code reaching an op the test didn't expect fails loudly.
 * `vi.resetAllMocks` restores each op's override or throwing stub.
 * @param project - the mock project directory, defaults to one at `/project`
 * @param overrides - implementations for some ops
 */
export const create_mock_fs_paths = (
	project: MockFsDir = create_mock_fs_dir('/project'),
	overrides: Partial<Omit<FsPaths, 'project'>> = {}
): MockFsPaths => {
	const fs_paths: Record<string, unknown> = { project };
	for (const op of FS_PATHS_OPS) {
		fs_paths[op] = vi.fn(
			overrides[op] ??
				(() => {
					throw Error(`mock FsPaths.${op} not set`);
				})
		);
	}
	return fs_paths as unknown as MockFsPaths;
};

/**
 * Creates a mock `FsPaths` whose ops resolve every path beneath its project,
 * lexically, following no symlink, a relative path against the project's `path`,
 * each op then run on the mock project as the real boundary runs it on `fs.project`.
 * A path outside the project throws, since the mock opens no other directory,
 * and so does `search_path`. `vi.resetAllMocks` keeps the ops resolving.
 * @param project - the mock project directory, defaults to one at `/project`
 */
export const create_mock_project_fs_paths = (
	project: MockFsDir = create_mock_fs_dir('/project')
): MockFsPaths => {
	const to_relative = (path: string): string => {
		const r = relative(project.path, resolve(project.path, path));
		if (r === '..' || r.startsWith('../')) {
			throw Error(`the mock FsPaths opens no path outside the project: ${path}`);
		}
		return r && path.endsWith('/') ? r + '/' : r;
	};
	const dir = project as unknown as FsDir;
	const open = (path: string): FsOpenedPath => ({ dir, path: to_relative(path) });
	const join_beneath = (path: string, name: string): string => (path ? path + '/' + name : name);
	return create_mock_fs_paths(project, {
		open_path: async (path) => open(path),
		open_dir_path: async (path) => open(path),
		copy_path: async (from, to, options) =>
			project.copy(to_relative(from), to_relative(to), options),
		remove_path: async (path, options) => project.remove(to_relative(path), options),
		empty_dir_path: async (path) => {
			const dir_path = to_relative(path);
			const names: Array<string> = [];
			for await (const { name } of project.list(dir_path)) names.push(name);
			await Promise.all(
				names.map((name) => project.remove(join_beneath(dir_path, name), { recursive: true }))
			);
		}
	});
};
