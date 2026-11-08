import type { Config as SvelteConfig } from '@sveltejs/kit/vite';
import type { CompileOptions, ModuleCompileOptions, PreprocessorGroup } from 'svelte/compiler';
import { isAbsolute, join, relative } from 'node:path';
// on `node:fs`: a loader helper, reading the Svelte config through Vite, which runs only on Node
import { existsSync } from 'node:fs';
import { EMPTY_OBJECT } from '@fuzdev/fuz_util/object.ts';
import { Logger } from '@fuzdev/fuz_util/log.ts';

import { VITE_CONFIG_FILENAMES } from './constants.ts';

// SvelteKit deprecates `alias` and `files` but still applies them - Gro reads `files`,
// and warns about `alias`, which it doesn't support
/* eslint-disable @typescript-eslint/no-deprecated */

/*

This module is intended to have minimal dependencies to avoid over-imports in the CLI.
Loading is lazy and memoized - see `load_default_svelte_config`.

The Svelte config is read through Vite, never from `svelte.config.js` directly,
with the same `resolveConfig` call SvelteKit's own `load_config` makes.
So this sees exactly what SvelteKit sees - the options passed to the `sveltekit()` plugin,
the only place SvelteKit reads its config from (it throws if a `svelte.config.js` exists).

A project with no Vite config, or one whose Vite config configures no Svelte plugin,
is read as having no Svelte config, so projects that don't use Vite keep working on the
defaults. Having a Vite config that can't be resolved is an error rather than a fallback,
because falling back would mean compiling against the wrong config in silence.

The `api.options` read back off `vite-plugin-svelte` is that plugin's *resolved* options,
not the user's, so a plain Svelte project's `compilerOptions` arrive with the plugin's own
`css`, `dev`, and `hmr` mixed in, resolved for the `build`/`production` pass below.
It deletes `generate` (along with `format` and `filename`), so Gro's server default survives,
and consumers that care about `dev` set it themselves - the loader always compiles for dev.

Always the project in the cwd, never an arbitrary directory - Gro resolves the cwd's
Vite config, the way SvelteKit's own CLI does.

*/

/**
 * The names of the Vite plugins that carry the resolved Svelte config, most specific first.
 * SvelteKit's `api.options` is its validated config, flat with its defaults filled in -
 * it's the only one SvelteKit itself reads. `vite-plugin-svelte`'s is that plugin's resolved
 * options, which lack SvelteKit's but overlap in `compilerOptions` and `preprocess`,
 * so a plain Svelte project still gets its compiler options and preprocessors.
 */
const CONFIG_PROVIDER_PLUGIN_NAMES = ['vite-plugin-sveltekit-setup', 'vite-plugin-svelte:config'];

/**
 * Whether the project in `dir` has a Vite config, and so anything to read a Svelte config through.
 * Which of several configs wins is Vite's call, not Gro's,
 * so their filenames are treated as a set rather than privileging one extension.
 */
const has_vite_config = (dir: string): boolean =>
	VITE_CONFIG_FILENAMES.some((filename) => existsSync(join(dir, filename)));

/**
 * Loads the Svelte config of the project in the cwd by resolving its Vite config.
 * @returns `null` if the project has no Vite config, or one that configures no Svelte plugin
 * @throws if the project has a Vite config but Vite isn't installed, or if it fails to resolve
 */
export const load_svelte_config = async (): Promise<SvelteConfig | null> => {
	const dir = process.cwd();
	if (!has_vite_config(dir)) return null;

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
	base_url: '' | `/${string}` | undefined;
	assets_url: '' | `http://${string}` | `https://${string}` | undefined;

	// TODO others, but maybe replace with a fuz_schema schema? https://svelte.dev/docs/kit/configuration
	/**
	 * Same as the SvelteKit `files.routes`, relative to the project directory.
	 */
	routes_path: string;
	/**
	 * Same as the SvelteKit `files.src`, relative to the project directory.
	 * The env declarations entry `env.ts` lives here.
	 */
	src_path: string;

	env_dir: string | undefined;
	/**
	 * Same as the SvelteKit `version.name`, exposed as `version` by the `$app/env` shim.
	 */
	version_name: string | undefined;
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

export interface ParseSvelteConfigOptions {
	/**
	 * An already-loaded config to parse instead of resolving the project's Vite config.
	 */
	svelte_config?: SvelteConfig;
}

const log = new Logger('svelte_config');

/**
 * SvelteKit still applies its deprecated `alias`, so a project that sets it resolves
 * differently under Vite than under Gro's loader - loud, not silent.
 */
const warn_unsupported_alias = (alias: Record<string, string> | undefined): void => {
	if (!alias) return;
	const keys = Object.keys(alias);
	if (!keys.length) return;
	log.warn(
		`SvelteKit's deprecated \`alias\` is set (${keys.join(', ')}) but Gro doesn't support it,` +
			' so Gro resolves those imports differently than Vite does.' +
			' Use package.json subpath imports like `#lib/*` instead.'
	);
};

/**
 * Returns Gro-relevant properties of a SvelteKit config
 * as a convenience wrapper around `load_svelte_config`.
 */
export const parse_svelte_config = async (
	options: ParseSvelteConfigOptions = EMPTY_OBJECT
): Promise<ParsedSvelteConfig> => {
	const svelte_config = options.svelte_config ?? (await load_svelte_config());

	warn_unsupported_alias(svelte_config?.alias);

	const routes_path = to_project_relative_path(svelte_config?.files?.routes) ?? 'src/routes';
	const src_path = to_project_relative_path(svelte_config?.files?.src) ?? 'src';

	const base_url = svelte_config?.paths?.base;
	const assets_url = svelte_config?.paths?.assets;

	// Relative like the paths above: `env_dir` is serialized into
	// the generated `$app/env/*` modules, which resolve it against the project.
	const env_dir = to_project_relative_path(svelte_config?.env?.dir);
	const version_name = svelte_config?.version?.name;

	const svelte_compile_options: CompileOptions = { ...svelte_config?.compilerOptions };
	// Gro compiles for the server, since SvelteKit handles the client
	svelte_compile_options.generate ??= 'server';
	// TODO will kit have these separately?
	// the loader sets `dev` and `filename` per module
	const { generate, rootDir, warningFilter } = svelte_compile_options;
	const svelte_compile_module_options: ModuleCompileOptions = { generate, rootDir, warningFilter };
	const svelte_preprocessors = svelte_config?.preprocess;

	return {
		svelte_config: svelte_config ?? null,
		base_url,
		assets_url,
		routes_path,
		src_path,
		env_dir,
		version_name,
		svelte_compile_options,
		svelte_compile_module_options,
		svelte_preprocessors
	};
};

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
