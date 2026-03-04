import {describe, test, expect, vi, beforeEach} from 'vitest';

import {create_build_cache_metadata} from '../lib/build_cache.ts';

import {
	create_mock_logger,
	create_mock_config,
	mock_file_stats,
	mock_dir_stats,
	mock_file_entry,
	mock_dir_entry,
} from './build_cache_test_helpers.ts';

// Mock dependencies
vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_current_commit_hash: vi.fn(),
}));

// Mock async fs functions for discover_build_output_dirs and collect_build_outputs
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

describe('create_build_cache_metadata', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		// Set up default async mocks for discover_build_output_dirs
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));
		vi.mocked(fs_exists).mockResolvedValue(false);
		vi.mocked(readdir).mockResolvedValue([] as any);
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);
	});

	test('creates complete metadata object', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(fs_exists).mockResolvedValue(false);
		vi.mocked(readdir).mockResolvedValue([] as any);
		vi.mocked(stat).mockResolvedValue(mock_dir_stats());
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		expect(result).toMatchObject({
			version: '1',
			git_commit: 'abc123',
		});
		expect(result.timestamp).toBeTruthy();
		expect(new Date(result.timestamp).getTime()).toBeGreaterThan(0);
	});

	test('creates metadata with actual build outputs', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat, readFile} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(fs_exists).mockImplementation((path: any) => Promise.resolve(path === 'build'));
		// Set up async mocks for discover_build_output_dirs and collect_build_outputs
		vi.mocked(readdir).mockImplementation((path: any) => {
			if (path === '.') return Promise.resolve([] as any);
			if (path === 'build') {
				return Promise.resolve([
					mock_file_entry('index.html'),
					mock_file_entry('bundle.js'),
				] as any);
			}
			return Promise.resolve([] as any);
		});
		vi.mocked(stat).mockImplementation((path: any) => {
			if (String(path) === 'build') return Promise.resolve(mock_dir_stats());
			return Promise.resolve(mock_file_stats());
		});
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		expect(result.outputs).toHaveLength(2);
		expect(result.outputs[0]).toMatchObject({
			path: 'build/index.html',
			hash: 'hash2', // hash1 is for config hash
			size: 1024,
		});
		expect(result.outputs[1]).toMatchObject({
			path: 'build/bundle.js',
			hash: 'hash3',
			size: 1024,
		});
	});

	test('creates metadata with multiple build directories', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat, readFile} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		vi.mocked(fs_exists).mockResolvedValue(true);
		// Set up async mocks for discover_build_output_dirs and collect_build_outputs
		vi.mocked(readdir).mockImplementation((path: any) => {
			if (path === '.') {
				return Promise.resolve(['dist_server', 'src', 'node_modules'] as any);
			}
			if (path === 'build') {
				return Promise.resolve([mock_file_entry('app.js')] as any);
			}
			if (path === 'dist') {
				return Promise.resolve([mock_file_entry('lib.js')] as any);
			}
			if (path === 'dist_server') {
				return Promise.resolve([mock_file_entry('server.js')] as any);
			}
			return Promise.resolve([] as any);
		});

		vi.mocked(stat).mockResolvedValue(mock_dir_stats());
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should have files from all three directories
		expect(result.outputs).toHaveLength(3);
		expect(result.outputs.find((o) => o.path === 'build/app.js')).toBeDefined();
		expect(result.outputs.find((o) => o.path === 'dist/lib.js')).toBeDefined();
		expect(result.outputs.find((o) => o.path === 'dist_server/server.js')).toBeDefined();
	});

	test('handles empty build directories', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(fs_exists).mockImplementation((path: any) => Promise.resolve(path === 'build'));
		// Set up async mocks for discover_build_output_dirs and collect_build_outputs
		vi.mocked(readdir).mockImplementation((path: any) => {
			if (path === '.') return Promise.resolve([] as any);
			if (path === 'build') return Promise.resolve([] as any);
			return Promise.resolve([] as any);
		});
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should succeed with empty outputs
		expect(result.outputs).toEqual([]);
		expect(result.git_commit).toBe('abc123');
	});

	test('creates metadata with deeply nested file structures', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat, readFile} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(fs_exists).mockImplementation((path: any) => Promise.resolve(path === 'build'));
		// Set up async mocks for discover_build_output_dirs and collect_build_outputs
		vi.mocked(readdir).mockImplementation((path: any) => {
			const path_str = String(path);
			if (path_str === '.') return Promise.resolve([] as any); // no dist_* directories
			if (path_str === 'build') {
				return Promise.resolve([mock_dir_entry('assets')] as any);
			}
			if (path_str === 'build/assets') {
				return Promise.resolve([mock_dir_entry('js')] as any);
			}
			if (path_str === 'build/assets/js') {
				return Promise.resolve([mock_dir_entry('lib')] as any);
			}
			if (path_str === 'build/assets/js/lib') {
				return Promise.resolve([mock_dir_entry('utils')] as any);
			}
			if (path_str === 'build/assets/js/lib/utils') {
				return Promise.resolve([mock_file_entry('helper.js')] as any);
			}
			return Promise.resolve([] as any);
		});

		vi.mocked(stat).mockImplementation((path: any) => {
			if (String(path).endsWith('.js')) {
				return Promise.resolve(mock_file_stats(256));
			}
			return Promise.resolve(mock_dir_stats());
		});
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));
		vi.mocked(hash_blake3).mockReturnValue('deep_hash');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should find the deeply nested file
		expect(result.outputs).toHaveLength(1);
		expect(result.outputs[0]!.path).toBe('build/assets/js/lib/utils/helper.js');
		expect(result.outputs[0]!.size).toBe(256);
	});

	test('handles build directories with many files', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat, readFile} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(fs_exists).mockImplementation((path: any) => Promise.resolve(path === 'build'));

		const files = Array.from({length: 15}, (_, i) => mock_file_entry(`file${i}.js`));
		// Set up async mocks for discover_build_output_dirs and collect_build_outputs
		vi.mocked(readdir).mockImplementation((path: any) => {
			if (path === '.') return Promise.resolve([] as any); // no dist_* directories
			if (path === 'build') return Promise.resolve(files as any);
			return Promise.resolve([] as any);
		});

		vi.mocked(stat).mockResolvedValue(mock_file_stats(2048));
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should hash all 15 files in parallel
		expect(result.outputs).toHaveLength(15);
		// Verify all files are present
		for (let i = 0; i < 15; i++) {
			expect(result.outputs.find((o) => o.path === `build/file${i}.js`)).toBeDefined();
		}
	});

	test('creates metadata with null git commit', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue(null);
		vi.mocked(fs_exists).mockResolvedValue(false);
		vi.mocked(readdir).mockResolvedValue([] as any);
		vi.mocked(stat).mockResolvedValue(mock_dir_stats());
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should have null git commit
		expect(result.git_commit).toBeNull();
		expect(result.version).toBe('1');
		expect(result.timestamp).toBeTruthy();
		expect(log.warn).toHaveBeenCalledWith(expect.stringContaining('Not in a git repository'));
	});

	test('includes correct build_cache_config_hash', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readdir, stat} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(fs_exists).mockResolvedValue(false);
		vi.mocked(readdir).mockResolvedValue([] as any);
		vi.mocked(stat).mockResolvedValue(mock_dir_stats());
		vi.mocked(hash_blake3).mockReturnValue('custom_config_hash');

		const config = await create_mock_config({
			build_cache_config: {
				api_endpoint: 'https://api.fuz.dev',
				feature_flags: {experimental: true},
			},
		});
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should include the hashed config
		expect(result.build_cache_config_hash).toBe('custom_config_hash');
		expect(result.git_commit).toBe('abc123');
	});
});
