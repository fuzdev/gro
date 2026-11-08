import { describe, test, expect, vi, beforeEach, afterEach } from 'vitest';

import { task as build_task, GIT_SHORT_HASH_LENGTH } from '#lib/build.task.ts';

import { create_mock_build_task_context, create_mock_plugins } from './build_task_test_helpers.ts';
import { get_mock_project, to_mock_listing } from './fs_mocks.ts';
import { git_ops } from './git_mocks.ts';

// Mock dependencies
vi.mock('#lib/build_cache_dependencies.ts', () => ({
	load_build_cache_dependencies: vi.fn(async () => ({
		gro_version: '1.0.0',
		node_version: 'v24.0.0',
		package_lock_hash: 'lock123'
	}))
}));

vi.mock('#lib/spawn.ts', async () =>
	(await import('./git_mocks.ts')).create_mock_git_spawn_module()
);

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

vi.mock('#lib/clean_fs.ts', () => ({
	clean_fs: vi.fn()
}));

vi.mock('#lib/plugin.ts', async (import_original) => ({
	...(await import_original<typeof import('#lib/plugin.ts')>()),
	Plugins: {
		create: vi.fn()
	}
}));

vi.mock('#lib/build_cache.ts', async (import_original) => {
	const original = await import_original<typeof import('#lib/build_cache.ts')>();
	return {
		...original,
		is_build_cache_valid: vi.fn(),
		create_build_cache_metadata: vi.fn(),
		save_build_cache_metadata: vi.fn()
	};
});

vi.mock('#lib/paths.ts', () => ({
	paths: {
		root: './',
		source: './src/',
		lib: './src/lib/',
		build: './.gro/'
	}
}));

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn()
}));

describe('build_task workspace state', () => {
	beforeEach(async () => {
		vi.clearAllMocks();

		// a dirty workspace removes the build cache, and no build output exists unless a test says so
		const { exists, remove } = await get_mock_project();
		vi.mocked(remove).mockResolvedValue(undefined);
		vi.mocked(exists).mockResolvedValue(false);

		// Setup default mocks
		const mock_plugins = create_mock_plugins();
		const { Plugins } = vi.mocked(await import('#lib/plugin.ts'));
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		const { clean_fs } = vi.mocked(await import('#lib/clean_fs.ts'));
		vi.mocked(clean_fs).mockResolvedValue(undefined);
	});

	afterEach(() => {
		vi.resetAllMocks();
	});

	test('GIT_SHORT_HASH_LENGTH constant matches git convention', () => {
		// Verify the constant is set to standard git short hash length
		expect(GIT_SHORT_HASH_LENGTH).toBe(7);
	});

	test('deletes cache and dist outputs when workspace is dirty', async () => {
		const { git_check_clean_workspace } = git_ops;
		const { exists, remove } = await get_mock_project();
		const { list } = await get_mock_project();

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');

		// all build dirs exist (for discover_build_output_dirs)
		vi.mocked(exists).mockResolvedValue(true);

		// Mock discover_build_output_dirs() internals - the project listing for the dist_* checks
		vi.mocked(list).mockImplementation(() =>
			to_mock_listing(
				['dist_artifacts', 'dist_worker', 'other'],
				['dist_artifacts', 'dist_worker', 'other']
			)
		);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should delete cache file
		expect(remove).toHaveBeenCalledWith('.gro/build.json', { missing_ok: true });

		// Should delete build/ directory (via discover_build_output_dirs)
		expect(remove).toHaveBeenCalledWith('build', { recursive: true, missing_ok: true });

		// Should delete dist/ directory
		expect(remove).toHaveBeenCalledWith('dist', { recursive: true, missing_ok: true });

		// Should delete all dist_* directories
		expect(remove).toHaveBeenCalledWith('dist_artifacts', { recursive: true, missing_ok: true });
		expect(remove).toHaveBeenCalledWith('dist_worker', { recursive: true, missing_ok: true });

		// Should not delete non-build directories
		expect(remove).not.toHaveBeenCalledWith('other', expect.anything());

		// Should log dirty workspace message
		expect(ctx.log.info).toHaveBeenCalledWith(
			expect.stringContaining('workspace has uncommitted changes')
		);
	});

	test('runs full build when workspace is dirty', async () => {
		const { git_check_clean_workspace } = git_ops;
		const { is_build_cache_valid } = vi.mocked(await import('#lib/build_cache.ts'));
		const { list } = await get_mock_project();
		const { Plugins } = vi.mocked(await import('#lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');
		vi.mocked(list).mockImplementation(() => to_mock_listing([]));

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should NOT check cache (skipped due to dirty workspace)
		expect(is_build_cache_valid).not.toHaveBeenCalled();

		// Should run full plugin lifecycle
		expect(mock_plugins.setup).toHaveBeenCalled();
		expect(mock_plugins.adapt).toHaveBeenCalled();
		expect(mock_plugins.teardown).toHaveBeenCalled();
	});

	test('does not save cache when workspace is dirty', async () => {
		const { git_check_clean_workspace } = git_ops;
		const { save_build_cache_metadata } = vi.mocked(await import('#lib/build_cache.ts'));
		const { list } = await get_mock_project();

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');
		vi.mocked(list).mockImplementation(() => to_mock_listing([]));

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should NOT save cache after build
		expect(save_build_cache_metadata).not.toHaveBeenCalled();
	});

	test('handles missing cache file gracefully when workspace is dirty', async () => {
		const { git_check_clean_workspace } = git_ops;
		const { exists, remove } = await get_mock_project();
		const { list } = await get_mock_project();
		const { Plugins } = vi.mocked(await import('#lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');

		// Cache file does not exist, which removing with `missing_ok` returns normally for, but build dirs do
		vi.mocked(remove).mockResolvedValue(undefined);
		vi.mocked(exists).mockResolvedValue(true);
		vi.mocked(list).mockImplementation(() => to_mock_listing([]));

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// removes the cache file without requiring it to exist
		expect(remove).toHaveBeenCalledWith('.gro/build.json', { missing_ok: true });

		// Should still delete build outputs (since they exist via discover_build_output_dirs)
		expect(remove).toHaveBeenCalledWith('build', { recursive: true, missing_ok: true });
		expect(remove).toHaveBeenCalledWith('dist', { recursive: true, missing_ok: true });

		// Should not throw
		expect(mock_plugins.setup).toHaveBeenCalled();
	});

	test('handles missing dist directories gracefully when workspace is dirty', async () => {
		const { git_check_clean_workspace } = git_ops;
		const { exists, remove } = await get_mock_project();
		const { list } = await get_mock_project();
		const { Plugins } = vi.mocked(await import('#lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');

		// No build directories exist
		vi.mocked(exists).mockResolvedValue(false);
		vi.mocked(list).mockImplementation(() => to_mock_listing([])); // No dist_* directories

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should delete cache file
		expect(remove).toHaveBeenCalledWith('.gro/build.json', { missing_ok: true });

		// Should NOT delete build/dist since they don't exist (discover_build_output_dirs returns empty)
		expect(remove).not.toHaveBeenCalledWith('build', expect.anything());
		expect(remove).not.toHaveBeenCalledWith('dist', expect.anything());

		// Should not throw
		expect(mock_plugins.setup).toHaveBeenCalled();
	});

	describe('post-build workspace verification', () => {
		test('throws TaskError when plugin modifies source files during build', async () => {
			const { git_check_clean_workspace } = git_ops;
			const { is_build_cache_valid } = vi.mocked(await import('#lib/build_cache.ts'));

			// Workspace starts clean, but becomes dirty after build
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async () => {
				call_count++;
				// First call: workspace is clean
				// Second call (after build): workspace is dirty!
				return call_count === 1 ? null : 'Modified files:\n  src/lib/foo.ts';
			});
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			const ctx = create_mock_build_task_context();

			// Should throw TaskError with the status after the build
			await expect(build_task.run(ctx)).rejects.toThrow(
				/Build process modified tracked files.*git has unstaged changes/s
			);
		});

		test('throws TaskError when clean workspace becomes dirty with untracked files', async () => {
			const { git_check_clean_workspace } = git_ops;
			const { is_build_cache_valid } = vi.mocked(await import('#lib/build_cache.ts'));

			// Workspace starts clean, but has untracked files after build
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async () => {
				call_count++;
				return call_count === 1 ? null : '?? src/generated_file.ts\0';
			});
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			const ctx = create_mock_build_task_context();

			// Should throw TaskError mentioning untracked files
			await expect(build_task.run(ctx)).rejects.toThrow(
				/Build process modified tracked files.*git has untracked files/s
			);
		});

		test('succeeds when workspace stays clean throughout build', async () => {
			const { git_check_clean_workspace } = git_ops;
			const { is_build_cache_valid } = vi.mocked(await import('#lib/build_cache.ts'));

			// Workspace stays clean throughout
			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			const ctx = create_mock_build_task_context();

			// Should complete successfully
			await expect(build_task.run(ctx)).resolves.toBeUndefined();
		});

		test('succeeds when dirty workspace stays dirty with same status', async () => {
			const { git_check_clean_workspace } = git_ops;
			const { list } = await get_mock_project();

			// Workspace stays dirty with same status throughout
			const dirty_status = 'Modified files:\n  src/foo.ts';
			vi.mocked(git_check_clean_workspace).mockResolvedValue(dirty_status);
			vi.mocked(list).mockImplementation(() => to_mock_listing([]));

			const ctx = create_mock_build_task_context();

			// Should complete successfully (dirty is expected, just can't change)
			await expect(build_task.run(ctx)).resolves.toBeUndefined();
		});

		test('throws TaskError when dirty workspace gets different dirty status during build', async () => {
			const { git_check_clean_workspace } = git_ops;
			const { list } = await get_mock_project();

			// Workspace starts with one dirty status, changes to different dirty status
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async () => {
				call_count++;
				return call_count === 1 ? ' M src/foo.ts\0' : ' M src/foo.ts\0?? src/bar.ts\0'; // Added file!
			});
			vi.mocked(list).mockImplementation(() => to_mock_listing([]));

			const ctx = create_mock_build_task_context();

			// Should throw - workspace status changed
			await expect(build_task.run(ctx)).rejects.toThrow(/Build process modified tracked files/);
		});
	});
});
