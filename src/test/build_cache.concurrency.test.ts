import { describe, test, expect, vi, beforeEach } from 'vitest';

import { is_build_cache_valid, save_build_cache_metadata } from '#lib/build_cache.ts';

import {
	create_mock_logger,
	create_mock_config,
	create_mock_build_cache_metadata,
	MOCK_BUILD_CACHE_DEPENDENCIES
} from './build_cache_test_helpers.ts';
import { create_mock_project_fs_paths } from './fs_mocks.ts';
import { create_mock_git_program, git_ops } from './git_mocks.ts';

vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn()
}));

const fs_paths = create_mock_project_fs_paths();
const git = create_mock_git_program();

const dependencies = MOCK_BUILD_CACHE_DEPENDENCIES;

describe('race condition: cache file modification during validation', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('handles cache file being modified while reading', async () => {
		const { read_text_if_exists } = fs_paths.project;
		const { git_current_commit_hash } = git_ops;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		// hash_blake3 mock returns 'hash123', so config's build_cache_config_hash will be 'hash123'
		const initial_metadata = create_mock_build_cache_metadata({
			git_commit: 'abc123',
			build_cache_config_hash: 'hash123'
		});
		const modified_metadata = create_mock_build_cache_metadata({ git_commit: 'def456' });

		// Simulate cache file being modified during validation
		let read_count = 0;
		vi.mocked(read_text_if_exists).mockImplementation(() => {
			read_count++;
			// First read gets initial metadata, second read (during validation) gets modified
			return Promise.resolve(
				JSON.stringify(read_count === 1 ? initial_metadata : modified_metadata)
			);
		});

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		// Metadata is loaded once and used throughout validation,
		// so later file modifications don't affect the result
		const result = await is_build_cache_valid({ fs_paths, log, git }, config, { dependencies });

		// initial metadata matches current git commit, so cache is valid
		expect(result).toBe(true);
	});

	test('handles concurrent cache writes', async () => {
		const { write, mkdir, stat } = fs_paths.project;

		const metadata1 = create_mock_build_cache_metadata({ git_commit: 'commit1' });
		const metadata2 = create_mock_build_cache_metadata({ git_commit: 'commit2' });

		// no `.gro/` yet, so each makes it and writes its marker
		vi.mocked(stat).mockResolvedValue(null);
		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(write).mockResolvedValue(undefined);

		// actually concurrent writes via Promise.all
		const deps = { fs_paths, log: create_mock_logger(), git };
		await Promise.all([
			save_build_cache_metadata(deps, metadata1),
			save_build_cache_metadata(deps, metadata2)
		]);

		// both should complete without throwing
		expect(write.mock.calls.filter(([path]) => path === '.gro/build.json')).toHaveLength(2);
		expect(mkdir).toHaveBeenCalledTimes(2);
	});

	test('handles multiple concurrent build validation operations', async () => {
		const { read_text_if_exists } = fs_paths.project;
		const { git_current_commit_hash } = git_ops;
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		// hash_blake3 mock returns 'hash123', so config's build_cache_config_hash will be 'hash123'
		const metadata = create_mock_build_cache_metadata({
			git_commit: 'abc123',
			build_cache_config_hash: 'hash123'
		});

		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		// multiple concurrent validations should all succeed
		const validations = await Promise.all([
			is_build_cache_valid({ fs_paths, log, git }, config, { dependencies }),
			is_build_cache_valid({ fs_paths, log, git }, config, { dependencies }),
			is_build_cache_valid({ fs_paths, log, git }, config, { dependencies })
		]);

		// all validations should return true since metadata matches
		expect(validations).toEqual([true, true, true]);
	});

	// Note: Git commit changing during build is tested at the integration level
	// in build.task.test.ts, where the task verifies commit hash before/after build
	// and prevents cache save if changed (build.task.ts:122-132)
});
