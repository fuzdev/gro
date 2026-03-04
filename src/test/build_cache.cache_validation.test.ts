import {describe, test, expect, vi, beforeEach} from 'vitest';

import {is_build_cache_valid} from '../lib/build_cache.ts';

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

describe('is_build_cache_valid', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns true when cache keys match and outputs valid', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readFile} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		const metadata = create_mock_build_cache_metadata({
			git_commit: 'abc123',
			build_cache_config_hash: 'jkl012',
		});

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('jkl012');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid(config, log);

		expect(result).toBe(true);
		expect(log.info).toHaveBeenCalledWith(
			expect.stringContaining('Build cache valid'),
			expect.anything(),
		);
	});

	test('returns false when no metadata exists', async () => {
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));

		vi.mocked(fs_exists).mockResolvedValue(false);

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid(config, log);

		expect(result).toBe(false);
		expect(log.debug).toHaveBeenCalledWith('No build cache metadata found');
	});

	test('returns false when git commit differs', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readFile} = vi.mocked(await import('node:fs/promises'));

		const metadata = create_mock_build_cache_metadata({git_commit: 'old_commit'});

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('new_commit');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid(config, log);

		expect(result).toBe(false);
		expect(log.debug).toHaveBeenCalledWith('Build cache invalid: git commit changed');
	});

	test('uses pre-computed git_commit and skips git_current_commit_hash', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readFile} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		const metadata = create_mock_build_cache_metadata({
			git_commit: 'precomputed_abc',
			build_cache_config_hash: 'jkl012',
		});

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(hash_blake3).mockReturnValue('jkl012');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid(config, log, 'precomputed_abc');

		expect(result).toBe(true);
		expect(git_current_commit_hash).not.toHaveBeenCalled();
	});

	test('returns false when build_cache_config hash differs', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {readFile} = vi.mocked(await import('node:fs/promises'));
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		const metadata = create_mock_build_cache_metadata({
			git_commit: 'abc123',
			build_cache_config_hash: 'old_config_hash',
		});

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('new_config_hash');

		const config = await create_mock_config({
			build_cache_config: {changed: true},
		});
		const log = create_mock_logger();

		const result = await is_build_cache_valid(config, log);

		expect(result).toBe(false);
		expect(log.debug).toHaveBeenCalledWith('Build cache invalid: build_cache_config changed');
	});
});
