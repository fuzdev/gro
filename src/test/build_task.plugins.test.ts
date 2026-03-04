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

describe('build_task plugin lifecycle', () => {
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

	test('runs plugin lifecycle in correct order', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
		const mock_plugins = create_mock_plugins();
		vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);

		const call_order: Array<string> = [];
		mock_plugins.setup.mockImplementation(() => call_order.push('setup'));
		mock_plugins.adapt.mockImplementation(() => call_order.push('adapt'));
		mock_plugins.teardown.mockImplementation(() => call_order.push('teardown'));

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		expect(call_order).toEqual(['setup', 'adapt', 'teardown']);
	});

	test('creates plugins with correct context', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		expect(Plugins.create).toHaveBeenCalledWith(
			expect.objectContaining({
				dev: false,
				watch: false,
				config: ctx.config,
				log: ctx.log,
			}),
		);
	});

	test('calls clean_fs before building', async () => {
		const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
		const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
		const {clean_fs} = vi.mocked(await import('../lib/clean_fs.ts'));

		vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
		vi.mocked(is_build_cache_valid).mockResolvedValue(false);

		const ctx = create_mock_build_task_context();

		await build_task.run(ctx);

		expect(clean_fs).toHaveBeenCalledWith({build_dist: true});
	});

	describe('error handling', () => {
		test('propagates error when setup() throws', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
			const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
			const mock_plugins = create_mock_plugins();
			vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			// setup() throws an error
			const setup_error = new Error('Plugin setup failed');
			mock_plugins.setup.mockRejectedValue(setup_error);

			const ctx = create_mock_build_task_context();

			// Should propagate the error
			await expect(build_task.run(ctx)).rejects.toThrow('Plugin setup failed');

			// Should have attempted setup
			expect(mock_plugins.setup).toHaveBeenCalled();

			// Should NOT call adapt or teardown after setup fails
			expect(mock_plugins.adapt).not.toHaveBeenCalled();
			expect(mock_plugins.teardown).not.toHaveBeenCalled();
		});

		test('propagates error when adapt() throws', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
			const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
			const mock_plugins = create_mock_plugins();
			vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			// adapt() throws an error
			const adapt_error = new Error('Adapter failed');
			mock_plugins.adapt.mockRejectedValue(adapt_error);

			const ctx = create_mock_build_task_context();

			// Should propagate the error
			await expect(build_task.run(ctx)).rejects.toThrow('Adapter failed');

			// Should have completed setup
			expect(mock_plugins.setup).toHaveBeenCalled();

			// Should have attempted adapt
			expect(mock_plugins.adapt).toHaveBeenCalled();

			// Should NOT call teardown after adapt fails
			expect(mock_plugins.teardown).not.toHaveBeenCalled();
		});

		test('propagates error when teardown() throws', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
			const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));
			const mock_plugins = create_mock_plugins();
			vi.mocked(Plugins.create).mockResolvedValue(mock_plugins as any);

			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			// teardown() throws an error
			const teardown_error = new Error('Teardown failed');
			mock_plugins.teardown.mockRejectedValue(teardown_error);

			const ctx = create_mock_build_task_context();

			// Should propagate the error
			await expect(build_task.run(ctx)).rejects.toThrow('Teardown failed');

			// Should have completed setup and adapt
			expect(mock_plugins.setup).toHaveBeenCalled();
			expect(mock_plugins.adapt).toHaveBeenCalled();

			// Should have attempted teardown
			expect(mock_plugins.teardown).toHaveBeenCalled();
		});

		test('propagates error when Plugins.create() throws', async () => {
			const {git_check_clean_workspace} = vi.mocked(await import('@fuzdev/fuz_util/git.js'));
			const {is_build_cache_valid} = vi.mocked(await import('../lib/build_cache.ts'));
			const {Plugins} = vi.mocked(await import('../lib/plugin.ts'));

			vi.mocked(git_check_clean_workspace).mockResolvedValue(null);
			vi.mocked(is_build_cache_valid).mockResolvedValue(false);

			// Plugin creation fails
			const create_error = new Error('Failed to create plugins');
			vi.mocked(Plugins.create).mockRejectedValue(create_error);

			const ctx = create_mock_build_task_context();

			// Should propagate the error
			await expect(build_task.run(ctx)).rejects.toThrow('Failed to create plugins');
		});
	});
});
