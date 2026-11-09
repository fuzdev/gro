import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';

import { package_json_load_for_gro } from './package_json.ts';
import { NODE_MODULES_DIRNAME } from './constants.ts';
import type { FsPaths } from './fs_paths.ts';

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
	 * The running runtime's name and version, like `process.runtime_version`.
	 */
	runtime_version: string;
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
 * @param fs_paths - opens the project's `node_modules/`
 * @param runtime_version - the running runtime's name and version, like `process.runtime_version`
 */
export const load_build_cache_dependencies = async (
	fs_paths: FsPaths,
	runtime_version: string
): Promise<BuildCacheDependencies> => {
	const package_lock_hash = await hash_npm_hidden_lockfile(fs_paths);
	return {
		gro_version: package_json_load_for_gro().version ?? '',
		runtime_version,
		package_lock_hash
	};
};

const hash_npm_hidden_lockfile = async (fs_paths: FsPaths): Promise<string | null> => {
	const lockfile = await fs_paths.open_path(NPM_HIDDEN_LOCKFILE_PATH);
	const contents = await lockfile.dir.read_if_exists(lockfile.path);
	return contents === null ? null : hash_blake3(contents);
};
