import {describe, test, expect, vi, beforeEach} from 'vitest';
import {json_stringify_deterministic} from '@fuzdev/fuz_util/json.js';

import {compute_build_cache_key} from '../lib/build_cache.ts';

import {create_mock_logger, create_mock_config} from './build_cache_test_helpers.ts';

/* eslint-disable @typescript-eslint/require-await */

// Mock dependencies
vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_current_commit_hash: vi.fn(),
}));

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn(),
}));

describe('compute_build_cache_key', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns consistent hash components for same inputs', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

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
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue(null);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await compute_build_cache_key(config, log);

		expect(result.git_commit).toBeNull();
		expect(log.warn).toHaveBeenCalledWith(expect.stringContaining('Not in a git repository'));
	});

	test('hashes build_cache_config when provided', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');
		const {hash_blake3} = await import('@fuzdev/fuz_util/hash_blake3.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('custom_hash');

		const config = await create_mock_config({
			build_cache_config: {api_url: 'https://fuz.dev'},
		});
		const log = create_mock_logger();

		const result = await compute_build_cache_key(config, log);

		expect(result.build_cache_config_hash).toBeTruthy();
		expect(hash_blake3).toHaveBeenCalledWith(
			json_stringify_deterministic({api_url: 'https://fuz.dev'}),
		);
	});

	test('produces consistent hash for build_cache_config regardless of key order', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		// Since hashing now happens during config creation with deterministic key ordering,
		// different key orders should produce the same hash
		const config1 = await create_mock_config({
			build_cache_config: {z_key: 'value_z', a_key: 'value_a', m_key: 'value_m'},
		});
		const config2 = await create_mock_config({
			build_cache_config: {a_key: 'value_a', m_key: 'value_m', z_key: 'value_z'},
		});
		const config3 = await create_mock_config({
			build_cache_config: {m_key: 'value_m', z_key: 'value_z', a_key: 'value_a'},
		});

		const log = create_mock_logger();

		const hash1 = (await compute_build_cache_key(config1, log)).build_cache_config_hash;
		const hash2 = (await compute_build_cache_key(config2, log)).build_cache_config_hash;
		const hash3 = (await compute_build_cache_key(config3, log)).build_cache_config_hash;

		expect(hash1).toBe(hash2);
		expect(hash2).toBe(hash3);
	});

	test('handles async build_cache_config function', async () => {
		const {git_current_commit_hash} = await import('@fuzdev/fuz_util/git.js');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		const config = await create_mock_config({
			build_cache_config: async () => ({feature_flag: true}),
		});
		const log = create_mock_logger();

		const result = await compute_build_cache_key(config, log);

		expect(result.build_cache_config_hash).toBeTruthy();
	});
});
