import { describe, test, expect, vi, beforeEach } from 'vitest';
import type { LibraryJson } from '@fuzdev/fuz_util/library_json.ts';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';

// Mock the git helpers so the cache-key/staleness logic can be tested in
// isolation, without depending on the surrounding repo's git state.
vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_current_commit_hash: vi.fn(),
	git_check_workspace: vi.fn(),
	git_workspace_is_clean: vi.fn()
}));

// Mock the filesystem so the cache read/write helpers can be tested without
// touching disk.
vi.mock('node:fs/promises', () => ({
	readFile: vi.fn(),
	writeFile: vi.fn(),
	mkdir: vi.fn()
}));
vi.mock('@fuzdev/fuz_util/fs.js', () => ({
	fs_exists: vi.fn()
}));

import {
	git_current_commit_hash,
	git_check_workspace,
	git_workspace_is_clean
} from '@fuzdev/fuz_util/git.ts';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';

import {
	LIBRARY_CACHE_VERSION,
	library_cache_key,
	library_cache_read,
	library_cache_write
} from '$lib/library_load.ts';

const mocked_commit = vi.mocked(git_current_commit_hash);
const mocked_workspace = vi.mocked(git_check_workspace);
const mocked_is_clean = vi.mocked(git_workspace_is_clean);
const mocked_read = vi.mocked(readFile);
const mocked_write = vi.mocked(writeFile);
const mocked_mkdir = vi.mocked(mkdir);
const mocked_exists = vi.mocked(fs_exists);

// A minimal stand-in for the analyzed library metadata - only the cache
// round-trip is under test here, not the shape of `LibraryJson`.
const fake_library = { name: 'example', version: '1.0.0' } as unknown as LibraryJson;
const fake_package_json = { name: 'example', version: '1.0.0' } as unknown as PackageJson;
const fake_result = { library_json: fake_library, package_json: fake_package_json };

describe('library_cache_key', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mocked_workspace.mockResolvedValue({} as any);
	});

	test('returns the bare commit hash when the workspace is clean', async () => {
		mocked_commit.mockResolvedValue('abc123');
		mocked_is_clean.mockReturnValue(true);

		const key = await library_cache_key('/repo');
		expect(key).toBe('abc123');
	});

	test('returns null when the workspace is dirty (uncacheable)', async () => {
		mocked_commit.mockResolvedValue('abc123');
		mocked_is_clean.mockReturnValue(false);

		const key = await library_cache_key('/repo');
		expect(key).toBeNull();
	});

	test('caches a clean commit but not a dirty one at the same commit', async () => {
		mocked_commit.mockResolvedValue('deadbeef');

		mocked_is_clean.mockReturnValue(true);
		const clean = await library_cache_key('/repo');
		expect(clean).toBe('deadbeef');

		mocked_is_clean.mockReturnValue(false);
		const dirty = await library_cache_key('/repo');
		expect(dirty).toBeNull();
	});

	test('returns null when not a git repo (no commit hash)', async () => {
		mocked_commit.mockResolvedValue(null);

		const key = await library_cache_key('/repo');
		expect(key).toBeNull();
		// Workspace status is not consulted when there's no commit.
		expect(mocked_workspace).not.toHaveBeenCalled();
	});

	test('passes `repo_dir` as cwd to the git helpers', async () => {
		mocked_commit.mockResolvedValue('abc123');
		mocked_is_clean.mockReturnValue(true);

		await library_cache_key('/some/repo');
		expect(mocked_commit).toHaveBeenCalledWith('HEAD', { cwd: '/some/repo' });
		expect(mocked_workspace).toHaveBeenCalledWith({ cwd: '/some/repo' });
	});
});

describe('library_cache_read', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('returns the cached result on a hash match', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue(
			JSON.stringify({ hash: 'abc123', version: LIBRARY_CACHE_VERSION, ...fake_result })
		);

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toEqual(fake_result);
	});

	test('returns null when the cache version is stale', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue(
			JSON.stringify({ hash: 'abc123', version: LIBRARY_CACHE_VERSION - 1, ...fake_result })
		);

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toBeNull();
	});

	// The realistic migration case: a legacy cache written before versioning has a
	// matching `hash` but no `version` field, and must be treated as stale.
	test('returns null for a legacy cache with a matching hash but no version', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue(JSON.stringify({ hash: 'abc123', library_json: fake_library }));

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toBeNull();
	});

	test('returns null and never reads when the cache file is absent', async () => {
		mocked_exists.mockResolvedValue(false);

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toBeNull();
		expect(mocked_read).not.toHaveBeenCalled();
	});

	test('returns null when the cached hash is stale', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue(JSON.stringify({ hash: 'old', ...fake_result }));

		const result = await library_cache_read('/repo/.gro/library.json', 'new');
		expect(result).toBeNull();
	});

	test('returns null when the cache is corrupt (invalid JSON)', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue('not json {');

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toBeNull();
	});

	test('returns null when the read itself throws', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockRejectedValue(new Error('EACCES'));

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toBeNull();
	});
});

describe('library_cache_write', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('creates the parent dir and writes the keyed cache', async () => {
		mocked_mkdir.mockResolvedValue(undefined);
		mocked_write.mockResolvedValue(undefined);

		await library_cache_write('/repo/.gro/library.json', 'abc123', fake_result);

		expect(mocked_mkdir).toHaveBeenCalledWith('/repo/.gro', { recursive: true });
		const [path, contents] = mocked_write.mock.calls[0]!;
		expect(path).toBe('/repo/.gro/library.json');
		expect(JSON.parse(contents as string)).toEqual({
			hash: 'abc123',
			version: LIBRARY_CACHE_VERSION,
			...fake_result
		});
	});

	test('swallows write failures (best effort) and warns', async () => {
		mocked_mkdir.mockResolvedValue(undefined);
		mocked_write.mockRejectedValue(new Error('ENOSPC'));
		const log = { warn: vi.fn(), debug: vi.fn() } as any;

		await expect(
			library_cache_write('/repo/.gro/library.json', 'abc123', fake_result, log)
		).resolves.toBeUndefined();
		expect(log.warn).toHaveBeenCalled();
	});

	test('swallows mkdir failures (best effort)', async () => {
		mocked_mkdir.mockRejectedValue(new Error('EACCES'));

		await expect(
			library_cache_write('/repo/.gro/library.json', 'abc123', fake_result)
		).resolves.toBeUndefined();
		expect(mocked_write).not.toHaveBeenCalled();
	});
});
