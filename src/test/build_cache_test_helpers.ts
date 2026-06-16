import type {BuildCacheMetadata, BuildOutputEntry} from '$lib/build_cache.ts';

export {
	create_mock_logger,
	create_mock_config,
	mock_file_stats,
	mock_dir_stats,
	mock_file_entry,
	mock_dir_entry,
} from './test_helpers.ts';

/**
 * Creates mock build cache metadata for testing.
 */
export const create_mock_build_cache_metadata = (
	overrides: Partial<BuildCacheMetadata> = {},
): BuildCacheMetadata => ({
	version: '1',
	git_commit: 'abc123',
	build_cache_config_hash: 'jkl012',
	timestamp: '2025-10-21T10:00:00.000Z',
	outputs: [],
	...overrides,
});

/**
 * Creates a mock build output entry for testing.
 */
export const create_mock_output_entry = (
	path = 'build/index.html',
	overrides: Partial<BuildOutputEntry> = {},
): BuildOutputEntry => ({
	path,
	hash: 'hash123',
	size: 1024,
	mtime: 1729512000000,
	ctime: 1729512000000,
	mode: 33188,
	...overrides,
});
