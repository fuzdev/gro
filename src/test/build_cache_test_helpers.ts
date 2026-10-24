import {
	BUILD_CACHE_VERSION,
	type BuildCacheMetadata,
	type BuildOutputEntry
} from '#lib/build_cache.ts';
import type { BuildCacheDependencies } from '#lib/build_cache_dependencies.ts';

export {
	create_mock_logger,
	create_mock_config,
	mock_file_stats,
	mock_dir_stats,
	mock_file_entry,
	mock_dir_entry
} from './test_helpers.ts';

/**
 * The dependency key components that test files mocking `build_cache_dependencies.ts` return.
 * Keep in sync with the literal in each `vi.mock` factory, which can't reference imports.
 */
export const MOCK_BUILD_CACHE_DEPENDENCIES: BuildCacheDependencies = {
	gro_version: '1.0.0',
	node_version: 'v24.0.0',
	package_lock_hash: 'lock123'
};

/**
 * Creates mock build cache metadata for testing.
 */
export const create_mock_build_cache_metadata = (
	overrides: Partial<BuildCacheMetadata> = {}
): BuildCacheMetadata => ({
	version: BUILD_CACHE_VERSION,
	git_commit: 'abc123',
	build_cache_config_hash: 'jkl012',
	...MOCK_BUILD_CACHE_DEPENDENCIES,
	timestamp: '2025-10-21T10:00:00.000Z',
	outputs: [],
	...overrides
});

/**
 * Creates a mock build output entry for testing.
 */
export const create_mock_output_entry = (
	path = 'build/index.html',
	overrides: Partial<BuildOutputEntry> = {}
): BuildOutputEntry => ({
	path,
	hash: 'hash123',
	size: 1024,
	mtime: 1729512000000,
	ctime: 1729512000000,
	mode: 33188,
	...overrides
});
