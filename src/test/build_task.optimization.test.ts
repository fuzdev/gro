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

describe('build_task optimization', () => {
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

	test('batches initial git calls together', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const ctx = create_mock_build_task_context();

		// Track when git functions are called
		const call_timestamps: Array<{fn: string; time: number}> = [];
		vi.mocked(git_check_clean_workspace).mockImplementation(async () => {
			call_timestamps.push({fn: 'git_check_clean_workspace', time: Date.now()});
			await new Promise((resolve) => setTimeout(resolve, 5)); // Small delay
			return null;
		});
		vi.mocked(git_current_commit_hash).mockImplementation(async () => {
			call_timestamps.push({fn: 'git_current_commit_hash', time: Date.now()});
			await new Promise((resolve) => setTimeout(resolve, 5)); // Small delay
			return 'abc123';
		});

		await build_task.run(ctx);

		// Both git functions should be called
		expect(call_timestamps).toContainEqual(
			expect.objectContaining({fn: 'git_check_clean_workspace'}),
		);
		expect(call_timestamps).toContainEqual(
			expect.objectContaining({fn: 'git_current_commit_hash'}),
		);

		// The initial calls should happen concurrently (within a few ms of each other)
		const first_calls = call_timestamps.slice(0, 2);
		if (first_calls.length === 2) {
			const time_diff = Math.abs(first_calls[0]!.time - first_calls[1]!.time);
			expect(time_diff).toBeLessThan(10); // Called within 10ms = effectively concurrent
		}
	});

	test('passes pre-computed values to avoid re-reading git', async () => {
		const {git_check_clean_workspace, git_current_commit_hash} = vi.mocked(
			await import('@fuzdev/fuz_util/git.js'),
		);
		const {is_build_cache_valid, create_build_cache_metadata} = vi.mocked(
			await import('../lib/build_cache.ts'),
		);
		const {hash_blake3} = vi.mocked(await import('@fuzdev/fuz_util/hash_blake3.js'));

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);
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

		// is_build_cache_valid should receive pre-computed commit
		expect(is_build_cache_valid).toHaveBeenCalledWith(
			ctx.config,
			ctx.log,
			'abc123', // pre-computed git commit
		);

		// create_build_cache_metadata should receive pre-computed commit
		expect(create_build_cache_metadata).toHaveBeenCalledWith(
			ctx.config,
			ctx.log,
			'abc123', // pre-computed git commit
			undefined, // build_dirs not pre-discovered in clean workspace path
		);
	});
});
