import type { Config as SvelteConfig } from '@sveltejs/kit';
import type { CompileOptions, ModuleCompileOptions, PreprocessorGroup } from 'svelte/compiler';
import { isAbsolute, join, relative } from 'node:path';
import { loadConfig } from '@sveltejs/load-config';
import { EMPTY_OBJECT } from '@fuzdev/fuz_util/object.ts';

import { SVELTE_CONFIG_FILENAME, SVELTEKIT_LIB_ALIAS } from './constants.ts';

/* eslint-disable @typescript-eslint/no-deprecated */
// see https://github.com/sveltejs/kit/discussions/14240

/*

This module is intended to have minimal dependencies to avoid over-imports in the CLI.
Loading is lazy and memoized - see `load_default_svelte_config`.

*/

export interface LoadSvelteConfigOptions {
	/**
	 * @default cwd
	 */
	dir?: string;
	/**
	 * @default `SVELTE_CONFIG_FILENAME`
	 */
	config_filename?: string;
	/**
	 * Resolve the config through `vite.config` when one is present,
	 * which applies SvelteKit's own defaults and supports projects
	 * that configure Svelte from Vite instead of `svelte.config.js`.
	 *
	 * Costs a full Vite config resolution, and is unavailable on worker threads
	 * because it calls `process.chdir`, so the Node loader opts out.
	 * @default true
	 */
	resolve_with_vite?: boolean;
}

/**
 * Whether a load actually goes through Vite.
 * Vite's resolution finds `svelte.config.*` on its own, so a custom filename
 * can only be honored by importing that file directly.
 */
const uses_vite = (resolve_with_vite: boolean, config_filename: string): boolean =>
	resolve_with_vite && config_filename === SVELTE_CONFIG_FILENAME;

/**
 * Loads a SvelteKit config at `dir`.
 * @returns `null` if no config is found
 * @throws if a config is found but fails to load
 */
export const load_svelte_config = async ({
	dir = process.cwd(),
	config_filename = SVELTE_CONFIG_FILENAME,
	resolve_with_vite = true
}: LoadSvelteConfigOptions = EMPTY_OBJECT): Promise<SvelteConfig | null> => {
	const use_vite = uses_vite(resolve_with_vite, config_filename);
	// Passing the config file instead of its directory tells `loadConfig` to import it directly,
	// skipping both the `vite.config` lookup and the `process.chdir` it performs.
	const loaded = await loadConfig(use_vite ? dir : join(dir, config_filename), {
		traverse: false
	});
	if (!loaded) return null;
	if ('error' in loaded) {
		throw new Error(`Failed to load SvelteKit config at ${loaded.configFilePath}`, {
			cause: loaded.error
		});
	}
	// `loadConfig` types the config loosely (`kit` is `unknown`) because it also handles
	// plain Svelte projects, but everything Gro reads off it is optional and guarded.
	return loaded.config as SvelteConfig;
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
const to_project_relative_path = (
	path: string | undefined,
	dir: string,
	fallback: string
): string => {
	if (path === undefined) return fallback;
	if (!isAbsolute(path)) return path;
	return relative(dir, path) || '.';
};

/**
 * Gro compiles for the server by default,
 * because SvelteKit handles the client in the normal cases.
 */
export const SVELTE_COMPILE_OPTIONS_DEFAULT: CompileOptions = { generate: 'server' };

export interface ParseSvelteConfigOptions extends LoadSvelteConfigOptions {
	/**
	 * An already-loaded config to parse instead of reading one from `dir`.
	 */
	svelte_config?: SvelteConfig;
}

/**
 * Returns Gro-relevant properties of a SvelteKit config
 * as a convenience wrapper around `load_svelte_config`.
 */
export const parse_svelte_config = async (
	options: ParseSvelteConfigOptions = EMPTY_OBJECT
): Promise<ParsedSvelteConfig> => {
	const { dir = process.cwd() } = options;

	const svelte_config = options.svelte_config ?? (await load_svelte_config(options));

	const kit = svelte_config?.kit;

	const assets_path = to_project_relative_path(kit?.files?.assets, dir, 'static');
	const lib_path = to_project_relative_path(kit?.files?.lib, dir, 'src/lib');
	const routes_path = to_project_relative_path(kit?.files?.routes, dir, 'src/routes');

	// SvelteKit always names this alias `$lib` and points it at `files.lib`.
	// @see https://svelte.dev/docs/kit/configuration#alias
	const alias = { [SVELTEKIT_LIB_ALIAS]: lib_path, ...kit?.alias };

	const base_url = kit?.paths?.base;
	const assets_url = kit?.paths?.assets;

	// Relative like the paths above, and for a sharper reason: `env_dir` is serialized into
	// the generated `$env/dynamic/*` modules, so an absolute path from Vite resolution would
	// bake the build machine's directory into server bundles.
	const env_dir =
		kit?.env?.dir === undefined ? undefined : to_project_relative_path(kit.env.dir, dir, '.');
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
 * The parsed SvelteKit config, memoized per directory and resolution mode.
 *
 * Loading a SvelteKit config is expensive - it imports the config module and its
 * preprocessors, and resolving through Vite costs a full Vite config resolution -
 * so callers pull it in on demand instead of paying for it on every Gro invocation.
 */
export const load_default_svelte_config = (
	options: LoadSvelteConfigOptions = EMPTY_OBJECT
): Promise<ParsedSvelteConfig> => {
	const {
		dir = process.cwd(),
		config_filename = SVELTE_CONFIG_FILENAME,
		resolve_with_vite = true
	} = options;
	// Keyed on whether the load *actually* goes through Vite, not on what was asked for,
	// so a custom filename doesn't get one cache entry per requested mode for the same load.
	const key = `${uses_vite(resolve_with_vite, config_filename) ? 'vite' : 'svelte'}:${join(dir, config_filename)}`;
	let loading = default_svelte_configs.get(key);
	if (loading === undefined) {
		loading = parse_svelte_config({ dir, config_filename, resolve_with_vite });
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
