import {mkdir, readdir, readFile, rm, stat, writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {styleText as st} from 'node:util';
import {z} from 'zod';
import type {Logger} from '@fuzdev/fuz_util/log.js';
import {git_current_commit_hash} from '@fuzdev/fuz_util/git.js';
import {fs_exists} from '@fuzdev/fuz_util/fs.js';
import {map_concurrent} from '@fuzdev/fuz_util/async.js';
import {hash_blake3} from '@fuzdev/fuz_util/hash_blake3.js';

import type {GroConfig} from './gro_config.ts';
import {paths} from './paths.ts';
import {SVELTEKIT_BUILD_DIRNAME, SVELTEKIT_DIST_DIRNAME, GRO_DIST_PREFIX} from './constants.ts';

export const BUILD_CACHE_METADATA_FILENAME = 'build.json';
export const BUILD_CACHE_VERSION = '1';

/**
 * Metadata about a single build output file.
 * Includes cryptographic hash for validation plus filesystem stats for debugging and optimization.
 */
export const BuildOutputEntry = z.strictObject({
	path: z
		.string()
		.meta({description: "relative path from project root (e.g., 'build/index.html')."}),
	hash: z.string().meta({description: 'BLAKE3 hash of file contents'}),
	size: z.number().meta({description: 'file size in bytes'}),
	mtime: z.number().meta({description: 'modification time in milliseconds since epoch'}),
	ctime: z.number().meta({
		description: 'POSIX change time in milliseconds since epoch',
	}),
	mode: z.number().meta({description: 'unix file permission mode (e.g., 33188 = 0644)'}),
});
export type BuildOutputEntry = z.infer<typeof BuildOutputEntry>;

/**
 * Metadata stored in .gro/ directory to track build cache validity.
 * Schema validates structure at load time to catch corrupted cache files.
 */
export const BuildCacheMetadata = z.strictObject({
	version: z.string().meta({description: 'schema version for future compatibility'}),
	git_commit: z.string().nullable().meta({description: 'git commit hash at time of build'}),
	build_cache_config_hash: z
		.string()
		.meta({description: "hash of user's custom build_cache_config from gro.config.ts."}),
	timestamp: z.string().meta({description: 'timestamp when build completed'}),
	outputs: z
		.array(BuildOutputEntry)
		.meta({description: 'build output files with hashes and filesystem stats'}),
});
export type BuildCacheMetadata = z.infer<typeof BuildCacheMetadata>;

/**
 * Computes the cache key components for a build.
 * This determines whether a cached build can be reused.
 *
 * @param config - Gro config (build_cache_config_hash is already computed during config load)
 * @param log - Logger
 * @param git_commit - optional pre-computed git commit hash (optimization to avoid re-reading)
 */
export const compute_build_cache_key = async (
	config: GroConfig,
	log: Logger,
	git_commit?: string | null,
): Promise<{
	git_commit: string | null;
	build_cache_config_hash: string;
}> => {
	// 1. Git commit hash - primary cache key
	const commit = git_commit !== undefined ? git_commit : await git_current_commit_hash();
	if (!commit) {
		log.warn('Not in a git repository - build cache will use null git commit');
	}

	// 2. Build cache config hash - already computed during config normalization
	return {
		git_commit: commit,
		build_cache_config_hash: config.build_cache_config_hash,
	};
};

/**
 * Loads build cache metadata from .gro/ directory.
 * Invalid or corrupted cache files are automatically deleted.
 */
export const load_build_cache_metadata = async (): Promise<BuildCacheMetadata | null> => {
	const metadata_path = join(paths.build, BUILD_CACHE_METADATA_FILENAME);

	if (!(await fs_exists(metadata_path))) {
		return null;
	}

	try {
		const contents = await readFile(metadata_path, 'utf-8');
		const parsed = JSON.parse(contents);

		// Validate structure with Zod
		const metadata = BuildCacheMetadata.parse(parsed);

		// Validate version
		if (metadata.version !== BUILD_CACHE_VERSION) {
			// Clean up stale cache with old schema version
			try {
				await rm(metadata_path, {force: true});
			} catch {
				// Ignore cleanup errors
			}
			return null;
		}

		return metadata;
	} catch {
		// Clean up corrupted/invalid cache file
		// (catches JSON.parse, Zod validation, and version errors)
		try {
			await rm(metadata_path, {force: true});
		} catch {
			// Ignore cleanup errors
		}
		return null;
	}
};

/**
 * Saves build cache metadata to .gro/ directory.
 * Errors are logged but don't fail the build (cache is optional).
 */
export const save_build_cache_metadata = async (
	metadata: BuildCacheMetadata,
	log?: Logger,
): Promise<void> => {
	try {
		// Ensure .gro directory exists
		await mkdir(paths.build, {recursive: true});

		const metadata_path = join(paths.build, BUILD_CACHE_METADATA_FILENAME);
		await writeFile(metadata_path, JSON.stringify(metadata, null, '\t'), 'utf-8');
	} catch (error) {
		// Cache writes are optional - log warning but don't fail the build
		log?.warn(
			st('yellow', 'Failed to save build cache'),
			st('dim', `(${error instanceof Error ? error.message : String(error)})`),
		);
	}
};

/**
 * Validates that a cached build is still valid by checking stats and hashing outputs.
 * Uses size as a fast negative check before expensive hashing.
 * This is comprehensive validation to catch manual tampering or corruption.
 */
export const validate_build_cache = async (metadata: BuildCacheMetadata): Promise<boolean> => {
	// Verify all tracked output files exist and have matching size
	// Sequential checks with early return for performance
	for (const output of metadata.outputs) {
		// eslint-disable-next-line no-await-in-loop
		if (!(await fs_exists(output.path))) {
			return false;
		}

		// Fast negative check: size mismatch = definitely invalid
		// This avoids expensive file reads and hashing for files that have clearly changed
		// eslint-disable-next-line no-await-in-loop
		const stats = await stat(output.path);
		if (stats.size !== output.size) {
			return false;
		}
	}

	// Size matches for all files - now verify content with cryptographic hashing
	// Hash files with controlled concurrency (could be 10k+ files)
	const results = await map_concurrent(metadata.outputs, 20, async (output) => {
		try {
			const contents = await readFile(output.path);
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
 * @param log - Logger
 * @param git_commit - optional pre-computed git commit hash (optimization)
 */
export const is_build_cache_valid = async (
	config: GroConfig,
	log: Logger,
	git_commit?: string | null,
): Promise<boolean> => {
	// Load existing metadata
	const metadata = await load_build_cache_metadata();
	if (!metadata) {
		log.debug('No build cache metadata found');
		return false;
	}

	// Compute current cache key
	const current = await compute_build_cache_key(config, log, git_commit);

	// Check if cache keys have changed
	if (metadata.git_commit !== current.git_commit) {
		log.debug('Build cache invalid: git commit changed');
		return false;
	}

	if (metadata.build_cache_config_hash !== current.build_cache_config_hash) {
		log.debug('Build cache invalid: build_cache_config changed');
		return false;
	}

	// Comprehensive validation: verify output files
	const outputs_valid = await validate_build_cache(metadata);
	if (!outputs_valid) {
		log.debug('Build cache invalid: output files missing or corrupted');
		return false;
	}

	log.info(st('green', 'Build cache valid'), st('dim', `(from ${metadata.timestamp})`));
	return true;
};

/**
 * Collects information about all files in build output directories.
 * Returns an array of entries with path, hash, size, mtime, ctime, and mode.
 *
 * Files are hashed in parallel for performance. For very large builds (10k+ files),
 * this may take several seconds but ensures complete cache validation.
 *
 * @param build_dirs - array of output directories to scan (e.g., ['build', 'dist', 'dist_server'])
 */
export const collect_build_outputs = async (
	build_dirs: Array<string>,
): Promise<Array<BuildOutputEntry>> => {
	// Collect all files to hash first
	interface FileEntry {
		full_path: string;
		cache_key: string;
	}

	const files_to_hash: Array<FileEntry> = [];

	// Recursively collect files
	const collect_files = async (
		dir: string,
		relative_base: string,
		dir_prefix: string,
	): Promise<void> => {
		const entries = await readdir(dir, {withFileTypes: true});

		for (const entry of entries) {
			// Skip metadata file itself
			if (entry.name === BUILD_CACHE_METADATA_FILENAME) {
				continue;
			}

			const full_path = join(dir, entry.name);
			const relative_path = relative_base ? join(relative_base, entry.name) : entry.name;
			const cache_key = join(dir_prefix, relative_path);

			if (entry.isDirectory()) {
				// eslint-disable-next-line no-await-in-loop
				await collect_files(full_path, relative_path, dir_prefix);
			} else if (entry.isFile()) {
				files_to_hash.push({full_path, cache_key});
			}
			// Symlinks are intentionally ignored - we only hash regular files
		}
	};

	// Collect files from all build directories sequentially
	for (const build_dir of build_dirs) {
		// eslint-disable-next-line no-await-in-loop
		if (!(await fs_exists(build_dir))) {
			continue; // Skip non-existent directories
		}
		// eslint-disable-next-line no-await-in-loop
		await collect_files(build_dir, '', build_dir);
	}

	// Hash files with controlled concurrency and collect stats (could be 10k+ files)
	return map_concurrent(
		files_to_hash,
		20,
		async ({full_path, cache_key}): Promise<BuildOutputEntry> => {
			const stats = await stat(full_path);
			const contents = await readFile(full_path);
			const hash = hash_blake3(contents);

			return {
				path: cache_key,
				hash,
				size: stats.size,
				mtime: stats.mtimeMs,
				ctime: stats.ctimeMs,
				mode: stats.mode,
			};
		},
	);
};

/**
 * Discovers all build output directories in the current working directory.
 * Returns an array of directory names that exist: build/, dist/, dist_*
 */
export const discover_build_output_dirs = async (): Promise<Array<string>> => {
	const build_dirs: Array<string> = [];

	// Check for SvelteKit app output (build/) and library output (dist/) in parallel
	const [build_exists, dist_exists] = await Promise.all([
		fs_exists(SVELTEKIT_BUILD_DIRNAME),
		fs_exists(SVELTEKIT_DIST_DIRNAME),
	]);

	if (build_exists) {
		build_dirs.push(SVELTEKIT_BUILD_DIRNAME);
	}
	if (dist_exists) {
		build_dirs.push(SVELTEKIT_DIST_DIRNAME);
	}

	// Check for server and other plugin outputs (dist_*)
	const root_entries = await readdir('.');
	const dist_dir_checks = await Promise.all(
		root_entries
			.filter((p) => p.startsWith(GRO_DIST_PREFIX))
			.map(async (p) => {
				try {
					const s = await stat(p);
					return s.isDirectory() ? p : null;
				} catch {
					// File was deleted/moved during iteration - skip it
					return null;
				}
			}),
	);
	build_dirs.push(...dist_dir_checks.filter((p): p is string => p !== null));

	return build_dirs;
};

/**
 * Creates build cache metadata after a successful build.
 * Automatically discovers all build output directories (build/, dist/, dist_*).
 *
 * @param config - Gro config
 * @param log - Logger
 * @param git_commit - optional pre-computed git commit hash (optimization)
 * @param build_dirs - optional pre-discovered build directories (optimization to avoid redundant filesystem scans)
 */
export const create_build_cache_metadata = async (
	config: GroConfig,
	log: Logger,
	git_commit?: string | null,
	build_dirs?: Array<string>,
): Promise<BuildCacheMetadata> => {
	const cache_key = await compute_build_cache_key(config, log, git_commit);
	const dirs = build_dirs ?? (await discover_build_output_dirs());
	const outputs = await collect_build_outputs(dirs);

	return {
		version: BUILD_CACHE_VERSION,
		...cache_key,
		timestamp: new Date().toISOString(),
		outputs,
	};
};
