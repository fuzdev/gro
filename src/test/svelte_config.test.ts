import { describe, test, expect } from 'vitest';
import type { Config as SvelteConfig } from '@sveltejs/kit';

import { load_default_svelte_config, parse_svelte_config } from '$lib/svelte_config.ts';

const DIR = '/fake/project';

const parse = (svelte_config: SvelteConfig) => parse_svelte_config({ svelte_config, dir: DIR });

describe('parse_svelte_config', () => {
	test('falls back to the conventional paths when nothing is configured', async () => {
		const parsed = await parse({});
		expect(parsed.lib_path).toBe('src/lib');
		expect(parsed.routes_path).toBe('src/routes');
		expect(parsed.assets_path).toBe('static');
		expect(parsed.env_dir).toBe(undefined);
	});

	test('keeps already-relative paths as authored', async () => {
		const parsed = await parse({ kit: { files: { lib: 'src/library', routes: 'src/pages' } } });
		expect(parsed.lib_path).toBe('src/library');
		expect(parsed.routes_path).toBe('src/pages');
	});

	// Resolving through Vite yields absolute paths, but Gro's vocabulary is project-relative.
	test('makes absolute paths relative to the project directory', async () => {
		const parsed = await parse({
			kit: {
				files: {
					lib: DIR + '/src/lib',
					routes: DIR + '/src/routes',
					assets: DIR + '/static'
				}
			}
		});
		expect(parsed.lib_path).toBe('src/lib');
		expect(parsed.routes_path).toBe('src/routes');
		expect(parsed.assets_path).toBe('static');
	});

	// `env_dir` is serialized into the generated `$env/dynamic/*` modules,
	// so an absolute path would bake the build machine's directory into server bundles.
	test('makes an absolute env dir relative so it stays portable', async () => {
		expect((await parse({ kit: { env: { dir: DIR } } })).env_dir).toBe('.');
		expect((await parse({ kit: { env: { dir: DIR + '/config' } } })).env_dir).toBe('config');
		expect((await parse({ kit: { env: { dir: 'config' } } })).env_dir).toBe('config');
	});

	describe('alias', () => {
		test('points `$lib` at the lib path, like SvelteKit', async () => {
			expect((await parse({})).alias.$lib).toBe('src/lib');
			expect((await parse({ kit: { files: { lib: 'src/library' } } })).alias.$lib).toBe(
				'src/library'
			);
		});

		test('includes configured aliases and lets them override `$lib`', async () => {
			const parsed = await parse({ kit: { alias: { $routes: 'src/routes', $lib: 'elsewhere' } } });
			expect(parsed.alias.$routes).toBe('src/routes');
			expect(parsed.alias.$lib).toBe('elsewhere');
		});
	});

	describe('svelte_compile_options', () => {
		test('defaults to generating for the server', async () => {
			expect((await parse({})).svelte_compile_options.generate).toBe('server');
			expect((await parse({})).svelte_compile_module_options.generate).toBe('server');
		});

		// `generate` is cast in because SvelteKit omits it from its own `compilerOptions` type -
		// it reaches Gro from plain Svelte projects, which configure the compiler through Vite.
		test('preserves configured compiler options', async () => {
			const parsed = await parse({
				compilerOptions: { runes: true, generate: 'client' } as SvelteConfig['compilerOptions']
			});
			expect(parsed.svelte_compile_options.generate).toBe('client');
			expect(parsed.svelte_compile_options.runes).toBe(true);
		});

		test('does not mutate the source config', async () => {
			const compilerOptions = { runes: true };
			await parse({ compilerOptions });
			expect(compilerOptions).toEqual({ runes: true });
		});
	});

	test('passes the config through unparsed properties', async () => {
		const svelte_config: SvelteConfig = { kit: { paths: { base: '/base' } } };
		const parsed = await parse(svelte_config);
		expect(parsed.svelte_config).toBe(svelte_config);
		expect(parsed.base_url).toBe('/base');
	});
});

describe('load_default_svelte_config', () => {
	// Uses `DIR`, which holds no config, so the Vite-resolving entry stays cheap -
	// resolving a real project through Vite costs a full Vite config resolution
	// and calls `process.chdir`, which is not something to do from a unit test.
	test('memoizes per directory and resolution mode', async () => {
		const authored = load_default_svelte_config({ dir: DIR, resolve_with_vite: false });
		expect(load_default_svelte_config({ dir: DIR, resolve_with_vite: false })).toBe(authored);
		// A different resolution mode is a different cache entry.
		const resolved_with_vite = load_default_svelte_config({ dir: DIR, resolve_with_vite: true });
		expect(resolved_with_vite).not.toBe(authored);
		// As is a different directory.
		const nested = load_default_svelte_config({ dir: DIR + '/nested', resolve_with_vite: false });
		expect(nested).not.toBe(authored);
		await Promise.all([authored, resolved_with_vite, nested]);
	});

	test('parses the config of the project it runs in', async () => {
		await expect(load_default_svelte_config({ resolve_with_vite: false })).resolves.toMatchObject({
			lib_path: 'src/lib'
		});
	});
});
