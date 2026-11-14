import { isAbsolute, join, normalize } from 'node:path';
import { styleText as st } from 'node:util';
import * as s from '@fuzdev/fuz_schema/schema.ts';
import { meta } from '@fuzdev/fuz_schema/meta.ts';
import type { Logger } from '@fuzdev/fuz_util/log.ts';
import { map_concurrent } from '@fuzdev/fuz_util/async.ts';
import { hash_blake3 } from '@fuzdev/fuz_util/hash_blake3.ts';
import { to_error_message } from '@fuzdev/fuz_util/error.ts';

import type { GroConfig } from './gro_config.ts';
import {
	SVELTEKIT_BUILD_DIRNAME,
	SVELTEKIT_DIST_DIRNAME,
	GRO_DIST_PREFIX,
	GRO_DIRNAME
} from './constants.ts';
import type { BuildCacheDependencies } from './build_cache_dependencies.ts';
import type { DirEntry, FsDir } from './fs.ts';
import { FsError, is_fs_error } from './fs_error.ts';
import type { FsOpenedPath, FsPaths } from './fs_paths.ts';
import { git_current_commit_hash } from './git.ts';
import type { SpawnProgram } from './spawn.ts';
import { gro_dir_ensure } from './internal/gro_dir.ts';

export const BUILD_CACHE_METADATA_FILENAME = 'build.json';
export const BUILD_CACHE_VERSION = '4';

/**
 * The build cache metadata's path, relative to the project.
 */
export const BUILD_CACHE_METADATA_PATH = GRO_DIRNAME + '/' + BUILD_CACHE_METADATA_FILENAME;

/**
 * What the build cache key reads when its commit wasn't given: git and a log.
 */
export interface BuildCacheKeyDeps {
	git: SpawnProgram;
	log: Logger;
}

/**
 * What saving the build cache's metadata reaches: the project's paths and a log.
 */
export interface BuildCacheSaveDeps {
	fs_paths: FsPaths;
	log: Logger;
}

/**
 * What the build cache's checks reach, its key and its save.
 */
export interface BuildCacheDeps extends BuildCacheKeyDeps, BuildCacheSaveDeps {}

/**
 * The inputs to the build cache key beside the config.
 */
export interface BuildCacheKeyOptions {
	/**
	 * The Gro and Node versions and the installed-package hash, loaded once by the caller
	 * (`load_build_cache_dependencies`) so one build checks and saves the same values.
	 */
	dependencies: BuildCacheDependencies;
	/**
	 * A pre-computed git commit hash, so a caller that has one needn't read it again;
	 * read from git when omitted.
	 */
	git_commit?: string | null;
}

export interface BuildCacheMetadataOptions extends BuildCacheKeyOptions {
	/**
	 * Pre-discovered build directories, so a caller that has them avoids another scan;
	 * discovered when omitted.
	 */
	build_dirs?: Array<string>;
}

/**
 * Metadata about a single build output file.
 * Includes cryptographic hash for validation plus filesystem stats for debugging and optimization.
 */
export const BuildOutputEntry = s.object({
	path: meta(s.string, {
		description: "relative path from project root (e.g., 'build/index.html')."
	}),
	hash: meta(s.string, { description: 'BLAKE3 hash of file contents' }),
	size: meta(s.number, { description: 'file size in bytes' }),
	mtime: meta(s.number, { description: 'modification time in milliseconds since epoch' }),
	mode: meta(s.number, {
		description: 'unix permission bits, without the file type bits (e.g., 420 = 0o644)'
	})
});
export type BuildOutputEntry = s.Infer<typeof BuildOutputEntry>;

/**
 * Metadata stored in .gro/ directory to track build cache validity.
 * Schema validates structure at load time to catch corrupted cache files.
 */
export const BuildCacheMetadata = s.object({
	version: meta(s.string, { description: 'schema version for future compatibility' }),
	git_commit: meta(s.nullable(s.string), { description: 'git commit hash at time of build' }),
	build_cache_config_hash: meta(s.string, {
		description: "hash of user's custom build_cache_config from gro.config.ts."
	}),
	gro_version: meta(s.string, { description: 'version of the Gro that ran the build' }),
	runtime_version: meta(s.string, {
		description: 'name and version of the runtime that ran the build, `runtime_version`'
	}),
	package_lock_hash: meta(s.nullable(s.string), {
		description: 'BLAKE3 hash of node_modules/.package-lock.json, null when missing'
	}),
	timestamp: meta(s.string, { description: 'timestamp when build completed' }),
	outputs: meta(s.array(BuildOutputEntry), {
		description: 'build output files with hashes and filesystem stats'
	})
});
export type BuildCacheMetadata = s.Infer<typeof BuildCacheMetadata>;

/**
 * The components of the build cache key. A cached build is reused only when all match.
 */
export interface BuildCacheKey extends BuildCacheDependencies {
	git_commit: string | null;
	build_cache_config_hash: string;
}

/**
 * Computes the cache key components for a build.
 * This determines whether a cached build can be reused.
 *
 * @param deps - `log` is warned without a git commit, and `git` reads the commit when none is given
 * @param config - Gro config (`build_cache_config_hash` is already computed during config load)
 */
export const compute_build_cache_key = async (
	{ git, log }: BuildCacheKeyDeps,
	config: GroConfig,
	{ dependencies: deps, git_commit }: BuildCacheKeyOptions
): Promise<BuildCacheKey> => {
	const commit = git_commit !== undefined ? git_commit : await git_current_commit_hash(git);
	if (!commit) {
		log.warn('Not in a git repository or no commits yet - the build cache will not be reused');
	}
	if (deps.package_lock_hash === null) {
		log.debug(
			'No node_modules/.package-lock.json - build cache will not detect installed package changes'
		);
	}

	return {
		git_commit: commit,
		// already computed during config normalization
		build_cache_config_hash: config.build_cache_config_hash,
		gro_version: deps.gro_version,
		runtime_version: deps.runtime_version,
		package_lock_hash: deps.package_lock_hash
	};
};

/**
 * Loads build cache metadata from .gro/ directory.
 * A cache file that's read but invalid (corrupt, the wrong shape, or an old version)
 * is deleted, the file entry itself, never what a symlink there points at;
 * one that can't be read is left in place.
 * @param fs_paths - opens the project's `.gro/`
 */
export const load_build_cache_metadata = async (
	fs_paths: FsPaths
): Promise<BuildCacheMetadata | null> => {
	let contents: string | null;
	try {
		const metadata_file = await fs_paths.open_path(BUILD_CACHE_METADATA_PATH);
		contents = await metadata_file.dir.read_text_if_exists(metadata_file.path);
	} catch (error) {
		// unreadable, so there's nothing known to be invalid to delete
		if (is_fs_error(error)) return null;
		throw error;
	}
	if (contents === null) return null;

	try {
		// Validate structure against the schema
		const metadata = s.parse_or_throw(BuildCacheMetadata, JSON.parse(contents));
		// Validate version
		if (metadata.version === BUILD_CACHE_VERSION) return metadata;
	} catch {
		// corrupt JSON or the wrong shape, deleted below like an old version
	}

	// Clean up the invalid or stale cache file
	try {
		await fs_paths.remove_path(BUILD_CACHE_METADATA_PATH, { missing_ok: true });
	} catch {
		// Ignore cleanup errors
	}
	return null;
};

/**
 * Saves build cache metadata to .gro/ directory.
 * Errors are logged but don't fail the build (cache is optional).
 */
export const save_build_cache_metadata = async (
	{ fs_paths, log }: BuildCacheSaveDeps,
	metadata: BuildCacheMetadata
): Promise<void> => {
	try {
		await gro_dir_ensure(fs_paths);

		const metadata_file = await fs_paths.open_path(BUILD_CACHE_METADATA_PATH);
		await metadata_file.dir.write(metadata_file.path, JSON.stringify(metadata, null, '\t'));
	} catch (error) {
		// Cache writes are optional - log warning but don't fail the build
		log.warn(st('yellow', 'Failed to save build cache'), st('dim', `(${to_error_message(error)})`));
	}
};

// whether `path` is relative and stays in the project, lexically
const is_project_relative = (path: string): boolean => {
	if (isAbsolute(path)) return false;
	const normalized = normalize(path);
	return normalized !== '..' && !normalized.startsWith('../');
};

/**
 * Validates that a cached build is still valid by checking stats and hashing outputs.
 * Uses size as a fast negative check before expensive hashing.
 * This is comprehensive validation to catch manual tampering or corruption.
 * @param fs_paths - opens each output, relative to the project
 */
export const validate_build_cache = async (
	fs_paths: FsPaths,
	metadata: BuildCacheMetadata
): Promise<boolean> => {
	// Verify all tracked output files exist and have matching size
	// Sequential checks with early return for performance
	// each opened file is kept for the hashing below, so each path is resolved once
	const opened_outputs: Array<FsOpenedPath> = [];
	for (const output of metadata.outputs) {
		// Fast negative check: size mismatch = definitely invalid
		// This avoids expensive file reads and hashing for files that have clearly changed
		// a tampered path naming something outside the project is an invalid cache
		if (!is_project_relative(output.path)) return false;
		let info;
		try {
			const opened = await fs_paths.open_path(output.path);
			info = await opened.dir.stat(opened.path);
			opened_outputs.push(opened);
		} catch (error) {
			// unreadable is an invalid cache
			if (is_fs_error(error)) return false;
			throw error;
		}
		if (info === null || info.size !== output.size) {
			return false;
		}
	}

	// Size matches for all files - now verify content with cryptographic hashing
	// Hash files with controlled concurrency (could be 10k+ files)
	const results = await map_concurrent(metadata.outputs, 20, async (output, i) => {
		const opened = opened_outputs[i]!;
		try {
			const contents = await opened.dir.read(opened.path);
			const actual_hash = hash_blake3(contents);
			return actual_hash === output.hash;
		} catch {
			// File deleted/inaccessible between checks = cache invalid
			return false;
		}
	});
	return results.every((valid) => valid);
};

/**
 * Main function to check if the build cache is valid.
 * Returns true if the cached build can be used, false if a fresh build is needed.
 *
 * @param config - Gro config
 */
export const is_build_cache_valid = async (
	{ fs_paths, log, git }: BuildCacheDeps,
	config: GroConfig,
	options: BuildCacheKeyOptions
): Promise<boolean> => {
	// Load existing metadata
	const metadata = await load_build_cache_metadata(fs_paths);
	if (!metadata) {
		log.debug('No build cache metadata found');
		return false;
	}

	// Compute current cache key
	const current = await compute_build_cache_key({ git, log }, config, options);

	// with no commit nothing ties the outputs to the source, so they can't be reused
	if (current.git_commit === null) {
		log.debug('Build cache invalid: no git commit');
		return false;
	}

	// Check if cache keys have changed
	if (metadata.git_commit !== current.git_commit) {
		log.debug('Build cache invalid: git commit changed');
		return false;
	}

	if (metadata.build_cache_config_hash !== current.build_cache_config_hash) {
		log.debug('Build cache invalid: build_cache_config changed');
		return false;
	}

	if (metadata.gro_version !== current.gro_version) {
		log.debug('Build cache invalid: Gro version changed');
		return false;
	}

	if (metadata.runtime_version !== current.runtime_version) {
		log.debug('Build cache invalid: runtime version changed');
		return false;
	}

	if (metadata.package_lock_hash !== current.package_lock_hash) {
		log.debug('Build cache invalid: installed packages changed');
		return false;
	}

	// Comprehensive validation: verify output files
	const outputs_valid = await validate_build_cache(fs_paths, metadata);
	if (!outputs_valid) {
		log.debug('Build cache invalid: output files missing or corrupted');
		return false;
	}

	log.info(st('green', 'Build cache valid'), st('dim', `(from ${metadata.timestamp})`));
	return true;
};

/**
 * Collects information about all files in build output directories.
 * Returns an array of entries with path, hash, size, mtime, and mode.
 * Each directory is opened once, its symlinks followed, and walked beneath its handle,
 * where only regular files are collected, never a symlink.
 *
 * Files are hashed in parallel for performance. For very large builds (10k+ files),
 * this may take several seconds but ensures complete cache validation.
 *
 * @param fs_paths - opens each directory, relative to the project
 * @param build_dirs - array of output directories to scan (e.g., ['build', 'dist'] and any `dist_*`)
 */
export const collect_build_outputs = async (
	fs_paths: FsPaths,
	build_dirs: Array<string>
): Promise<Array<BuildOutputEntry>> => {
	// Collect all files to hash first
	interface FileEntry {
		dir: FsDir;
		// beneath `dir`
		path: string;
		cache_key: string;
	}

	const files_to_hash: Array<FileEntry> = [];

	// Recursively collect files
	const collect_files = async (
		dir: FsDir,
		dir_path: string,
		relative_base: string,
		dir_prefix: string
	): Promise<void> => {
		const list_path = relative_base ? join(dir_path, relative_base) : dir_path;
		const entries: Array<DirEntry> = [];
		for await (const entry of dir.list(list_path)) entries.push(entry);
		// sorted, since a listing's order is volatile
		entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
		for (const entry of entries) {
			// Skip metadata file itself
			if (entry.name === BUILD_CACHE_METADATA_FILENAME) {
				continue;
			}

			const relative_path = relative_base ? join(relative_base, entry.name) : entry.name;
			const path = dir_path ? join(dir_path, relative_path) : relative_path;
			const cache_key = join(dir_prefix, relative_path);

			if (entry.kind === 'directory') {
				await collect_files(dir, dir_path, relative_path, dir_prefix);
			} else if (entry.kind === 'file') {
				files_to_hash.push({ dir, path, cache_key });
			}
			// Symlinks are intentionally ignored - we only hash regular files
		}
	};

	// Collect files from all build directories sequentially
	for (const build_dir of build_dirs) {
		const opened = await fs_paths.open_path(build_dir);
		if (!(await opened.dir.exists(opened.path))) {
			continue; // Skip non-existent directories
		}

		await collect_files(opened.dir, opened.path, '', build_dir);
	}

	// Hash files with controlled concurrency and collect stats (could be 10k+ files)
	return map_concurrent(
		files_to_hash,
		20,
		async ({ dir, path, cache_key }): Promise<BuildOutputEntry> => {
			const info = await dir.stat(path);
			// a file removed since the listing fails here with `not_found`
			const contents = await dir.read(path);
			// read but not found by the stat before it, so replaced between the two
			if (info === null) {
				const target = join(dir.path, path);
				throw new FsError(
					'io_error',
					target,
					`io_error: ${target} was replaced while it was hashed`
				);
			}
			const hash = hash_blake3(contents);

			return {
				path: cache_key,
				hash,
				size: info.size,
				mtime: info.mtime_ms,
				mode: info.mode
			};
		}
	);
};

/**
 * Discovers all build output directories in the project.
 * Returns an array of directory names that exist: build/, dist/, dist_*
 */
export const discover_build_output_dirs = async (fs_paths: FsPaths): Promise<Array<string>> => {
	const build_dirs: Array<string> = [];

	// Check for SvelteKit app output (build/) and library output (dist/) in parallel
	const [build_exists, dist_exists] = await Promise.all(
		[SVELTEKIT_BUILD_DIRNAME, SVELTEKIT_DIST_DIRNAME].map(async (dirname) => {
			const opened = await fs_paths.open_path(dirname);
			return opened.dir.exists(opened.path);
		})
	);

	if (build_exists) {
		build_dirs.push(SVELTEKIT_BUILD_DIRNAME);
	}
	if (dist_exists) {
		build_dirs.push(SVELTEKIT_DIST_DIRNAME);
	}

	build_dirs.push(...(await discover_dist_dirs(fs_paths)));

	return build_dirs;
};

/**
 * Discovers the `dist_*` plugin output directories in the project.
 */
export const discover_dist_dirs = async (fs_paths: FsPaths): Promise<Array<string>> => {
	const root = await fs_paths.open_path('.');
	const candidates: Array<Promise<string | null>> = [];
	for await (const { name, kind } of root.dir.list(root.path)) {
		if (!name.startsWith(GRO_DIST_PREFIX)) continue;
		if (kind === 'directory') {
			candidates.push(Promise.resolve(name));
		} else if (kind === 'symlink') {
			// a symlink is followed wherever it points, as Gro follows its own paths
			candidates.push(
				(async () => {
					try {
						const opened = await fs_paths.open_path(name);
						const info = await opened.dir.stat(opened.path);
						return info?.kind === 'directory' ? name : null;
					} catch (error) {
						// unreadable, or deleted or moved during iteration
						if (is_fs_error(error)) return null;
						throw error;
					}
				})()
			);
		}
	}
	const dist_dirs = await Promise.all(candidates);
	// sorted, since a listing's order is volatile
	return dist_dirs.filter((p): p is string => p !== null).sort();
};

/**
 * Creates build cache metadata after a successful build.
 * Automatically discovers all build output directories (build/, dist/, dist_*).
 *
 * @param config - Gro config
 */
export const create_build_cache_metadata = async (
	{ fs_paths, log, git }: BuildCacheDeps,
	config: GroConfig,
	{ build_dirs, ...key_options }: BuildCacheMetadataOptions
): Promise<BuildCacheMetadata> => {
	const cache_key = await compute_build_cache_key({ git, log }, config, key_options);
	const dirs = build_dirs ?? (await discover_build_output_dirs(fs_paths));
	const outputs = await collect_build_outputs(fs_paths, dirs);

	return {
		version: BUILD_CACHE_VERSION,
		...cache_key,
		timestamp: new Date().toISOString(),
		outputs
	};
};
