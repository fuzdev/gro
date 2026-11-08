import { describe, test, expect, vi, beforeEach } from 'vitest';

import { validate_build_cache } from '#lib/build_cache.ts';

import {
	create_mock_build_cache_metadata,
	create_mock_output_entry
} from './build_cache_test_helpers.ts';
import { get_mock_project, to_mock_file_info } from './fs_mocks.ts';

// Mock dependencies
vi.mock('node:fs/promises', async (import_original) => {
	// only the real `realpath` and `readlink`, which `fs_open_path` resolves paths with
	const { realpath, readlink } = await import_original<typeof import('node:fs/promises')>();
	return {
		realpath,
		readlink,
		readFile: vi.fn(),
		writeFile: vi.fn(),
		mkdir: vi.fn(),
		rm: vi.fn(),
		stat: vi.fn(),
		readdir: vi.fn()
	};
});

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

vi.mock('@fuzdev/fuz_util/hash_blake3.js', () => ({
	hash_blake3: vi.fn()
}));

describe('validate_build_cache', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns true when all output files match hashes and sizes', async () => {
		const { stat: project_stat } = await get_mock_project();
		const { readFile } = vi.mocked(await import('node:fs/promises'));
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({
			outputs: [
				create_mock_output_entry('build/index.html', { hash: 'hash1', size: 1024 }),
				create_mock_output_entry('build/bundle.js', { hash: 'hash2', size: 2048 })
			]
		});

		vi.mocked(project_stat).mockImplementation(async (path) =>
			to_mock_file_info(path === 'build/index.html' ? 1024 : 2048)
		);
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));

		let call_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => {
			call_count++;
			return call_count === 1 ? 'hash1' : 'hash2';
		});

		const result = await validate_build_cache(metadata);

		expect(result).toBe(true);
	});

	test('returns false when output file is missing', async () => {
		const { stat: project_stat } = await get_mock_project();
		const { readFile } = vi.mocked(await import('node:fs/promises'));
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({
			outputs: [create_mock_output_entry('build/index.html')]
		});

		vi.mocked(project_stat).mockResolvedValue(null);

		const result = await validate_build_cache(metadata);

		expect(result).toBe(false);
		// should not attempt to hash missing files
		expect(readFile).not.toHaveBeenCalled();
		expect(hash_blake3).not.toHaveBeenCalled();
	});

	test('returns false when output file size differs (fast path)', async () => {
		const { stat: project_stat } = await get_mock_project();
		const { readFile } = vi.mocked(await import('node:fs/promises'));
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({
			outputs: [create_mock_output_entry('build/index.html', { hash: 'expected_hash', size: 1024 })]
		});

		vi.mocked(project_stat).mockResolvedValue(to_mock_file_info(2048)); // Different size

		const result = await validate_build_cache(metadata);

		expect(result).toBe(false);
		// fast path: size mismatch skips expensive hashing
		expect(readFile).not.toHaveBeenCalled();
		expect(hash_blake3).not.toHaveBeenCalled();
	});

	test('returns false when output file hash does not match', async () => {
		const { stat: project_stat } = await get_mock_project();
		const { readFile } = vi.mocked(await import('node:fs/promises'));
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({
			outputs: [create_mock_output_entry('build/index.html', { hash: 'expected_hash' })]
		});

		vi.mocked(project_stat).mockResolvedValue(to_mock_file_info());
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));
		vi.mocked(hash_blake3).mockReturnValue('different_hash');

		const result = await validate_build_cache(metadata);

		expect(result).toBe(false);
	});

	test('returns false when some files exist but others are missing', async () => {
		const { stat: project_stat } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata({
			outputs: [
				create_mock_output_entry('build/index.html', { hash: 'hash1' }),
				create_mock_output_entry('build/missing.js', { hash: 'hash2', size: 2048 }),
				create_mock_output_entry('build/another.css', { hash: 'hash3', size: 512 })
			]
		});

		vi.mocked(project_stat).mockImplementation(async (path) =>
			path === 'build/index.html' ? to_mock_file_info() : null
		);

		const result = await validate_build_cache(metadata);

		expect(result).toBe(false);
	});

	test('returns false when parallel hash validation has mixed results', async () => {
		const { stat: project_stat } = await get_mock_project();
		const { readFile } = vi.mocked(await import('node:fs/promises'));
		const { hash_blake3 } = await import('@fuzdev/fuz_util/hash_blake3.ts');

		const metadata = create_mock_build_cache_metadata({
			outputs: [
				create_mock_output_entry('build/file1.js', { hash: 'correct_hash' }),
				create_mock_output_entry('build/file2.js', { hash: 'correct_hash' }),
				create_mock_output_entry('build/file3.js', { hash: 'correct_hash' })
			]
		});

		vi.mocked(project_stat).mockResolvedValue(to_mock_file_info());
		vi.mocked(readFile).mockResolvedValue(Buffer.from('content'));

		let call_count = 0;
		vi.mocked(hash_blake3).mockImplementation(() => {
			call_count++;
			return call_count <= 2 ? 'correct_hash' : 'wrong_hash';
		});

		const result = await validate_build_cache(metadata);

		expect(result).toBe(false);
	});

	test('returns false when file is deleted between size check and hash validation', async () => {
		const { stat: project_stat } = await get_mock_project();
		const { readFile } = vi.mocked(await import('node:fs/promises'));

		const metadata = create_mock_build_cache_metadata({
			outputs: [create_mock_output_entry('build/index.html', { size: 1024 })]
		});

		vi.mocked(project_stat).mockResolvedValue(to_mock_file_info());

		vi.mocked(readFile).mockImplementation(() => {
			return Promise.reject(new Error('ENOENT: no such file or directory'));
		});

		const result = await validate_build_cache(metadata);

		expect(result).toBe(false);
	});

	test('returns false when an output path fails with an FsError', async () => {
		const { stat: project_stat } = await get_mock_project();
		const { FsError } = await import('#lib/fs.ts');

		const metadata = create_mock_build_cache_metadata({
			outputs: [create_mock_output_entry('../outside.js')]
		});

		vi.mocked(project_stat).mockRejectedValue(new FsError('outside_root', '../outside.js'));
		expect(await validate_build_cache(metadata)).toBe(false);

		vi.mocked(project_stat).mockRejectedValue(new FsError('permission_denied', 'build/x.js'));
		expect(await validate_build_cache(metadata)).toBe(false);
	});

	test('rethrows an error that is not an FsError', async () => {
		const { stat: project_stat } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata({
			outputs: [create_mock_output_entry('build/index.html')]
		});

		vi.mocked(project_stat).mockRejectedValue(new TypeError('bug'));
		await expect(validate_build_cache(metadata)).rejects.toThrow('bug');
	});

	test('returns true when metadata has no outputs', async () => {
		const metadata = create_mock_build_cache_metadata({ outputs: [] });

		const result = await validate_build_cache(metadata);

		expect(result).toBe(true);
	});

	test('returns false when file becomes inaccessible during hash validation', async () => {
		const { stat: project_stat } = await get_mock_project();
		const { readFile } = vi.mocked(await import('node:fs/promises'));

		const metadata = create_mock_build_cache_metadata({
			outputs: [
				create_mock_output_entry('build/index.html', { hash: 'hash1', size: 1024 }),
				create_mock_output_entry('build/bundle.js', { hash: 'hash2', size: 2048 })
			]
		});

		vi.mocked(project_stat).mockImplementation(async (path) =>
			to_mock_file_info(path === 'build/index.html' ? 1024 : 2048)
		);

		vi.mocked(readFile).mockImplementation((path: any) => {
			if (String(path) === 'build/index.html') {
				return Promise.resolve(Buffer.from('content'));
			}
			return Promise.reject(new Error('EACCES: permission denied'));
		});

		const result = await validate_build_cache(metadata);

		expect(result).toBe(false);
	});
});
