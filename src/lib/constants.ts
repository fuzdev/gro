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
export const README_FILENAME = 'README.md';
export const SVELTE_CONFIG_FILENAME = 'svelte.config.js';
export const VITE_CONFIG_FILENAME = 'vite.config.ts';
export const NODE_MODULES_DIRNAME = 'node_modules';
export const PACKAGE_JSON_FILENAME = 'package.json';
export const LOCKFILE_FILENAME = 'package-lock.json';
export const SVELTEKIT_DEV_DIRNAME = '.svelte-kit'; // TODO use Svelte config value `outDir`
export const SVELTEKIT_BUILD_DIRNAME = 'build';
export const SVELTEKIT_DIST_DIRNAME = 'dist';
export const SVELTEKIT_VITE_CACHE_PATH = NODE_MODULES_DIRNAME + '/.vite';
export const GITHUB_DIRNAME = '.github';
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
export const VITE_CLI = 'vite';
export const VITEST_CLI = 'vitest';
