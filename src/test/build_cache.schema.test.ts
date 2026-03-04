import {describe, test, expect} from 'vitest';

import {BuildCacheMetadata, BuildOutputEntry} from '../lib/build_cache.ts';

import {
	create_mock_build_cache_metadata,
	create_mock_output_entry,
} from './build_cache_test_helpers.ts';

describe('BuildOutputEntry schema', () => {
	test('validates correct output entry', () => {
		expect(() => BuildOutputEntry.parse(create_mock_output_entry())).not.toThrow();
	});

	test('rejects entry with missing hash', () => {
		expect(() =>
			BuildOutputEntry.parse({
				path: 'build/file.js',
				size: 1024,
				mtime: 1729512000000,
				ctime: 1729512000000,
				mode: 33188,
			}),
		).toThrow();
	});

	test('rejects entry with wrong size type', () => {
		expect(() =>
			BuildOutputEntry.parse({
				path: 'build/file.js',
				hash: 'abc',
				size: '1024', // should be number
				mtime: 1729512000000,
				ctime: 1729512000000,
				mode: 33188,
			}),
		).toThrow();
	});

	test('rejects entry with extra fields', () => {
		expect(() =>
			BuildOutputEntry.parse({
				...create_mock_output_entry(),
				extra: 'bad',
			}),
		).toThrow();
	});
});

describe('BuildCacheMetadata schema', () => {
	test('validates correct metadata structure', () => {
		const metadata = create_mock_build_cache_metadata({
			outputs: [create_mock_output_entry('file.js')],
		});
		expect(() => BuildCacheMetadata.parse(metadata)).not.toThrow();
	});

	test('rejects metadata with missing fields', () => {
		expect(() =>
			BuildCacheMetadata.parse({
				version: '1',
				git_commit: 'abc123',
				// missing build_cache_config_hash
				timestamp: '2025-10-23T12:00:00Z',
				outputs: [],
			}),
		).toThrow();
	});

	test('rejects metadata with wrong types', () => {
		expect(() =>
			BuildCacheMetadata.parse({
				version: 1, // should be string
				git_commit: 'abc123',
				build_cache_config_hash: 'hash',
				timestamp: '2025-10-23T12:00:00Z',
				outputs: [],
			}),
		).toThrow();
	});

	test('rejects metadata with unexpected extra fields', () => {
		expect(() =>
			BuildCacheMetadata.parse({
				version: '1',
				git_commit: 'abc',
				build_cache_config_hash: 'hash',
				timestamp: '2025-10-23T12:00:00Z',
				outputs: [],
				unexpected_field: 'bad',
			}),
		).toThrow();
	});
});
