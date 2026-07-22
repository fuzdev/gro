import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { styleText as st } from 'node:util';
import { analyzeFromFiles } from 'svelte-docinfo';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import {
	git_current_commit_hash,
	git_check_workspace,
	git_workspace_is_clean
} from '@fuzdev/fuz_util/git.ts';
import { fs_exists } from '@fuzdev/fuz_util/fs.ts';
import { PackageJson } from '@fuzdev/fuz_util/package_json.ts';
import { library_json_from_modules, type LibraryJson } from '@fuzdev/fuz_util/library_json.ts';
import { to_error_message } from '@fuzdev/fuz_util/error.ts';

import { GRO_DIRNAME } from './constants.ts';

/**
 * Cache filename inside a repo's `.gro` directory for `library_load_from_repo`.
 */
export const LIBRARY_CACHE_FILENAME = 'library.json';

/**
 * Format version for the `.gro/library.json` cache. The cache key is the git
 * commit hash, which does NOT change when the cached *shape* changes — so bump
 * this whenever `LibraryCache`'s shape changes (e.g. the `LibraryJson` /
 * `PkgJson` split, then slimming `LibraryJson` to the raw `pkg_json`/`source_json`
 * pair) to self-invalidate stale caches across the ecosystem rather than serve
 * old-shaped data at an unchanged commit.
 */
export const LIBRARY_CACHE_VERSION = 1;

/**
 * Result of loading a repo's library metadata: the curated `LibraryJson`
 * (carrying the publish-safe `pkg_json`) plus the repo's full `package.json`.
 *
 * The full `package_json` is kept alongside — not folded into `LibraryJson` —
 * because tooling like fuz_gitops needs `dependencies`/`devDependencies`, which
 * the curated `LibraryJson.pkg_json` deliberately omits.
 */
export interface LibraryLoadResult {
	library_json: LibraryJson;
	package_json: PackageJson;
}

/**
 * On-disk shape of the `.gro/library.json` cache file.
 * The `hash` is the git-based cache key the result was computed at; `version`
 * is the `LIBRARY_CACHE_VERSION` it was written under.
 */
export interface LibraryCache extends LibraryLoadResult {
	hash: string;
	version: number;
}

export interface LibraryLoadOptions {
	log?: Logger;
	/** Set to `false` to bypass the `.gro` cache (always re-analyze, but still write the result). */
	cache?: boolean;
}

/**
 * Computes the cache key for a repo at `repo_dir`: the git `HEAD` commit hash.
 *
 * Returns `null` (uncacheable — analysis still runs, caching is skipped) when
 * the dir is not a git repo OR when the working tree is dirty. A dirty tree is
 * deliberately uncacheable: the commit hash doesn't capture uncommitted edits,
 * so a single `-dirty` key would serve a stale analysis across successive edits.
 * Skipping the cache while dirty guarantees fresh metadata; clean commits cache.
 */
export const library_cache_key = async (repo_dir: string): Promise<string | null> => {
	const options = { cwd: repo_dir };
	const commit = await git_current_commit_hash('HEAD', options);
	if (!commit) return null;
	const status = await git_check_workspace(options);
	return git_workspace_is_clean(status) ? commit : null;
};

/**
 * Reads and validates the `.gro/library.json` cache at `cache_path`.
 *
 * Returns the cached `{library_json, package_json}` only when the file exists
 * and its stored `hash` matches `key`. Returns `null` on every miss - absent,
 * stale (different `hash`), or unreadable/corrupt - signalling the caller to
 * re-analyze.
 *
 * @param cache_path - absolute path to the cache file
 * @param key - the expected cache key (a clean git commit hash)
 * @returns the cached result, or `null` on any miss
 */
export const library_cache_read = async (
	cache_path: string,
	key: string,
	log?: Logger
): Promise<LibraryLoadResult | null> => {
	if (!(await fs_exists(cache_path))) return null;
	try {
		const contents = await readFile(cache_path, 'utf-8');
		const parsed: LibraryCache = JSON.parse(contents);
		if (parsed.hash === key && parsed.version === LIBRARY_CACHE_VERSION) {
			log?.debug('library cache hit', st('dim', `(${cache_path} @ ${key})`));
			return { library_json: parsed.library_json, package_json: parsed.package_json };
		}
		log?.debug('library cache stale', st('dim', `(${cache_path})`));
	} catch {
		// Corrupted or unreadable cache - fall through to re-analyze.
		log?.debug('library cache unreadable, re-analyzing', st('dim', `(${cache_path})`));
	}
	return null;
};

/**
 * Writes `result` to the `.gro/library.json` cache at `cache_path`, keyed by
 * `key` and stamped with the current `LIBRARY_CACHE_VERSION`, creating the
 * parent directory as needed.
 *
 * Best effort: caching is optional, so write failures are logged as a warning
 * and swallowed rather than thrown.
 *
 * @param cache_path - absolute path to the cache file
 * @param key - the cache key to store (a clean git commit hash)
 * @param result - the `{library_json, package_json}` to cache
 */
export const library_cache_write = async (
	cache_path: string,
	key: string,
	result: LibraryLoadResult,
	log?: Logger
): Promise<void> => {
	try {
		await mkdir(dirname(cache_path), { recursive: true });
		const data: LibraryCache = { hash: key, version: LIBRARY_CACHE_VERSION, ...result };
		await writeFile(cache_path, JSON.stringify(data, null, '\t') + '\n', 'utf-8');
		log?.debug('library cache written', st('dim', `(${cache_path})`));
	} catch (error) {
		log?.warn(
			st('yellow', 'failed to write library cache'),
			st('dim', `(${to_error_message(error)})`)
		);
	}
};

/**
 * Loads a repo's library metadata via `svelte-docinfo`, with a `.gro` cache
 * keyed by git hash.
 *
 * Analyzes `repo_dir` with `analyzeFromFiles` and combines the result with the
 * repo's `package.json` into a `LibraryJson`, returned alongside the full
 * `package.json`. Results are cached at `<repo_dir>/.gro/library.json` keyed by
 * the current git `HEAD`, so repeated loads at the same commit skip the
 * (potentially slow) analysis. A dirty working tree (or a non-git dir) is
 * uncacheable, so analysis re-runs on every load until the changes are
 * committed - see `library_cache_key`.
 *
 * @param repo_dir - absolute path to the repo to analyze
 * @returns the repo's `LibraryLoadResult` (`library_json` + full `package_json`)
 */
export const library_load_from_repo = async (
	repo_dir: string,
	options?: LibraryLoadOptions
): Promise<LibraryLoadResult> => {
	const { log, cache = true } = options ?? {};

	const cache_path = join(repo_dir, GRO_DIRNAME, LIBRARY_CACHE_FILENAME);

	const key = await library_cache_key(repo_dir);

	// Try the cache first. A `null` key (not a git repo, or a dirty tree) is never cacheable.
	if (cache && key !== null) {
		const cached = await library_cache_read(cache_path, key, log);
		if (cached !== null) return cached;
	}

	log?.info('analyzing library', st('dim', `(${repo_dir})`));

	// Read and validate the repo's package.json directly (not the CWD's).
	const package_json_path = join(repo_dir, 'package.json');
	const package_json_contents = await readFile(package_json_path, 'utf-8');
	const package_json = PackageJson.parse(JSON.parse(package_json_contents));
	if (!package_json.name) {
		throw Error(`library_load_from_repo: missing \`name\` in ${package_json_path}`);
	}
	if (!package_json.version) {
		throw Error(`library_load_from_repo: missing \`version\` in ${package_json_path}`);
	}

	const { modules } = await analyzeFromFiles({ projectRoot: repo_dir });

	const library_json = library_json_from_modules(package_json, modules);

	const result: LibraryLoadResult = { library_json, package_json };

	// Cache the result (best effort). Skip when there's no usable key, e.g. not a
	// git repo or a dirty working tree.
	if (key !== null) {
		await library_cache_write(cache_path, key, result, log);
	}

	return result;
};
