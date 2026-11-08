import { assert, describe, test, expect, vi, beforeEach, afterEach } from 'vitest';
import type { PackageJson } from '@fuzdev/fuz_util/package_json.ts';
import { SchemaError } from '@fuzdev/fuz_schema/schema.ts';

// Mock the git helpers so the cache-key/staleness logic can be tested in
// isolation, without depending on the surrounding repo's git state.
vi.mock('@fuzdev/fuz_util/git.js', () => ({
	git_current_commit_hash: vi.fn(),
	git_check_workspace: vi.fn(),
	git_workspace_is_clean: vi.fn()
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
import { analyzeFromFiles } from 'svelte-docinfo';
import { symlinkSync } from 'node:fs';
import { join } from 'node:path';
import { library_json_from_modules, type LibraryJson } from '@fuzdev/fuz_util/library_json.ts';

import {
	LIBRARY_CACHE_VERSION,
	SVELTE_DOCINFO_VERSION,
	library_cache_key,
	library_cache_read,
	library_cache_write,
	library_load_from_repo
} from '#lib/library_load.ts';
import { SVELTEKIT_TSCONFIG_PATH } from '#lib/constants.ts';
import { fs, type FsDir } from '#lib/fs.ts';

const mocked_commit = vi.mocked(git_current_commit_hash);
const mocked_workspace = vi.mocked(git_check_workspace);
const mocked_is_clean = vi.mocked(git_workspace_is_clean);
const mocked_analyze = vi.mocked(analyzeFromFiles);
const mocked_library_json_from_modules = vi.mocked(library_json_from_modules);

// A minimal stand-in for the analyzed library metadata - only the cache
// round-trip is under test here, not the shape of `LibraryJson`.
const fake_library = { name: 'example', version: '1.0.0' } as unknown as LibraryJson;
const fake_package_json = { name: 'example', version: '1.0.0' } as unknown as PackageJson;
const fake_result = { library_json: fake_library, package_json: fake_package_json };

const fresh_cache_contents = JSON.stringify({
	hash: 'abc123',
	version: LIBRARY_CACHE_VERSION,
	svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
	...fake_result
});

/**
 * A real repo directory named `repo` in a fresh temp directory,
 * so the log child is labelled with a known name.
 */
let temp: FsDir & AsyncDisposable;
let repo: FsDir;
let repo_dir: string;

const LIBRARY_CACHE_PATH = '.gro/library.json';

const cache_path = (): string => join(repo_dir, LIBRARY_CACHE_PATH);

const write = async (path: string, contents: string): Promise<void> => {
	const slash = path.lastIndexOf('/');
	if (slash !== -1) await repo.mkdir(path.slice(0, slash), { recursive: true });
	await repo.write(path, contents);
};

const read_cache = async (): Promise<any> => {
	const contents = await repo.read_text_if_exists(LIBRARY_CACHE_PATH);
	return contents === null ? null : JSON.parse(contents);
};

beforeEach(async () => {
	temp = await fs.temp_dir('gro-library-load-');
	await temp.mkdir('repo');
	repo = temp.dir('repo');
	repo_dir = repo.path;
});

afterEach(async () => {
	await temp[Symbol.asyncDispose]();
});

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
		await write(LIBRARY_CACHE_PATH, fresh_cache_contents);

		const result = await library_cache_read(cache_path(), 'abc123');
		expect(result).toEqual(fake_result);
	});

	test('returns null when the cache version is stale', async () => {
		await write(
			LIBRARY_CACHE_PATH,
			JSON.stringify({
				hash: 'abc123',
				version: LIBRARY_CACHE_VERSION - 1,
				svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
				...fake_result
			})
		);

		const result = await library_cache_read(cache_path(), 'abc123');
		expect(result).toBeNull();
	});

	// The analyzer-side upgrade case: the analyzed repo's commit (the cache key)
	// doesn't move when the *analyzer's* svelte-docinfo changes output shape, so
	// the stamped version is what invalidates.
	test('returns null when the cache was analyzed by a different svelte-docinfo version', async () => {
		await write(
			LIBRARY_CACHE_PATH,
			JSON.stringify({
				hash: 'abc123',
				version: LIBRARY_CACHE_VERSION,
				svelte_docinfo_version: '0.0.0-other',
				...fake_result
			})
		);

		const result = await library_cache_read(cache_path(), 'abc123');
		expect(result).toBeNull();
	});

	test('returns null when the cache has no svelte-docinfo version stamp', async () => {
		await write(
			LIBRARY_CACHE_PATH,
			JSON.stringify({ hash: 'abc123', version: LIBRARY_CACHE_VERSION, ...fake_result })
		);

		const result = await library_cache_read(cache_path(), 'abc123');
		expect(result).toBeNull();
	});

	// The realistic migration case: a legacy cache written before versioning has a
	// matching `hash` but no `version` field, and must be treated as stale.
	test('returns null for a legacy cache with a matching hash but no version', async () => {
		await write(LIBRARY_CACHE_PATH, JSON.stringify({ hash: 'abc123', library_json: fake_library }));

		const result = await library_cache_read(cache_path(), 'abc123');
		expect(result).toBeNull();
	});

	test('returns null without logging a bad read when the cache file is absent', async () => {
		const log = create_mock_log();

		const result = await library_cache_read(cache_path(), 'abc123', log);
		expect(result).toBeNull();
		expect(log.debug).not.toHaveBeenCalled();
	});

	test('returns null when the cached hash is stale', async () => {
		await write(LIBRARY_CACHE_PATH, JSON.stringify({ hash: 'old', ...fake_result }));

		const result = await library_cache_read(cache_path(), 'new');
		expect(result).toBeNull();
	});

	test('returns null when the cache is corrupt (invalid JSON)', async () => {
		await write(LIBRARY_CACHE_PATH, 'not json {');

		const result = await library_cache_read(cache_path(), 'abc123');
		expect(result).toBeNull();
	});

	test('returns null when the read itself throws', async () => {
		// a directory where the cache file should be, which reading fails on
		await repo.mkdir(LIBRARY_CACHE_PATH, { recursive: true });

		const result = await library_cache_read(cache_path(), 'abc123');
		expect(result).toBeNull();
	});

	test('reads the cache through a symlinked `.gro` outside the repo', async () => {
		await temp.mkdir('shared');
		await temp.write('shared/library.json', fresh_cache_contents);
		symlinkSync(join(temp.path, 'shared'), join(repo_dir, '.gro'));

		const result = await library_cache_read(cache_path(), 'abc123');
		expect(result).toEqual(fake_result);
	});
});

describe('library_cache_write', () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test('creates the parent dir and writes the keyed cache', async () => {
		await library_cache_write(cache_path(), 'abc123', fake_result);

		expect(await read_cache()).toEqual({
			hash: 'abc123',
			version: LIBRARY_CACHE_VERSION,
			svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
			...fake_result
		});
	});

	test('swallows write failures (best effort) and warns', async () => {
		// a directory where the cache file should be, which writing fails on
		await repo.mkdir(LIBRARY_CACHE_PATH, { recursive: true });
		const log = { warn: vi.fn(), debug: vi.fn() } as any;

		await expect(
			library_cache_write(cache_path(), 'abc123', fake_result, log)
		).resolves.toBeUndefined();
		expect(log.warn).toHaveBeenCalled();
	});

	test('swallows mkdir failures (best effort)', async () => {
		// a file where the `.gro` directory should be
		await repo.write('.gro', '');

		await expect(library_cache_write(cache_path(), 'abc123', fake_result)).resolves.toBeUndefined();
		expect(await repo.read_text('.gro')).toBe('');
	});
});

describe('library_load_from_repo', () => {
	const valid_package_json_contents = JSON.stringify({ name: 'example', version: '1.0.0' });

	beforeEach(() => {
		vi.clearAllMocks();
		mocked_commit.mockResolvedValue('abc123');
		mocked_workspace.mockResolvedValue({} as any);
		mocked_is_clean.mockReturnValue(true);
		mocked_analyze.mockResolvedValue({ modules: [] } as any);
		mocked_library_json_from_modules.mockReturnValue(fake_library);
	});

	test('returns the cached result without analyzing on a cache hit', async () => {
		await write(LIBRARY_CACHE_PATH, fresh_cache_contents);

		const result = await library_load_from_repo(repo_dir);
		expect(result).toEqual(fake_result);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(await repo.read_text(LIBRARY_CACHE_PATH)).toBe(fresh_cache_contents);
	});

	// The end-to-end behavior of the version stamps: a cache analyzed by a
	// different svelte-docinfo re-analyzes and rewrites with the current stamp.
	test('re-analyzes and rewrites when the svelte-docinfo stamp is stale', async () => {
		await write('package.json', valid_package_json_contents);
		await write(
			LIBRARY_CACHE_PATH,
			JSON.stringify({
				hash: 'abc123',
				version: LIBRARY_CACHE_VERSION,
				svelte_docinfo_version: '0.0.0-other',
				...fake_result
			})
		);

		const result = await library_load_from_repo(repo_dir);
		expect(mocked_analyze).toHaveBeenCalledWith({ projectRoot: repo_dir });
		expect(result.library_json).toBe(fake_library);
		expect((await read_cache()).svelte_docinfo_version).toBe(SVELTE_DOCINFO_VERSION);
	});

	test('`cache: false` skips the cache read but still writes the result', async () => {
		await write('package.json', valid_package_json_contents);
		await write(LIBRARY_CACHE_PATH, fresh_cache_contents);

		await library_load_from_repo(repo_dir, { cache: false });
		expect(mocked_analyze).toHaveBeenCalled();
		expect(await read_cache()).toEqual({
			hash: 'abc123',
			version: LIBRARY_CACHE_VERSION,
			svelte_docinfo_version: SVELTE_DOCINFO_VERSION,
			library_json: fake_library,
			package_json: JSON.parse(valid_package_json_contents)
		});
	});

	test('a dirty tree analyzes fresh and skips both cache read and write', async () => {
		mocked_is_clean.mockReturnValue(false);
		await write('package.json', valid_package_json_contents);
		await write(LIBRARY_CACHE_PATH, fresh_cache_contents);

		await library_load_from_repo(repo_dir);
		expect(mocked_analyze).toHaveBeenCalled();
		expect(await repo.read_text(LIBRARY_CACHE_PATH)).toBe(fresh_cache_contents);
	});

	// An empty name passes the `PackageJson` schema (`s.string`), so this
	// exercises gro's own guard rather than schema validation.
	test('throws on an empty package.json name', async () => {
		mocked_is_clean.mockReturnValue(false);
		await write('package.json', JSON.stringify({ name: '', version: '1.0.0' }));

		await expect(library_load_from_repo(repo_dir)).rejects.toThrow(/name/);
	});

	test('throws `SchemaError` on an invalid package.json, before analyzing', async () => {
		mocked_is_clean.mockReturnValue(false);
		await write('package.json', JSON.stringify({ name: 1, version: '1.0.0' }));

		const error = await library_load_from_repo(repo_dir).catch((e: unknown) => e);
		assert(error instanceof SchemaError);
		assert.strictEqual(error.message, 'name: expected string, received number');
		expect(mocked_analyze).not.toHaveBeenCalled();
	});
});

describe('library_load_from_repo with a SvelteKit tsconfig', () => {
	const package_json_contents = JSON.stringify({ name: 'example', version: '1.0.0' });
	const kit_tsconfig_contents =
		'{\n\t// SvelteKit projects extend the synced config\n\t"extends": "$app/tsconfig"\n}';

	beforeEach(async () => {
		vi.clearAllMocks();
		mocked_commit.mockResolvedValue('abc123');
		mocked_workspace.mockResolvedValue({} as any);
		mocked_is_clean.mockReturnValue(true);
		mocked_analyze.mockResolvedValue({ modules: [] } as any);
		mocked_library_json_from_modules.mockReturnValue(fake_library);
		await write('package.json', package_json_contents);
		await write('tsconfig.json', kit_tsconfig_contents);
	});

	// An unsynced tree would analyze on default compiler options (external types
	// as `any`), so it's refused before analysis and nothing is cached.
	test('an unsynced tree throws without analyzing or writing the cache', async () => {
		const error = await library_load_from_repo(repo_dir).catch((e: unknown) => e);
		assert(error instanceof Error);
		expect(error.message).toContain(repo_dir + '/node_modules/$app/tsconfig.json');
		expect(error.message).toMatch(/npm install/);
		expect(error.message).toMatch(/gro sync/);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(await read_cache()).toBeNull();
	});

	test('an unsynced dirty tree throws', async () => {
		mocked_is_clean.mockReturnValue(false);

		await expect(library_load_from_repo(repo_dir)).rejects.toThrow(/gro sync/);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(await read_cache()).toBeNull();
	});

	// The check runs only on a cache miss - a hit is served as written.
	test('a cache hit for an unsynced tree is served without the unsynced check', async () => {
		await write(LIBRARY_CACHE_PATH, fresh_cache_contents);
		const log = create_mock_log();

		const result = await library_load_from_repo(repo_dir, { log });
		expect(result).toEqual(fake_result);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(await repo.read_text(LIBRARY_CACHE_PATH)).toBe(fresh_cache_contents);
		expect(log.warn).not.toHaveBeenCalled();
	});

	test('an unsynced tree throws under `cache: false` too', async () => {
		await expect(library_load_from_repo(repo_dir, { cache: false })).rejects.toThrow(/gro sync/);
		expect(mocked_analyze).not.toHaveBeenCalled();
		expect(await read_cache()).toBeNull();
	});

	test('a synced tree analyzes and caches as before', async () => {
		await write(SVELTEKIT_TSCONFIG_PATH, '{}');
		const log = create_mock_log();

		const result = await library_load_from_repo(repo_dir, { log });
		expect(result.library_json).toBe(fake_library);
		expect(log.warn).not.toHaveBeenCalled();
		expect(mocked_analyze).toHaveBeenCalled();
		expect((await read_cache()).hash).toBe('abc123');
	});

	test('reads a tsconfig.json and package.json symlinked from outside the repo', async () => {
		await repo.remove('tsconfig.json');
		await repo.remove('package.json');
		await temp.write('tsconfig.shared.json', kit_tsconfig_contents);
		await temp.write('package.shared.json', JSON.stringify({ name: 'linked', version: '2.0.0' }));
		symlinkSync(join(temp.path, 'tsconfig.shared.json'), join(repo_dir, 'tsconfig.json'));
		symlinkSync('../package.shared.json', join(repo_dir, 'package.json'));

		// the linked tsconfig extends `$app/tsconfig`, so the repo is unsynced until it's generated
		await expect(library_load_from_repo(repo_dir)).rejects.toThrow(/gro sync/);
		await write(SVELTEKIT_TSCONFIG_PATH, '{}');

		await library_load_from_repo(repo_dir);
		expect(mocked_library_json_from_modules.mock.calls[0]![0]).toEqual({
			name: 'linked',
			version: '2.0.0'
		});
	});

	test('a non-SvelteKit tsconfig analyzes and caches as before', async () => {
		await write('tsconfig.json', '{"compilerOptions": {"strict": true}}');

		await library_load_from_repo(repo_dir);
		expect(mocked_analyze).toHaveBeenCalled();
		expect(await read_cache()).not.toBeNull();
	});

	test('logs the analysis to a child of `log`, with its progress at debug level', async () => {
		await write(SVELTEKIT_TSCONFIG_PATH, '{}');
		const log = create_mock_log();
		const { child_log } = log;

		await library_load_from_repo(repo_dir, { log });
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
