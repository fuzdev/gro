import { describe, test, expect, vi, beforeEach } from 'vitest';
import { json_stringify_deterministic } from '@fuzdev/fuz_util/json.ts';

import { compute_build_cache_key } from '#lib/build_cache.ts';

import {
	create_mock_logger,
	create_mock_config,
	MOCK_BUILD_CACHE_DEPENDENCIES
} from './build_cache_test_helpers.ts';

// Mock dependencies
vi.mock('#lib/build_cache_dependencies.ts', () => ({
	load_build_cache_dependencies: vi.fn(async () => ({
		gro_version: '1.0.0',
		node_version: 'v24.0.0',
		package_lock_hash: 'lock123'
	}))
}));

vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_current_commit_hash: vi.fn()
}));

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn()
}));

describe('compute_build_cache_key', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns consistent hash components for same inputs', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result1 = await compute_build_cache_key(config, log);
		const result2 = await compute_build_cache_key(config, log);

		expect(result1).toEqual(result2);
		expect(result1.git_commit).toBe('abc123');
		expect(result1.build_cache_config_hash).toBe('hash123');
	});

	test('handles missing git repository', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue(null);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await compute_build_cache_key(config, log);

		expect(result.git_commit).toBeNull();
		expect(log.warn).toHaveBeenCalledWith(expect.stringContaining('Not in a git repository'));
	});

	test('hashes build_cache_config when provided', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('custom_hash');

		const config = await create_mock_config({
			build_cache_config: { api_url: 'https://fuz.dev' }
		});
		const log = create_mock_logger();

		const result = await compute_build_cache_key(config, log);

		expect(result.build_cache_config_hash).toBeTruthy();
		expect(hash_blake3).toHaveBeenCalledWith(
			json_stringify_deterministic({ api_url: 'https://fuz.dev' })
		);
	});

	test('produces consistent hash for build_cache_config regardless of key order', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		// Since hashing now happens during config creation with deterministic key ordering,
		// different key orders should produce the same hash
		const config1 = await create_mock_config({
			build_cache_config: { z_key: 'value_z', a_key: 'value_a', m_key: 'value_m' }
		});
		const config2 = await create_mock_config({
			build_cache_config: { a_key: 'value_a', m_key: 'value_m', z_key: 'value_z' }
		});
		const config3 = await create_mock_config({
			build_cache_config: { m_key: 'value_m', z_key: 'value_z', a_key: 'value_a' }
		});

		const log = create_mock_logger();

		const hash1 = (await compute_build_cache_key(config1, log)).build_cache_config_hash;
		const hash2 = (await compute_build_cache_key(config2, log)).build_cache_config_hash;
		const hash3 = (await compute_build_cache_key(config3, log)).build_cache_config_hash;

		expect(hash1).toBe(hash2);
		expect(hash2).toBe(hash3);
	});

	test('handles async build_cache_config function', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		const config = await create_mock_config({
			build_cache_config: async () => ({ feature_flag: true })
		});
		const log = create_mock_logger();

		const result = await compute_build_cache_key(config, log);

		expect(result.build_cache_config_hash).toBeTruthy();
	});

	test('includes the Gro version, Node version, and package lock hash', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { load_build_cache_dependencies } = await import('#lib/build_cache_dependencies.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await compute_build_cache_key(config, log);

		expect(result).toMatchObject(MOCK_BUILD_CACHE_DEPENDENCIES);
		expect(load_build_cache_dependencies).toHaveBeenCalledTimes(1);
	});

	test('uses pre-loaded dependencies without loading them', async () => {
		const { load_build_cache_dependencies } = await import('#lib/build_cache_dependencies.ts');

		const config = await create_mock_config();
		const log = create_mock_logger();
		const dependencies = {
			gro_version: '9.9.9',
			node_version: 'v1.2.3',
			package_lock_hash: null
		};

		const result = await compute_build_cache_key(config, log, 'abc123', dependencies);

		expect(result).toMatchObject(dependencies);
		expect(load_build_cache_dependencies).not.toHaveBeenCalled();
		expect(log.debug).toHaveBeenCalledWith(
			expect.stringContaining('No node_modules/.package-lock.json')
		);
	});
});
