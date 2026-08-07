import type { Config as SvelteConfig } from '@sveltejs/kit';
import type { CompileOptions, ModuleCompileOptions, PreprocessorGroup } from 'svelte/compiler';
import { isAbsolute, join, relative, resolve } from 'node:path';
import { existsSync } from 'node:fs';
import { EMPTY_OBJECT } from '@fuzdev/fuz_util/object.ts';

import { SVELTEKIT_LIB_ALIAS, VITE_CONFIG_BASENAME, VITE_CONFIG_EXTENSIONS } from './constants.ts';

/* eslint-disable @typescript-eslint/no-deprecated */
// see https://github.com/sveltejs/kit/discussions/14240

/*

This module is intended to have minimal dependencies to avoid over-imports in the CLI.
Loading is lazy and memoized - see `load_default_svelte_config`.

The Svelte config is read through Vite, never from `svelte.config.js` directly.
SvelteKit resolves its own config the same way, so this sees exactly what
`vite dev` and `vite build` see - inline `sveltekit()` options when a project passes
them, and otherwise whatever SvelteKit loaded from `svelte.config.js` on its own.

*/

/**
 * The names of the Vite plugins that carry the resolved Svelte config,
 * in the order SvelteKit itself prefers them.
 */
const CONFIG_PROVIDER_PLUGIN_NAMES = ['vite-plugin-sveltekit-setup', 'vite-plugin-svelte:config'];

/**
 * Whether `dir` has a Vite config at all, used only to skip the work of loading Vite.
 * Which of several Vite configs wins is Vite's call, not Gro's.
 */
const has_vite_config = (dir: string): boolean =>
	VITE_CONFIG_EXTENSIONS.some((ext) => existsSync(join(dir, VITE_CONFIG_BASENAME + '.' + ext)));

export interface LoadSvelteConfigOptions {
	/**
	 * @default cwd
	 */
	dir?: string;
}

/**
 * Loads the Svelte config at `dir` by resolving its Vite config.
 * @returns `null` if `dir` has no Vite config, or one that configures no Svelte plugin
 * @throws if the Vite config is found but fails to resolve
 */
export const load_svelte_config = async (
	options: LoadSvelteConfigOptions = EMPTY_OBJECT
): Promise<SvelteConfig | null> => {
	// Normalized because SvelteKit compares the `root` it's given against the one it enforces
	// and prints a red warning when they differ - a trailing slash is enough to trip it.
	const dir = resolve(options.dir ?? process.cwd());
	if (!has_vite_config(dir)) return null;

	let vite;
	try {
		vite = await import('vite');
	} catch (_err) {
		// Vite isn't installed, so the project can't be built with it either.
		// Degrading beats throwing here - this runs in the Node loader on every invocation,
		// and a project in this state still needs to be able to run tasks like `gro sync`.
		return null;
	}

	let resolved;
	try {
		// No `configFile`, so Vite picks its own config the way it does everywhere else.
		// Unlike `@sveltejs/load-config` this doesn't `process.chdir` - Gro always resolves
		// the project it's running in, and chdir is unavailable on the loader's worker thread.
		resolved = await vite.resolveConfig({ root: dir, logLevel: 'error' }, 'serve');
	} catch (err) {
		throw new Error(`Failed to resolve the Vite config at ${dir}`, { cause: err });
	}

	for (const name of CONFIG_PROVIDER_PLUGIN_NAMES) {
		// `api.options` is the split config shape, with SvelteKit's options under `kit`.
		const options = resolved.plugins.find((p) => p.name === name)?.api?.options;
		if (options) return options as SvelteConfig;
	}
	return null;
};

/**
 * A subset of SvelteKit's config in a form that Gro uses.
 * Flattens things out to keep them simple and easy to pass around,
 * and doesn't deal with most properties, but includes the full `svelte_config`.
 * The `base` and `assets` in particular are renamed for clarity with Gro's internal systems,
 * so these properties become first-class vocabulary inside Gro.
 */
export interface ParsedSvelteConfig {
	svelte_config: SvelteConfig | null;
	alias: Record<string, string>;
	base_url: '' | `/${string}` | undefined;
	assets_url: '' | `http://${string}` | `https://${string}` | undefined;

	// TODO others, but maybe replace with a Zod schema? https://svelte.dev/docs/kit/configuration
	/**
	 * Same as the SvelteKit `files.assets`, relative to the project directory.
	 */
	assets_path: string;
	/**
	 * Same as the SvelteKit `files.lib`, relative to the project directory.
	 */
	lib_path: string;
	/**
	 * Same as the SvelteKit `files.routes`, relative to the project directory.
	 */
	routes_path: string;

	env_dir: string | undefined;
	private_prefix: string | undefined;
	public_prefix: string | undefined;
	svelte_compile_options: CompileOptions;
	svelte_compile_module_options: ModuleCompileOptions;
	svelte_preprocessors: PreprocessorGroup | Array<PreprocessorGroup> | undefined;
}

/**
 * Resolving through Vite yields absolute `files` paths,
 * but Gro's vocabulary is relative to the project directory.
 */
const to_project_relative_path = (path: string | undefined, dir: string): string | undefined =>
	path === undefined || !isAbsolute(path) ? path : relative(dir, path) || '.';

/**
 * Gro compiles for the server by default,
 * because SvelteKit handles the client in the normal cases.
 * Frozen because it's handed out as a default value.
 */
export const SVELTE_COMPILE_OPTIONS_DEFAULT: CompileOptions = Object.freeze({ generate: 'server' });

export type ParseSvelteConfigOptions =
	| LoadSvelteConfigOptions
	| {
			/**
			 * An already-loaded config to parse instead of reading one from `dir`.
			 */
			svelte_config: SvelteConfig;
			/**
			 * @default cwd
			 */
			dir?: string;
	  };

/**
 * Returns Gro-relevant properties of a SvelteKit config
 * as a convenience wrapper around `load_svelte_config`.
 */
export const parse_svelte_config = async (
	options: ParseSvelteConfigOptions = EMPTY_OBJECT
): Promise<ParsedSvelteConfig> => {
	const { dir = process.cwd() } = options;

	const svelte_config =
		'svelte_config' in options ? options.svelte_config : await load_svelte_config(options);

	const kit = svelte_config?.kit;

	const assets_path = to_project_relative_path(kit?.files?.assets, dir) ?? 'static';
	const lib_path = to_project_relative_path(kit?.files?.lib, dir) ?? 'src/lib';
	const routes_path = to_project_relative_path(kit?.files?.routes, dir) ?? 'src/routes';

	// SvelteKit always names this alias `$lib` and points it at `files.lib`.
	// @see https://svelte.dev/docs/kit/configuration#alias
	const alias = { [SVELTEKIT_LIB_ALIAS]: lib_path, ...kit?.alias };

	const base_url = kit?.paths?.base;
	const assets_url = kit?.paths?.assets;

	// Relative like the paths above, and for a sharper reason: `env_dir` is serialized into
	// the generated `$env/dynamic/*` modules, so an absolute path from Vite resolution would
	// bake the build machine's directory into server bundles.
	const env_dir = to_project_relative_path(kit?.env?.dir, dir);
	const private_prefix = kit?.env?.privatePrefix;
	const public_prefix = kit?.env?.publicPrefix;

	const svelte_compile_options: CompileOptions = { ...svelte_config?.compilerOptions };
	if (svelte_compile_options.generate === undefined) {
		svelte_compile_options.generate = SVELTE_COMPILE_OPTIONS_DEFAULT.generate;
	}
	const svelte_compile_module_options = to_default_compile_module_options(svelte_compile_options); // TODO will kit have these separately?
	const svelte_preprocessors = svelte_config?.preprocess;

	return {
		svelte_config: svelte_config ?? null,
		alias,
		base_url,
		assets_url,
		assets_path,
		lib_path,
		routes_path,
		env_dir,
		private_prefix,
		public_prefix,
		svelte_compile_options,
		svelte_compile_module_options,
		svelte_preprocessors
	};
};

export const to_default_compile_module_options = ({
	dev,
	generate,
	filename,
	rootDir,
	warningFilter
}: CompileOptions): ModuleCompileOptions => ({ dev, generate, filename, rootDir, warningFilter });

const default_svelte_configs: Map<string, Promise<ParsedSvelteConfig>> = new Map();

/**
 * The parsed Svelte config, memoized per directory.
 *
 * Reading it costs a full Vite config resolution, which runs every Vite plugin's
 * config hooks, so callers pull it in on demand instead of paying for it
 * on every Gro invocation.
 */
export const load_default_svelte_config = (
	options: LoadSvelteConfigOptions = EMPTY_OBJECT
): Promise<ParsedSvelteConfig> => {
	const { dir = process.cwd() } = options;
	const key = resolve(dir);
	let loading = default_svelte_configs.get(key);
	if (loading === undefined) {
		loading = parse_svelte_config({ dir });
		default_svelte_configs.set(key, loading);
		// Evict failures so a long-lived process like `gro dev` picks up a fixed config.
		// Attaching the handler here also keeps the cached promise from being reported
		// as an unhandled rejection when nothing has awaited it yet.
		const failed = loading;
		void loading.catch(() => {
			if (default_svelte_configs.get(key) === failed) {
				default_svelte_configs.delete(key);
			}
		});
	}
	return loading;
};
