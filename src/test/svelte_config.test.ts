import { describe, test, expect } from 'vitest';
import type { Config as SvelteConfig } from '@sveltejs/kit/vite';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	load_default_svelte_config,
	load_svelte_config,
	parse_svelte_config
} from '#lib/svelte_config.ts';

// The project directory is always the cwd - see `svelte_config.ts` for why it can't be anything else.
const DIR = process.cwd();

const parse = (svelte_config: SvelteConfig) => parse_svelte_config({ svelte_config });

describe('parse_svelte_config', () => {
	test('falls back to the conventional paths when nothing is configured', async () => {
		const parsed = await parse({});
		expect(parsed.routes_path).toBe('src/routes');
		expect(parsed.src_path).toBe('src');
		expect(parsed.assets_path).toBe('static');
		expect(parsed.env_dir).toBe(undefined);
		expect(parsed.version_name).toBe(undefined);
	});

	test('keeps already-relative paths as authored', async () => {
		const parsed = await parse({ files: { src: 'source', routes: 'source/pages' } });
		expect(parsed.src_path).toBe('source');
		expect(parsed.routes_path).toBe('source/pages');
	});

	// Resolving through Vite yields absolute paths, but Gro's vocabulary is project-relative.
	test('makes absolute paths relative to the project directory', async () => {
		const parsed = await parse({
			files: {
				src: DIR + '/src',
				routes: DIR + '/src/routes',
				assets: DIR + '/static'
			}
		});
		expect(parsed.src_path).toBe('src');
		expect(parsed.routes_path).toBe('src/routes');
		expect(parsed.assets_path).toBe('static');
	});

	// `env_dir` is serialized into the generated `$app/env/*` modules,
	// so an absolute path would bake the build machine's directory into server bundles.
	test('makes an absolute env dir relative so it stays portable', async () => {
		expect((await parse({ env: { dir: DIR } })).env_dir).toBe('.');
		expect((await parse({ env: { dir: DIR + '/config' } })).env_dir).toBe('config');
		expect((await parse({ env: { dir: 'config' } })).env_dir).toBe('config');
	});

	test('reads the version name', async () => {
		expect((await parse({ version: { name: 'abc' } })).version_name).toBe('abc');
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
		const svelte_config: SvelteConfig = { paths: { base: '/base' } };
		const parsed = await parse(svelte_config);
		expect(parsed.svelte_config).toBe(svelte_config);
		expect(parsed.base_url).toBe('/base');
	});
});

describe('load_default_svelte_config', () => {
	test('memoizes', () => {
		expect(load_default_svelte_config()).toBe(load_default_svelte_config());
	});

	// Resolves this project's own `vite.config.ts` through Vite, the way every Gro
	// invocation does, so it covers reading the config off the SvelteKit plugin.
	// `env_dir` is `'.'` rather than undefined because SvelteKit defaults `env.dir`
	// to its own cwd, so the real path always yields an absolute one to rebase.
	// The timeout is raised because a real resolution imports and runs the whole plugin
	// graph - ~3s cold in this repo, close enough to the 5s default to time out when
	// another test file is competing for the machine.
	test('resolves the config of the project it runs in', async () => {
		const parsed = await load_default_svelte_config();
		expect(parsed).toMatchObject({ src_path: 'src', routes_path: 'src/routes', env_dir: '.' });
		// the flat SvelteKit 3 shape, read off the plugin's validated options
		expect(parsed.svelte_config?.paths?.relative).toBe(false);
		expect(parsed.version_name).toMatch(/^[0-9a-f]{40}$/);
	}, 30_000);
});

/**
 * Runs `fn` in an empty directory. The config is always read from the cwd,
 * so moving the cwd is the only way to point `load_svelte_config` somewhere else.
 * Note that `process.chdir` throws on a worker thread, so these tests need Vitest's
 * default `forks` pool - switching to `threads` would break them.
 */
const in_empty_dir = async <T>(fn: () => Promise<T>): Promise<T> => {
	const cwd = process.cwd();
	const dir = mkdtempSync(join(tmpdir(), 'gro_svelte_config_'));
	process.chdir(dir);
	try {
		return await fn();
	} finally {
		process.chdir(cwd);
		rmSync(dir, { recursive: true, force: true });
	}
};

describe('load_svelte_config', () => {
	// The path that keeps non-Vite projects working. Cheap too -
	// it short-circuits before Vite is imported at all.
	test('returns null when the project has no Vite config', async () => {
		await expect(in_empty_dir(load_svelte_config)).resolves.toBe(null);
	});

	// a Vite config that sets up something other than Svelte, Vitest most plausibly
	test('returns null when the Vite config configures no Svelte plugin', async () => {
		const loaded = await in_empty_dir(async () => {
			writeFileSync('vite.config.js', 'export default {};');
			return load_svelte_config();
		});
		expect(loaded).toBe(null);
	});

	// Vite's `resolveConfig` writes `NODE_ENV` when it's unset, and the `development` it would
	// leave behind is inherited by the `vite build` that `gro build` spawns, which then builds
	// for production as if for dev. Uses `load_svelte_config` rather than the memoized wrapper
	// so the resolution actually runs.
	// Raised for the same reason as the resolution test above.
	test('leaves NODE_ENV as it found it', async () => {
		const node_env = process.env.NODE_ENV;
		delete process.env.NODE_ENV;
		try {
			await load_svelte_config();
			expect('NODE_ENV' in process.env).toBe(false);
		} finally {
			if (node_env === undefined) {
				delete process.env.NODE_ENV;
			} else {
				process.env.NODE_ENV = node_env;
			}
		}
	}, 30_000);
});
