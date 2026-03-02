import type {Config as SvelteConfig} from '@sveltejs/kit';
import type {CompileOptions, ModuleCompileOptions, PreprocessorGroup} from 'svelte/compiler';
import {join} from 'node:path';
import {EMPTY_OBJECT} from '@fuzdev/fuz_util/object.js';

import {SVELTE_CONFIG_FILENAME} from './constants.ts';

/* eslint-disable @typescript-eslint/no-deprecated */
// see https://github.com/sveltejs/kit/discussions/14240

/*

This module is intended to have minimal dependencies to avoid over-imports in the CLI.

*/

/**
 * Loads a SvelteKit config at `dir`.
 * @returns `null` if no config is found
 */
export const load_svelte_config = async ({
	dir = process.cwd(),
	config_filename = SVELTE_CONFIG_FILENAME,
}: {dir?: string; config_filename?: string} = EMPTY_OBJECT): Promise<SvelteConfig | null> => {
	try {
		return (await import(join(dir, config_filename))).default;
	} catch (_err) {
		return null;
	}
};

/**
 * A subset of SvelteKit's config in a form that Gro uses
 * because SvelteKit doesn't expose its config resolver.
 * Flattens things out to keep them simple and easy to pass around,
 * and doesn't deal with most properties, but includes the full `svelte_config`.
 * The `base` and `assets` in particular are renamed for clarity with Gro's internal systems,
 * so these properties become first-class vocabulary inside Gro.
 */
export interface ParsedSvelteConfig {
	// TODO probably fill these out with defaults
	svelte_config: SvelteConfig | null;
	alias: Record<string, string>;
	base_url: '' | `/${string}` | undefined;
	assets_url: '' | `http://${string}` | `https://${string}` | undefined;

	// TODO others, but maybe replace with a Zod schema? https://svelte.dev/docs/kit/configuration
	/**
	 * Same as the SvelteKit `files.assets`.
	 */
	assets_path: string;
	/**
	 * Same as the SvelteKit `files.lib`.
	 */
	lib_path: string;
	/**
	 * Same as the SvelteKit `files.routes`.
	 */
	routes_path: string;

	env_dir: string | undefined;
	private_prefix: string | undefined;
	public_prefix: string | undefined;
	svelte_compile_options: CompileOptions;
	svelte_compile_module_options: ModuleCompileOptions;
	svelte_preprocessors: PreprocessorGroup | Array<PreprocessorGroup> | undefined;
}

// TODO currently incomplete and hack - maybe rethink
/**
 * Returns Gro-relevant properties of a SvelteKit config
 * as a convenience wrapper around `load_svelte_config`.
 * Needed because SvelteKit doesn't expose its config resolver.
 */
export const parse_svelte_config = async ({
	dir_or_config = process.cwd(), // TODO maybe not the best API, maybe a type union? `({svelte_config} | {dir}) & {config_filename}`
	config_filename = SVELTE_CONFIG_FILENAME,
}: {
	dir_or_config?: string | SvelteConfig;
	config_filename?: string;
} = EMPTY_OBJECT): Promise<ParsedSvelteConfig> => {
	const svelte_config =
		typeof dir_or_config === 'string'
			? await load_svelte_config({dir: dir_or_config, config_filename})
			: dir_or_config;

	const kit = svelte_config?.kit;

	const alias = {$lib: 'src/lib', ...kit?.alias};

	const base_url = kit?.paths?.base;
	const assets_url = kit?.paths?.assets;

	// TODO probably a Zod schema instead
	const assets_path = kit?.files?.assets ?? 'static';
	const lib_path = kit?.files?.lib ?? 'src/lib';
	const routes_path = kit?.files?.routes ?? 'src/routes';

	const env_dir = kit?.env?.dir;
	const private_prefix = kit?.env?.privatePrefix;
	const public_prefix = kit?.env?.publicPrefix;

	const svelte_compile_options: CompileOptions = svelte_config?.compilerOptions ?? {};
	// Change the default to `generate: 'server'`,
	// because SvelteKit handles the client in the normal cases.
	if (svelte_compile_options.generate === undefined) {
		svelte_compile_options.generate = 'server';
	}
	const svelte_compile_module_options = to_default_compile_module_options(svelte_compile_options); // TODO will kit have these separately?
	const svelte_preprocessors = svelte_config?.preprocess;

	return {
		svelte_config,
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
		svelte_preprocessors,
	};
};

export const to_default_compile_module_options = ({
	dev,
	generate,
	filename,
	rootDir,
	warningFilter,
}: CompileOptions): ModuleCompileOptions => ({dev, generate, filename, rootDir, warningFilter});

/**
 * The parsed SvelteKit config for the cwd, cached globally at the module level.
 */
export const default_svelte_config = await parse_svelte_config(); // always load it to keep things simple ahead
