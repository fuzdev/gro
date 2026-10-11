import { assert, describe, test, expect, vi, beforeEach } from 'vitest';
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

// Mock the analyzer and the `LibraryJson` assembly so `library_load_from_repo`
// can be tested end-to-end without a real svelte-docinfo analysis.
vi.mock('svelte-docinfo', () => ({
	analyzeFromFiles: vi.fn()
}));
vi.mock('@fuzdev/fuz_util/library_json.js', () => ({
	library_json_from_modules: vi.fn()
}));

import {
	git_current_commit_hash,
	git_check_workspace,
	git_workspace_is_clean
} from '@fuzdev/fuz_util/git.ts';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';
import { analyzeFromFiles } from 'svelte-docinfo';
import { library_json_from_modules, type LibraryJson } from '@fuzdev/fuz_util/library_json.ts';

import {
	LIBRARY_CACHE_VERSION,
	SVELTE_DOCINFO_VERSION,
	library_cache_key,
	library_cache_read,
	library_cache_write,
	library_load_from_repo
} from '#lib/library_load.ts';

const mocked_commit = vi.mocked(git_current_commit_hash);
const mocked_workspace = vi.mocked(git_check_workspace);
const mocked_is_clean = vi.mocked(git_workspace_is_clean);
const mocked_read = vi.mocked(readFile);
const mocked_write = vi.mocked(writeFile);
const mocked_mkdir = vi.mocked(mkdir);
const mocked_exists = vi.mocked(fs_exists);
const mocked_analyze = vi.mocked(analyzeFromFiles);
const mocked_library_json_from_modules = vi.mocked(library_json_from_modules);

// A minimal stand-in for the analyzed library metadata - only the cache
// round-trip is under test here, not the shape of `LibraryJson`.
const fake_library = { name: 'example', version: '1.0.0' } as unknown as LibraryJson;
const fake_package_json = { name: 'example', version: '1.0.0' } as unknown as PackageJson;
const fake_result = { library_json: fake_library, package_json: fake_package_json };

/**
 * Serves `files` (absolute path to contents) through the mocked `readFile`,
 * rejecting any other path the way a missing file does.
 */
const mock_files = (files: Record<string, string>): void => {
	mocked_read.mockImplementation(async (path) => {
		const contents = files[path as string];
		if (contents === undefined) throw Object.assign(new Error('ENOENT'), { code: 'ENOENT' });
		return contents;
	});
};

/**
 * Makes the mocked `fs_exists` report exactly `paths` as present.
 */
const mock_existing = (paths: Array<string>): void => {
	mocked_exists.mockImplementation(async (path) => paths.includes(path));
};

const KIT_TSCONFIG = '/repo/node_modules/$app/tsconfig.json';

const create_mock_log = (): any => {
	const child_log = { debug: vi.fn(), warn: vi.fn(), error: vi.fn() };
	return {
		info: vi.fn(),
		warn: vi.fn(),
		debug: vi.fn(),
		child: vi.fn(() => child_log),
		child_log
	};
};

describe('SVELTE_DOCINFO_VERSION', () => {
	test('resolves the installed svelte-docinfo version', () => {
		expect(typeof SVELTE_DOCINFO_VERSION).toBe('string');
		expect(SVELTE_DOCINFO_VERSION.length).toBeGreaterThan(0);
	});
});

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
			JSON.stringify({
				hash: 'abc123',
				version: LIBRARY_CACHE_VERSION,
				svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
				...fake_result
			})
		);

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toEqual(fake_result);
	});

	test('returns null when the cache version is stale', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue(
			JSON.stringify({
				hash: 'abc123',
				version: LIBRARY_CACHE_VERSION - 1,
				svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
				...fake_result
			})
		);

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toBeNull();
	});

	// The analyzer-side upgrade case: the analyzed repo's commit (the cache key)
	// doesn't move when the *analyzer's* svelte-docinfo changes output shape, so
	// the stamped version is what invalidates.
	test('returns null when the cache was analyzed by a different svelte-docinfo version', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue(
			JSON.stringify({
				hash: 'abc123',
				version: LIBRARY_CACHE_VERSION,
				svelte_docinfo_version: '0.0.0-other',
				...fake_result
			})
		);

		const result = await library_cache_read('/repo/.gro/library.json', 'abc123');
		expect(result).toBeNull();
	});

	test('returns null when the cache has no svelte-docinfo version stamp', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue(
			JSON.stringify({ hash: 'abc123', version: LIBRARY_CACHE_VERSION, ...fake_result })
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
			svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
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

describe('library_load_from_repo', () => {
	const valid_package_json_contents = JSON.stringify({ name: 'example', version: '1.0.0' });
	const fresh_cache_contents = JSON.stringify({
		hash: 'abc123',
		version: LIBRARY_CACHE_VERSION,
		svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
		...fake_result
	});

	beforeEach(() => {
		vi.clearAllMocks();
		mocked_commit.mockResolvedValue('abc123');
		mocked_workspace.mockResolvedValue({} as any);
		mocked_is_clean.mockReturnValue(true);
		mocked_analyze.mockResolvedValue({ modules: [] } as any);
		mocked_library_json_from_modules.mockReturnValue(fake_library);
		mocked_mkdir.mockResolvedValue(undefined);
		mocked_write.mockResolvedValue(undefined);
	});

	test('returns the cached result without analyzing on a cache hit', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockResolvedValue(fresh_cache_contents);

		const result = await library_load_from_repo('/repo');
		expect(result).toEqual(fake_result);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(mocked_write).not.toHaveBeenCalled();
	});

	// The end-to-end behavior of the version stamps: a cache analyzed by a
	// different svelte-docinfo re-analyzes and rewrites with the current stamp.
	test('re-analyzes and rewrites when the svelte-docinfo stamp is stale', async () => {
		mocked_exists.mockResolvedValue(true);
		mocked_read.mockImplementation(async (path) =>
			(path as string).endsWith('library.json')
				? JSON.stringify({
						hash: 'abc123',
						version: LIBRARY_CACHE_VERSION,
						svelte_docinfo_version: '0.0.0-other',
						...fake_result
					})
				: valid_package_json_contents
		);

		const result = await library_load_from_repo('/repo');
		expect(mocked_analyze).toHaveBeenCalledWith({ projectRoot: '/repo' });
		expect(result.library_json).toBe(fake_library);
		const [path, contents] = mocked_write.mock.calls[0]!;
		expect(path).toBe('/repo/.gro/library.json');
		expect(JSON.parse(contents as string).svelte_docinfo_version).toBe(SVELTE_DOCINFO_VERSION);
	});

	test('`cache: false` skips the cache read but still writes the result', async () => {
		mocked_read.mockResolvedValue(valid_package_json_contents);

		await library_load_from_repo('/repo', { cache: false });
		expect(mocked_exists).not.toHaveBeenCalled();
		expect(mocked_analyze).toHaveBeenCalled();
		expect(mocked_write).toHaveBeenCalled();
	});

	test('a dirty tree analyzes fresh and skips both cache read and write', async () => {
		mocked_is_clean.mockReturnValue(false);
		mocked_read.mockResolvedValue(valid_package_json_contents);

		await library_load_from_repo('/repo');
		expect(mocked_exists).not.toHaveBeenCalled();
		expect(mocked_analyze).toHaveBeenCalled();
		expect(mocked_write).not.toHaveBeenCalled();
	});

	// An empty name passes the `PackageJson` schema (`z.string()`), so this
	// exercises gro's own guard rather than zod validation.
	test('throws on an empty package.json name', async () => {
		mocked_is_clean.mockReturnValue(false);
		mocked_read.mockResolvedValue(JSON.stringify({ name: '', version: '1.0.0' }));

		await expect(library_load_from_repo('/repo')).rejects.toThrow(/name/);
	});
});

describe('library_load_from_repo with a SvelteKit tsconfig', () => {
	const package_json_contents = JSON.stringify({ name: 'example', version: '1.0.0' });
	const kit_tsconfig_contents =
		'{\n\t// SvelteKit projects extend the synced config\n\t"extends": "$app/tsconfig"\n}';

	beforeEach(() => {
		vi.clearAllMocks();
		mocked_commit.mockResolvedValue('abc123');
		mocked_workspace.mockResolvedValue({} as any);
		mocked_is_clean.mockReturnValue(true);
		mocked_analyze.mockResolvedValue({ modules: [] } as any);
		mocked_library_json_from_modules.mockReturnValue(fake_library);
		mocked_mkdir.mockResolvedValue(undefined);
		mocked_write.mockResolvedValue(undefined);
		mock_files({
			'/repo/package.json': package_json_contents,
			'/repo/tsconfig.json': kit_tsconfig_contents
		});
	});

	// An unsynced tree would analyze on default compiler options (external types
	// as `any`), so it's refused before analysis and nothing is cached.
	test('an unsynced tree throws without analyzing or writing the cache', async () => {
		mock_existing([]);

		const error = await library_load_from_repo('/repo').catch((e: unknown) => e);
		assert(error instanceof Error);
		expect(error.message).toContain('/repo/node_modules/$app/tsconfig.json');
		expect(error.message).toMatch(/npm install/);
		expect(error.message).toMatch(/gro sync/);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(mocked_write).not.toHaveBeenCalled();
	});

	test('an unsynced dirty tree throws', async () => {
		mocked_is_clean.mockReturnValue(false);
		mock_existing([]);

		await expect(library_load_from_repo('/repo')).rejects.toThrow(/gro sync/);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(mocked_write).not.toHaveBeenCalled();
	});

	// The check runs only on a cache miss - a hit is served as written.
	test('a cache hit for an unsynced tree is served without the unsynced check', async () => {
		mock_files({
			'/repo/package.json': package_json_contents,
			'/repo/tsconfig.json': kit_tsconfig_contents,
			'/repo/.gro/library.json': JSON.stringify({
				hash: 'abc123',
				version: LIBRARY_CACHE_VERSION,
				svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
				...fake_result
			})
		});
		mock_existing(['/repo/.gro/library.json']);
		const log = create_mock_log();

		const result = await library_load_from_repo('/repo', { log });
		expect(result).toEqual(fake_result);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(mocked_write).not.toHaveBeenCalled();
		expect(log.warn).not.toHaveBeenCalled();
	});

	test('an unsynced tree throws under `cache: false` too', async () => {
		mock_existing([]);

		await expect(library_load_from_repo('/repo', { cache: false })).rejects.toThrow(/gro sync/);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(mocked_write).not.toHaveBeenCalled();
	});

	test('a synced tree analyzes and caches as before', async () => {
		mock_existing([KIT_TSCONFIG]);
		const log = create_mock_log();

		const result = await library_load_from_repo('/repo', { log });
		expect(result.library_json).toBe(fake_library);
		expect(log.warn).not.toHaveBeenCalled();
		expect(mocked_exists).toHaveBeenCalledWith(KIT_TSCONFIG);
		expect(mocked_analyze).toHaveBeenCalled();
		const [path] = mocked_write.mock.calls[0]!;
		expect(path).toBe('/repo/.gro/library.json');
	});

	test('a non-SvelteKit tsconfig analyzes and caches as before', async () => {
		mock_files({
			'/repo/package.json': package_json_contents,
			'/repo/tsconfig.json': '{"compilerOptions": {"strict": true}}'
		});
		mock_existing([]);

		await library_load_from_repo('/repo');
		expect(mocked_analyze).toHaveBeenCalled();
		expect(mocked_write).toHaveBeenCalled();
	});

	test('logs the analysis to a child of `log`, with its progress at debug level', async () => {
		mock_existing([KIT_TSCONFIG]);
		const log = create_mock_log();
		const { child_log } = log;

		await library_load_from_repo('/repo', { log });
		expect(log.child).toHaveBeenCalledWith('repo');
		const analysis_log = mocked_analyze.mock.calls[0]![0].log!;
		analysis_log.info('progress');
		analysis_log.warn('diagnostic');
		analysis_log.error('failure');
		expect(child_log.debug).toHaveBeenCalledWith('progress');
		expect(child_log.warn).toHaveBeenCalledWith('diagnostic');
		expect(child_log.error).toHaveBeenCalledWith('failure');
		expect(log.info).not.toHaveBeenCalledWith('progress');
	});
});
