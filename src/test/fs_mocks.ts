import { vi, type Mock } from 'vitest';

import type { DirEntry, FileInfo, FsDir, FsErrorKind, FsStatOptions } from '#lib/fs.ts';

/**
 * A stand-in for the host module's `FsError`, which `is_fs_error` recognizes by its class name,
 * defined here since a test that mocks `#lib/fs.ts` builds the mock from this module.
 */
class FsError extends Error {
	override name = 'FsError';
	readonly kind: FsErrorKind;
	readonly path: string;
	constructor(kind: FsErrorKind, path: string) {
		super(`${kind}: ${path}`);
		this.kind = kind;
		this.path = path;
	}
}

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
 * The fs host module with `fs.project` a mock directory on the cwd and the rest real
 * (`FsError`, `is_fs_error`), for a test of a task, the composition root that imports it:
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
