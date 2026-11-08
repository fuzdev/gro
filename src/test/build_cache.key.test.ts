import { describe, test, expect, vi, beforeEach } from 'vitest';
import { json_stringify_deterministic } from '@fuzdev/fuz_util/json.ts';

import { compute_build_cache_key } from '#lib/build_cache.ts';

import {
	create_mock_logger,
	create_mock_config,
	MOCK_BUILD_CACHE_DEPENDENCIES
} from './build_cache_test_helpers.ts';
import { create_mock_git_program, git_ops } from './git_mocks.ts';

vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

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

	test('hashes build_cache_config when provided', async () => {
		const { git_current_commit_hash } = git_ops;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('custom_hash');

		const config = await create_mock_config({
			build_cache_config: { api_url: 'https://fuz.dev' }
		});
		const log = create_mock_logger();

		const result = await compute_build_cache_key({ log, git }, config, { dependencies });

		expect(result.build_cache_config_hash).toBeTruthy();
		expect(hash_blake3).toHaveBeenCalledWith(
			json_stringify_deterministic({ api_url: 'https://fuz.dev' })
		);
	});

	test('produces consistent hash for build_cache_config regardless of key order', async () => {
		const { git_current_commit_hash } = git_ops;

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

		const hash1 = (await compute_build_cache_key({ log, git }, config1, { dependencies }))
			.build_cache_config_hash;
		const hash2 = (await compute_build_cache_key({ log, git }, config2, { dependencies }))
			.build_cache_config_hash;
		const hash3 = (await compute_build_cache_key({ log, git }, config3, { dependencies }))
			.build_cache_config_hash;

		expect(hash1).toBe(hash2);
		expect(hash2).toBe(hash3);
	});

	test('handles async build_cache_config function', async () => {
		const { git_current_commit_hash } = git_ops;

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');

		const config = await create_mock_config({
			build_cache_config: async () => ({ feature_flag: true })
		});
		const log = create_mock_logger();

		const result = await compute_build_cache_key({ log, git }, config, { dependencies });

		expect(result.build_cache_config_hash).toBeTruthy();
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
			node_version: 'v1.2.3',
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
