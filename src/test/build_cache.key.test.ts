import { describe, test, expect, vi, beforeEach } from 'vitest';

import { compute_build_cache_key } from '#lib/build_cache.ts';

import { create_mock_logger, create_mock_config } from './test_helpers.ts';
import { MOCK_BUILD_CACHE_DEPENDENCIES } from './build_cache_dependencies_mocks.ts';
import { create_mock_git_program, git_ops } from './git_mocks.ts';

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn()
}));

const git = create_mock_git_program();

const dependencies = MOCK_BUILD_CACHE_DEPENDENCIES;

describe('compute_build_cache_key', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns consistent hash components for same inputs', async () => {
		const { git_current_commit_hash } = git_ops;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result1 = await compute_build_cache_key({ log, git }, config, { dependencies });
		const result2 = await compute_build_cache_key({ log, git }, config, { dependencies });

		expect(result1).toEqual(result2);
		expect(result1.git_commit).toBe('abc123');
		expect(result1.build_cache_config_hash).toBe('hash123');
	});

	test('handles missing git repository', async () => {
		const { git_current_commit_hash } = git_ops;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue(null);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await compute_build_cache_key({ log, git }, config, { dependencies });

		expect(result.git_commit).toBeNull();
		expect(log.warn).toHaveBeenCalledWith(expect.stringContaining('Not in a git repository'));
	});

	test('takes `build_cache_config_hash` from the config as it is', async () => {
		const { git_current_commit_hash } = git_ops;

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		const config = await create_mock_config({ build_cache_config_hash: 'from_config' });
		const log = create_mock_logger();

		const result = await compute_build_cache_key({ log, git }, config, { dependencies });

		expect(result.build_cache_config_hash).toBe('from_config');
	});

	test('includes the Gro version, Node version, and package lock hash', async () => {
		const { git_current_commit_hash } = git_ops;

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await compute_build_cache_key({ log, git }, config, { dependencies });

		expect(result).toMatchObject(MOCK_BUILD_CACHE_DEPENDENCIES);
	});

	test('uses a given commit without reading git', async () => {
		const { git_current_commit_hash } = git_ops;

		const config = await create_mock_config();
		const log = create_mock_logger();
		const no_lockfile = {
			gro_version: '9.9.9',
			runtime_version: 'node/v1.2.3',
			package_lock_hash: null
		};

		const result = await compute_build_cache_key({ log, git }, config, {
			dependencies: no_lockfile,
			git_commit: 'abc123'
		});

		expect(result).toMatchObject({ ...no_lockfile, git_commit: 'abc123' });
		expect(git_current_commit_hash).not.toHaveBeenCalled();
		expect(log.debug).toHaveBeenCalledWith(
			expect.stringContaining('No node_modules/.package-lock.json')
		);
	});
});
