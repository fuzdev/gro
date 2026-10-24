import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';

import { package_json_load_for_gro } from './package_json.ts';
import { paths } from './paths.ts';
import { NODE_MODULES_DIRNAME } from './constants.ts';

/**
 * npm's hidden lockfile, which records the packages installed in `node_modules`.
 */
export const NPM_HIDDEN_LOCKFILE_PATH = NODE_MODULES_DIRNAME + '/.package-lock.json';

/**
 * The build inputs outside git that the build cache key folds in:
 * the tools that run the build and the installed packages.
 */
export interface BuildCacheDependencies {
	/**
	 * Version of the running Gro, from its `package.json`.
	 * A Gro rebuilt without a version change keeps the same value.
	 */
	gro_version: string;
	/**
	 * `process.version` of the running Node.
	 */
	node_version: string;
	/**
	 * BLAKE3 hash of `node_modules/.package-lock.json`, which npm rewrites when it changes
	 * what's installed. `null` when the file is missing, as with other package managers,
	 * an npm workspace member, or no install, and then installed-package drift doesn't
	 * invalidate the cache.
	 * Edits made directly inside `node_modules` don't change the file either.
	 */
	package_lock_hash: string | null;
}

/**
 * Reads the build inputs outside git that the build cache key folds in.
 *
 * @param dir - project root holding `node_modules/`
 */
export const load_build_cache_dependencies = async (
	dir = paths.root
): Promise<BuildCacheDependencies> => {
	const [gro_package_json, package_lock_hash] = await Promise.all([
		package_json_load_for_gro(),
		hash_npm_hidden_lockfile(dir)
	]);
	return {
		gro_version: gro_package_json.version ?? '',
		node_version: process.version,
		package_lock_hash
	};
};

const hash_npm_hidden_lockfile = async (dir: string): Promise<string | null> => {
	let contents: Buffer;
	try {
		contents = await readFile(join(dir, NPM_HIDDEN_LOCKFILE_PATH));
	} catch (error) {
		if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null;
		throw error;
	}
	return hash_blake3(contents);
};
