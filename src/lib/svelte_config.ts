import type { Config as SvelteConfig } from '@sveltejs/kit';
import type { CompileOptions, ModuleCompileOptions, PreprocessorGroup } from 'svelte/compiler';
import { isAbsolute, join, relative } from 'node:path';
import { existsSync } from 'node:fs';
import { EMPTY_OBJECT } from '@fuzdev/fuz_util/object.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';

import {
	SVELTE_CONFIG_FILENAMES,
	SVELTEKIT_LIB_ALIAS,
	VITE_CONFIG_FILENAMES
} from './constants.ts';

/* eslint-disable @typescript-eslint/no-deprecated */
// see https://github.com/sveltejs/kit/discussions/14240

/*

This module is intended to have minimal dependencies to avoid over-imports in the CLI.
Loading is lazy and memoized - see `load_default_svelte_config`.

The Svelte config is read through Vite, never from `svelte.config.js` directly,
with the same `resolveConfig` call SvelteKit's own `load_config` makes.
So this sees exactly what SvelteKit sees - inline `sveltekit()` options when a project
passes them, and otherwise whatever SvelteKit loaded from `svelte.config.js` on its own.

A project with no Vite config is read as having no Svelte config, since there's nothing
to read one through, so projects that don't use Vite keep working on the defaults.
Having a Vite config that can't be resolved is an error rather than a fallback,
because falling back would mean compiling against the wrong config in silence.

Always the project in the cwd, never an arbitrary directory. SvelteKit resolves its
`files` and `env.dir` against its own cwd rather than the Vite `root` it's handed,
so a `dir` parameter here could only ever be half-honored.

*/

/**
 * The names of the Vite plugins that carry the resolved Svelte config, most specific first.
 * SvelteKit's `api.options` is the split config shape, with its own options under `kit` -
 * it's the only one SvelteKit itself reads. `vite-plugin-svelte`'s is that plugin's resolved
 * options, which carry no `kit` but overlap in `compilerOptions` and `preprocess`,
 * so a plain Svelte project still gets its compiler options and preprocessors.
 */
const CONFIG_PROVIDER_PLUGIN_NAMES = ['vite-plugin-sveltekit-setup', 'vite-plugin-svelte:config'];

/**
 * This module has no logger in scope - it's called from the Node loader and from
 * `load_default_svelte_config`, neither of which has one to pass in.
 * Exported so it can be silenced or redirected, e.g. `svelte_config_log.level = 'off'`.
 */
export const svelte_config_log = new Logger('svelte_config');

/**
 * The first of `filenames` that exists in `dir`, if any.
 * Which of several configs wins is Vite's and SvelteKit's call, not Gro's,
 * so their filenames are treated as a set rather than privileging one extension.
 */
const find_config_file = (dir: string, filenames: Array<string>): string | undefined =>
	filenames.find((filename) => existsSync(join(dir, filename)));

/**
 * Loads the Svelte config of the project in the cwd by resolving its Vite config.
 * @returns `null` if the project has no Vite config, or one that configures no Svelte plugin
 * @throws if the project has a Vite config but Vite isn't installed, or if it fails to resolve
 */
export const load_svelte_config = async (): Promise<SvelteConfig | null> => {
	const dir = process.cwd();
	if (!find_config_file(dir, VITE_CONFIG_FILENAMES)) {
		// A project with neither config simply isn't a Svelte project, but one with a Svelte config
		// and no Vite config looks configured while being silently ignored, so it gets a warning.
		const svelte_config_filename = find_config_file(dir, SVELTE_CONFIG_FILENAMES);
		if (svelte_config_filename) {
			svelte_config_log.warn(
				`Found ${svelte_config_filename} but no Vite config in ${dir},` +
					' so its preprocessors, aliases, and compiler options are being ignored.' +
					' Gro reads the Svelte config through Vite, the same as SvelteKit does.'
			);
		}
		return null;
	}

	let vite;
	try {
		vite = await import('vite');
	} catch (err) {
		// Only reachable with a Vite config in hand, so the project is meant to build with Vite
		// and can't. Degrading would mean compiling against the wrong config in silence.
		throw new Error(`Found a Vite config at ${dir} but failed to import Vite`, { cause: err });
	}

	let resolved;
	// `resolveConfig` writes `process.env.NODE_ENV` when it's unset, and reading the config is
	// not a decision about the process. Left alone, the `development` it writes is inherited by
	// the `vite build` that `gro build` spawns, which builds for production as if for dev.
	// The restore can't cover the await itself, but it bounds the window to this call.
	const node_env = process.env.NODE_ENV;
	try {
		// The same call SvelteKit's `load_config` makes, minus the `process.chdir` it needs
		// only to point at another directory - so Gro reads the config SvelteKit would read.
		// No `root` or `configFile`, so Vite picks its own config the way it does everywhere
		// else; `logLevel` is Gro's, to keep config reads off the CLI's output.
		resolved = await vite.resolveConfig(
			{ logLevel: 'error' },
			'build',
			process.env.MODE ?? 'production'
		);
	} catch (err) {
		throw new Error(`Failed to resolve the Vite config at ${dir}`, { cause: err });
	} finally {
		if (node_env === undefined) {
			delete process.env.NODE_ENV;
		} else {
			process.env.NODE_ENV = node_env;
		}
	}

	for (const name of CONFIG_PROVIDER_PLUGIN_NAMES) {
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
 * Resolving through Vite yields absolute `files` paths, but Gro's vocabulary is relative
 * to the project directory. The cwd is the base because that's what SvelteKit resolved
 * them against, and what every consumer of these paths resolves them against in turn.
 */
const to_project_relative_path = (path: string | undefined): string | undefined =>
	path === undefined || !isAbsolute(path) ? path : relative(process.cwd(), path) || '.';

/**
 * Gro compiles for the server by default,
 * because SvelteKit handles the client in the normal cases.
 * Frozen because it's handed out as a default value.
 */
export const SVELTE_COMPILE_OPTIONS_DEFAULT: CompileOptions = Object.freeze({ generate: 'server' });

export interface ParseSvelteConfigOptions {
	/**
	 * An already-loaded config to parse instead of resolving the project's Vite config.
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
	const svelte_config = options.svelte_config ?? (await load_svelte_config());

	const kit = svelte_config?.kit;

	const assets_path = to_project_relative_path(kit?.files?.assets) ?? 'static';
	const lib_path = to_project_relative_path(kit?.files?.lib) ?? 'src/lib';
	const routes_path = to_project_relative_path(kit?.files?.routes) ?? 'src/routes';

	// SvelteKit always names this alias `$lib` and points it at `files.lib`.
	// @see https://svelte.dev/docs/kit/configuration#alias
	const alias = { [SVELTEKIT_LIB_ALIAS]: lib_path, ...kit?.alias };

	const base_url = kit?.paths?.base;
	const assets_url = kit?.paths?.assets;

	// Relative like the paths above, and for a sharper reason: `env_dir` is serialized into
	// the generated `$env/dynamic/*` modules, so an absolute path from Vite resolution would
	// bake the build machine's directory into server bundles.
	const env_dir = to_project_relative_path(kit?.env?.dir);
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

let default_svelte_config: Promise<ParsedSvelteConfig> | undefined;

/**
 * The parsed Svelte config for the project in the cwd, memoized.
 *
 * Reading it costs a full Vite config resolution, which runs every Vite plugin's
 * config hooks, so callers pull it in on demand instead of paying for it
 * on every Gro invocation.
 */
export const load_default_svelte_config = (): Promise<ParsedSvelteConfig> => {
	if (default_svelte_config === undefined) {
		const loading = (default_svelte_config = parse_svelte_config());
		// Evict failures so a long-lived process like `gro dev` picks up a fixed config.
		// Attaching the handler here also keeps the cached promise from being reported
		// as an unhandled rejection when nothing has awaited it yet.
		void loading.catch(() => {
			if (default_svelte_config === loading) {
				default_svelte_config = undefined;
			}
		});
	}
	return default_svelte_config;
};
