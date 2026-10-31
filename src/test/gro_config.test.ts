import { test, expect, vi } from 'vitest';
import { resolve } from 'node:path';
import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';

import {
	SEARCH_EXCLUDER_DEFAULT,
	EMPTY_BUILD_CACHE_CONFIG_HASH,
	cook_gro_config,
	load_gro_config
} from '#lib/gro_config.ts';
import { paths } from '#lib/paths.ts';

// Makes `load_gro_config` see no `gro.config.ts` and fall back to the default config.
// At the top level because that's where Vitest hoists it to anyway.
vi.mock('node:fs', () => ({
	existsSync: vi.fn().mockReturnValue(false)
}));

test('load_gro_config', async () => {
	const config = await load_gro_config();
	expect(config).toBeTruthy();
	expect(config.plugins).toBeDefined();
	expect('task_root_dirs' in config).toBe(false);
});

test('SEARCH_EXCLUDER_DEFAULT', () => {
	const assert_includes = (path: string, exclude: boolean) => {
		const b = (v: boolean) => (exclude ? !v : v);
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`a/${path}/c`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`a/${path}/c/d.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`a/${path}/c/d.e.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`a/${path}/`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`a/${path}`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/a/${path}/c`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/a/${path}/c/d.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/a/${path}/c/d.e.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/a/${path}/`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/a/${path}`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/${path}/a`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/${path}/a/b.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/${path}/a/b.e.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/${path}/`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`/${path}`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`./${path}/a`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`./${path}/a/b.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`./${path}/a/b.c.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`./${path}/`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`./${path}`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`${path}/a`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`${path}/a/b.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`${path}/a/b.c.js`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(`${path}/`))).toBeTruthy();
		expect(b(SEARCH_EXCLUDER_DEFAULT.test(path))).toBeTruthy();
	};

	assert_includes('node_modules', false);
	assert_includes('dist', false);
	assert_includes('build', false);
	assert_includes('.git', false);
	assert_includes('.gro', false);
	assert_includes('.svelte-kit', false);
	assert_includes('a', true);
	assert_includes('test', true);
	assert_includes('testing', true);
	assert_includes('benchmark', true);
	assert_includes('benchmarks', true);
	assert_includes('nodemodules', true);
	assert_includes('dist_server', false);
	assert_includes('gro/dist', false);
	assert_includes('node_modules/@someuser/gro/dist', false);
	assert_includes('Ebuild', true);
	assert_includes('buildE', true);
	assert_includes('grobuild', true);
	assert_includes('distE', true);
	assert_includes('Edist', true);
	assert_includes('grodist', true);
});

test('EMPTY_BUILD_CACHE_CONFIG_HASH matches hash of empty string', () => {
	const computed_hash = hash_blake3('');
	expect(EMPTY_BUILD_CACHE_CONFIG_HASH).toBe(computed_hash);
});

test('cook_gro_config defaults each `watch` field and resolves a relative `dir`', async () => {
	const defaults = await cook_gro_config({});
	expect(defaults.watch).toEqual({ dir: resolve(paths.source), filter: null });

	const filter = (path: string) => !path.endsWith('.md');
	const with_filter = await cook_gro_config({ watch: { filter } });
	expect(with_filter.watch).toEqual({ dir: resolve(paths.source), filter });

	const with_dir = await cook_gro_config({ watch: { dir: 'lib/' } });
	expect(with_dir.watch).toEqual({ dir: resolve('lib'), filter: null });
});
