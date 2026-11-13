import {
	BUILD_CACHE_VERSION,
	type BuildCacheMetadata,
	type BuildOutputEntry
} from '#lib/build_cache.ts';

import { MOCK_BUILD_CACHE_DEPENDENCIES } from './build_cache_dependencies_mocks.ts';

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
	mode: 0o644,
	...overrides
});
