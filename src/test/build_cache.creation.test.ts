import { describe, test, expect, vi, beforeEach } from 'vitest';

import { BUILD_CACHE_VERSION, create_build_cache_metadata } from '#lib/build_cache.ts';

import {
	create_mock_logger,
	create_mock_config,
	MOCK_BUILD_CACHE_DEPENDENCIES
} from './build_cache_test_helpers.ts';
import { get_mock_project, mock_tree } from './fs_mocks.ts';

// Mock dependencies
vi.mock('#lib/build_cache_dependencies.ts', () => ({
	load_build_cache_dependencies: vi.fn(async () => ({
		gro_version: '1.0.0',
		node_version: 'v24.0.0',
		package_lock_hash: 'lock123'
	}))
}));

vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_current_commit_hash: vi.fn()
}));

// Mock the fs host module
vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn()
}));

describe('create_build_cache_metadata', () => {
	beforeEach(async () => {
		vi.resetAllMocks();
		mock_tree(await get_mock_project(), {});
	});

	test('creates complete metadata object', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		expect(result).toMatchObject({
			version: BUILD_CACHE_VERSION,
			git_commit: 'abc123',
			...MOCK_BUILD_CACHE_DEPENDENCIES
		});
		expect(result.timestamp).toBeTruthy();
		expect(new Date(result.timestamp).getTime()).toBeGreaterThan(0);
	});

	test('creates metadata with actual build outputs', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		mock_tree(await get_mock_project(), {
			'build/index.html': { size: 1024 },
			'build/bundle.js': { size: 1024 }
		});

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		expect(result.outputs).toHaveLength(2);
		expect(result.outputs[0]).toMatchObject({
			path: 'build/bundle.js',
			hash: 'hash2', // hash1 is for config hash
			size: 1024
		});
		expect(result.outputs[1]).toMatchObject({
			path: 'build/index.html',
			hash: 'hash3',
			size: 1024
		});
	});

	test('creates metadata with multiple build directories', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		mock_tree(await get_mock_project(), {
			'build/app.js': '',
			'dist/lib.js': '',
			'dist_artifacts/server.js': '',
			'src/': '',
			'node_modules/': ''
		});

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should have files from all three directories
		expect(result.outputs.map((o) => o.path)).toEqual([
			'build/app.js',
			'dist/lib.js',
			'dist_artifacts/server.js'
		]);
	});

	test('handles empty build directories', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		mock_tree(await get_mock_project(), { 'build/': '' });
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should succeed with empty outputs
		expect(result.outputs).toEqual([]);
		expect(result.git_commit).toBe('abc123');
	});

	test('creates metadata with deeply nested file structures', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		mock_tree(await get_mock_project(), {
			'build/assets/js/lib/utils/helper.js': { size: 256 }
		});
		vi.mocked(hash_blake3).mockReturnValue('deep_hash');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should find the deeply nested file
		expect(result.outputs).toHaveLength(1);
		expect(result.outputs[0]!.path).toBe('build/assets/js/lib/utils/helper.js');
		expect(result.outputs[0]!.size).toBe(256);
	});

	test('handles build directories with many files', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		const tree: Record<string, string> = {};
		for (let i = 0; i < 15; i++) tree[`build/file${i}.js`] = '';
		mock_tree(await get_mock_project(), tree);

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should hash all 15 files in parallel
		expect(result.outputs).toHaveLength(15);
		// Verify all files are present
		for (let i = 0; i < 15; i++) {
			expect(result.outputs.find((o) => o.path === `build/file${i}.js`)).toBeDefined();
		}
	});

	test('creates metadata with null git commit', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue(null);
		vi.mocked(hash_blake3).mockReturnValue('hash123');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should have null git commit
		expect(result.git_commit).toBeNull();
		expect(result.version).toBe(BUILD_CACHE_VERSION);
		expect(result.timestamp).toBeTruthy();
		expect(log.warn).toHaveBeenCalledWith(expect.stringContaining('Not in a git repository'));
	});

	test('uses pre-passed build_dirs and skips discovery', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');
		const project = await get_mock_project();

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		mock_tree(project, { 'custom_out/app.js': '', 'build/other.js': '' });
		vi.mocked(hash_blake3).mockReturnValue('hash_val');

		const config = await create_mock_config();
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log, 'abc123', ['custom_out']);

		expect(result.outputs).toHaveLength(1);
		expect(result.outputs[0]!.path).toBe('custom_out/app.js');
		// listing the project is for discovery, which pre-passed build_dirs skip
		expect(project.list).not.toHaveBeenCalledWith('');
	});

	test('includes correct build_cache_config_hash', async () => {
		const { git_current_commit_hash } = await import('@fuzdev/fuz_util/git.ts');
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		vi.mocked(git_current_commit_hash).mockResolvedValue('abc123');
		vi.mocked(hash_blake3).mockReturnValue('custom_config_hash');

		const config = await create_mock_config({
			build_cache_config: {
				api_endpoint: 'https://api.fuz.dev',
				feature_flags: { experimental: true }
			}
		});
		const log = create_mock_logger();

		const result = await create_build_cache_metadata(config, log);

		// Should include the hashed config
		expect(result.build_cache_config_hash).toBe('custom_config_hash');
		expect(result.git_commit).toBe('abc123');
	});
});
