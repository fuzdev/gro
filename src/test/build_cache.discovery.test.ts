import { describe, test, expect, vi, beforeEach, assert } from 'vitest';

import { discover_build_output_dirs, collect_build_outputs } from '#lib/build_cache.ts';
import { is_fs_error } from '#lib/fs_error.ts';

import { create_mock_project_fs_paths, mock_tree } from './fs_mocks.ts';

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn()
}));

const fs_paths = create_mock_project_fs_paths();

describe('discover_build_output_dirs', () => {
	beforeEach(() => {
		vi.resetAllMocks();
	});

	test('returns all existing build directories', async () => {
		mock_tree(fs_paths.project, {
			'build/': '',
			'dist/': '',
			'dist_worker/': '',
			'dist_artifacts/': '',
			'node_modules/': '',
			'src/': ''
		});

		const result = await discover_build_output_dirs(fs_paths);

		expect(result).toEqual(['build', 'dist', 'dist_artifacts', 'dist_worker']);
	});

	test('returns only directories that exist', async () => {
		mock_tree(fs_paths.project, { 'dist/': '', 'dist_artifacts/': '', 'other/': '' });

		const result = await discover_build_output_dirs(fs_paths);

		expect(result).toEqual(['dist', 'dist_artifacts']);
	});

	test('returns empty array when no build directories exist', async () => {
		mock_tree(fs_paths.project, {});

		const result = await discover_build_output_dirs(fs_paths);

		expect(result).toEqual([]);
	});

	test('filters out non-directory dist_ entries', async () => {
		mock_tree(fs_paths.project, {
			'build/': '',
			'dist_artifacts/': '',
			'dist_readme.md': 'file, not directory'
		});

		const result = await discover_build_output_dirs(fs_paths);

		expect(result).toEqual(['build', 'dist_artifacts']);
	});

	test('follows a dist_ symlink to a directory', async () => {
		mock_tree(fs_paths.project, {
			'real_dir/': '',
			real_file: '',
			dist_linked: { target: 'real_dir' },
			dist_file_link: { target: 'real_file' },
			dist_dangling: { target: 'missing' }
		});

		const result = await discover_build_output_dirs(fs_paths);

		expect(result).toEqual(['dist_linked']);
	});

	test('a listing error is fatal', async () => {
		const project = fs_paths.project;
		mock_tree(project, { 'build/': '' });
		project.list.mockImplementation(() => {
			throw new Error('EACCES: permission denied');
		});

		await expect(discover_build_output_dirs(fs_paths)).rejects.toThrow('EACCES: permission denied');
	});

	test('skips a dist_ entry that vanished during iteration', async () => {
		const project = fs_paths.project;
		mock_tree(project, { 'dist_ok/': '' });
		const listing = async function* () {
			yield { name: 'dist_vanished', kind: 'symlink' as const };
			yield { name: 'dist_ok', kind: 'directory' as const };
		};
		project.list.mockImplementation(() => listing());

		const result = await discover_build_output_dirs(fs_paths);

		expect(result).toEqual(['dist_ok']);
	});
});

describe('collect_build_outputs', () => {
	beforeEach(() => {
		vi.resetAllMocks();
	});

	test('returns empty array for empty dirs array', async () => {
		const result = await collect_build_outputs(fs_paths, []);

		expect(result).toEqual([]);
	});

	test('hashes all files in build directory, sorted by name', async () => {
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');
		mock_tree(fs_paths.project, {
			'build/index.html': { size: 1024 },
			'build/bundle.js': { size: 1024 }
		});

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const result = await collect_build_outputs(fs_paths, ['build']);

		expect(result).toEqual([
			{ path: 'build/bundle.js', hash: 'hash1', size: 1024, mtime: 1729512000000, mode: 0o644 },
			{ path: 'build/index.html', hash: 'hash2', size: 1024, mtime: 1729512000000, mode: 0o644 }
		]);
	});

	test('skips build.json file', async () => {
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');
		mock_tree(fs_paths.project, { 'build/build.json': '{}', 'build/index.html': '' });
		vi.mocked(hash_blake3).mockReturnValue('hash');

		const result = await collect_build_outputs(fs_paths, ['build']);

		expect(result.map((o) => o.path)).toEqual(['build/index.html']);
	});

	test('skips symlinks', async () => {
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');
		mock_tree(fs_paths.project, {
			'build/index.html': '',
			'build/link.html': { target: 'build/index.html' }
		});
		vi.mocked(hash_blake3).mockReturnValue('hash');

		const result = await collect_build_outputs(fs_paths, ['build']);

		expect(result.map((o) => o.path)).toEqual(['build/index.html']);
	});

	test('returns empty array for non-existent directory', async () => {
		mock_tree(fs_paths.project, {});

		const result = await collect_build_outputs(fs_paths, ['build']);

		expect(result).toEqual([]);
	});

	test('hashes the contents of each file', async () => {
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');
		mock_tree(fs_paths.project, { 'build/a.js': 'aaa', 'build/b.js': 'bbb' });
		vi.mocked(hash_blake3).mockImplementation((data) =>
			new TextDecoder().decode(data as Uint8Array)
		);

		const result = await collect_build_outputs(fs_paths, ['build']);

		expect(result.map((o) => o.hash)).toEqual(['aaa', 'bbb']);
		expect(result.map((o) => o.size)).toEqual([3, 3]);
	});

	test('hashes files from multiple directories', async () => {
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');
		mock_tree(fs_paths.project, {
			'build/index.html': '',
			'dist/index.js': '',
			'dist_artifacts/server.js': ''
		});

		let hash_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => `hash${++hash_count}`);

		const result = await collect_build_outputs(fs_paths, ['build', 'dist', 'dist_artifacts']);

		expect(result.map(({ path, hash }) => ({ path, hash }))).toEqual([
			{ path: 'build/index.html', hash: 'hash1' },
			{ path: 'dist/index.js', hash: 'hash2' },
			{ path: 'dist_artifacts/server.js', hash: 'hash3' }
		]);
	});

	test('hashes files in deeply nested directories', async () => {
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');
		const project = fs_paths.project;
		mock_tree(project, { 'build/assets/js/vendor/libs/foo.js': '' });
		vi.mocked(hash_blake3).mockReturnValue('deep_hash');

		const result = await collect_build_outputs(fs_paths, ['build']);

		expect(result).toEqual([
			expect.objectContaining({ path: 'build/assets/js/vendor/libs/foo.js', hash: 'deep_hash' })
		]);
		// each directory is listed beneath the one handle, never opened by path
		expect(project.list).toHaveBeenCalledWith('build/assets/js/vendor/libs');
	});

	test('a file the stat misses but the read finds, replaced between them, throws `io_error`', async () => {
		const project = fs_paths.project;
		mock_tree(project, { 'build/index.html': '<html></html>' });
		// found by the listing and the read, missed by the stat in between
		project.stat.mockImplementation(async () => null);

		const error = await collect_build_outputs(fs_paths, ['build']).catch((e: unknown) => e);

		assert.ok(is_fs_error(error, 'io_error'), String(error));
		assert.include(String(error), 'build/index.html');
	});
});
