import { describe, test, expect, vi, beforeEach } from 'vitest';

import {
	BUILD_CACHE_VERSION,
	is_build_cache_valid,
	load_build_cache_metadata
} from '#lib/build_cache.ts';
import type { BuildCacheDependencies } from '#lib/build_cache_dependencies.ts';

import {
	create_mock_logger,
	create_mock_config,
	create_mock_build_cache_metadata,
	MOCK_BUILD_CACHE_DEPENDENCIES
} from './build_cache_test_helpers.ts';
import { create_mock_project_fs_paths } from './fs_mocks.ts';
import { create_mock_git_program, git_ops } from './git_mocks.ts';

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn()
}));

const fs_paths = create_mock_project_fs_paths();
const git = create_mock_git_program();

const dependencies = MOCK_BUILD_CACHE_DEPENDENCIES;

describe('is_build_cache_valid', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns true when cache keys match and outputs valid', async () => {
		const { git_current_commit_hash } = git_ops;
		const { read_text_if_exists } = fs_paths.project;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({
			git_commit: 'abc123',
			build_cache_config_hash: 'jkl012'
		});

		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('jkl012');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid({ fs_paths, log, git }, config, { dependencies });

		expect(result).toBe(true);
		expect(log.info).toHaveBeenCalledWith(
			expect.stringContaining('Build cache valid'),
			expect.anything()
		);
	});

	test('returns false when no metadata exists', async () => {
		const { read_text_if_exists } = fs_paths.project;

		vi.mocked(read_text_if_exists).mockResolvedValue(null);

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid({ fs_paths, log, git }, config, { dependencies });

		expect(result).toBe(false);
		expect(log.debug).toHaveBeenCalledWith('No build cache metadata found');
	});

	test('returns false when git commit differs', async () => {
		const { git_current_commit_hash } = git_ops;
		const { read_text_if_exists } = fs_paths.project;

		const metadata = create_mock_build_cache_metadata({ git_commit: 'old_commit' });

		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('new_commit');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid({ fs_paths, log, git }, config, { dependencies });

		expect(result).toBe(false);
		expect(log.debug).toHaveBeenCalledWith('Build cache invalid: git commit changed');
	});

	test('uses pre-computed git_commit and skips git_current_commit_hash', async () => {
		const { git_current_commit_hash } = git_ops;
		const { read_text_if_exists } = fs_paths.project;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({
			git_commit: 'precomputed_abc',
			build_cache_config_hash: 'jkl012'
		});

		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(hash_blake3).mockReturnValue('jkl012');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid({ fs_paths, log, git }, config, {
			dependencies,
			git_commit: 'precomputed_abc'
		});

		expect(result).toBe(true);
		expect(git_current_commit_hash).not.toHaveBeenCalled();
	});

	test('returns false with no git commit, even when the metadata has none too', async () => {
		const { read_text_if_exists } = fs_paths.project;

		const metadata = create_mock_build_cache_metadata({ git_commit: null });
		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid({ fs_paths, log, git }, config, {
			dependencies,
			git_commit: null
		});

		expect(result).toBe(false);
		expect(log.debug).toHaveBeenCalledWith('Build cache invalid: no git commit');
	});

	test('returns false when build_cache_config hash differs', async () => {
		const { git_current_commit_hash } = git_ops;
		const { read_text_if_exists } = fs_paths.project;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({
			git_commit: 'abc123',
			build_cache_config_hash: 'old_config_hash'
		});

		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('new_config_hash');

		const config = await create_mock_config({
			build_cache_config: { changed: true }
		});
		const log = create_mock_logger();

		const result = await is_build_cache_valid({ fs_paths, log, git }, config, { dependencies });

		expect(result).toBe(false);
		expect(log.debug).toHaveBeenCalledWith('Build cache invalid: build_cache_config changed');
	});

	test.each([
		['gro_version', { gro_version: '2.0.0' }, 'Build cache invalid: Gro version changed'],
		[
			'runtime_version',
			{ runtime_version: 'node/v99.0.0' },
			'Build cache invalid: runtime version changed'
		],
		[
			'package_lock_hash',
			{ package_lock_hash: 'other_lock' },
			'Build cache invalid: installed packages changed'
		],
		[
			'package_lock_hash to null',
			{ package_lock_hash: null },
			'Build cache invalid: installed packages changed'
		]
	] as const)(
		'returns false when %s differs',
		async (_name, changed: Partial<BuildCacheDependencies>, message) => {
			const { git_current_commit_hash } = git_ops;
			const { read_text_if_exists } = fs_paths.project;
			const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

			const metadata = create_mock_build_cache_metadata();

			vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
			vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
			vi.mocked(hash_blake3).mockReturnValue('jkl012');

			const config = await create_mock_config();
			const log = create_mock_logger();

			const result = await is_build_cache_valid({ fs_paths, log, git }, config, {
				dependencies: { ...MOCK_BUILD_CACHE_DEPENDENCIES, ...changed }
			});

			expect(result).toBe(false);
			expect(log.debug).toHaveBeenCalledWith(message);
		}
	);

	test('returns true when the package lock hash is null on both sides', async () => {
		const { git_current_commit_hash } = git_ops;
		const { read_text_if_exists } = fs_paths.project;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({ package_lock_hash: null });

		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('jkl012');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await is_build_cache_valid({ fs_paths, log, git }, config, {
			dependencies: { ...MOCK_BUILD_CACHE_DEPENDENCIES, package_lock_hash: null }
		});

		expect(result).toBe(true);
	});
});

describe('load_build_cache_metadata across the runtime_version rekey', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	// the file an older Gro wrote: `node_version` where `runtime_version` is now
	const to_old_metadata = (version: string) => {
		const { runtime_version: _, ...rest } = create_mock_build_cache_metadata();
		return JSON.stringify({ ...rest, version, node_version: 'v24.0.0' });
	};

	test.each([
		['an older version', '3'],
		['the current version', BUILD_CACHE_VERSION]
	])('a file from an older Gro at %s is a miss, deleted, never a crash', async (_name, version) => {
		vi.mocked(fs_paths.project.read_text_if_exists).mockResolvedValue(to_old_metadata(version));
		vi.mocked(fs_paths.project.remove).mockResolvedValue(undefined);
		expect(await load_build_cache_metadata(fs_paths)).toBeNull();
		expect(fs_paths.project.remove).toHaveBeenCalledTimes(1);
	});

	test('a file with `runtime_version` loads', async () => {
		const metadata = create_mock_build_cache_metadata();
		vi.mocked(fs_paths.project.read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
		expect(await load_build_cache_metadata(fs_paths)).toEqual(metadata);
		expect(fs_paths.project.remove).not.toHaveBeenCalled();
	});
});
