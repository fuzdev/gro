import { test, expect, assert } from 'vitest';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';
import { json_stringify_deterministic } from '@fuzdev/fuz_util/json.ts';

import {
	type GroConfig,
	SEARCH_EXCLUDER_DEFAULT,
	EMPTY_BUILD_CACHE_CONFIG_HASH,
	cook_gro_config,
	create_empty_gro_config,
	load_gro_config
} from '#lib/gro_config.ts';
import { fs } from '#lib/fs.ts';
import { create_fs_paths } from '#lib/fs_paths.ts';
import { import_module_node } from '#lib/internal/node_import_module.ts';
import { create_project_fs_paths, to_fs_paths_options } from './fs_test_helpers.ts';
import { paths } from './test_helpers.ts';

test('load_gro_config', async () => {
	const config = await load_gro_config(
		create_fs_paths(fs, to_fs_paths_options()),
		import_module_node
	);
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
	assert_includes('src/lib/dist_helpers.ts', true);
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

test('create_empty_gro_config watches the relative `src/`, which cooking resolves against the root', async () => {
	const empty = create_empty_gro_config();
	assert.strictEqual(empty.watch.dir, 'src/');
	// a root apart from the working directory, so a cook that reads the process fails
	const root = '/gro_config_test/project';
	assert.strictEqual((await cook_gro_config(empty, root)).watch.dir, root + '/src');
	assert.strictEqual((await cook_gro_config({}, root)).watch.dir, root + '/src');
});

test('cook_gro_config defaults each `watch` field and resolves a relative `dir`', async () => {
	const defaults = await cook_gro_config({}, paths.root);
	expect(defaults.watch).toEqual({ dir: resolve(paths.source), filter: null });

	const filter = (path: string) => !path.endsWith('.md');
	const with_filter = await cook_gro_config({ watch: { filter } }, paths.root);
	expect(with_filter.watch).toEqual({ dir: resolve(paths.source), filter });

	const with_dir = await cook_gro_config({ watch: { dir: 'lib/' } }, paths.root);
	expect(with_dir.watch).toEqual({ dir: resolve('lib'), filter: null });
});

test('cook_gro_config hashes `build_cache_config` as deterministic JSON and drops the raw value', async () => {
	const expected = hash_blake3(json_stringify_deterministic({ a: '1', m: '2', z: '3' }));
	for (const build_cache_config of [
		{ z: '3', a: '1', m: '2' },
		{ a: '1', m: '2', z: '3' }
	]) {
		const raw = { build_cache_config };
		const cooked = await cook_gro_config(raw, paths.root);
		expect(cooked.build_cache_config_hash).toBe(expected);
		expect('build_cache_config' in raw).toBe(false);
	}
});

test('cook_gro_config hashes what a `build_cache_config` function resolves to', async () => {
	const cooked = await cook_gro_config(
		{
			build_cache_config: async () => ({ feature_flag: true })
		},
		paths.root
	);
	expect(cooked.build_cache_config_hash).toBe(
		hash_blake3(json_stringify_deterministic({ feature_flag: true }))
	);
});

test('cook_gro_config without `build_cache_config` has the empty hash', async () => {
	const cooked = await cook_gro_config({}, paths.root);
	expect(cooked.build_cache_config_hash).toBe(EMPTY_BUILD_CACHE_CONFIG_HASH);
});

test('load_gro_config imports the config through `import_module`', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_load_config_'));
	try {
		await writeFile(join(dir, 'gro.config.ts'), "throw Error('read by the fake, never run');\n");
		const ids: Array<string> = [];
		const config = await load_gro_config(await create_project_fs_paths(dir), (id) => {
			ids.push(id);
			return Promise.resolve({ default: { pm_cli: 'fake_pm' } });
		});
		assert.deepEqual(ids, [join(dir, 'gro.config.ts')]);
		assert.strictEqual(config.pm_cli, 'fake_pm');
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});

test('load_gro_config resolves `watch.dir` against the project root, with a config file or without', async () => {
	const dir = await mkdtemp(join(tmpdir(), 'gro_load_config_watch_'));
	try {
		const fs_paths = await create_project_fs_paths(dir);
		const src = fs_paths.project.path + '/src';
		const without = await load_gro_config(fs_paths, () => Promise.reject(Error('no config file')));
		assert.strictEqual(without.watch.dir, src);
		await writeFile(join(dir, 'gro.config.ts'), "throw Error('read by the fake, never run');\n");
		const declared = await load_gro_config(fs_paths, () =>
			Promise.resolve({ default: create_empty_gro_config() })
		);
		assert.strictEqual(declared.watch.dir, src);
		const created = await load_gro_config(fs_paths, () =>
			Promise.resolve({ default: (cfg: GroConfig) => cfg })
		);
		assert.strictEqual(created.watch.dir, src);
	} finally {
		await rm(dir, { recursive: true, force: true });
	}
});
