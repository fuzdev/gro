import {describe, test, expect, vi, beforeEach} from 'vitest';

import {discover_build_output_dirs, collect_build_outputs} from '../lib/build_cache.ts';

import {mock_file_stats, mock_file_entry, mock_dir_entry} from './build_cache_test_helpers.ts';

// Mock dependencies - discover_build_output_dirs and collect_build_outputs now use async fs functions
vi.mock('node:fs/promises', () => ({
	readdir: vi.fn(),
	stat: vi.fn(),
	readFile: vi.fn(),
}));

// Mock fs_exists from fuz_util
vi.mock('@fuzdev/fuz_util/fs.js', () => ({
	fs_exists: vi.fn(),
}));

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn(),
}));

describe('discover_build_output_dirs', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns all existing build directories', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));

		// Mock all directories exist
		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readdir).mockResolvedValue([
			'dist_server',
			'dist_worker',
			'node_modules',
			'src',
		] as any);
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);

		const result = await discover_build_output_dirs();

		expect(result).toEqual(['build', 'dist', 'dist_server', 'dist_worker']);
	});

	test('returns only directories that exist', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));

		// Only dist and dist_server exist
		vi.mocked(fs_exists).mockImplementation((path: any) => {
			return Promise.resolve(path === 'dist');
		});
		vi.mocked(readdir).mockResolvedValue(['dist_server', 'other'] as any);
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);

		const result = await discover_build_output_dirs();

		expect(result).toEqual(['dist', 'dist_server']);
	});

	test('returns empty array when no build directories exist', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir} = vi.mocked(await import('node:fs/promises'));

		vi.mocked(fs_exists).mockResolvedValue(false);
		vi.mocked(readdir).mockResolvedValue([] as any);

		const result = await discover_build_output_dirs();

		expect(result).toEqual([]);
	});

	test('filters out non-directory dist_ entries', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));

		vi.mocked(fs_exists).mockImplementation((path: any) => Promise.resolve(path === 'build'));
		vi.mocked(readdir).mockResolvedValue([
			'dist_server',
			'dist_readme.md', // file, not directory
		] as any);
		vi.mocked(stat).mockImplementation((path: any) => {
			if (String(path) === 'dist_server') {
				return Promise.resolve({isDirectory: () => true} as any);
			}
			return Promise.resolve({isDirectory: () => false} as any);
		});

		const result = await discover_build_output_dirs();

		expect(result).toEqual(['build', 'dist_server']);
		expect(result).not.toContain('dist_readme.md');
	});

	test('handles permission errors on directory listing', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));

		vi.mocked(fs_exists).mockResolvedValue(true);

		// readdir throws permission error
		vi.mocked(readdir).mockRejectedValue(new Error('EACCES: permission denied'));
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);

		// Should throw - permission errors are fatal
		await expect(discover_build_output_dirs()).rejects.toThrow('EACCES: permission denied');
	});

	test('handles files named with dist_ prefix', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readdir).mockResolvedValue([
			'dist_config.json', // file, not directory
			'dist_server', // directory
		] as any);

		// Mock stat to differentiate files vs directories
		vi.mocked(stat).mockImplementation((path: any) => {
			if (String(path) === 'dist_server') {
				return Promise.resolve({isDirectory: () => true} as any);
			}
			return Promise.resolve({isDirectory: () => false} as any);
		});

		const result = await discover_build_output_dirs();

		// Should include directory but not file
		expect(result).toContain('dist_server');
		expect(result).not.toContain('dist_config.json');
	});

	test('skips dist_ entry when stat fails during iteration', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));

		vi.mocked(fs_exists).mockResolvedValue(false);
		vi.mocked(readdir).mockResolvedValue(['dist_vanished', 'dist_ok'] as any);
		vi.mocked(stat).mockImplementation((path: any) => {
			if (String(path) === 'dist_vanished') {
				return Promise.reject(new Error('ENOENT: no such file or directory'));
			}
			return Promise.resolve({isDirectory: () => true} as any);
		});

		const result = await discover_build_output_dirs();

		// vanished entry is skipped, surviving entry is included
		expect(result).toContain('dist_ok');
		expect(result).not.toContain('dist_vanished');
	});

	test('skips non-dist_ prefixed directories', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readdir).mockResolvedValue([
			'node_modules',
			'src',
			'dist_server', // should be included
			'output', // should not be included
		] as any);
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);

		const result = await discover_build_output_dirs();

		expect(result).toEqual(['build', 'dist', 'dist_server']);
		expect(result).not.toContain('node_modules');
		expect(result).not.toContain('src');
		expect(result).not.toContain('output');
	});
});

describe('collect_build_outputs', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns empty array for empty dirs array', async () => {
		const result = await collect_build_outputs([]);

		expect(result).toEqual([]);
	});

	test('hashes all files in build directory', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, readFile, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readdir).mockResolvedValue([
			mock_file_entry('index.html'),
			mock_file_entry('bundle.js'),
		] as any);
		vi.mocked(stat).mockResolvedValue(mock_file_stats());
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const result = await collect_build_outputs(['build']);

		expect(result).toHaveLength(2);
		expect(result[0]).toEqual({
			path: 'build/index.html',
			hash: 'hash1',
			size: 1024,
			mtime: 1729512000000,
			ctime: 1729512000000,
			mode: 33188,
		});
		expect(result[1]).toEqual({
			path: 'build/bundle.js',
			hash: 'hash2',
			size: 1024,
			mtime: 1729512000000,
			ctime: 1729512000000,
			mode: 33188,
		});
	});

	test('skips build.json file', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, readFile, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readdir).mockResolvedValue([
			mock_file_entry('build.json'),
			mock_file_entry('index.html'),
		] as any);
		vi.mocked(stat).mockResolvedValue(mock_file_stats());
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));
		vi.mocked(hash_blake3).mockReturnValue('hash');

		const result = await collect_build_outputs(['build']);

		expect(result).toHaveLength(1);
		expect(result.find((o) => o.path === 'build/build.json')).toBeUndefined();
		expect(result.find((o) => o.path === 'build/index.html')).toBeDefined();
	});

	test('returns empty array for non-existent directory', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

		vi.mocked(fs_exists).mockResolvedValue(false);

		const result = await collect_build_outputs(['build']);

		expect(result).toEqual([]);
	});

	test('hashes all files in directory', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, readFile, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readdir).mockResolvedValue([
			mock_file_entry('file1.js'),
			mock_file_entry('file2.js'),
			mock_file_entry('file3.js'),
		] as any);
		vi.mocked(stat).mockResolvedValue(mock_file_stats());
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));
		vi.mocked(hash_blake3).mockReturnValue('hash');

		const result = await collect_build_outputs(['build']);

		// Should hash all 3 files
		expect(result).toHaveLength(3);
		expect(result.find((o) => o.path === 'build/file1.js')).toBeDefined();
		expect(result.find((o) => o.path === 'build/file2.js')).toBeDefined();
		expect(result.find((o) => o.path === 'build/file3.js')).toBeDefined();
	});

	test('hashes files from multiple directories', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, readFile, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(fs_exists).mockResolvedValue(true);

		vi.mocked(readdir).mockImplementation((path: any) => {
			if (path === 'build') {
				return Promise.resolve([mock_file_entry('index.html')] as any);
			}
			if (path === 'dist') {
				return Promise.resolve([mock_file_entry('index.js')] as any);
			}
			if (path === 'dist_server') {
				return Promise.resolve([mock_file_entry('server.js')] as any);
			}
			return Promise.resolve([] as any);
		});

		vi.mocked(stat).mockResolvedValue(mock_file_stats());
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const result = await collect_build_outputs(['build', 'dist', 'dist_server']);

		// Should hash files from all three directories
		expect(result).toHaveLength(3);
		expect(result.find((o) => o.path === 'build/index.html')).toBeDefined();
		expect(result.find((o) => o.path === 'dist/index.js')).toBeDefined();
		expect(result.find((o) => o.path === 'dist_server/server.js')).toBeDefined();
		// Each file should have a unique hash
		expect(result.find((o) => o.path === 'build/index.html')?.hash).toBe('hash1');
		expect(result.find((o) => o.path === 'dist/index.js')?.hash).toBe('hash2');
		expect(result.find((o) => o.path === 'dist_server/server.js')?.hash).toBe('hash3');
	});

	test('hashes files in deeply nested directories', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, readFile, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(fs_exists).mockResolvedValue(true);

		vi.mocked(readdir).mockImplementation((path: any) => {
			const path_str = String(path);
			if (path_str === 'build') {
				return Promise.resolve([mock_dir_entry('assets')] as any);
			}
			if (path_str === 'build/assets') {
				return Promise.resolve([mock_dir_entry('js')] as any);
			}
			if (path_str === 'build/assets/js') {
				return Promise.resolve([mock_dir_entry('vendor')] as any);
			}
			if (path_str === 'build/assets/js/vendor') {
				return Promise.resolve([mock_dir_entry('libs')] as any);
			}
			if (path_str === 'build/assets/js/vendor/libs') {
				return Promise.resolve([mock_file_entry('foo.js')] as any);
			}
			return Promise.resolve([] as any);
		});

		vi.mocked(stat).mockResolvedValue(mock_file_stats());
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));
		vi.mocked(hash_blake3).mockReturnValue('deep_hash');

		const result = await collect_build_outputs(['build']);

		// Should recursively hash deeply nested file
		const deep_file = result.find((o) => o.path === 'build/assets/js/vendor/libs/foo.js');
		expect(deep_file).toBeDefined();
		expect(deep_file?.hash).toBe('deep_hash');
	});
});
