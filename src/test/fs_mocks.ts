import { vi, type Mock } from 'vitest';

import type { DirEntry, FileInfo, FsDir } from '#lib/fs.ts';

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
