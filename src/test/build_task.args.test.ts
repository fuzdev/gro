import {describe, test, expect, vi, beforeEach, afterEach} from 'vitest';

import {task as build_task} from '../lib/build.task.ts';

import {create_mock_build_task_context, create_mock_plugins} from './build_task_test_helpers.ts';

// Mock dependencies
vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_check_clean_workspace: vi.fn(),
	git_current_commit_hash: vi.fn(),
}));

vi.mock('node:fs', () => ({
	existsSync: vi.fn(),
	rmSync: vi.fn(),
	mkdirSync: vi.fn(),
	readFileSync: vi.fn(),
	writeFileSync: vi.fn(),
	readdirSync: vi.fn(),
	statSync: vi.fn(),
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

describe('build_task args and sync/install', () => {
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

	test('calls sync task when sync is true', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);

		const ctx = create_mock_build_task_context({sync: true, install: true});

		await build_task.run(ctx);

		// Should call sync task with install arg and gen: false
		expect(ctx.invoke_task).toHaveBeenCalledWith('sync', {install: true, gen: false});
	});

	test('skips sync task when sync is false', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);

		const ctx = create_mock_build_task_context({sync: false, install: false, gen: false});

		await build_task.run(ctx);

		// Should NOT call any tasks
		expect(ctx.invoke_task).not.toHaveBeenCalled();
	});

	test('warns when sync is false but install is true', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);

		const ctx = create_mock_build_task_context({sync: false, install: true});

		await build_task.run(ctx);

		// Should log warning
		expect(ctx.log.warn).toHaveBeenCalledWith(
			expect.stringContaining('sync is false but install is true'),
		);

		// Should still call sync (install takes precedence) with gen: false
		expect(ctx.invoke_task).toHaveBeenCalledWith('sync', {install: true, gen: false});
	});

	describe('sync task failures', () => {
		test('propagates error when sync task fails', async () => {
			const ctx = create_mock_build_task_context({sync: true, install: true});

			// Mock invoke_task to throw an error
			const sync_error = new Error('Sync task failed');
			ctx.invoke_task = vi.fn().mockRejectedValue(sync_error);

			// Should propagate the error
			await expect(build_task.run(ctx)).rejects.toThrow('Sync task failed');

			// Should have attempted to invoke sync with gen: false
			expect(ctx.invoke_task).toHaveBeenCalledWith('sync', {install: true, gen: false});
		});

		test('fails build early without running plugin lifecycle when sync fails', async () => {
			const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
			const mock_plugins = create_mock_plugins();
			vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

			const ctx = create_mock_build_task_context({sync: true, install: true});

			// Mock invoke_task to throw an error
			const sync_error = new Error('Sync task failed');
			ctx.invoke_task = vi.fn().mockRejectedValue(sync_error);

			// Should propagate the error
			await expect(build_task.run(ctx)).rejects.toThrow('Sync task failed');

			// Should NOT run plugin lifecycle (sync failed early)
			expect(mock_plugins.setup).not.toHaveBeenCalled();
			expect(mock_plugins.adapt).not.toHaveBeenCalled();
			expect(mock_plugins.teardown).not.toHaveBeenCalled();
		});
	});
});
