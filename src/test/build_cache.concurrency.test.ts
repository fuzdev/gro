import {describe, test, expect, vi, beforeEach} from 'vitest';

import {is_build_cache_valid, save_build_cache_metadata} from '../lib/build_cache.ts';

import {
	create_mock_logger,
	create_mock_config,
	create_mock_build_cache_metadata,
} from './build_cache_test_helpers.ts';

// Mock dependencies
vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_current_commit_hash: vi.fn(),
}));

vi.mock('$lib/paths.js', () => ({
	paths: {
		root: './',
		source: './src/',
		lib: './src/lib/',
		build: './.gro/',
		build_dev: './.gro/dev/',
		config: './gro.config.ts',
	},
}));

vi.mock('node:fs/promises', () => ({
	readFile: vi.fn(),
	writeFile: vi.fn(),
	mkdir: vi.fn(),
	rm: vi.fn(),
	stat: vi.fn(),
	readdir: vi.fn(),
}));

vi.mock('@fuzdev/fuz_util/fs.js', () => ({
	fs_exists: vi.fn(),
}));

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn(),
}));

describe('race condition: cache file modification during validation', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('handles cache file being modified while reading', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readFile} = vi.mocked(await import('node:fs/promises'));
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		// hash_blake3 mock returns 'hash123', so config's build_cache_config_hash will be 'hash123'
		const initial_metadata = create_mock_build_cache_metadata({
			git_commit: 'abc123',
			build_cache_config_hash: 'hash123',
		});
		const modified_metadata = create_mock_build_cache_metadata({git_commit: 'def456'});

		// Simulate cache file being modified during validation
		let read_count = 0;
		vi.mocked(readFile).mockImplementation(() => {
			read_count++;
			// First read gets initial metadata, second read (during validation) gets modified
			return Promise.resolve(
				JSON.stringify(read_count === 1 ? initial_metadata : modified_metadata),
			);
		});

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		// Metadata is loaded once and used throughout validation,
		// so later file modifications don't affect the result
		const result = await is_build_cache_valid(config, log);

		// initial metadata matches current git commit, so cache is valid
		expect(result).toBe(true);
	});

	test('handles concurrent cache writes', async () => {
		const {writeFile, mkdir} = vi.mocked(await import('node:fs/promises'));

		const metadata1 = create_mock_build_cache_metadata({git_commit: 'commit1'});
		const metadata2 = create_mock_build_cache_metadata({git_commit: 'commit2'});

		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(writeFile).mockResolvedValue(undefined);

		// actually concurrent writes via Promise.all
		await Promise.all([save_build_cache_metadata(metadata1), save_build_cache_metadata(metadata2)]);

		// both should complete without throwing
		expect(writeFile).toHaveBeenCalledTimes(2);
		expect(mkdir).toHaveBeenCalledTimes(2);
	});

	test('handles multiple concurrent build validation operations', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readFile} = vi.mocked(await import('node:fs/promises'));
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		// hash_blake3 mock returns 'hash123', so config's build_cache_config_hash will be 'hash123'
		const metadata = create_mock_build_cache_metadata({
			git_commit: 'abc123',
			build_cache_config_hash: 'hash123',
		});

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		// multiple concurrent validations should all succeed
		const validations = await Promise.all([
			is_build_cache_valid(config, log),
			is_build_cache_valid(config, log),
			is_build_cache_valid(config, log),
		]);

		// all validations should return true since metadata matches
		expect(validations).toEqual([true, true, true]);
	});

	// Note: Git commit changing during build is tested at the integration level
	// in build.task.test.ts, where the task verifies commit hash before/after build
	// and prevents cache save if changed (build.task.ts:122-132)
});
