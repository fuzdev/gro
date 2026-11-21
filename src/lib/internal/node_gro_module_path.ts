/**
 * Finds the Gro that the CLI and the tasks that spawn `node` with Gro's loader run.
 *
 * Node-only: it reads the filesystem synchronously, before any task, and finds the running
 * Gro from this module's own URL.
 *
 * @module
 */

import { existsSync, realpathSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { NODE_MODULES_DIRNAME, SVELTEKIT_DIST_DIRNAME } from '../constants.ts';

/**
 * Resolves a path to an internal Gro file, preferring a local installation of Gro
 * and falling back to the running CLI's. Used by the CLI, `gro run`, and `gro dev` and `gro build`
 * for their plugins' `loader_path`.
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
	return join(fileURLToPath(import.meta.url), '../..', path);
};
