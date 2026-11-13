// on `node:fs`: finds the Gro install that the CLI spawns `node` with, bootstrapping only Node
import { realpathSync, existsSync } from 'node:fs';
import { constants } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { NODE_MODULES_DIRNAME, SVELTEKIT_DIST_DIRNAME } from './constants.ts';
import type { SpawnProgram, SpawnResult } from './spawn.ts';

/*

This module is intended to have minimal dependencies to avoid over-imports in the CLI.

*/

/**
 * Resolves a path to an internal Gro file, preferring a local installation of Gro
 * and falling back to the running CLI's. Used by the CLI, `gro run`, and the server plugin.
 *
 * Paths resolve against the cwd, and the first case that applies wins:
 *
 * 1. a local install at `node_modules/.bin/gro`, followed through its symlink,
 *    so a project runs its own Gro rather than the global one
 * 2. a `dist/gro.js` with `dist/` holding `path`, as in Gro's own repo once built,
 *    so Gro runs its own build; an unbuilt repo falls through
 * 3. the Gro of the running CLI, which for the global CLI is the global installation
 */
export const resolve_gro_module_path = (path = ''): string => {
	const gro_bin_path = resolve(NODE_MODULES_DIRNAME, '.bin/gro');
	// case 1
	// Prefer any locally installed version of Gro.
	// This is really confusing if Gro is installed inside Gro itself,
	// so avoid that when developing Gro.
	if (existsSync(gro_bin_path)) {
		return join(realpathSync(gro_bin_path), '..', path);
	}
	// case 2
	// a built `dist/` in the cwd, as in Gro's own repo; without one, the running CLI's
	if (
		existsSync(join(SVELTEKIT_DIST_DIRNAME, 'gro.js')) &&
		existsSync(join(SVELTEKIT_DIST_DIRNAME, path))
	) {
		return resolve(SVELTEKIT_DIST_DIRNAME, path);
	}
	// case 3
	// Fall back to the version associated with the running CLI.
	const file_path = fileURLToPath(import.meta.url);
	return join(file_path, '..', path);
};

/**
 * The exit code for a child killed by `signal`, `128` plus its number, like a shell.
 * @param signal - a signal name from a `SpawnResult`, like `SIGTERM`
 */
export const to_signal_exit_code = (signal: string): number =>
	128 + constants.signals[signal as keyof typeof constants.signals];

/**
 * Runs a file using the Gro loader.
 *
 * Uses conditional exports to correctly set up `esm-env` as development by default,
 * so if you want production set `NODE_ENV=production`.
 *
 * @param node - the `node` program, whose environment the root decides
 * @param loader_path - path to loader
 * @param invoke_path - path to file to spawn with `node`
 * @param argv - the args after `invoke_path`
 * @param node_env - the host's `NODE_ENV`, read by the root
 * @see https://nodejs.org/api/packages.html#conditional-exports
 */
export const spawn_with_loader = (
	node: SpawnProgram,
	loader_path: string,
	invoke_path: string,
	argv: ReadonlyArray<string>,
	node_env: string | undefined
): Promise<SpawnResult> =>
	node.run(spawn_with_loader_args(loader_path, invoke_path, argv, node_env));

/**
 * The Node args that run `invoke_path` with Gro's loader registered.
 * @param node_env - the host's `NODE_ENV`, read by the root
 */
export const spawn_with_loader_args = (
	loader_path: string,
	invoke_path: string,
	argv: ReadonlyArray<string>,
	node_env: string | undefined
): Array<string> => {
	const args = [
		'--import',
		// does what `register.ts` does without importing another file; the source is encoded,
		// and the loader registered by its file URL, so URL or JS string syntax in its path is data,
		// though Node's resolver refuses a `\` in it, which the file URL encodes
		to_register_loader_url(loader_path),
		// @sync Node options to `gro.ts`
		'--experimental-import-meta-resolve' // for `import.meta.resolve`
	];
	// In almost all cases we want the exports condition to be `"development"`. Needed for `esm-env`.
	if (node_env !== 'production') {
		args.push('-C', 'development'); // same as `--conditions`
	}
	args.push(invoke_path, ...argv);
	return args;
};

// the `data:` URL of a module that registers the loader at `loader_path`, for `--import`
const to_register_loader_url = (loader_path: string): string =>
	'data:text/javascript,' +
	encodeURIComponent(
		`import {register} from "node:module"; register(${JSON.stringify(pathToFileURL(loader_path).href)});`
	);
