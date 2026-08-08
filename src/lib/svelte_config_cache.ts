import { mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

import {
	GRO_DIRNAME,
	PACKAGE_JSON_FILENAME,
	SVELTE_CONFIG_FILENAMES,
	VITE_CONFIG_FILENAMES
} from './constants.ts';

/*

Caches the one piece of the Svelte config the Node loader needs before it can resolve
anything, so the common case doesn't pay a full Vite config resolution per invocation.

The loader's `resolve` hook reads `alias` and nothing else - every other field it uses is
read inside `load`, which can await the real config lazily. So this holds an alias map and
nothing else, and an alias map is always plain strings: `$lib` pointing at `kit.files.lib`
plus whatever `kit.alias` declares. There's nothing here that doesn't survive JSON, which
is why this caches a slice of the config rather than the config, whose preprocessors and
`compilerOptions.warningFilter` are functions.

Validation is hand-rolled rather than a Zod schema because this module is on the loader's
critical path, where the point is to be cheaper than the thing it replaces.

*/

export const SVELTE_CONFIG_CACHE_FILENAME = 'svelte_config.json';

/**
 * Bump when `SvelteConfigCache`'s shape changes, or when the meaning of a field does,
 * so caches written by an older Gro self-invalidate instead of being read as the new shape.
 */
export const SVELTE_CONFIG_CACHE_VERSION = 1;

/**
 * The mtime and size of each cache input, or `null` for one that doesn't exist.
 * Absence is recorded rather than omitted so that *adding* a config file invalidates too.
 */
export type SvelteConfigCacheStamps = Record<string, string | null>;

export interface SvelteConfigCache {
	version: number;
	stamps: SvelteConfigCacheStamps;
	/**
	 * The `alias` of a `ParsedSvelteConfig`.
	 */
	alias: Record<string, string>;
	/**
	 * `false` when the Vite config resolved but configured no Svelte plugin, so a cached read
	 * can repeat the warning a fresh load would have emitted instead of going quiet about it.
	 */
	svelte_config_found: boolean;
}

/**
 * The files whose state invalidates the cache, relative to the project directory.
 *
 * Deliberately the config files themselves and `package.json`, not the modules a Vite config
 * imports: `alias` comes from `kit.alias` and `kit.files.lib`, which are authored in one of
 * these. A project that factors them out into an imported module keeps a stale alias until
 * something else invalidates, which surfaces as an unresolved import rather than a bad
 * compile - `gro clean` clears it.
 */
const CACHE_INPUT_FILENAMES = [
	PACKAGE_JSON_FILENAME,
	...VITE_CONFIG_FILENAMES,
	...SVELTE_CONFIG_FILENAMES
];

const to_cache_path = (dir: string): string => join(dir, GRO_DIRNAME, SVELTE_CONFIG_CACHE_FILENAME);

const to_file_stamp = (path: string): string | null => {
	try {
		const stats = statSync(path);
		return `${stats.mtimeMs}:${stats.size}`;
	} catch {
		return null; // absent, or unreadable, which is the same thing for cache purposes
	}
};

/**
 * Snapshots the state of the cache inputs in `dir`.
 *
 * Taken by the caller *before* resolving the config, not inside `svelte_config_cache_write`
 * afterwards: a config edited mid-resolution should leave a stamp that no longer matches, so
 * the next invocation resolves again. Stamping afterwards would pair the old alias with the
 * new state and serve it as fresh.
 */
export const svelte_config_cache_stamps = (dir = process.cwd()): SvelteConfigCacheStamps => {
	const stamps: SvelteConfigCacheStamps = {};
	for (const filename of CACHE_INPUT_FILENAMES) {
		stamps[filename] = to_file_stamp(join(dir, filename));
	}
	return stamps;
};

/**
 * Reads the cache and returns it only if every input still matches `stamps`.
 * Every kind of miss - absent, corrupt, wrong version, stale - reads the same to the caller,
 * which resolves the config and rewrites.
 */
export const svelte_config_cache_read = (
	stamps: SvelteConfigCacheStamps,
	dir = process.cwd()
): SvelteConfigCache | null => {
	let cache: SvelteConfigCache;
	try {
		cache = JSON.parse(readFileSync(to_cache_path(dir), 'utf8'));
	} catch {
		return null;
	}
	if (
		cache?.version !== SVELTE_CONFIG_CACHE_VERSION ||
		typeof cache.alias !== 'object' ||
		cache.alias === null ||
		typeof cache.svelte_config_found !== 'boolean' ||
		typeof cache.stamps !== 'object' ||
		cache.stamps === null
	) {
		return null;
	}
	for (const filename of CACHE_INPUT_FILENAMES) {
		if (cache.stamps[filename] !== stamps[filename]) return null;
	}
	return cache;
};

/**
 * Writes the cache for the config that `stamps` was taken before resolving.
 *
 * Best effort - a cache that can't be written just means the next invocation resolves again,
 * so a read-only or otherwise unwritable project directory costs speed and nothing else.
 */
export const svelte_config_cache_write = (
	stamps: SvelteConfigCacheStamps,
	alias: Record<string, string>,
	svelte_config_found: boolean,
	dir = process.cwd()
): void => {
	const cache: SvelteConfigCache = {
		version: SVELTE_CONFIG_CACHE_VERSION,
		stamps,
		alias,
		svelte_config_found
	};
	const path = to_cache_path(dir);
	try {
		mkdirSync(dirname(path), { recursive: true });
		writeFileSync(path, JSON.stringify(cache, null, '\t') + '\n', 'utf8');
	} catch {
		// see the doc comment - caching is an optimization, not a requirement
	}
};
