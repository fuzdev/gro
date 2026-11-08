import { describe, test, expect, vi, beforeEach } from 'vitest';

import { load_build_cache_metadata, save_build_cache_metadata } from '#lib/build_cache.ts';
import { GRO_DIR_MARKER_CONTENT, GRO_DIR_MARKER_FILENAME } from '#lib/internal/gro_dir.ts';

import {
	create_mock_logger,
	create_mock_build_cache_metadata
} from './build_cache_test_helpers.ts';
import { get_mock_project } from './fs_mocks.ts';

// Mock dependencies
vi.mock('#lib/paths.ts', () => ({
	paths: {
		root: './',
		source: './src/',
		lib: './src/lib/',
		build: './.gro/'
	}
}));

vi.mock('#lib/fs.ts', async () => (await import('./fs_mocks.ts')).create_mock_fs_module());

describe('load_build_cache_metadata', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		// an invalid cache file is removed
		const { remove } = await get_mock_project();
		vi.mocked(remove).mockReset().mockResolvedValue(undefined);
	});

	test('loads valid metadata file', async () => {
		const { read_text_if_exists } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata();
		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));

		const result = await load_build_cache_metadata();

		expect(result).toEqual(metadata);
	});

	test('returns null for non-existent file', async () => {
		const { read_text_if_exists } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue(null);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('returns null for invalid JSON', async () => {
		const { read_text_if_exists } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue('invalid json{');

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('returns null for wrong schema version', async () => {
		const { read_text_if_exists } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata({ version: '999' });
		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('deletes cache file on schema version mismatch', async () => {
		const { read_text_if_exists, remove } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata({ version: '999' });
		vi.mocked(read_text_if_exists).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(remove).mockResolvedValue(undefined);

		await load_build_cache_metadata();

		expect(remove).toHaveBeenCalledWith('.gro/build.json', { missing_ok: true });
	});

	test('deletes cache file on corrupted JSON', async () => {
		const { read_text_if_exists, remove } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue('invalid json{');
		vi.mocked(remove).mockResolvedValue(undefined);

		await load_build_cache_metadata();

		expect(remove).toHaveBeenCalledWith('.gro/build.json', { missing_ok: true });
	});

	test('leaves a cache file it cannot read in place', async () => {
		const { read_text_if_exists, remove } = await get_mock_project();
		const { FsError } = await import('#lib/fs.ts');

		vi.mocked(read_text_if_exists).mockRejectedValue(
			new FsError('permission_denied', '.gro/build.json')
		);

		expect(await load_build_cache_metadata()).toBeNull();
		expect(remove).not.toHaveBeenCalled();
	});

	test('handles cleanup errors gracefully', async () => {
		const { read_text_if_exists, remove } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue('invalid json{');
		vi.mocked(remove).mockRejectedValue(new Error('Permission denied'));

		// Should not throw despite cleanup error
		await expect(load_build_cache_metadata()).resolves.not.toThrow();
		expect(await load_build_cache_metadata()).toBeNull();
	});

	test('returns null for empty file', async () => {
		const { read_text_if_exists } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue('');

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('returns null for valid JSON with wrong version', async () => {
		const { read_text_if_exists } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue(
			JSON.stringify(create_mock_build_cache_metadata({ version: 'wrong' }))
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('returns null for truncated JSON file', async () => {
		const { read_text_if_exists } = await get_mock_project();

		// Simulate truncated write (incomplete JSON)
		const truncated = '{"version":"1","git_commit":"abc123","build_cache_config_hash":"hash","tim';

		vi.mocked(read_text_if_exists).mockResolvedValue(truncated);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('rejects cache with missing required fields (schema validation)', async () => {
		const { read_text_if_exists } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue(
			JSON.stringify({
				version: '1',
				git_commit: 'abc123',
				// missing build_cache_config_hash
				timestamp: '2025-10-23T12:00:00Z',
				outputs: []
			})
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('rejects cache with wrong field types (schema validation)', async () => {
		const { read_text_if_exists } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue(
			JSON.stringify({
				version: 1, // should be string
				git_commit: 'abc123',
				build_cache_config_hash: 'hash',
				timestamp: '2025-10-23T12:00:00Z',
				outputs: []
			})
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('rejects cache with unexpected extra fields (strict object schema)', async () => {
		const { read_text_if_exists } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue(
			JSON.stringify({ ...create_mock_build_cache_metadata(), unexpected_field: 'bad' })
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('rejects cache with invalid outputs type', async () => {
		const { read_text_if_exists } = await get_mock_project();

		vi.mocked(read_text_if_exists).mockResolvedValue(
			JSON.stringify({ ...create_mock_build_cache_metadata(), outputs: 'not-an-array' })
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});
});

describe('save_build_cache_metadata', () => {
	beforeEach(async () => {
		vi.clearAllMocks();
		// no `.gro/` yet, so it's made with its marker
		const { stat } = await get_mock_project();
		vi.mocked(stat).mockReset().mockResolvedValue(null);
	});

	test('writes metadata to correct path, making `.gro/` with its marker', async () => {
		const { write, mkdir } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata();
		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(write).mockResolvedValue(undefined);

		await save_build_cache_metadata(metadata);

		expect(mkdir).toHaveBeenCalledWith('.gro');
		expect(write).toHaveBeenCalledWith('.gro/' + GRO_DIR_MARKER_FILENAME, GRO_DIR_MARKER_CONTENT, {
			create_new: true
		});
		expect(write).toHaveBeenCalledWith('.gro/build.json', expect.any(String));
	});

	test('uses proper JSON formatting with tabs', async () => {
		const { write, mkdir } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata();
		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(write).mockResolvedValue(undefined);

		await save_build_cache_metadata(metadata);

		const written_content = vi
			.mocked(write)
			.mock.calls.find(([path]) => path === '.gro/build.json')![1] as string;
		expect(written_content).toContain('\t');
		expect(JSON.parse(written_content)).toEqual(metadata);
	});

	test('logs warning on write error', async () => {
		const { write, mkdir } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata();
		const log = create_mock_logger();

		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(write).mockRejectedValue(new Error('ENOSPC: no space left on device'));

		await save_build_cache_metadata(metadata, log);

		expect(log.warn).toHaveBeenCalledWith(
			expect.stringContaining('Failed to save build cache'),
			expect.stringContaining('no space left on device')
		);
	});

	test('does not throw on write error', async () => {
		const { write, mkdir } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata();

		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(write).mockRejectedValue(new Error('EACCES: permission denied'));

		// Should not throw despite write error
		await expect(save_build_cache_metadata(metadata)).resolves.not.toThrow();
	});

	test('handles write error without logger', async () => {
		const { write, mkdir } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata();

		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(write).mockRejectedValue(new Error('Write failed'));

		// Should not throw even without logger (log?.warn is safe)
		await expect(save_build_cache_metadata(metadata)).resolves.not.toThrow();
	});

	test('does not throw when mkdir fails', async () => {
		const { mkdir } = await get_mock_project();

		const metadata = create_mock_build_cache_metadata();
		const log = create_mock_logger();

		vi.mocked(mkdir).mockRejectedValue(new Error('EACCES: permission denied'));

		await expect(save_build_cache_metadata(metadata, log)).resolves.not.toThrow();
		expect(log.warn).toHaveBeenCalledWith(
			expect.stringContaining('Failed to save build cache'),
			expect.stringContaining('permission denied')
		);
	});
});
