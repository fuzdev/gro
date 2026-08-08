import { describe, test, expect } from 'vitest';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	SVELTE_CONFIG_CACHE_FILENAME,
	SVELTE_CONFIG_CACHE_VERSION,
	svelte_config_cache_read,
	svelte_config_cache_stamps,
	svelte_config_cache_write
} from '$lib/svelte_config_cache.ts';
import { GRO_DIRNAME } from '$lib/constants.ts';

const ALIAS = { $lib: 'src/lib', $routes: 'src/routes' };

/**
 * Runs `fn` in a fresh directory seeded with `files`, and cleans it up after.
 * Unlike the `svelte_config` tests this doesn't need the cwd, since every entry point
 * takes an explicit `dir` - only the loader relies on the cwd default.
 */
const in_dir = <T>(files: Record<string, string>, fn: (dir: string) => T): T => {
	const dir = mkdtempSync(join(tmpdir(), 'gro_svelte_config_cache_'));
	try {
		for (const [filename, content] of Object.entries(files)) {
			writeFileSync(join(dir, filename), content);
		}
		return fn(dir);
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
};

const cache_path = (dir: string) => join(dir, GRO_DIRNAME, SVELTE_CONFIG_CACHE_FILENAME);

describe('svelte_config_cache_stamps', () => {
	test('stamps present files and records absent ones as null', () => {
		in_dir({ 'vite.config.ts': 'a', 'package.json': '{}' }, (dir) => {
			const stamps = svelte_config_cache_stamps(dir);
			expect(stamps['vite.config.ts']).toBeTypeOf('string');
			expect(stamps['package.json']).toBeTypeOf('string');
			// Absent inputs are recorded rather than omitted, so that adding one invalidates.
			expect(stamps['svelte.config.js']).toBe(null);
			expect(stamps['vite.config.mjs']).toBe(null);
		});
	});
});

describe('svelte_config_cache_read', () => {
	test('returns null when there is no cache', () => {
		in_dir({ 'vite.config.ts': 'a' }, (dir) => {
			expect(svelte_config_cache_read(svelte_config_cache_stamps(dir), dir)).toBe(null);
		});
	});

	test('round-trips a written cache', () => {
		in_dir({ 'vite.config.ts': 'a', 'package.json': '{}' }, (dir) => {
			const stamps = svelte_config_cache_stamps(dir);
			svelte_config_cache_write(stamps, ALIAS, true, dir);
			const cache = svelte_config_cache_read(stamps, dir);
			expect(cache?.alias).toEqual(ALIAS);
			expect(cache?.svelte_config_found).toBe(true);
			expect(cache?.version).toBe(SVELTE_CONFIG_CACHE_VERSION);
		});
	});

	test('carries `svelte_config_found: false` so the warning can repeat', () => {
		in_dir({ 'vite.config.ts': 'a' }, (dir) => {
			const stamps = svelte_config_cache_stamps(dir);
			svelte_config_cache_write(stamps, ALIAS, false, dir);
			expect(svelte_config_cache_read(stamps, dir)?.svelte_config_found).toBe(false);
		});
	});

	test('misses when a tracked file changes', () => {
		in_dir({ 'vite.config.ts': 'a', 'package.json': '{}' }, (dir) => {
			svelte_config_cache_write(svelte_config_cache_stamps(dir), ALIAS, true, dir);
			writeFileSync(join(dir, 'vite.config.ts'), 'a different length');
			expect(svelte_config_cache_read(svelte_config_cache_stamps(dir), dir)).toBe(null);
		});
	});

	// The reason absent inputs are stamped: a project that adds a Svelte config has to
	// re-resolve, or it would keep running on aliases read before the file existed.
	test('misses when a tracked file appears', () => {
		in_dir({ 'vite.config.ts': 'a' }, (dir) => {
			svelte_config_cache_write(svelte_config_cache_stamps(dir), ALIAS, true, dir);
			writeFileSync(join(dir, 'svelte.config.js'), 'export default {};');
			expect(svelte_config_cache_read(svelte_config_cache_stamps(dir), dir)).toBe(null);
		});
	});

	test('misses when a tracked file is removed', () => {
		in_dir({ 'vite.config.ts': 'a', 'svelte.config.js': 'b' }, (dir) => {
			svelte_config_cache_write(svelte_config_cache_stamps(dir), ALIAS, true, dir);
			rmSync(join(dir, 'svelte.config.js'));
			expect(svelte_config_cache_read(svelte_config_cache_stamps(dir), dir)).toBe(null);
		});
	});

	test('misses on a stale version', () => {
		in_dir({ 'vite.config.ts': 'a' }, (dir) => {
			const stamps = svelte_config_cache_stamps(dir);
			svelte_config_cache_write(stamps, ALIAS, true, dir);
			const cache = JSON.parse(readFileSync(cache_path(dir), 'utf8'));
			writeFileSync(
				cache_path(dir),
				JSON.stringify({ ...cache, version: SVELTE_CONFIG_CACHE_VERSION + 1 })
			);
			expect(svelte_config_cache_read(stamps, dir)).toBe(null);
		});
	});

	test('misses on malformed contents rather than throwing', () => {
		in_dir({ 'vite.config.ts': 'a' }, (dir) => {
			const stamps = svelte_config_cache_stamps(dir);
			mkdirSync(join(dir, GRO_DIRNAME), { recursive: true });
			for (const contents of ['not json', 'null', '[]', '{"version":1}']) {
				writeFileSync(cache_path(dir), contents);
				expect(svelte_config_cache_read(stamps, dir)).toBe(null);
			}
		});
	});
});

describe('svelte_config_cache_write', () => {
	test('creates the .gro directory', () => {
		in_dir({ 'vite.config.ts': 'a' }, (dir) => {
			svelte_config_cache_write(svelte_config_cache_stamps(dir), ALIAS, true, dir);
			expect(JSON.parse(readFileSync(cache_path(dir), 'utf8')).alias).toEqual(ALIAS);
		});
	});

	// Caching is an optimization, so an unwritable directory costs speed and nothing else.
	test('swallows write failures', () => {
		in_dir({ 'vite.config.ts': 'a' }, (dir) => {
			// A file where the `.gro` directory would go makes `mkdirSync` fail.
			writeFileSync(join(dir, GRO_DIRNAME), '');
			expect(() =>
				svelte_config_cache_write(svelte_config_cache_stamps(dir), ALIAS, true, dir)
			).not.toThrow();
		});
	});
});
