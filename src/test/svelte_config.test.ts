import { describe, test, expect, vi } from 'vitest';
import type { Config as SvelteConfig } from '@sveltejs/kit';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
	load_default_svelte_config,
	load_svelte_config,
	parse_svelte_config,
	svelte_config_log
} from '$lib/svelte_config.ts';

// The project directory is always the cwd - see `svelte_config.ts` for why it can't be anything else.
const DIR = process.cwd();

const parse = (svelte_config: SvelteConfig) => parse_svelte_config({ svelte_config });

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
	test('memoizes', () => {
		expect(load_default_svelte_config()).toBe(load_default_svelte_config());
	});

	// Resolves this project's own `vite.config.ts` through Vite, the way every Gro
	// invocation does, so it covers reading the config off the SvelteKit plugin.
	// `env_dir` is `'.'` rather than undefined because SvelteKit defaults `kit.env.dir`
	// to its own cwd, so the real path always yields an absolute one to rebase.
	test('resolves the config of the project it runs in', async () => {
		await expect(load_default_svelte_config()).resolves.toMatchObject({
			lib_path: 'src/lib',
			routes_path: 'src/routes',
			env_dir: '.'
		});
	});
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

	/**
	 * Loads the config in an empty dir seeded with `files`, capturing anything warned.
	 * `Logger` defaults to `'off'` under Vitest, so the level is opted back in here.
	 */
	const load_with_warnings = async (
		files: Record<string, string>
	): Promise<{ loaded: SvelteConfig | null; warnings: Array<string> }> => {
		const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
		svelte_config_log.level = 'warn';
		try {
			const loaded = await in_empty_dir(async () => {
				for (const [filename, content] of Object.entries(files)) writeFileSync(filename, content);
				return load_svelte_config();
			});
			return { loaded, warnings: warn.mock.calls.map((c) => c.join(' ')) };
		} finally {
			svelte_config_log.clear_level_override();
			warn.mockRestore();
		}
	};

	// A project with neither config isn't a Svelte project, so it gets no warning,
	// but one with a Svelte config and no Vite config is silently ignored without this.
	test('warns when a Svelte config has no Vite config to be read through', async () => {
		const { loaded, warnings } = await load_with_warnings({
			'svelte.config.js': 'export default {};'
		});
		expect(loaded).toBe(null);
		expect(warnings).toHaveLength(1);
		expect(warnings[0]).toContain('svelte.config.js');
		expect(warnings[0]).toContain('no Vite config');
	});

	// The same silent ignore, one step further in: a `vite.config.js` that sets up something other
	// than Svelte - Vitest, most plausibly - alongside a `svelte.config.js` doing the real work.
	test('warns when the Vite config configures no Svelte plugin', async () => {
		const { loaded, warnings } = await load_with_warnings({
			'vite.config.js': 'export default {};',
			'svelte.config.js': 'export default {};'
		});
		expect(loaded).toBe(null);
		expect(warnings).toHaveLength(1);
		expect(warnings[0]).toContain('svelte.config.js');
		expect(warnings[0]).toContain('no Svelte plugin');
	});

	// Only the Svelte config makes the silence worth warning about -
	// a project with a Vite config and no Svelte config is configuring nothing to ignore.
	test('stays quiet when there is no Svelte config to ignore', async () => {
		const { loaded, warnings } = await load_with_warnings({
			'vite.config.js': 'export default {};'
		});
		expect(loaded).toBe(null);
		expect(warnings).toHaveLength(0);
	});

	// Vite's `resolveConfig` writes `NODE_ENV` when it's unset, and the `development` it would
	// leave behind is inherited by the `vite build` that `gro build` spawns, which then builds
	// for production as if for dev. Uses `load_svelte_config` rather than the memoized wrapper
	// so the resolution actually runs.
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
	});
});
