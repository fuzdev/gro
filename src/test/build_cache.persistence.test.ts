import {describe, test, expect, vi, beforeEach} from 'vitest';

import {load_build_cache_metadata, save_build_cache_metadata} from '../lib/build_cache.ts';

import {create_mock_logger, create_mock_build_cache_metadata} from './build_cache_test_helpers.ts';

// Mock dependencies
vi.mock('$lib/paths.js', () => ({
	paths: {
		root: './',
		source: './src/',
		lib: './src/lib/',
		build: './.gro/',
		build_dev: './.gro/dev/',
		config: './gro.config.ts',
	},
}));

vi.mock('node:fs/promises', () => ({
	readFile: vi.fn(),
	writeFile: vi.fn(),
	mkdir: vi.fn(),
	rm: vi.fn(),
}));

vi.mock('@fuzdev/fuz_util/fs.js', () => ({
	fs_exists: vi.fn(),
}));

describe('load_build_cache_metadata', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('loads valid metadata file', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		const metadata = create_mock_build_cache_metadata();
		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(JSON.stringify(metadata));

		const result = await load_build_cache_metadata();

		expect(result).toEqual(metadata);
	});

	test('returns null for non-existent file', async () => {
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(false);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('returns null for invalid JSON', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue('invalid json{');

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('returns null for wrong schema version', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		const metadata = create_mock_build_cache_metadata({version: '999'});
		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(JSON.stringify(metadata));

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('deletes cache file on schema version mismatch', async () => {
		const {readFile, rm} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		const metadata = create_mock_build_cache_metadata({version: '999'});
		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(JSON.stringify(metadata));
		vi.mocked(rm).mockResolvedValue(undefined);

		await load_build_cache_metadata();

		expect(rm).toHaveBeenCalledWith('.gro/build.json', {force: true});
	});

	test('deletes cache file on corrupted JSON', async () => {
		const {readFile, rm} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue('invalid json{');
		vi.mocked(rm).mockResolvedValue(undefined);

		await load_build_cache_metadata();

		expect(rm).toHaveBeenCalledWith('.gro/build.json', {force: true});
	});

	test('handles cleanup errors gracefully', async () => {
		const {readFile, rm} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue('invalid json{');
		vi.mocked(rm).mockRejectedValue(new Error('Permission denied'));

		// Should not throw despite cleanup error
		await expect(load_build_cache_metadata()).resolves.not.toThrow();
		expect(await load_build_cache_metadata()).toBeNull();
	});

	test('returns null for empty file', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue('');

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('returns null for valid JSON with wrong version', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(
			JSON.stringify(create_mock_build_cache_metadata({version: 'wrong'})),
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('returns null for truncated JSON file', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		// Simulate truncated write (incomplete JSON)
		const truncated = '{"version":"1","git_commit":"abc123","build_cache_config_hash":"hash","tim';

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(truncated);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('rejects cache with missing required fields (Zod validation)', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(
			JSON.stringify({
				version: '1',
				git_commit: 'abc123',
				// missing build_cache_config_hash
				timestamp: '2025-10-23T12:00:00Z',
				outputs: [],
			}),
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('rejects cache with wrong field types (Zod validation)', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(
			JSON.stringify({
				version: 1, // should be string
				git_commit: 'abc123',
				build_cache_config_hash: 'hash',
				timestamp: '2025-10-23T12:00:00Z',
				outputs: [],
			}),
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('rejects cache with unexpected extra fields (strictObject)', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(
			JSON.stringify({...create_mock_build_cache_metadata(), unexpected_field: 'bad'}),
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});

	test('rejects cache with invalid outputs type', async () => {
		const {readFile} = await import('node:fs/promises');
		const {fs_exists} = await import('@fuzdev/fuz_util/fs.js');

		vi.mocked(fs_exists).mockResolvedValue(true);
		vi.mocked(readFile).mockResolvedValue(
			JSON.stringify({...create_mock_build_cache_metadata(), outputs: 'not-an-array'}),
		);

		const result = await load_build_cache_metadata();

		expect(result).toBeNull();
	});
});

describe('save_build_cache_metadata', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('writes metadata to correct path', async () => {
		const {writeFile, mkdir} = await import('node:fs/promises');

		const metadata = create_mock_build_cache_metadata();
		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(writeFile).mockResolvedValue(undefined);

		await save_build_cache_metadata(metadata);

		expect(mkdir).toHaveBeenCalledWith('./.gro/', {recursive: true});
		expect(writeFile).toHaveBeenCalledWith('.gro/build.json', expect.any(String), 'utf-8');
	});

	test('uses proper JSON formatting with tabs', async () => {
		const {writeFile, mkdir} = await import('node:fs/promises');

		const metadata = create_mock_build_cache_metadata();
		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(writeFile).mockResolvedValue(undefined);

		await save_build_cache_metadata(metadata);

		const written_content = vi.mocked(writeFile).mock.calls[0]![1] as string;
		expect(written_content).toContain('\t');
		expect(JSON.parse(written_content)).toEqual(metadata);
	});

	test('logs warning on write error', async () => {
		const {writeFile, mkdir} = await import('node:fs/promises');

		const metadata = create_mock_build_cache_metadata();
		const log = create_mock_logger();

		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(writeFile).mockRejectedValue(new Error('ENOSPC: no space left on device'));

		await save_build_cache_metadata(metadata, log);

		expect(log.warn).toHaveBeenCalledWith(
			expect.stringContaining('Failed to save build cache'),
			expect.stringContaining('no space left on device'),
		);
	});

	test('does not throw on write error', async () => {
		const {writeFile, mkdir} = await import('node:fs/promises');

		const metadata = create_mock_build_cache_metadata();

		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(writeFile).mockRejectedValue(new Error('EACCES: permission denied'));

		// Should not throw despite write error
		await expect(save_build_cache_metadata(metadata)).resolves.not.toThrow();
	});

	test('handles write error without logger', async () => {
		const {writeFile, mkdir} = await import('node:fs/promises');

		const metadata = create_mock_build_cache_metadata();

		vi.mocked(mkdir).mockResolvedValue(undefined);
		vi.mocked(writeFile).mockRejectedValue(new Error('Write failed'));

		// Should not throw even without logger (log?.warn is safe)
		await expect(save_build_cache_metadata(metadata)).resolves.not.toThrow();
	});

	test('does not throw when mkdir fails', async () => {
		const {mkdir} = await import('node:fs/promises');

		const metadata = create_mock_build_cache_metadata();
		const log = create_mock_logger();

		vi.mocked(mkdir).mockRejectedValue(new Error('EACCES: permission denied'));

		await expect(save_build_cache_metadata(metadata, log)).resolves.not.toThrow();
		expect(log.warn).toHaveBeenCalledWith(
			expect.stringContaining('Failed to save build cache'),
			expect.stringContaining('permission denied'),
		);
	});
});
