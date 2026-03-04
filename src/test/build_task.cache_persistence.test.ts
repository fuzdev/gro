import {describe, test, expect, vi, beforeEach, afterEach} from 'vitest';

import {task as build_task} from '../lib/build.task.ts';

import {create_mock_build_task_context, create_mock_plugins} from './build_task_test_helpers.ts';

// Mock dependencies
vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_check_clean_workspace: vi.fn(),
	git_current_commit_hash: vi.fn(),
}));

// Mock async fs functions used by build.task.ts and build_cache.ts (discover_build_output_dirs)
vi.mock('node:fs/promises', () => ({
	rm: vi.fn(),
	readdir: vi.fn(),
	stat: vi.fn(),
}));

// Mock fs_exists from fuz_util
vi.mock('@fuzdev/fuz_util/fs.js', () => ({
	fs_exists: vi.fn(),
}));

vi.mock('../lib/clean_fs.ts', () => ({
	clean_fs: vi.fn(),
}));

vi.mock('../lib/plugin.ts', () => ({
	Plugins: {
		create: vi.fn(),
	},
}));

vi.mock('../lib/build_cache.ts', async (import_original) => {
	const original = await import_original<typeof import('../lib/build_cache.ts')>();
	return {
		...original,
		is_build_cache_valid: vi.fn(),
		create_build_cache_metadata: vi.fn(),
		save_build_cache_metadata: vi.fn(),
	};
});

vi.mock('../lib/paths.ts', () => ({
	paths: {
		root: './',
		source: './src/',
		lib: './src/lib/',
		build: './.gro/',
		build_dev: './.gro/dev/',
		config: './gro.config.ts',
	},
}));

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn(),
}));

describe('build_task cache persistence', () => {
	beforeEach(async () => {
		vi.clearAllMocks();

		// Setup default mocks
		const mock_plugins = create_mock_plugins();
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		const {clean_fs} = vi.mocked(await import('../lib/clean_fs.ts'));
		vi.mocked(clean_fs).mockResolvedValue(undefined);
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	test('runs build when cache is invalid and saves cache after successful build', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid, create_build_cache_metadata, save_build_cache_metadata} =
			vi.mocked(await import('../lib/build_cache.ts'));
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// Workspace is clean, cache is invalid
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const mock_metadata = {
			version: '1',
			git_commit: 'abc123',
			build_cache_config_hash: 'hash123',
			timestamp: '2025-10-21T10:00:00.000Z',
			outputs: [],
		};
		vi.mocked(create_build_cache_metadata).mockResolvedValue(mock_metadata);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should check cache with pre-computed git commit
		expect(is_build_cache_valid).toHaveBeenCalledWith(ctx.config, ctx.log, 'abc123');

		// Should run full plugin lifecycle
		expect(mock_plugins.setup).toHaveBeenCalled();
		expect(mock_plugins.adapt).toHaveBeenCalled();
		expect(mock_plugins.teardown).toHaveBeenCalled();

		// Should save cache after successful build
		expect(create_build_cache_metadata).toHaveBeenCalledWith(
			ctx.config,
			ctx.log,
			'abc123',
			undefined, // build_dirs not pre-discovered in clean workspace path
		);
		expect(save_build_cache_metadata).toHaveBeenCalledWith(mock_metadata, ctx.log);
	});

	test('saves cache when force_build with clean workspace', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {save_build_cache_metadata, create_build_cache_metadata} = vi.mocked(
			await import('../lib/build_cache.ts'),
		);
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));

		// Workspace is clean, force_build is true
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const mock_metadata = {
			version: '1',
			git_commit: 'abc123',
			build_cache_config_hash: 'hash123',
			timestamp: '2025-10-21T10:00:00.000Z',
			outputs: [],
		};
		vi.mocked(create_build_cache_metadata).mockResolvedValue(mock_metadata);

		const ctx = create_mock_build_task_context({force_build: true});

		await build_task.run(ctx);

		// Should save cache after successful build (workspace is clean)
		expect(save_build_cache_metadata).toHaveBeenCalledWith(mock_metadata, ctx.log);
	});

	test('still deletes dist when force_build with dirty workspace', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {rm, readdir, stat} = vi.mocked(await import('node:fs/promises'));

		// Workspace is dirty, force_build is true
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');
		vi.mocked(fs_exists).mockResolvedValue(true); // All files exist
		vi.mocked(readdir).mockResolvedValue(['dist_server'] as any);
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);

		const ctx = create_mock_build_task_context({force_build: true});

		await build_task.run(ctx);

		// Should still delete all build outputs (dirty workspace protection via discover_build_output_dirs)
		expect(rm).toHaveBeenCalledWith('build', {recursive: true, force: true});
		expect(rm).toHaveBeenCalledWith('dist', {recursive: true, force: true});
		expect(rm).toHaveBeenCalledWith('dist_server', {recursive: true, force: true});
	});

	test('build_dirs parameter is correctly threaded through clean workspace path', async () => {
		// This test documents that build_dirs caching optimization is present in the signature
		// but not utilized in current code paths: dirty workspace doesn't save cache,
		// clean workspace doesn't pre-discover, so build_dirs is always undefined when passed
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid, create_build_cache_metadata} = vi.mocked(
			await import('../lib/build_cache.ts'),
		);
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));

		// Clean workspace
		vi.mocked(git_check_clean_workspace).mockResolvedValueOnce(null);
		vi.mocked(git_check_clean_workspace).mockResolvedValueOnce(null);
		vi.mocked(git_current_commit_hash).mockResolvedValueOnce('abc123');
		vi.mocked(git_current_commit_hash).mockResolvedValueOnce('abc123');
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const mock_metadata = {
			version: '1',
			git_commit: 'abc123',
			build_cache_config_hash: 'hash123',
			timestamp: new Date().toISOString(),
			outputs: [],
		};
		vi.mocked(create_build_cache_metadata).mockResolvedValue(mock_metadata);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Verify 4th parameter is undefined in clean workspace path
		expect(create_build_cache_metadata).toHaveBeenCalledWith(
			ctx.config,
			ctx.log,
			'abc123',
			undefined, // build_dirs is not pre-discovered in clean path
		);
	});

	test('handles build when not in a git repository', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid, create_build_cache_metadata, save_build_cache_metadata} =
			vi.mocked(await import('../lib/build_cache.ts'));
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// Not in a git repository
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(git_current_commit_hash).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const mock_metadata = {
			version: '1',
			git_commit: null,
			build_cache_config_hash: 'hash123',
			timestamp: '2025-10-21T10:00:00.000Z',
			outputs: [],
		};
		vi.mocked(create_build_cache_metadata).mockResolvedValue(mock_metadata);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should run full build
		expect(mock_plugins.setup).toHaveBeenCalled();
		expect(mock_plugins.adapt).toHaveBeenCalled();
		expect(mock_plugins.teardown).toHaveBeenCalled();

		// Should save cache with null git_commit
		expect(create_build_cache_metadata).toHaveBeenCalledWith(
			ctx.config,
			ctx.log,
			null, // null git commit
			undefined, // build_dirs not pre-discovered in clean workspace path
		);
		expect(save_build_cache_metadata).toHaveBeenCalledWith(mock_metadata, ctx.log);
	});

	describe('cache metadata creation failures', () => {
		test('propagates error when create_build_cache_metadata throws', async () => {
			const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {is_build_cache_valid, create_build_cache_metadata} = vi.mocked(
				await import('../lib/build_cache.ts'),
			);
			const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));
			const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
			const mock_plugins = create_mock_plugins();
			vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

			// Workspace is clean, cache is invalid
			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);
			vi.mocked(hash_blake3).mockReturnValue('hash123');

			// create_build_cache_metadata throws
			const metadata_error = new Error('Failed to create cache metadata');
			vi.mocked(create_build_cache_metadata).mockRejectedValue(metadata_error);

			const ctx = create_mock_build_task_context();

			// Should propagate the error (cache metadata creation is part of build success)
			await expect(build_task.run(ctx)).rejects.toThrow('Failed to create cache metadata');

			// Build should have completed
			expect(mock_plugins.setup).toHaveBeenCalled();
			expect(mock_plugins.adapt).toHaveBeenCalled();
			expect(mock_plugins.teardown).toHaveBeenCalled();

			// Should have attempted to create metadata
			expect(create_build_cache_metadata).toHaveBeenCalled();
		});

		test('build fails if cache metadata creation throws (cache errors are fatal in clean workspace)', async () => {
			const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
				await import('@fuzdev/fuz_util/git.js'),
			);
			const {is_build_cache_valid, create_build_cache_metadata, save_build_cache_metadata} =
				vi.mocked(await import('../lib/build_cache.ts'));
			const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));

			// Workspace is clean
			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);
			vi.mocked(hash_blake3).mockReturnValue('hash123');

			// Metadata creation throws
			vi.mocked(create_build_cache_metadata).mockRejectedValue(
				new Error('Disk full - cannot create metadata'),
			);

			const ctx = create_mock_build_task_context();

			// Build should fail
			await expect(build_task.run(ctx)).rejects.toThrow('Disk full - cannot create metadata');

			// Should NOT save cache (never got metadata)
			expect(save_build_cache_metadata).not.toHaveBeenCalled();
		});
	});
});
