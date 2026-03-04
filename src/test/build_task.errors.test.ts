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

describe('build_task error handling', () => {
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

	test('handles git command failures gracefully', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));

		// Git command throws an error
		vi.mocked(git_check_clean_workspace).mockRejectedValue(new Error('git command not found'));

		const ctx = create_mock_build_task_context();

		// Should throw - git errors are fatal
		await expect(build_task.run(ctx)).rejects.toThrow('git command not found');
	});

	test('handles file system errors during cache deletion', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {fs_exists} = vi.mocked(await import('@fuzdev/fuz_util/fs.js'));
		const {rm, readdir} = vi.mocked(await import('node:fs/promises'));

		// Workspace is dirty
		vi.mocked(git_check_clean_workspace).mockResolvedValue('Modified files:\n  src/foo.ts');
		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readdir).mockResolvedValue([]);

		// rm rejects with an error
		vi.mocked(rm).mockRejectedValue(new Error('Permission denied'));

		const ctx = create_mock_build_task_context();

		// Should throw - filesystem errors are fatal
		await expect(build_task.run(ctx)).rejects.toThrow('Permission denied');
	});

	test('handles cache validation errors gracefully', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));

		// Workspace is clean, but cache validation throws
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockRejectedValue(new Error('Failed to read cache file'));

		const ctx = create_mock_build_task_context();

		// Should throw - cache validation errors are fatal
		await expect(build_task.run(ctx)).rejects.toThrow('Failed to read cache file');
	});
});
