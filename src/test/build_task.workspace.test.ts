import {describe, test, expect, vi, beforeEach, afterEach} from 'vitest';
import {join} from 'node:path';

import {task as build_task, GIT_SHORT_HASH_LENGTH} from '../lib/build.task.ts';

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

describe('build_task workspace state', () => {
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

	test('GIT_SHORT_HASH_LENGTH constant matches git convention', () => {
		// Verify the constant is set to standard git short hash length
		expect(GIT_SHORT_HASH_LENGTH).toBe(7);
	});

	test('deletes cache and dist outputs when workspace is dirty', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {rm, readdir, stat} = vi.mocked(await import('node:fs/promises'));

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');

		// Cache file and all build dirs exist (fs_exists used for cache check and discover_build_output_dirs)
		vi.mocked(fs_exists).mockResolvedValue(true);

		// Mock discover_build_output_dirs() internals - async readdir/stat for dist_* checks
		vi.mocked(readdir).mockResolvedValue(['dist_server', 'dist_worker', 'other'] as any);
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should delete cache file
		expect(rm).toHaveBeenCalledWith(join('./.gro/', 'build.json'), {force: true});

		// Should delete build/ directory (via discover_build_output_dirs)
		expect(rm).toHaveBeenCalledWith('build', {recursive: true, force: true});

		// Should delete dist/ directory
		expect(rm).toHaveBeenCalledWith('dist', {recursive: true, force: true});

		// Should delete all dist_* directories
		expect(rm).toHaveBeenCalledWith('dist_server', {recursive: true, force: true});
		expect(rm).toHaveBeenCalledWith('dist_worker', {recursive: true, force: true});

		// Should not delete non-build directories
		expect(rm).not.toHaveBeenCalledWith('other', expect.anything());

		// Should log dirty workspace message
		expect(ctx.log.info).toHaveBeenCalledWith(
			expect.stringContaining('workspace has uncommitted changes'),
		);
	});

	test('runs full build when workspace is dirty', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {readdir} = vi.mocked(await import('node:fs/promises'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');
		vi.mocked(readdir).mockResolvedValue([]);

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
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {save_build_cache_metadata} = vi.mocked(await import('../lib/build_cache.ts'));
		const {readdir} = vi.mocked(await import('node:fs/promises'));

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');
		vi.mocked(readdir).mockResolvedValue([]);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should NOT save cache after build
		expect(save_build_cache_metadata).not.toHaveBeenCalled();
	});

	test('handles missing cache file gracefully when workspace is dirty', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {rm, readdir, stat} = vi.mocked(await import('node:fs/promises'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');

		// Cache file does not exist, but build dirs do
		vi.mocked(fs_exists).mockImplementation(async (path: any) => {
			const path_str = String(path);
			if (path_str.includes('build.json')) return false; // Cache file doesn't exist
			return true; // build dirs exist
		});
		vi.mocked(readdir).mockResolvedValue([]);
		vi.mocked(stat).mockResolvedValue({isDirectory: () => true} as any);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should NOT delete cache file since it doesn't exist
		expect(rm).not.toHaveBeenCalledWith(join('./.gro/', 'build.json'), {force: true});

		// Should still delete build outputs (since they exist via discover_build_output_dirs)
		expect(rm).toHaveBeenCalledWith('build', {recursive: true, force: true});
		expect(rm).toHaveBeenCalledWith('dist', {recursive: true, force: true});

		// Should not throw
		expect(mock_plugins.setup).toHaveBeenCalled();
	});

	test('handles missing dist directories gracefully when workspace is dirty', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {rm, readdir} = vi.mocked(await import('node:fs/promises'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// workspace has uncommitted changes
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');

		// Cache file exists, no build directories exist
		vi.mocked(fs_exists).mockImplementation(async (path: any) => {
			const path_str = String(path);
			if (path_str.includes('build.json')) return true; // Cache file exists
			return false; // No build dirs exist
		});
		vi.mocked(readdir).mockResolvedValue([]); // No dist_* directories

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should delete cache file (since it exists)
		expect(rm).toHaveBeenCalledWith(join('./.gro/', 'build.json'), {force: true});

		// Should NOT delete build/dist since they don't exist (discover_build_output_dirs returns empty)
		expect(rm).not.toHaveBeenCalledWith('build', expect.anything());
		expect(rm).not.toHaveBeenCalledWith('dist', expect.anything());

		// Should not throw
		expect(mock_plugins.setup).toHaveBeenCalled();
	});

	describe('post-build workspace verification', () => {
		test('throws TaskError when plugin modifies source files during build', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));

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

			// Should throw TaskError with specific message about modified files
			await expect(build_task.run(ctx)).rejects.toThrow(
				/Build process modified tracked files.*src\/lib\/foo\.ts/s,
			);
		});

		test('throws TaskError when clean workspace becomes dirty with untracked files', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));

			// Workspace starts clean, but has untracked files after build
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async () => {
				call_count++;
				return call_count === 1 ? null : 'Untracked files:\n  src/generated_file.ts';
			});
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			const ctx = create_mock_build_task_context();

			// Should throw TaskError mentioning untracked files
			await expect(build_task.run(ctx)).rejects.toThrow(
				/Build process modified tracked files.*Untracked files/s,
			);
		});

		test('succeeds when workspace stays clean throughout build', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));

			// Workspace stays clean throughout
			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			const ctx = create_mock_build_task_context();

			// Should complete successfully
			await expect(build_task.run(ctx)).resolves.toBeUndefined();
		});

		test('succeeds when dirty workspace stays dirty with same status', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {readdir} = vi.mocked(await import('node:fs/promises'));

			// Workspace stays dirty with same status throughout
			const dirty_status = 'Modified files:\n  src/foo.ts';
			vi.mocked(git_check_clean_workspace).mockResolvedValue(dirty_status);
			vi.mocked(readdir).mockResolvedValue([]);

			const ctx = create_mock_build_task_context();

			// Should complete successfully (dirty is expected, just can't change)
			await expect(build_task.run(ctx)).resolves.toBeUndefined();
		});

		test('throws TaskError when dirty workspace gets different dirty status during build', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {readdir} = vi.mocked(await import('node:fs/promises'));

			// Workspace starts with one dirty status, changes to different dirty status
			let call_count = 0;
			vi.mocked(git_check_clean_workspace).mockImplementation(async () => {
				call_count++;
				return call_count === 1
					? 'Modified files:\n  src/foo.ts'
					: 'Modified files:\n  src/foo.ts\n  src/bar.ts'; // Added file!
			});
			vi.mocked(readdir).mockResolvedValue([]);

			const ctx = create_mock_build_task_context();

			// Should throw - workspace status changed
			await expect(build_task.run(ctx)).rejects.toThrow(/Build process modified tracked files/);
		});
	});
});
