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

describe('build_task cache race conditions', () => {
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

	test('detects commit change during build and skips cache save', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid, save_build_cache_metadata} = vi.mocked(
			await import('../lib/build_cache.ts'),
		);
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// Workspace clean, cache invalid, so build will run
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		// Simulate commit happening during build:
		// Initial call returns 'commit_a', after build check returns 'commit_b'
		let call_count = 0;
		vi.mocked(git_current_commit_hash).mockImplementation(() => {
			call_count++;
			// First call (batched initial): commit_a
			// Second call (after build check): commit_b (commit happened during build!)
			return call_count === 1 ? 'commit_a' : 'commit_b';
		});

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should run full build
		expect(mock_plugins.setup).toHaveBeenCalled();
		expect(mock_plugins.adapt).toHaveBeenCalled();
		expect(mock_plugins.teardown).toHaveBeenCalled();

		// Should NOT save cache (commit changed during build)
		expect(save_build_cache_metadata).not.toHaveBeenCalled();

		// Should log warning about commit change
		// Note: Commit hashes are sliced to GIT_SHORT_HASH_LENGTH chars in the log
		expect(ctx.log.warn).toHaveBeenCalledWith(
			expect.stringContaining('git commit changed during build'),
			expect.stringContaining('commit_'), // "commit_a".slice(0, GIT_SHORT_HASH_LENGTH) = "commit_"
			expect.stringContaining('cache not saved'),
		);
	});

	test('saves cache when commit is stable throughout build', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid, create_build_cache_metadata, save_build_cache_metadata} =
			vi.mocked(await import('../lib/build_cache.ts'));
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// Workspace clean, cache invalid
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		// Commit stays stable throughout build
		vi.mocked(git_current_commit_hash).mockResolvedValue('stable_commit');

		const mock_metadata = {
			version: '1',
			git_commit: 'stable_commit',
			build_cache_config_hash: 'hash123',
			timestamp: '2025-10-21T10:00:00.000Z',
			outputs: [],
		};
		vi.mocked(create_build_cache_metadata).mockResolvedValue(mock_metadata);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should run full build
		expect(mock_plugins.setup).toHaveBeenCalled();

		// Should save cache (commit was stable)
		expect(save_build_cache_metadata).toHaveBeenCalledWith(mock_metadata, ctx.log);

		// Should NOT log warning
		expect(ctx.log.warn).not.toHaveBeenCalledWith(
			expect.stringContaining('git commit changed during build'),
			expect.anything(),
			expect.anything(),
		);
	});

	test('logs commit hashes when race condition detected', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		// Different commits before/after build
		let call_count = 0;
		vi.mocked(git_current_commit_hash).mockImplementation(() => {
			call_count++;
			return call_count === 1 ? 'abc1234567890' : 'def9876543210';
		});

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should log shortened commit hashes in warning
		expect(ctx.log.warn).toHaveBeenCalledWith(
			expect.stringContaining('git commit changed during build'),
			expect.stringContaining('abc1234'), // First GIT_SHORT_HASH_LENGTH chars
			expect.stringContaining('cache not saved'),
		);
	});
});
