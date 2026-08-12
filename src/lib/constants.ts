/*

This module is intended to have no dependencies to avoid over-imports in the CLI and loader.
If any of these become customizable from SvelteKit or Gro's configs, move them to `./paths.ts`.

*/

// TODO the slashes here are kinda gross - do we want to maintain the convention to have the trailing slash in most usage?

export const SOURCE_DIRNAME = 'src';
export const GRO_DIRNAME = '.gro';
export const GRO_DIST_PREFIX = 'dist_'; //
export const SERVER_DIST_PATH = 'dist_server'; // TODO should all of these be `_PATH` or should this be `DIRNAME`? also, add `_PLUGIN` to this name?
export const GRO_DEV_DIRNAME = GRO_DIRNAME + '/dev';
/** @trailing_slash */
export const SOURCE_DIR = SOURCE_DIRNAME + '/';
/** @trailing_slash */
export const GRO_DIR = GRO_DIRNAME + '/';
/** @trailing_slash */
export const GRO_DEV_DIR = GRO_DEV_DIRNAME + '/';
export const GRO_CONFIG_FILENAME = 'gro.config.ts';
/**
 * The conventional library directory, not the SvelteKit `kit.files.lib`, which lives here
 * rather than in `./paths.ts` because it's a constant that reads no config. Code that has to
 * honor a customized `files.lib` reads `lib_path` off a `ParsedSvelteConfig` instead, and
 * projects that move it can point `task_root_dirs` at the new location in `gro.config.ts`.
 */
export const LIB_DIRNAME = 'lib';
export const LIB_PATH = SOURCE_DIR + LIB_DIRNAME;
/** @trailing_slash */
export const LIB_DIR = LIB_PATH + '/';
/**
 * Every filename a Svelte config is loaded from, in `vite-plugin-svelte`'s precedence order.
 * Gro reads the Svelte config through Vite, never from these directly, but one of them is
 * still loaded when `sveltekit()` gets no inline options - so they're project files that Gro
 * formats, watches for a config it can't read through Vite, and keys its config cache on.
 * The list is `vite-plugin-svelte`'s rather than SvelteKit's, which is the wider of the two:
 * SvelteKit's own fallback loader reads only `.js` and `.ts`, but that fallback is the path
 * Gro doesn't take, and a config Gro can't see is one whose edits don't invalidate the cache.
 * @see https://svelte.dev/docs/kit/configuration
 */
export const SVELTE_CONFIG_FILENAMES = [
	'svelte.config.js',
	'svelte.config.ts',
	'svelte.config.mjs',
	'svelte.config.mts'
];
/**
 * SvelteKit's alias for the library directory.
 * Always `$lib` no matter where `files.lib` points.
 * @see https://svelte.dev/docs/kit/configuration#files
 */
export const SVELTEKIT_LIB_ALIAS = '$lib';
/**
 * Every filename Vite picks up as its config, in Vite's own precedence order.
 * Which one wins is Vite's call, so Gro treats them as a set rather than privileging
 * one extension - it detects a Vite config with all of them, and formats all of them.
 * @see https://vite.dev/config/
 */
export const VITE_CONFIG_FILENAMES = [
	'vite.config.js',
	'vite.config.mjs',
	'vite.config.ts',
	'vite.config.cjs',
	'vite.config.mts',
	'vite.config.cts'
];
export const NODE_MODULES_DIRNAME = 'node_modules';
export const PACKAGE_JSON_FILENAME = 'package.json';
export const LOCKFILE_FILENAME = 'package-lock.json';
export const SVELTEKIT_DEV_DIRNAME = '.svelte-kit'; // TODO use Svelte config value `outDir`
export const SVELTEKIT_BUILD_DIRNAME = 'build';
export const SVELTEKIT_DIST_DIRNAME = 'dist';
export const SVELTEKIT_VITE_CACHE_PATH = NODE_MODULES_DIRNAME + '/.vite';
export const GIT_DIRNAME = '.git';
export const TSCONFIG_FILENAME = 'tsconfig.json';

export const TS_MATCHER = /\.(ts|mts|cts)$/;
export const JS_MATCHER = /\.(js|mjs|cjs)$/;
export const JSON_MATCHER = /\.json$/;
export const SVELTE_MATCHER = /\.svelte$/;
export const SVELTE_RUNES_MATCHER = /\.svelte\.(js|ts)$/; // TODO probably let `.svelte.` appear anywhere - https://github.com/sveltejs/svelte/issues/11536
export const CSS_MATCHER = /\.css$/;
/** Extracts the script content from Svelte files. */
export const SVELTE_SCRIPT_MATCHER = /<script(?:\s+[^>]*)?>([\s\S]*?)<\/script>/gim; // TODO maybe this shouldnt be global? or make a getter?
export const SVELTEKIT_ENV_MATCHER = /^\$env\/(static|dynamic)\/(public|private)$/;
export const SVELTEKIT_GLOBAL_SPECIFIER = /^\$(env|app)\//;
export const EVERYTHING_MATCHER = /.*/;

export const JS_CLI_DEFAULT = 'node';
export const PM_CLI_DEFAULT = 'npm';
export const SVELTEKIT_CLI = 'svelte-kit';
export const SVELTE_CHECK_CLI = 'svelte-check';
export const SVELTE_PACKAGE_CLI = 'svelte-package';
export const SVELTE_PACKAGE_DEP_NAME = '@sveltejs/package';
export const SVELTEKIT_DEP_NAME = '@sveltejs/kit';
export const VITE_CLI = 'vite';
export const VITEST_CLI = 'vitest';
