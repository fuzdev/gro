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

describe('build_task cache validation', () => {
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

	test('skips build when cache is valid', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// Workspace is clean, cache is valid
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');
		vi.mocked(is_build_cache_valid).mockResolvedValue(true);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should check cache with pre-computed git commit
		expect(is_build_cache_valid).toHaveBeenCalledWith(
			ctx.config,
			ctx.log,
			'abc123', // pre-computed git commit
		);

		// Should not run plugins
		expect(mock_plugins.setup).not.toHaveBeenCalled();
		expect(mock_plugins.adapt).not.toHaveBeenCalled();
		expect(mock_plugins.teardown).not.toHaveBeenCalled();

		// Should log skip message
		expect(ctx.log.info).toHaveBeenCalledWith(
			expect.stringContaining('skipping build, cache is valid'),
			expect.anything(),
		);
	});

	test('forces rebuild even when cache is valid with force_build flag', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// Workspace is clean, cache would be valid, but force_build is true
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(true);

		const ctx = create_mock_build_task_context({force_build: true});

		await build_task.run(ctx);

		// Should NOT check cache (bypassed by force_build)
		expect(is_build_cache_valid).not.toHaveBeenCalled();

		// Should run full plugin lifecycle
		expect(mock_plugins.setup).toHaveBeenCalled();
		expect(mock_plugins.adapt).toHaveBeenCalled();
		expect(mock_plugins.teardown).toHaveBeenCalled();

		// Should log force build message
		expect(ctx.log.info).toHaveBeenCalledWith(expect.stringContaining('forcing fresh build'));
	});

	test('cache validation works with null git commit', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		// Not in a git repository
		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(git_current_commit_hash).mockResolvedValue(null);
		vi.mocked(hash_blake3).mockReturnValue('hash123');
		vi.mocked(is_build_cache_valid).mockResolvedValue(true);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		// Should check cache with null git_commit
		expect(is_build_cache_valid).toHaveBeenCalledWith(
			ctx.config,
			ctx.log,
			null, // null git commit
		);

		// Should skip build due to valid cache
		expect(mock_plugins.setup).not.toHaveBeenCalled();
	});
});
