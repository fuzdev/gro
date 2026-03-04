import {test, expect, vi} from 'vitest';
import {hash_blake3} from '@fuzdev/fuz_util/hash_blake3.js';

import {
	SEARCH_EXCLUDER_DEFAULT,
	EMPTY_BUILD_CACHE_CONFIG_HASH,
	load_gro_config,
} from '../lib/gro_config.ts';

test('load_gro_config', async () => {
	// Mock the dynamic import to avoid module resolution issues
	vi.mock('node:fs', () => ({
		existsSync: vi.fn().mockReturnValue(false),
	}));

	const config = await load_gro_config();
	expect(config).toBeTruthy();
	expect(config.plugins).toBeDefined();
	expect(config.task_root_dirs).toBeDefined();
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

	// Special exception for `gro/dist/`, but not `gro/build/` etc because they're not usecases.
	assert_includes('gro/build', false);
	assert_includes('gro/buildE', true);
	assert_includes('groE/build', false);
	assert_includes('gro/dist', true);
	assert_includes('node_modules/gro/dist', true);
	assert_includes('node_modules/@someuser/gro/dist', true);
	assert_includes('node_modules/@someuser/foo/gro/dist', false);
	assert_includes('gro/distE', true);
	assert_includes('groE/dist', false);
	assert_includes('Egro/dist', false);
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
